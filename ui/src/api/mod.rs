//! Backend abstraction.
//! In the packaged Tauri app -> calls Rust commands + listens to real events.
//! In a plain browser (dev / design preview on non-Windows) -> a mock so the
//! whole flow is drivable without two PCs.

mod mock;
pub mod tauri;

use crate::log;
use crate::types::*;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::json;

pub use tauri::is_tauri;

/// Fixed TCP port the receiver prefers, so manual IP entry needs no port.
pub const TRANSFER_PORT: u16 = 50738;

/// Mock mode: no Tauri runtime (plain browser preview).
pub fn mock() -> bool {
    !is_tauri()
}

/// Commands whose success is too frequent to log (the link poll runs every
/// 1.5 s). Failures are always logged, however often they happen.
const QUIET: &[&str] = &["get_link_status"];

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or_else(js_sys::Date::now)
}

async fn invoke<T: DeserializeOwned>(cmd: &str, args: impl Serialize) -> Result<T, String> {
    let quiet = QUIET.contains(&cmd);
    let t = now_ms();
    if !quiet {
        log::debug("ui::invoke", &format!("→ {cmd}"));
    }
    match tauri::invoke_raw(cmd, args).await {
        Ok(out) => {
            if !quiet {
                log::debug("ui::invoke", &format!("← {cmd} ok in {}ms", (now_ms() - t).round()));
            }
            Ok(out)
        }
        Err(e) => {
            // Every backend failure surfaces here, whatever the caller does with it.
            log::error(
                "ui::invoke",
                &format!("✕ {cmd} failed after {}ms — {e}", (now_ms() - t).round()),
            );
            Err(e)
        }
    }
}

/// Direct-cable network link state; the UI polls this.
pub async fn link_status() -> Result<LinkStatus, String> {
    if mock() {
        return Ok(mock::link_status().await);
    }
    invoke("get_link_status", ()).await
}

/// All network adapters this PC sees, classified — for the connect diagnostics.
pub async fn list_adapters() -> Result<Vec<AdapterInfo>, String> {
    if mock() {
        return Ok(mock::list_adapters().await);
    }
    invoke("list_adapters", ()).await
}

/// RECEIVE side: open listener, return the 6-digit pairing code + our address.
pub async fn start_receiver(name: &str) -> Result<ReceiverInfo, String> {
    if mock() {
        return Ok(mock::start_receiver().await);
    }
    invoke("start_receiver", json!({ "name": name })).await
}

/// SEND side: look for a receiver beacon on the link-local subnet.
pub async fn discover_peer() -> Result<Option<Peer>, String> {
    if mock() {
        return Ok(mock::discover_peer().await);
    }
    invoke("discover_peer", ()).await
}

/// Every IPv4 on this PC (direct-cable link first), for manual pairing.
pub async fn list_local_ips() -> Result<Vec<String>, String> {
    if mock() {
        return Ok(mock::list_local_ips().await);
    }
    invoke("list_local_ips", ()).await
}

/// SEND side: connect to peer and verify the code.
pub async fn pair(peer: &Peer, code: &str) -> Result<(), String> {
    if mock() {
        return mock::pair(code).await;
    }
    invoke("pair", json!({ "peer": peer, "code": code })).await
}

/// SEND side: the default + browser data groups discovered on this pc.
pub async fn list_sources() -> Result<Vec<SourceGroup>, String> {
    if mock() {
        return Ok(mock::list_sources().await);
    }
    invoke("list_sources", ()).await
}

/// SEND side: size up explorer-picked files/folders (ids are "custom-<path>").
pub async fn measure_paths(paths: Vec<String>) -> Result<Vec<SourceGroup>, String> {
    if mock() {
        return Ok(mock::measure_paths(paths).await);
    }
    invoke("measure_paths", json!({ "paths": paths })).await
}

/// Open Explorer for folders or files (multi-select). Empty = cancelled.
pub async fn pick_paths(directory: bool) -> Result<Vec<String>, String> {
    if mock() {
        return Ok(mock::pick_paths(directory));
    }
    // the dialog plugin's own command — what @tauri-apps/plugin-dialog's open() calls
    let got: Option<serde_json::Value> = invoke(
        "plugin:dialog|open",
        json!({ "options": { "directory": directory, "multiple": true } }),
    )
    .await?;
    Ok(match got {
        Some(serde_json::Value::Array(a)) => a
            .into_iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        Some(serde_json::Value::String(s)) => vec![s],
        _ => Vec::new(),
    })
}

/// SEND side: begin streaming selected groups; progress via cb.
pub async fn start_send(
    group_ids: Vec<String>,
    on_progress: impl Fn(TransferProgress) + 'static,
) -> Result<(), String> {
    if mock() {
        mock::start_send(&group_ids, &on_progress).await;
        return Ok(());
    }
    // pairing already bound the connection server-side
    let _sub = tauri::listen("winc://progress", on_progress).await?;
    invoke("start_send", json!({ "groupIds": group_ids })).await
}

/// RECEIVE side: wait for + accept an incoming transfer; progress via cb.
/// Resolves with the folder the crossing landed in.
pub async fn receive(
    on_peer: impl Fn(Peer) + 'static,
    on_progress: impl Fn(TransferProgress) + 'static,
) -> Result<String, String> {
    if mock() {
        return Ok(mock::receive(&on_peer, &on_progress).await);
    }
    let _p = tauri::listen("winc://paired", on_peer).await?;
    let _g = tauri::listen("winc://progress", on_progress).await?;
    invoke("receive", ()).await
}

/// RECEIVE side: move a finished crossing into the real folders / browsers —
/// or, with `target`, under a folder the user picked.
pub async fn import_received(dir: &str, target: Option<&str>) -> Result<ImportReport, String> {
    if mock() {
        return Ok(mock::import_received(target).await);
    }
    invoke("import_received", json!({ "dir": dir, "target": target })).await
}

/// RECEIVE side: where crossings land + the last import folder.
pub async fn get_drop_settings() -> Result<DropSettings, String> {
    if mock() {
        return Ok(mock::drop_settings());
    }
    invoke("get_drop_settings", ()).await
}

/// RECEIVE side: point incoming data at another folder (None = Documents).
pub async fn set_receive_root(root: Option<String>) -> Result<DropSettings, String> {
    if mock() {
        return Ok(mock::set_receive_root(root));
    }
    invoke("set_receive_root", json!({ "root": root })).await
}

/// Free bytes on the drive holding `path`.
pub async fn free_space(path: &str) -> Result<Option<u64>, String> {
    if mock() {
        return Ok(Some(if path.starts_with('D') { 1_800_000_000_000 } else { 96_000_000_000 }));
    }
    invoke("free_space", json!({ "path": path })).await
}

/// Open Explorer for one folder. None = cancelled.
pub async fn pick_folder() -> Result<Option<String>, String> {
    if mock() {
        return Ok(Some("D:\\Data".into()));
    }
    let got: Option<String> = invoke(
        "plugin:dialog|open",
        json!({ "options": { "directory": true, "multiple": false } }),
    )
    .await?;
    Ok(got)
}

/// RECEIVE side: force one browser's import ("Overwrite?"), backing up originals first.
pub async fn import_browser_overwrite(dir: &str, label: &str) -> Result<ImportEntry, String> {
    if mock() {
        return Ok(mock::import_browser_overwrite(label).await);
    }
    invoke("import_browser_overwrite", json!({ "dir": dir, "label": label })).await
}

/// RECEIVE side: re-check one browser after "Open First" — re-runs its non-force
/// import so a newly-opened browser profile is picked up.
pub async fn import_browser_retry(dir: &str, label: &str) -> Result<ImportEntry, String> {
    if mock() {
        return Ok(mock::import_browser_retry(label).await);
    }
    invoke("import_browser_retry", json!({ "dir": dir, "label": label })).await
}

/// SEND side, paired: ~5 s link speed test with in-memory data (no disk).
/// Returns bytes/sec.
pub async fn link_benchmark() -> Result<f64, String> {
    if mock() {
        return Ok(mock::link_benchmark().await);
    }
    invoke("link_benchmark", ()).await
}

/// Stop everything in flight (accept loop, transfer, beacon).
pub async fn cancel() -> Result<(), String> {
    if mock() {
        mock::cancel().await;
        return Ok(());
    }
    invoke("cancel", ()).await
}

/// Add a Windows Firewall allow-rule for WINCI on all profiles (UAC prompt).
pub async fn allow_firewall() -> Result<(), String> {
    if mock() {
        mock::allow_firewall().await;
        return Ok(());
    }
    invoke("allow_firewall", ()).await
}
