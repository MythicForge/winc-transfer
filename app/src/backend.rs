//! Everything the UI asks the backend to do. Plain functions: the quick ones
//! the UI calls directly, the blocking ones (accept loop, transfer, folder
//! scans, UAC wait) it runs through `blocking` so the window never freezes.
//! Things the backend reports on its own (progress, a sender connecting, new
//! log lines) arrive over the `Event` channel.

use crate::model::*;
use crate::{crypto, net, settings, sources};
use iced::futures::channel::{mpsc, oneshot};
use rand::Rng;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Run blocking net/disk work on its own thread and await the result, so the
/// UI thread keeps drawing ("Not Responding" otherwise).
pub async fn blocking<T, F>(f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.await {
        Ok(v) => v,
        // the sender only drops unsent if `f` panicked — the panic hook has
        // logged it; never resolving beats taking the whole UI down with it
        Err(_) => std::future::pending().await,
    }
}

/// Pushed from backend threads to the UI.
#[derive(Debug, Clone)]
pub enum Event {
    Progress(TransferProgress),
    /// RECEIVE side: a sender connected and proved the code.
    Paired(Peer),
    /// New log lines are in the ring (coalesced — one nudge per UI re-read).
    Log,
}

type Chan = (
    mpsc::UnboundedSender<Event>,
    Mutex<Option<mpsc::UnboundedReceiver<Event>>>,
);
static CHAN: LazyLock<Chan> = LazyLock::new(|| {
    let (tx, rx) = mpsc::unbounded();
    (tx, Mutex::new(Some(rx)))
});

pub fn emit(e: Event) {
    let _ = CHAN.0.unbounded_send(e);
}

/// The UI's end of the event channel. Only the first caller gets it.
pub fn take_events() -> Option<mpsc::UnboundedReceiver<Event>> {
    CHAN.1.lock().ok()?.take()
}

/// Shared session state.
#[derive(Default)]
pub struct Session {
    pub cancel: Arc<AtomicBool>,
    pub code: Mutex<Option<String>>, // 6 digits, no spaces
    pub beacon_stop: Mutex<Option<Arc<AtomicBool>>>,
    pub listener: Mutex<Option<TcpListener>>,
    pub send_stream: Mutex<Option<crypto::EncryptedStream>>,
    /// Bumped on every pair, so a keep-alive thread from an earlier pairing
    /// knows to stop.
    pub pair_gen: AtomicU64,
    /// Who we paired with and the code — kept so a broken transfer can
    /// reconnect and resume without the user re-pairing.
    pub pair_info: Mutex<Option<(Peer, String)>>,
    /// Next unused speed-test chunk on the paired connection (nonce hygiene
    /// across repeated tests); reset on every pair.
    pub bench_next: AtomicU64,
}

static SESSION: LazyLock<Session> = LazyLock::new(Session::default);

fn session() -> &'static Session {
    &SESSION
}

#[derive(Debug, Clone)]
pub struct ReceiverInfo {
    pub code: String, // display form "418 620"
    pub peer: Peer,
}

fn emit_progress(p: TransferProgress) {
    emit(Event::Progress(p));
}

/// The UI polls link status every 1.5 s, so only *changes* are logged —
/// otherwise the view is nothing but link lines.
static LAST_LINK: Mutex<Option<String>> = Mutex::new(None);

pub fn link_status() -> LinkStatus {
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
static LAST_ADAPTERS: Mutex<Option<String>> = Mutex::new(None);

/// Polled every 2 s while on the connect step, so — like the link — only
/// changes are logged.
pub fn list_adapters() -> Vec<AdapterInfo> {
    let v = net::list_adapters();
    let sig: String = v.iter().map(|a| format!("{}={}|{};", a.name, a.ip, a.cable)).collect();
    let mut last = LAST_ADAPTERS.lock().unwrap();
    if last.as_deref() == Some(sig.as_str()) {
        return v;
    }
    *last = Some(sig);
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

pub fn start_receiver(name: &str) -> Result<ReceiverInfo, String> {
    let state = session();
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
    let bname = net::host_name(name);
    std::thread::spawn(move || net::run_beacon(bname, port, stop));

    let peer = Peer {
        name: net::host_name(name),
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

/// Blocking (~10 s).
pub fn discover_peer() -> Option<Peer> {
    log::info!("discovery: listening for a receiver beacon (10s)");
    let found = net::discover(Duration::from_secs(10));
    match &found {
        Some(p) => log::info!("discovery: found '{}' at {}:{}", p.name, p.ip, p.port),
        None => log::warn!(
            "discovery: no beacon in 10s — cable down, receiver not started, or inbound UDP {DISCOVERY_PORT} blocked by the firewall"
        ),
    }
    found
}

/// All IPv4 addresses on this PC (direct-cable link first), for manual pairing.
pub fn list_local_ips() -> Vec<String> {
    let ips = net::local_ipv4s();
    log::info!("local IPv4s (cable first): {}", ips.join(", "));
    ips
}

/// Blocking. Connects and runs the SPAKE2 handshake.
pub fn pair(peer: Peer, code: String) -> Result<(), String> {
    {
        let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
        let name = net::host_name("This PC");
        log::info!("pairing with {}:{} as '{}'", peer.ip, peer.port, name);
        let stream = net::connect_and_pair(&peer, &name, &digits).map_err(|e| {
            log::error!("pairing with {}:{} failed: {e}", peer.ip, peer.port);
            e.to_string()
        })?;
        log::info!("paired with {}:{} — encrypted channel open", peer.ip, peer.port);
        let state = session();
        *state.pair_info.lock().unwrap() = Some((peer.clone(), digits.clone()));
        *state.send_stream.lock().unwrap() = Some(stream);
        state.bench_next.store(0, Ordering::Relaxed);
        let gen = state.pair_gen.fetch_add(1, Ordering::Relaxed) + 1;
        std::thread::spawn(move || keep_alive(gen));
        Ok(())
    }
}

/// Ping the receiver while the paired connection sits idle (folder scan, the
/// user choosing, file-list build). Stops once start_send takes the stream,
/// on cancel, or when a newer pairing replaces this one.
fn keep_alive(gen: u64) {
    loop {
        std::thread::sleep(net::HEARTBEAT);
        let state = session();
        if state.pair_gen.load(Ordering::Relaxed) != gen {
            return;
        }
        let mut guard = state.send_stream.lock().unwrap();
        let Some(stream) = guard.as_mut() else { return };
        if let Err(e) = net::heartbeat(stream) {
            // leave the stream in place: start_send will surface the real error
            log::warn!("keep-alive to the receiver failed: {e}");
            return;
        }
        log::trace!("keep-alive sent");
    }
}

/// How long the link speed test streams for.
const BENCH_TIME: Duration = Duration::from_secs(5);

/// Link speed test on the paired connection (sender, before choosing data):
/// in-memory data only, so the result is the cable + TCP + encryption limit
/// with no disk involved. Returns bytes/sec.
/// Blocking (~5 s).
pub fn link_benchmark() -> Result<f64, String> {
    {
        let state = session();
        // the keep-alive thread quits when it finds the stream gone; a fresh
        // one is started once the stream is back
        let mut stream = state.send_stream.lock().unwrap().take().ok_or_else(|| {
            log::error!("speed test called with no paired connection");
            "not paired — pair with the new PC first".to_string()
        })?;
        log::info!("speed test: streaming test data for {}s", BENCH_TIME.as_secs());
        let first = state.bench_next.load(Ordering::Relaxed);
        match crate::transfer::bench_send(&mut stream, first, BENCH_TIME) {
            Ok((bps, next)) => {
                log::info!("speed test: {:.0} MB/s ({:.1} Gbit/s)", bps / 1e6, bps * 8.0 / 1e9);
                state.bench_next.store(next, Ordering::Relaxed);
                *state.send_stream.lock().unwrap() = Some(stream);
                let gen = state.pair_gen.fetch_add(1, Ordering::Relaxed) + 1;
                std::thread::spawn(move || keep_alive(gen));
                Ok(bps)
            }
            Err(e) => {
                log::error!("speed test failed: {e}");
                Err(format!("speed test failed: {e} — pair again"))
            }
        }
    }
}

/// Blocking: measures folder sizes on disk — can take seconds on a big Documents.
pub fn list_sources() -> Vec<SourceGroup> {
    {
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
        groups
    }
}

/// Blocking: size up explorer picks (files or folders) for the Custom Transfer list.
pub fn measure_paths(paths: Vec<String>) -> Vec<SourceGroup> {
    sources::measure_paths(&paths)
}

/// Blocking: Explorer's picker for folders or files (multi-select). Empty = cancelled.
pub fn pick_paths(directory: bool) -> Vec<String> {
    let d = rfd::FileDialog::new();
    let got = if directory { d.pick_folders() } else { d.pick_files() };
    got.unwrap_or_default()
        .into_iter()
        .map(|p| p.display().to_string())
        .collect()
}

/// Blocking: Explorer's picker for one folder. None = cancelled.
pub fn pick_folder() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|p| p.display().to_string())
}

/// Blocking for the whole transfer; progress arrives as `Event::Progress`.
pub fn start_send(group_ids: Vec<String>) -> Result<(), String> {
    {
        let state = session();
        state.cancel.store(false, Ordering::Relaxed);
        if state.send_stream.lock().unwrap().is_none() {
            log::error!("start_send called with no paired connection");
            return Err("not paired — pair with the new PC first".into());
        }

        // Walk everything BEFORE taking the stream, so the keep-alive thread
        // keeps the receiver's connection alive however long this takes.
        log::info!("send starting — groups: {}", group_ids.join(", "));
        let t = std::time::Instant::now();
        let mut items = Vec::new();
        for (id, expanded) in sources::expand_all(&group_ids) {
            log::info!("  group '{id}' expands to {} file(s)", expanded.len());
            items.extend(expanded);
        }
        log::info!("file list built in {:.1}s", t.elapsed().as_secs_f64());
        if state.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let stream = state.send_stream.lock().unwrap().take().ok_or_else(|| {
            log::error!("paired connection went away while building the file list");
            "the connection to the new PC was closed — pair again"
        })?;

        let (peer, digits) = state.pair_info.lock().unwrap().clone().ok_or("not paired")?;
        let name = net::host_name("This PC");
        let reconnect = || {
            log::info!("reconnecting to {}:{} to resume", peer.ip, peer.port);
            net::connect_and_pair(&peer, &name, &digits)
        };

        let cancel = state.cancel.clone();
        let t = std::time::Instant::now();
        let out = crate::transfer::send(stream, &items, &cancel, reconnect, emit_progress).map_err(|e| {
            log::error!("send failed: {e}");
            emit_progress(TransferProgress::error(&e.to_string()));
            e.to_string()
        });
        if out.is_ok() {
            log::info!("send finished in {:.1}s", t.elapsed().as_secs_f64());
        }
        out
    }
}

/// Blocking: waits for the sender, then the whole transfer. Returns the
/// folder the crossing landed in, for "Import into place".
pub fn receive() -> Result<String, String> {
    {
        let state = session();
        state.cancel.store(false, Ordering::Relaxed);
        let listener = state.listener.lock().unwrap().take().ok_or_else(|| {
            log::error!("receive called before start_receiver");
            "receiver not started"
        })?;
        let code = state.code.lock().unwrap().clone().unwrap_or_default();
        let cancel = state.cancel.clone();

        log::info!("waiting for a sender to connect…");
        let (stream, peer) =
            net::accept_and_verify(&listener, &code, &cancel, None).map_err(|e| {
                log::error!("accept/verify failed: {e}");
                e.to_string()
            })?;
        log::info!("sender '{}' verified from {}", peer.name, peer.ip);
        emit(Event::Paired(peer.clone()));

        // stop advertising once someone connected
        if let Some(stop) = state.beacon_stop.lock().unwrap().take() {
            stop.store(true, Ordering::Relaxed);
        }

        // read now, not at start: the user may change the drop folder while waiting
        let dest = incoming_dir(&settings::receive_root(&settings::load()));
        std::fs::create_dir_all(&dest).map_err(|e| {
            log::error!("could not create {}: {e}", dest.display());
            e.to_string()
        })?;
        log::info!("receiving into {}", dest.display());

        let t = std::time::Instant::now();
        // The listener stays open for the whole transfer: if the link breaks,
        // the sender reconnects here with the same code and resumes.
        let reaccept = |deadline: std::time::Instant, cancel: &AtomicBool| {
            let (s, p) = net::accept_and_verify(&listener, &code, cancel, Some(deadline))?;
            log::info!("sender '{}' reconnected from {}", p.name, p.ip);
            Ok(s)
        };
        crate::transfer::receive(stream, &dest, &cancel, reaccept, emit_progress).map_err(|e| {
            log::error!("receive failed: {e}");
            emit_progress(TransferProgress::error(&e.to_string()));
            e.to_string()
        })?;
        log::info!("receive finished in {:.1}s", t.elapsed().as_secs_f64());
        Ok(dest.display().to_string())
    }
}

/// Only crossing folders (…\WINC Received\crossing-<ts>) may be imported
/// from — wherever the receiver's drop folder was pointed.
fn crossing_dir(dir: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::PathBuf::from(dir);
    let is_crossing = p.is_dir()
        && p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("crossing-"))
        && p.parent()
            .and_then(|q| q.file_name())
            .is_some_and(|n| n == "WINC Received");
    if !is_crossing {
        log::error!("refused import from a non-crossing folder: {}", p.display());
        return Err("not a WINC received folder".into());
    }
    Ok(p)
}

/// The receiver's drop locations, for the UI.
#[derive(Debug, Clone)]
pub struct DropSettings {
    /// Effective root crossings land under (<root>\WINC Received\...).
    pub receive_root: String,
    pub receive_is_default: bool,
    /// Last chosen import folder; None = the user folders.
    pub import_dir: Option<String>,
}

pub fn drop_settings() -> DropSettings {
    let s = settings::load();
    let root = settings::receive_root(&s);
    DropSettings {
        receive_is_default: root == settings::default_receive_root(),
        receive_root: root.display().to_string(),
        import_dir: s.import_dir.filter(|d| std::path::Path::new(d).is_dir()),
    }
}

/// Point incoming crossings at another folder/drive (None = back to Documents).
/// Remembered for next time.
pub fn set_receive_root(root: Option<String>) -> Result<DropSettings, String> {
    if let Some(r) = &root {
        if !std::path::Path::new(r).is_dir() {
            return Err(format!("{r} is not a folder"));
        }
    }
    let mut s = settings::load();
    s.receive_root = root;
    settings::save(&s);
    let out = drop_settings();
    log::info!("receive folder set to {}", out.receive_root);
    Ok(out)
}

/// Free bytes on the drive holding `path`.
pub fn free_space(path: &str) -> Option<u64> {
    settings::free_space(std::path::Path::new(path))
}

/// Move a finished crossing out of its WINC Received folder into the real user
/// folders (or under `target`, if the user picked one) and browser profiles.
/// Blocking.
pub fn import_received(
    dir: String,
    target: Option<String>,
) -> Result<crate::import::ImportReport, String> {
    {
        let p = crossing_dir(&dir)?;
        let target = target.map(std::path::PathBuf::from);
        if let Some(t) = &target {
            if !t.is_dir() {
                return Err(format!("{} is not a folder", t.display()));
            }
        }
        // remember the choice (or the switch back to user folders) for next time
        let mut s = settings::load();
        s.import_dir = target.as_ref().map(|t| t.display().to_string());
        settings::save(&s);
        match &target {
            Some(t) => log::info!("import from {} into {}", p.display(), t.display()),
            None => log::info!("import into place from {}", p.display()),
        }
        let report = crate::import::import_received(&p, target.as_deref());
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
    }
}

/// "Overwrite?" for one browser whose import was skipped as not-fresh.
/// Backs the new PC's existing files up to <dump>\Backup\<Label>\ first.
/// Blocking.
pub fn import_browser_overwrite(dir: String, label: String) -> Result<crate::import::ImportEntry, String> {
    {
        let p = crossing_dir(&dir)?;
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
    }
}

/// "Open First" re-check for one browser whose import was skipped as
/// not-installed. Re-runs the non-force import so a newly-opened browser
/// profile is picked up and routed to import / not-fresh as applicable.
/// Blocking.
pub fn import_browser_retry(dir: String, label: String) -> Result<crate::import::ImportEntry, String> {
    {
        let p = crossing_dir(&dir)?;
        log::info!("re-checking {label} after 'Open First'");
        let out = crate::import::retry_browser(&p, &label);
        match &out {
            Ok(e) => log::info!("retry {label}: {} ({} file(s))", e.action, e.count),
            Err(e) => log::error!("retry {label} failed: {e}"),
        }
        out
    }
}

/// Add a Windows Firewall allow-rule for this exe on ALL profiles. The direct
/// cable comes up as an "Unidentified network", which Windows puts on the
/// Public profile — where inbound is blocked by default and the standard
/// firewall prompt (Private-only) doesn't help. That silently kills discovery
/// (inbound UDP 50737) and the receiver's listener (inbound TCP 50738).
/// Elevates via UAC; returns Err if the user declines. Blocking.
pub fn allow_firewall() -> Result<(), String> {
    #[cfg(windows)]
    {
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
    }
    #[cfg(not(windows))]
    Err("Only needed on Windows".into())
}

/// Stop whatever's in flight: unblocks the receiver's accept loop, ends any
/// transfer, and stops the discovery beacon. Called on "Start over".
pub fn cancel() {
    let state = session();
    log::warn!("cancel — stopping beacon, accept loop and any transfer in flight");
    state.cancel.store(true, Ordering::Relaxed);
    if let Some(stop) = state.beacon_stop.lock().unwrap().take() {
        stop.store(true, Ordering::Relaxed);
    }
    *state.send_stream.lock().unwrap() = None;
}

fn incoming_dir(base: &std::path::Path) -> std::path::PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    base.join("WINC Received").join(format!("crossing-{ts}"))
}
