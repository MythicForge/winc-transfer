//! Frontend end of the log bus.
//!
//! In the packaged app the authority is Rust: UI lines are pushed over `ui_log`
//! and come back on the `winc://log` event, so both windows see one stream in
//! one order, and everything also lands in the on-disk log file.
//! In a plain browser (mock mode) the ring lives here so the view still works
//! for design/demo without a cable.

use crate::api::{is_tauri, tauri};
use crate::types::LogEntry;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;

pub const LEVELS: [(&str, &str); 5] = [
    ("error", "Errors"),
    ("warn", "Warnings"),
    ("info", "Info"),
    ("debug", "Debug"),
    ("trace", "Trace (every file)"),
];

/// Lower = more severe.
fn rank(level: &str) -> u8 {
    match level {
        "error" => 0,
        "warn" => 1,
        "info" => 2,
        "debug" => 3,
        _ => 4,
    }
}

/// True when `level` is at least as severe as `floor`.
pub fn at_least(level: &str, floor: &str) -> bool {
    rank(level) <= rank(floor)
}

const CAP: usize = 3000;

thread_local! {
    /// Mutated in place — copied into SNAP once per frame in notify(), never
    /// per entry, so a burst of a few hundred lines costs one clone.
    static LIVE: RefCell<Vec<LogEntry>> = const { RefCell::new(Vec::new()) };
    static SEEN: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
    static SNAP: ArcRwSignal<Vec<LogEntry>> = ArcRwSignal::new(Vec::new());
    static QUEUED: Cell<bool> = const { Cell::new(false) };
    static LOCAL_SEQ: Cell<u64> = const { Cell::new(1_000_000_000) }; // mock-only ids, far from any Rust seq
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// The live log, as a signal views can subscribe to.
pub fn entries() -> Signal<Vec<LogEntry>> {
    SNAP.with(|s| s.read_only()).into()
}

fn notify() {
    // A transfer can emit in bursts; coalesce so views re-render once a frame.
    if QUEUED.with(|q| q.replace(true)) {
        return;
    }
    request_animation_frame(|| {
        QUEUED.with(|q| q.set(false));
        let snap = LIVE.with(|l| l.borrow().clone());
        SNAP.with(|s| s.set(snap));
    });
}

fn push(e: LogEntry) {
    // hydration overlapping the live stream
    if !SEEN.with(|s| s.borrow_mut().insert(e.seq)) {
        return;
    }
    LIVE.with(|l| {
        let mut l = l.borrow_mut();
        l.push(e);
        if l.len() > CAP {
            let n = l.len() - CAP;
            SEEN.with(|s| {
                let mut s = s.borrow_mut();
                for d in l.drain(..n) {
                    s.remove(&d.seq);
                }
            });
        }
    });
    notify();
}

fn push_local(level: &str, target: &str, msg: &str) {
    let seq = LOCAL_SEQ.with(|s| s.replace(s.get() + 1));
    push(LogEntry {
        seq,
        ts: js_sys::Date::now(),
        level: level.into(),
        target: target.into(),
        source: "ui".into(),
        msg: msg.into(),
    });
}

/// Emit a UI-side line. Fire-and-forget: logging must never break a flow.
pub fn ui(level: &str, target: &str, msg: &str) {
    if !is_tauri() {
        push_local(level, target, msg);
        return;
    }
    let (level, target, msg) = (level.to_string(), target.to_string(), msg.to_string());
    spawn_local(async move {
        let sent: Result<(), String> = tauri::invoke_raw(
            "ui_log",
            json!({ "level": level, "target": target, "msg": msg }),
        )
        .await;
        if sent.is_err() {
            // Rust unreachable — keep the line locally so it isn't simply lost.
            push_local(&level, &target, &msg);
        }
    });
}

pub fn error(target: &str, msg: &str) {
    ui("error", target, msg)
}
pub fn warn(target: &str, msg: &str) {
    ui("warn", target, msg)
}
pub fn info(target: &str, msg: &str) {
    ui("info", target, msg)
}
pub fn debug(target: &str, msg: &str) {
    ui("debug", target, msg)
}

pub async fn clear() {
    LIVE.with(|l| l.borrow_mut().clear());
    SEEN.with(|s| s.borrow_mut().clear());
    notify();
    if is_tauri() {
        let _: Result<(), _> = tauri::invoke_raw("clear_logs", ()).await;
    }
}

pub async fn set_level(level: &str) {
    if is_tauri() {
        let _: Result<(), _> = tauri::invoke_raw("set_log_level", json!({ "level": level })).await;
    }
}

pub async fn get_level() -> String {
    if !is_tauri() {
        return "debug".into();
    }
    tauri::invoke_raw("get_log_level", ()).await.unwrap_or_else(|_| "info".into())
}

pub async fn file_path() -> String {
    if !is_tauri() {
        return "(mock mode — no log file)".into();
    }
    tauri::invoke_raw("log_file_path", ()).await.unwrap_or_default()
}

pub async fn open_dir() {
    if !is_tauri() {
        warn("ui::log", "no log file in mock mode — run the packaged app");
        return;
    }
    if let Err(e) = tauri::invoke_raw::<()>("open_log_dir", ()).await {
        error("ui::log", &format!("could not reveal the log file: {e}"));
    }
}

/// Open (or focus) the detachable log window. Built on the Rust side.
pub async fn open_window() {
    if !is_tauri() {
        warn("ui::log", "pop-out needs the packaged app (mock mode is one browser tab)");
        return;
    }
    if let Err(e) = tauri::invoke_raw::<()>("open_log_window", ()).await {
        error("ui::log", &format!("could not open the log window: {e}"));
    }
}

/// True when this document is the popout (index.html?view=log), not the main UI.
pub fn is_log_window() -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|q| q.split(['?', '&']).any(|kv| kv == "view=log"))
}

/// Attach to the Rust stream and backfill history. Safe to call more than once.
pub fn init() {
    if STARTED.with(|s| s.replace(true)) {
        return;
    }
    if !is_tauri() {
        ui("info", "ui::boot", "mock mode — no backend; log shows UI activity only");
        return;
    }
    spawn_local(async {
        // Listen BEFORE hydrating: anything emitted during the round-trip still
        // arrives, and SEEN drops the overlap.
        let attached = async {
            let sub = tauri::listen::<LogEntry>("winc://log", push).await?;
            std::mem::forget(sub); // lives as long as the window
            let history: Vec<LogEntry> = tauri::invoke_raw("get_logs", ()).await?;
            history.into_iter().for_each(push);
            Ok::<_, String>(())
        };
        if let Err(e) = attached.await {
            push_local("error", "ui::log", &format!("log bus failed to attach: {e}"));
        }
    });
}

