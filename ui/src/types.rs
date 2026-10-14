//! Wire types shared with the Tauri backend (src-tauri/src/model.rs,
//! import.rs, logbus.rs). Field names follow the backend's camelCase serde.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Send,
    Receive,
}

/// Sender's payload mode: preset user data, or explorer-picked files/folders.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransferMode {
    User,
    Custom,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStatus {
    pub up: bool,
    /// e.g. "Thunderbolt Bridge" / "USB4 Net"
    pub adapter: Option<String>,
    pub local_ip: Option<String>,
    /// "thunderbolt" | "usb4" | "other"
    pub kind: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterInfo {
    pub name: String,
    pub ip: String,
    /// 169.254/16 APIPA — the direct-cable signature
    pub link_local: bool,
    pub cable: bool,
    /// "thunderbolt" | "usb4" | "other" | "network"
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub name: String,
    pub ip: String,
    pub port: u16,
}

/// What start_receiver hands back: the code to show + our own address.
#[derive(Clone, Debug, Deserialize)]
pub struct ReceiverInfo {
    /// display form "418 620"
    pub code: String,
    pub peer: Peer,
}

/// A selectable group of data on the OLD pc.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceGroup {
    pub id: String,
    pub label: String,
    pub hint: String,
    /// "folder" | "file" | "browser" — "file" only for Custom Transfer picks
    pub kind: String,
    /// None for browser aggregates
    pub path: Option<String>,
    pub bytes: u64,
    pub items: u64,
    /// caveat text, e.g. saved passwords are DPAPI-bound.
    #[serde(default)]
    pub caveat: Option<String>,
    pub selected: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum XferState {
    #[default]
    Idle,
    Running,
    /// link broke mid-transfer; the two PCs are re-pairing to resume
    Reconnecting,
    Paused,
    Done,
    Error,
}

impl XferState {
    pub fn as_str(self) -> &'static str {
        match self {
            XferState::Idle => "idle",
            XferState::Running => "running",
            XferState::Reconnecting => "reconnecting",
            XferState::Paused => "paused",
            XferState::Done => "done",
            XferState::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub state: XferState,
    pub bytes_sent: u64,
    pub bytes_total: u64,
    pub files_sent: u64,
    pub files_total: u64,
    pub bytes_per_sec: f64,
    pub current_file: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// One group's outcome from "Import into place" on the receiver.
/// `action`: "imported" | "skipped-not-fresh" | "skipped-not-installed" | "error"
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEntry {
    pub label: String,
    pub action: String,
    pub count: u64,
    pub detail: Option<String>,
    /// Raw catalog label ("Chrome") on browser entries — used for "Overwrite?".
    pub browser_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ImportReport {
    pub entries: Vec<ImportEntry>,
}

/// Receiver drop locations (remembered between runs by the backend).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropSettings {
    /// Root crossings land under (<root>\WINC Received\...).
    pub receive_root: String,
    pub receive_is_default: bool,
    /// Last chosen import folder; None = the user folders.
    pub import_dir: Option<String>,
}

/// One line of the activity log, from Rust (logbus.rs) or the UI.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct LogEntry {
    pub seq: u64,
    /// ms since epoch
    pub ts: f64,
    /// "error" | "warn" | "info" | "debug" | "trace"
    pub level: String,
    /// rust module path, or a UI area like "ui::pair"
    pub target: String,
    /// "rust" | "ui"
    pub source: String,
    pub msg: String,
}
