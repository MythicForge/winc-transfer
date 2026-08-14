//! Structured log bus.
//!
//! One `log::Log` implementation fans every record out three ways:
//!   1. an in-memory ring buffer, so a log window opened *mid*-transfer still
//!      shows the history that explains how it got there;
//!   2. a `winc://log` Tauri event, so every open window (main + popout) can
//!      tail it live;
//!   3. a rotating file under the app log dir, so a user can send the log after
//!      the app has closed.
//!
//! Everything routes through the plain `log` facade (`log::info!`) so code with
//! no `AppHandle` in scope — net.rs, sources.rs, import.rs — logs freely.

use serde::Serialize;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

/// How many entries the in-memory ring holds. ~3k lines covers a whole session
/// including a chatty debug-level transfer, at a few hundred KB of RAM.
const RING_CAP: usize = 3000;
/// Rotate at 5 MB, keeping one previous file.
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// Monotonic id — lets a late-joining window de-dupe hydration against the
    /// live event stream.
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub ts: u64,
    /// "error" | "warn" | "info" | "debug" | "trace"
    pub level: String,
    /// Module path ("app_lib::net") for Rust, or a UI area for frontend lines.
    pub target: String,
    /// "rust" | "ui"
    pub source: String,
    pub msg: String,
}

static RING: Mutex<VecDeque<LogEntry>> = Mutex::new(VecDeque::new());
static SEQ: AtomicU64 = AtomicU64::new(0);
static APP: OnceLock<AppHandle> = OnceLock::new();
static FILE: Mutex<Option<File>> = Mutex::new(None);
static FILE_PATH: OnceLock<PathBuf> = OnceLock::new();
/// Current threshold as `log::LevelFilter as usize`. Runtime-adjustable from
/// the UI so a user can flip to Debug while reproducing a fault.
static LEVEL: AtomicUsize = AtomicUsize::new(log::LevelFilter::Info as usize);

thread_local! {
    /// Emitting an event or writing a file can itself log; without this a
    /// single record could recurse forever.
    static IN_LOG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// UTC "YYYY-MM-DD HH:MM:SS.mmm" for the file lines (the UI renders local time
/// from the raw `ts` instead). Hand-rolled to avoid a date dependency.
fn stamp(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}Z",
        y,
        m,
        d,
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60,
        ms % 1000
    )
}

/// Record an entry: ring + event + file. Shared by the `log::Log` impl and by
/// UI lines arriving over `ui_log`.
///
/// The reentrancy guard lives here, not in `Log::log`, because emitting the
/// event runs Tauri code that may itself log — on either path.
pub fn record(level: &str, target: &str, source: &str, msg: String) {
    struct Guard;
    impl Drop for Guard {
        // released even if the emit path unwinds, so one bad record can't
        // silence this thread's logging for the rest of the session
        fn drop(&mut self) {
            IN_LOG.with(|f| f.set(false));
        }
    }

    if IN_LOG.with(|f| f.replace(true)) {
        return;
    }
    let _g = Guard;
    emit_entry(level, target, source, msg);
}

fn emit_entry(level: &str, target: &str, source: &str, msg: String) {
    let entry = LogEntry {
        seq: SEQ.fetch_add(1, Ordering::Relaxed),
        ts: now_ms(),
        level: level.to_string(),
        target: target.to_string(),
        source: source.to_string(),
        msg,
    };

    if let Ok(mut ring) = RING.lock() {
        if ring.len() == RING_CAP {
            ring.pop_front();
        }
        ring.push_back(entry.clone());
    }

    write_file(&entry);

    if let Some(app) = APP.get() {
        let _ = app.emit("winc://log", &entry);
    }
}

fn write_file(e: &LogEntry) {
    let Ok(mut guard) = FILE.lock() else { return };
    let Some(f) = guard.as_mut() else { return };
    let line = format!(
        "{} {:5} [{}] {}\n",
        stamp(e.ts),
        e.level.to_uppercase(),
        e.target,
        e.msg
    );
    let _ = f.write_all(line.as_bytes());
    let _ = f.flush();

    // rotate once the active file is oversized
    if f.metadata().map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES {
        if let Some(path) = FILE_PATH.get() {
            *guard = None; // close before renaming (Windows locks open handles)
            let _ = std::fs::rename(path, path.with_extension("1.log"));
            *guard = OpenOptions::new().create(true).append(true).open(path).ok();
        }
    }
}

struct Bus;

impl log::Log for Bus {
    fn enabled(&self, m: &log::Metadata) -> bool {
        let target = m.target();
        // Our own crate obeys the user-selected level; everything else (tao,
        // wry, hyper…) only gets through at Warn+, so the view stays readable.
        let ceiling = if target.starts_with("app_lib") || target.starts_with("winc") {
            LEVEL.load(Ordering::Relaxed)
        } else {
            log::LevelFilter::Warn as usize
        };
        (m.level() as usize) <= ceiling
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        record(
            &r.level().to_string().to_lowercase(),
            r.target(),
            "rust",
            r.args().to_string(),
        );
    }

    fn flush(&self) {}
}

/// Install the logger and open the log file. Called once from `setup`.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());

    if let Ok(dir) = app.path().app_log_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("winc.log");
        if let Ok(f) = OpenOptions::new().create(true).append(true).open(&path) {
            *FILE.lock().unwrap() = Some(f);
        }
        let _ = FILE_PATH.set(path);
    }

    if cfg!(debug_assertions) {
        LEVEL.store(log::LevelFilter::Debug as usize, Ordering::Relaxed);
    }
    // Our `enabled()` does the real filtering, so let everything reach us.
    log::set_max_level(log::LevelFilter::Trace);
    let _ = log::set_boxed_logger(Box::new(Bus));

    // A panic in a spawned worker (beacon thread, blocking transfer) otherwise
    // dies silently — the transfer just stops with no explanation anywhere.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record("error", "app_lib::panic", "rust", format!("PANIC: {info}"));
        prev(info);
    }));

    log::info!(
        "WINC {} starting — log file: {}",
        app.package_info().version,
        FILE_PATH
            .get()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(none)".into())
    );
}

/* ---------------- commands ---------------- */

/// Full ring buffer, for a window that just opened.
#[tauri::command]
pub fn get_logs() -> Vec<LogEntry> {
    RING.lock().map(|r| r.iter().cloned().collect()).unwrap_or_default()
}

#[tauri::command]
pub fn clear_logs() {
    if let Ok(mut r) = RING.lock() {
        r.clear();
    }
    log::info!("log view cleared");
}

/// Accepts a line from the frontend so UI-side events land in the same ring and
/// the same file as the Rust ones.
#[tauri::command]
pub fn ui_log(level: String, target: String, msg: String) {
    record(&level, &target, "ui", msg);
}

#[tauri::command]
pub fn set_log_level(level: String) {
    let f = match level.as_str() {
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "debug" => log::LevelFilter::Debug,
        "trace" => log::LevelFilter::Trace,
        _ => log::LevelFilter::Info,
    };
    LEVEL.store(f as usize, Ordering::Relaxed);
    log::warn!("log level set to {}", f);
}

#[tauri::command]
pub fn get_log_level() -> String {
    match LEVEL.load(Ordering::Relaxed) {
        x if x == log::LevelFilter::Error as usize => "error",
        x if x == log::LevelFilter::Warn as usize => "warn",
        x if x == log::LevelFilter::Debug as usize => "debug",
        x if x == log::LevelFilter::Trace as usize => "trace",
        _ => "info",
    }
    .into()
}

#[tauri::command]
pub fn log_file_path() -> String {
    FILE_PATH
        .get()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

/// Reveal the log file in Explorer (selected), so "send me the log" is one click.
#[tauri::command]
pub fn open_log_dir() -> Result<(), String> {
    let Some(path) = FILE_PATH.get() else {
        return Err("no log file".into());
    };
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let dir = path.parent().unwrap_or(path.as_path());
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Open (or focus) the detachable log window. Built here rather than in JS so
/// the main window needs no window-creation capability.
#[tauri::command]
pub fn open_log_window(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("logs") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        "logs",
        tauri::WebviewUrl::App("index.html?view=log".into()),
    )
    .title("WINCI — Activity log")
    .inner_size(900.0, 560.0)
    .min_inner_size(520.0, 300.0)
    .build()
    .map_err(|e| e.to_string())?;
    log::info!("log window opened");
    Ok(())
}
