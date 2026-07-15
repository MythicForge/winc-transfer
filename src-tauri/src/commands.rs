use crate::model::*;
use crate::{net, sources};
use rand::Rng;
use serde::Serialize;
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

/// Shared session state, managed by Tauri.
#[derive(Default)]
pub struct Session {
    pub cancel: Arc<AtomicBool>,
    pub code: Mutex<Option<String>>, // 6 digits, no spaces
    pub beacon_stop: Mutex<Option<Arc<AtomicBool>>>,
    pub listener: Mutex<Option<TcpListener>>,
    pub send_stream: Mutex<Option<TcpStream>>,
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

#[tauri::command]
pub fn start_receiver(name: String, state: State<'_, Session>) -> Result<ReceiverInfo, String> {
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
pub fn discover_peer() -> Option<Peer> {
    net::discover(Duration::from_secs(10))
}

/// All IPv4 addresses on this PC (direct-cable link first), for manual pairing.
#[tauri::command]
pub fn list_local_ips() -> Vec<String> {
    net::local_ipv4s()
}

#[tauri::command]
pub fn pair(peer: Peer, code: String, state: State<'_, Session>) -> Result<(), String> {
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    let name = net::host_name("This PC");
    let stream = net::connect_and_pair(&peer, &name, &digits).map_err(|e| e.to_string())?;
    *state.send_stream.lock().unwrap() = Some(stream);
    Ok(())
}

#[tauri::command]
pub fn list_sources() -> Vec<SourceGroup> {
    sources::list_sources()
}

#[tauri::command]
pub fn start_send(
    group_ids: Vec<String>,
    app: AppHandle,
    state: State<'_, Session>,
) -> Result<(), String> {
    state.cancel.store(false, Ordering::Relaxed);
    let stream = state
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
    net::send_files(&stream, &items, &cancel, emit).map_err(|e| {
        let _ = app.emit("winc://progress", TransferProgress::error(&e.to_string()));
        e.to_string()
    })
}

#[tauri::command]
pub fn receive(app: AppHandle, state: State<'_, Session>) -> Result<(), String> {
    state.cancel.store(false, Ordering::Relaxed);
    let listener = state
        .listener
        .lock()
        .unwrap()
        .take()
        .ok_or("receiver not started")?;
    let code = state.code.lock().unwrap().clone().unwrap_or_default();

    let (stream, peer) = net::accept_and_verify(&listener, &code).map_err(|e| e.to_string())?;
    let _ = app.emit("winc://paired", &peer);

    // stop advertising once someone connected
    if let Some(stop) = state.beacon_stop.lock().unwrap().take() {
        stop.store(true, Ordering::Relaxed);
    }

    let dest = incoming_dir();
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

    let cancel = state.cancel.clone();
    let emit = emitter(&app);
    net::receive_files(&stream, &dest, &cancel, emit).map_err(|e| {
        let _ = app.emit("winc://progress", TransferProgress::error(&e.to_string()));
        e.to_string()
    })
}

#[tauri::command]
pub fn cancel(state: State<'_, Session>) {
    state.cancel.store(true, Ordering::Relaxed);
}

fn incoming_dir() -> std::path::PathBuf {
    let base = dirs::document_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    base.join("WINC Received").join(format!("crossing-{ts}"))
}
