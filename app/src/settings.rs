//! Receiver drop locations, remembered between runs in
//! %APPDATA%\com.winc.app\settings.json (best-effort: a missing or unreadable file
//! just means defaults).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Root the receiver drops crossings under (<root>\WINC Received\...).
    /// None = Documents.
    pub receive_root: Option<String>,
    /// Last folder "Import into place" was pointed at. None = the user folders.
    pub import_dir: Option<String>,
}

/// Same file Tauri used, so earlier versions' choices carry over.
fn file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(crate::APP_ID).join("settings.json"))
}

pub fn load() -> Settings {
    file()
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(s: &Settings) {
    let Some(f) = file() else { return };
    let out = f
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(&f, serde_json::to_vec_pretty(s).unwrap_or_default()));
    if let Err(e) = out {
        log::warn!("could not save settings to {}: {e}", f.display());
    }
}

pub fn default_receive_root() -> PathBuf {
    dirs::document_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Where crossings land: the saved root if it still exists, else Documents.
pub fn receive_root(s: &Settings) -> PathBuf {
    s.receive_root
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(default_receive_root)
}

/// Free bytes on the volume holding `path` (None if it can't be read).
pub fn free_space(path: &Path) -> Option<u64> {
    fs4::available_space(path).ok()
}
