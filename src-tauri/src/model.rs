use serde::{Deserialize, Serialize};

pub const DISCOVERY_PORT: u16 = 50737; // UDP beacon
pub const TRANSFER_PORT: u16 = 50738; // preferred TCP port (so manual IP entry needs no port)
/// Bumped to 2 in 0.9.0 (heartbeats + chunked manifest), 3 in 0.11.0
/// (auto-resume) and 4 in 0.12.0 (position-keyed data frames, pipelined) —
/// older builds see "not a WINC peer" instead of a garbled stream.
pub const PROTO_MAGIC: &str = "WINC4";

/// Direct-cable network link state, surfaced to the UI.
#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct LinkStatus {
    pub up: bool,
    pub adapter: Option<String>,
    pub local_ip: Option<String>,
    /// "thunderbolt" | "usb4" | "other" | null
    pub kind: Option<String>,
}

/// One network adapter, classified — used for detection and the diagnostics list.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AdapterInfo {
    pub name: String,
    pub ip: String,
    pub link_local: bool, // 169.254/16 APIPA — the direct-cable signature
    pub cable: bool,      // link_local or a Thunderbolt/USB4/bridge-named adapter
    pub kind: String,     // "thunderbolt" | "usb4" | "other" | "network"
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Peer {
    pub name: String,
    pub ip: String,
    pub port: u16,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceGroup {
    pub id: String,
    pub label: String,
    pub hint: String,
    /// "folder" | "browser"
    pub kind: String,
    pub path: Option<String>,
    pub bytes: u64,
    pub items: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caveat: Option<String>,
    pub selected: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    /// "idle" | "running" | "reconnecting" | "paused" | "done" | "error"
    pub state: String,
    pub bytes_sent: u64,
    pub bytes_total: u64,
    pub files_sent: u64,
    pub files_total: u64,
    pub bytes_per_sec: f64,
    pub current_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl TransferProgress {
    pub fn running(sent: u64, total: u64, fsent: u64, ftotal: u64, bps: f64, file: &str) -> Self {
        Self {
            state: "running".into(),
            bytes_sent: sent,
            bytes_total: total,
            files_sent: fsent,
            files_total: ftotal,
            bytes_per_sec: bps,
            current_file: Some(file.to_string()),
            error: None,
        }
    }
    /// The link broke mid-transfer and is being re-established.
    pub fn reconnecting(sent: u64, total: u64, fsent: u64, ftotal: u64, note: &str) -> Self {
        Self {
            state: "reconnecting".into(),
            bytes_sent: sent,
            bytes_total: total,
            files_sent: fsent,
            files_total: ftotal,
            bytes_per_sec: 0.0,
            current_file: Some(note.to_string()),
            error: None,
        }
    }
    pub fn done(total: u64, ftotal: u64) -> Self {
        Self {
            state: "done".into(),
            bytes_sent: total,
            bytes_total: total,
            files_sent: ftotal,
            files_total: ftotal,
            bytes_per_sec: 0.0,
            current_file: None,
            error: None,
        }
    }
    pub fn error(msg: &str) -> Self {
        Self {
            state: "error".into(),
            bytes_sent: 0,
            bytes_total: 0,
            files_sent: 0,
            files_total: 0,
            bytes_per_sec: 0.0,
            current_file: None,
            error: Some(msg.to_string()),
        }
    }
}

/// A concrete file to move: absolute source path + destination-relative path.
#[derive(Serialize, Deserialize, Clone)]
pub struct FileEntry {
    pub rel: String,
    pub size: u64,
}

/// Wire beacon the RECEIVER broadcasts so the SENDER can find it.
#[derive(Serialize, Deserialize)]
pub struct Beacon {
    pub magic: String,
    pub name: String,
    pub ip: String, // the receiver's cable IP, so the sender connects over the cable
    pub port: u16,
}

/// First encrypted message, SENDER -> RECEIVER. The pairing code is never
/// transmitted — knowledge of it is proven by the SPAKE2 handshake (a wrong
/// code makes this message undecryptable). Do not add the code back.
#[derive(Serialize, Deserialize)]
pub struct Hello {
    pub magic: String,
    pub name: String,
}

#[derive(Serialize, Deserialize)]
pub struct HelloAck {
    pub ok: bool,
}

/// First real message on every connection, SENDER -> RECEIVER (after any
/// heartbeats). `Start` begins a transfer: `chunks` frames of JSON
/// `Vec<FileEntry>` follow, then the file stream. Chunked so a tree of hundreds
/// of thousands of files never hits the per-frame size cap. `Resume` re-attaches
/// a reconnecting sender to a transfer that broke; the receiver answers with
/// `ResumeAt`. `Cancel` tells a receiver waiting for a resume that the user
/// stopped the transfer on the old PC, so it ends now instead of waiting.
#[derive(Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Opening {
    Start {
        transfer_id: u64,
        total_bytes: u64,
        total_files: u64,
        chunks: u32,
    },
    Resume {
        transfer_id: u64,
    },
    Cancel {
        transfer_id: u64,
    },
    /// Link speed test: sealed throwaway frames from chunk `first` on, until
    /// an empty one; the receiver drops them and answers `Finished`. Then the
    /// connection carries on as before (heartbeats, `Start`).
    Bench {
        first: u64,
    },
}

/// RECEIVER -> SENDER reply to `Opening::Resume`: restart the file stream at
/// manifest index `next` (the file that was in flight is rewritten whole).
/// `stopped` (with `ok: false`): the user stopped it on the new PC.
#[derive(Serialize, Deserialize)]
pub struct ResumeAt {
    pub ok: bool,
    pub next: u64,
    #[serde(default)]
    pub stopped: bool,
}

/// RECEIVER -> SENDER after the last file is on disk. The sender only calls
/// the transfer done once it has this — damage in the final frames would
/// otherwise go unnoticed by the sender.
#[derive(Serialize, Deserialize)]
pub struct Finished {
    pub ok: bool,
}
