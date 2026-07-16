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

#[tauri::command]
pub fn get_link_status() -> LinkStatus {
    net::link_status()
}

/// All network adapters this PC sees, classified — for the connect-screen diagnostics.
#[tauri::command]
pub fn list_adapters() -> Vec<AdapterInfo> {
    net::list_adapters()
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
    let (listener, port) = net::bind_listener().map_err(|e| e.to_string())?;
    *state.listener.lock().unwrap() = Some(listener);

    // broadcast beacon until stopped
    let stop = Arc::new(AtomicBool::new(false));
    *state.beacon_stop.lock().unwrap() = Some(stop.clone());
    let bname = net::host_name(&name);
    std::thread::spawn(move || net::run_beacon(bname, port, stop));

    Ok(ReceiverInfo {
        code: display,
        peer: Peer {
            name: net::host_name(&name),
            ip: net::advertised_ip(),
            port,
        },
    })
}

#[tauri::command]
pub async fn discover_peer() -> Result<Option<Peer>, String> {
    blocking(|| Ok(net::discover(Duration::from_secs(10)))).await
}

/// All IPv4 addresses on this PC (direct-cable link first), for manual pairing.
#[tauri::command]
pub fn list_local_ips() -> Vec<String> {
    net::local_ipv4s()
}

#[tauri::command]
pub async fn pair(peer: Peer, code: String, app: AppHandle) -> Result<(), String> {
    blocking(move || {
        let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
        let name = net::host_name("This PC");
        let stream = net::connect_and_pair(&peer, &name, &digits).map_err(|e| e.to_string())?;
        let state = app.state::<Session>();
        *state.send_stream.lock().unwrap() = Some(stream);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn list_sources() -> Result<Vec<SourceGroup>, String> {
    // measures folder sizes on disk — can take seconds on a big Documents
    blocking(|| Ok(sources::list_sources())).await
}

#[tauri::command]
pub async fn start_send(group_ids: Vec<String>, app: AppHandle) -> Result<(), String> {
    blocking(move || {
        let state = app.state::<Session>();
        state.cancel.store(false, Ordering::Relaxed);
        let mut stream = state
            .send_stream
            .lock()
            .unwrap()
            .take()
            .ok_or("not paired — pair with the new PC first")?;

        let mut items = Vec::new();
        for id in &group_ids {
            items.extend(sources::expand(id));
        }

        let cancel = state.cancel.clone();
        let emit = emitter(&app);
        net::send_files(&mut stream, &items, &cancel, emit).map_err(|e| {
            let _ = app.emit("winc://progress", TransferProgress::error(&e.to_string()));
            e.to_string()
        })
    })
    .await
}

/// Returns the folder the crossing landed in, for "Import into place".
#[tauri::command]
pub async fn receive(app: AppHandle) -> Result<String, String> {
    blocking(move || {
        let state = app.state::<Session>();
        state.cancel.store(false, Ordering::Relaxed);
        let listener = state
            .listener
            .lock()
            .unwrap()
            .take()
            .ok_or("receiver not started")?;
        let code = state.code.lock().unwrap().clone().unwrap_or_default();
        let cancel = state.cancel.clone();

        let (mut stream, peer) =
            net::accept_and_verify(&listener, &code, &cancel).map_err(|e| e.to_string())?;
        let _ = app.emit("winc://paired", &peer);

        // stop advertising once someone connected
        if let Some(stop) = state.beacon_stop.lock().unwrap().take() {
            stop.store(true, Ordering::Relaxed);
        }

        let dest = incoming_dir();
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

        let emit = emitter(&app);
        net::receive_files(&mut stream, &dest, &cancel, emit).map_err(|e| {
            let _ = app.emit("winc://progress", TransferProgress::error(&e.to_string()));
            e.to_string()
        })?;
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
            return Err("not a WINC received folder".into());
        }
        Ok(crate::import::import_received(&p))
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
            return Err("Firewall rule was not added (admin prompt declined?)".into());
        }
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
