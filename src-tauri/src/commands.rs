use crate::model::*;
use crate::{crypto, net, sources};
use rand::Rng;
use serde::Serialize;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};

/// Run blocking net/disk work off Tauri's main thread. Sync commands run ON
/// the main thread in Tauri v2 — a command that blocks (accept loop, transfer,
/// folder-size scan, UAC wait) freezes the whole window ("Not Responding") and
/// stalls event delivery to the UI, so every long-running command must hop
/// through here.
async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

/// Shared session state, managed by Tauri.
#[derive(Default)]
pub struct Session {
    pub cancel: Arc<AtomicBool>,
    pub code: Mutex<Option<String>>, // 6 digits, no spaces
    pub beacon_stop: Mutex<Option<Arc<AtomicBool>>>,
    pub listener: Mutex<Option<TcpListener>>,
    pub send_stream: Mutex<Option<crypto::EncryptedStream>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiverInfo {
    pub code: String, // display form "418 620"
    pub peer: Peer,
}

fn emitter(app: &AppHandle) -> impl Fn(TransferProgress) + '_ {
    move |p: TransferProgress| {
        let _ = app.emit("winc://progress", p);
    }
}

/// The UI polls link status every 1.5 s, so only *changes* are logged —
/// otherwise the view is nothing but link lines.
static LAST_LINK: Mutex<Option<String>> = Mutex::new(None);

#[tauri::command]
pub fn get_link_status() -> LinkStatus {
    let s = net::link_status();
    let sig = format!(
        "up={} adapter={:?} ip={:?} kind={:?}",
        s.up, s.adapter, s.local_ip, s.kind
    );
    let mut last = LAST_LINK.lock().unwrap();
    if last.as_deref() != Some(sig.as_str()) {
        log::info!("link changed: {sig}");
        *last = Some(sig);
    }
    s
}

/// All network adapters this PC sees, classified — for the connect-screen diagnostics.
#[tauri::command]
pub fn list_adapters() -> Vec<AdapterInfo> {
    let v = net::list_adapters();
    log::info!("adapters: {} found", v.len());
    for a in &v {
        log::debug!(
            "  adapter '{}' ip={} linkLocal={} cable={} kind={}",
            a.name,
            if a.ip.is_empty() { "(none yet)" } else { &a.ip },
            a.link_local,
            a.cable,
            a.kind
        );
    }
    v
}

#[tauri::command]
pub fn start_receiver(name: String, state: State<'_, Session>) -> Result<ReceiverInfo, String> {
    // clear any leftover state from a previous "Start over"
    state.cancel.store(false, Ordering::Relaxed);
    if let Some(stop) = state.beacon_stop.lock().unwrap().take() {
        stop.store(true, Ordering::Relaxed);
    }
    *state.listener.lock().unwrap() = None;

    // fresh pairing code
    let n: u32 = rand::thread_rng().gen_range(0..1_000_000);
    let digits = format!("{:06}", n);
    let display = format!("{} {}", &digits[..3], &digits[3..]);
    *state.code.lock().unwrap() = Some(digits);

    // listener the sender will connect to
    let (listener, port) = net::bind_listener().map_err(|e| {
        log::error!("start_receiver: could not bind a listener: {e}");
        e.to_string()
    })?;
    if port != TRANSFER_PORT {
        log::warn!(
            "port {TRANSFER_PORT} was taken — listening on {port} instead; manual IP pairing will need the port"
        );
    }
    *state.listener.lock().unwrap() = Some(listener);

    // broadcast beacon until stopped
    let stop = Arc::new(AtomicBool::new(false));
    *state.beacon_stop.lock().unwrap() = Some(stop.clone());
    let bname = net::host_name(&name);
    std::thread::spawn(move || net::run_beacon(bname, port, stop));

    let peer = Peer {
        name: net::host_name(&name),
        ip: net::advertised_ip(),
        port,
    };
    log::info!(
        "receiver ready as '{}' on {}:{} — pairing code issued",
        peer.name,
        peer.ip,
        peer.port
    );
    Ok(ReceiverInfo {
        code: display,
        peer,
    })
}

#[tauri::command]
pub async fn discover_peer() -> Result<Option<Peer>, String> {
    blocking(|| {
        log::info!("discovery: listening for a receiver beacon (10s)");
        let found = net::discover(Duration::from_secs(10));
        match &found {
            Some(p) => log::info!("discovery: found '{}' at {}:{}", p.name, p.ip, p.port),
            None => log::warn!(
                "discovery: no beacon in 10s — cable down, receiver not started, or inbound UDP {DISCOVERY_PORT} blocked by the firewall"
            ),
        }
        Ok(found)
    })
    .await
}

/// All IPv4 addresses on this PC (direct-cable link first), for manual pairing.
#[tauri::command]
pub fn list_local_ips() -> Vec<String> {
    let ips = net::local_ipv4s();
    log::info!("local IPv4s (cable first): {}", ips.join(", "));
    ips
}

#[tauri::command]
pub async fn pair(peer: Peer, code: String, app: AppHandle) -> Result<(), String> {
    blocking(move || {
        let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
        let name = net::host_name("This PC");
        log::info!("pairing with {}:{} as '{}'", peer.ip, peer.port, name);
        let stream = net::connect_and_pair(&peer, &name, &digits).map_err(|e| {
            log::error!("pairing with {}:{} failed: {e}", peer.ip, peer.port);
            e.to_string()
        })?;
        log::info!("paired with {}:{} — encrypted channel open", peer.ip, peer.port);
        let state = app.state::<Session>();
        *state.send_stream.lock().unwrap() = Some(stream);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn list_sources() -> Result<Vec<SourceGroup>, String> {
    // measures folder sizes on disk — can take seconds on a big Documents
    blocking(|| {
        log::info!("scanning sources (measuring folder sizes)…");
        let t = std::time::Instant::now();
        let groups = sources::list_sources();
        let bytes: u64 = groups.iter().map(|g| g.bytes).sum();
        log::info!(
            "sources: {} groups, {} files, {:.1} GB — scanned in {:.1}s",
            groups.len(),
            groups.iter().map(|g| g.items).sum::<u64>(),
            bytes as f64 / 1e9,
            t.elapsed().as_secs_f64()
        );
        Ok(groups)
    })
    .await
}

#[tauri::command]
pub async fn start_send(group_ids: Vec<String>, app: AppHandle) -> Result<(), String> {
    blocking(move || {
        let state = app.state::<Session>();
        state.cancel.store(false, Ordering::Relaxed);
        let mut stream = state.send_stream.lock().unwrap().take().ok_or_else(|| {
            log::error!("start_send called with no paired connection");
            "not paired — pair with the new PC first"
        })?;

        log::info!("send starting — groups: {}", group_ids.join(", "));
        let mut items = Vec::new();
        for id in &group_ids {
            let expanded = sources::expand(id);
            log::info!("  group '{id}' expands to {} file(s)", expanded.len());
            items.extend(expanded);
        }

        let cancel = state.cancel.clone();
        let emit = emitter(&app);
        let t = std::time::Instant::now();
        let out = net::send_files(&mut stream, &items, &cancel, emit).map_err(|e| {
            log::error!("send failed: {e}");
            let _ = app.emit("winc://progress", TransferProgress::error(&e.to_string()));
            e.to_string()
        });
        if out.is_ok() {
            log::info!("send finished in {:.1}s", t.elapsed().as_secs_f64());
        }
        out
    })
    .await
}

/// Returns the folder the crossing landed in, for "Import into place".
#[tauri::command]
pub async fn receive(app: AppHandle) -> Result<String, String> {
    blocking(move || {
        let state = app.state::<Session>();
        state.cancel.store(false, Ordering::Relaxed);
        let listener = state.listener.lock().unwrap().take().ok_or_else(|| {
            log::error!("receive called before start_receiver");
            "receiver not started"
        })?;
        let code = state.code.lock().unwrap().clone().unwrap_or_default();
        let cancel = state.cancel.clone();

        log::info!("waiting for a sender to connect…");
        let (mut stream, peer) =
            net::accept_and_verify(&listener, &code, &cancel).map_err(|e| {
                log::error!("accept/verify failed: {e}");
                e.to_string()
            })?;
        log::info!("sender '{}' verified from {}", peer.name, peer.ip);
        let _ = app.emit("winc://paired", &peer);

        // stop advertising once someone connected
        if let Some(stop) = state.beacon_stop.lock().unwrap().take() {
            stop.store(true, Ordering::Relaxed);
        }

        let dest = incoming_dir();
        std::fs::create_dir_all(&dest).map_err(|e| {
            log::error!("could not create {}: {e}", dest.display());
            e.to_string()
        })?;
        log::info!("receiving into {}", dest.display());

        let emit = emitter(&app);
        let t = std::time::Instant::now();
        net::receive_files(&mut stream, &dest, &cancel, emit).map_err(|e| {
            log::error!("receive failed: {e}");
            let _ = app.emit("winc://progress", TransferProgress::error(&e.to_string()));
            e.to_string()
        })?;
        log::info!("receive finished in {:.1}s", t.elapsed().as_secs_f64());
        Ok(dest.display().to_string())
    })
    .await
}

/// Move a finished crossing out of Documents\WINC Received into the real user
/// folders / browser profiles. Only accepts paths under WINC Received.
#[tauri::command]
pub async fn import_received(dir: String) -> Result<crate::import::ImportReport, String> {
    blocking(move || {
        let p = std::path::PathBuf::from(&dir);
        let base = dirs::document_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("WINC Received");
        if !p.starts_with(&base) || !p.is_dir() {
            log::error!("refused import from outside WINC Received: {}", p.display());
            return Err("not a WINC received folder".into());
        }
        log::info!("import into place from {}", p.display());
        let report = crate::import::import_received(&p);
        for e in &report.entries {
            log::info!(
                "import: {} → {} ({} file(s)){}",
                e.label,
                e.action,
                e.count,
                e.detail.as_deref().map(|d| format!(" — {d}")).unwrap_or_default()
            );
        }
        Ok(report)
    })
    .await
}

/// "Overwrite?" for one browser whose import was skipped as not-fresh.
/// Backs the new PC's existing files up to <dump>\Backup\<Label>\ first.
#[tauri::command]
pub async fn import_browser_overwrite(
    dir: String,
    label: String,
) -> Result<crate::import::ImportEntry, String> {
    blocking(move || {
        let p = std::path::PathBuf::from(&dir);
        let base = dirs::document_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("WINC Received");
        if !p.starts_with(&base) || !p.is_dir() {
            log::error!("refused import from outside WINC Received: {}", p.display());
            return Err("not a WINC received folder".into());
        }
        log::warn!("overwriting {label}'s data on this PC (originals backed up first)");
        let out = crate::import::overwrite_browser(&p, &label);
        match &out {
            Ok(e) => log::info!(
                "overwrite {label}: {} ({} file(s)){}",
                e.action,
                e.count,
                e.detail.as_deref().map(|d| format!(" — {d}")).unwrap_or_default()
            ),
            Err(e) => log::error!("overwrite {label} failed: {e}"),
        }
        out
    })
    .await
}

/// "Open First" re-check for one browser whose import was skipped as
/// not-installed. Re-runs the non-force import so a newly-opened browser
/// profile is picked up and routed to import / not-fresh as applicable.
#[tauri::command]
pub async fn import_browser_retry(
    dir: String,
    label: String,
) -> Result<crate::import::ImportEntry, String> {
    blocking(move || {
        let p = std::path::PathBuf::from(&dir);
        let base = dirs::document_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("WINC Received");
        if !p.starts_with(&base) || !p.is_dir() {
            log::error!("refused import from outside WINC Received: {}", p.display());
            return Err("not a WINC received folder".into());
        }
        log::info!("re-checking {label} after 'Open First'");
        let out = crate::import::retry_browser(&p, &label);
        match &out {
            Ok(e) => log::info!("retry {label}: {} ({} file(s))", e.action, e.count),
            Err(e) => log::error!("retry {label} failed: {e}"),
        }
        out
    })
    .await
}

/// Add a Windows Firewall allow-rule for this exe on ALL profiles. The direct
/// cable comes up as an "Unidentified network", which Windows puts on the
/// Public profile — where inbound is blocked by default and the standard
/// firewall prompt (Private-only) doesn't help. That silently kills discovery
/// (inbound UDP 50737) and the receiver's listener (inbound TCP 50738).
/// Elevates via UAC; returns Err if the user declines.
#[tauri::command]
pub async fn allow_firewall() -> Result<(), String> {
    #[cfg(windows)]
    return blocking(|| {
        log::info!("requesting elevated firewall rule (UAC prompt)…");
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.display().to_string();
        // one elevated cmd: replace any old rule, then allow this program on
        // every profile (public included) for both TCP and UDP
        let cmdline = format!(
            "/c netsh advfirewall firewall delete rule name=\"WINC Data Crossing\" & \
             netsh advfirewall firewall add rule name=\"WINC Data Crossing\" \
             dir=in action=allow enable=yes profile=any protocol=any program=\"{exe}\""
        );
        let ps = format!(
            "Start-Process cmd -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '{}'",
            cmdline.replace('\'', "''")
        );
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            log::error!("firewall rule not added — admin prompt declined or netsh failed");
            return Err("Firewall rule was not added (admin prompt declined?)".into());
        }
        log::info!("firewall rule 'WINC Data Crossing' added for all profiles");
        Ok(())
    })
    .await;
    #[cfg(not(windows))]
    return Err("Only needed on Windows".into());
}

/// Stop whatever's in flight: unblocks the receiver's accept loop, ends any
/// transfer, and stops the discovery beacon. Called on "Start over".
#[tauri::command]
pub fn cancel(state: State<'_, Session>) {
    log::warn!("cancel — stopping beacon, accept loop and any transfer in flight");
    state.cancel.store(true, Ordering::Relaxed);
    if let Some(stop) = state.beacon_stop.lock().unwrap().take() {
        stop.store(true, Ordering::Relaxed);
    }
    *state.send_stream.lock().unwrap() = None;
}

fn incoming_dir() -> std::path::PathBuf {
    let base = dirs::document_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    base.join("WINC Received").join(format!("crossing-{ts}"))
}
