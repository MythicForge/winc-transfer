// Prevents an extra console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod crypto;
mod import;
mod logbus;
mod model;
mod net;
mod settings;
mod sources;
mod transfer;
mod ui;

/// Folder name under %APPDATA% / %LOCALAPPDATA% — the same one the Tauri
/// builds used, so settings and logs carry over.
pub const APP_ID: &str = "com.winc.app";

fn main() -> iced::Result {
    // Always on, release included: the log view and the on-disk log are the
    // only way a fault on someone else's PC can be diagnosed.
    logbus::init();
    ui::run()
}
