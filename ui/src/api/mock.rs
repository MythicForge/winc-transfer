//! Mock backend for a plain browser (dev / design preview on non-Windows), so
//! the whole flow is drivable without two PCs.

use crate::log;
use crate::types::*;
use gloo_timers::future::TimeoutFuture;
use std::cell::{Cell, RefCell};

thread_local! {
    static CANCELLED: Cell<bool> = const { Cell::new(false) };
    /// measure_paths results, so custom sends have sizes
    static PICKED: RefCell<Vec<SourceGroup>> = const { RefCell::new(Vec::new()) };
    /// first link poll — the "cable" comes up 2.6 s later
    static LINK_T0: Cell<Option<f64>> = const { Cell::new(None) };
}

const DPAPI: &str = "Passwords are locked to this Windows account (DPAPI). They transfer but only unlock if you sign in with the same Microsoft account on the new PC.";

async fn sleep(ms: u32) {
    TimeoutFuture::new(ms).await;
}

fn now() -> f64 {
    js_sys::Date::now()
}

fn peer() -> Peer {
    Peer { name: "SURFACE-9".into(), ip: "169.254.42.9".into(), port: super::TRANSFER_PORT }
}

fn group(id: &str, label: &str, hint: &str, kind: &str, bytes: u64, items: u64, caveat: Option<&str>, on: bool) -> SourceGroup {
    SourceGroup {
        id: id.into(),
        label: label.into(),
        hint: hint.into(),
        kind: kind.into(),
        path: (kind == "folder").then(|| label.to_string()),
        bytes,
        items,
        caveat: caveat.map(Into::into),
        selected: on,
    }
}

fn sources() -> Vec<SourceGroup> {
    vec![
        group("docs", "Documents", "C:\\Users\\you\\Documents", "folder", 4_820_000_000, 12840, None, true),
        group("desktop", "Desktop", "C:\\Users\\you\\Desktop", "folder", 640_000_000, 214, None, true),
        group("pictures", "Pictures", "C:\\Users\\you\\Pictures", "folder", 18_200_000_000, 9021, None, true),
        group("downloads", "Downloads", "C:\\Users\\you\\Downloads", "folder", 7_100_000_000, 512, None, false),
        group("chrome", "Chrome — bookmarks, history & passwords", "Default profile", "browser", 210_400_000, 9, Some(DPAPI), true),
        group("edge", "Edge — bookmarks, history & passwords", "Default profile", "browser", 88_400_000, 9, Some(DPAPI), false),
    ]
}

pub async fn link_status() -> LinkStatus {
    let t0 = LINK_T0.with(|t| {
        let v = t.get().unwrap_or_else(now);
        t.set(Some(v));
        v
    });
    if now() - t0 < 2600.0 {
        return LinkStatus::default();
    }
    LinkStatus {
        up: true,
        adapter: Some("Thunderbolt Bridge".into()),
        local_ip: Some("169.254.42.7".into()),
        kind: Some("thunderbolt".into()),
    }
}

pub async fn list_adapters() -> Vec<AdapterInfo> {
    sleep(150).await;
    vec![
        AdapterInfo { name: "Thunderbolt Bridge".into(), ip: "169.254.42.7".into(), link_local: true, cable: true, kind: "thunderbolt".into() },
        AdapterInfo { name: "Wi-Fi".into(), ip: "192.168.1.24".into(), link_local: false, cable: false, kind: "network".into() },
    ]
}

pub async fn start_receiver() -> ReceiverInfo {
    sleep(400).await;
    log::info("mock", &format!("receiver ready on 169.254.42.9:{} — code issued", super::TRANSFER_PORT));
    ReceiverInfo { code: "418 620".into(), peer: peer() }
}

pub async fn discover_peer() -> Option<Peer> {
    log::info("mock", "discovery: listening for a receiver beacon");
    sleep(1800).await;
    log::info("mock", "discovery: found 'SURFACE-9' at 169.254.42.9");
    Some(peer())
}

pub async fn list_local_ips() -> Vec<String> {
    sleep(200).await;
    vec!["169.254.42.9".into(), "192.168.1.24".into()]
}

pub async fn pair(code: &str) -> Result<(), String> {
    sleep(900).await;
    if code.chars().filter(|c| !c.is_whitespace()).count() != 6 {
        log::error("mock", "pairing rejected — wrong code");
        return Err("wrong code".into());
    }
    log::info("mock", "paired — encrypted channel open");
    Ok(())
}

pub async fn list_sources() -> Vec<SourceGroup> {
    sleep(600).await;
    sources()
}

pub async fn measure_paths(paths: Vec<String>) -> Vec<SourceGroup> {
    sleep(300).await;
    // fake but stable sizes so the custom list looks plausible in the preview
    let out: Vec<SourceGroup> = paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let name = p.rsplit(['\\', '/']).next().unwrap_or(p).to_string();
            let is_file = name.rsplit_once('.').is_some_and(|(_, ext)| (1..=5).contains(&ext.len()));
            let n = i as u64 + 1;
            SourceGroup {
                id: format!("custom-{p}"),
                label: name,
                hint: p.clone(),
                kind: if is_file { "file" } else { "folder" }.into(),
                path: Some(p.clone()),
                bytes: if is_file { 4_200_000 * n } else { 1_300_000_000 * n },
                items: if is_file { 1 } else { 340 * n },
                caveat: None,
                selected: true,
            }
        })
        .collect();
    PICKED.with(|p| p.borrow_mut().extend(out.iter().cloned()));
    out
}

pub fn pick_paths(directory: bool) -> Vec<String> {
    if directory {
        vec!["C:\\Projects\\Thesis".into(), "D:\\Photos 2019".into()]
    } else {
        vec!["C:\\Users\\you\\notes.txt".into()]
    }
}

pub async fn start_send(group_ids: &[String], on_progress: &dyn Fn(TransferProgress)) {
    CANCELLED.with(|c| c.set(false));
    let picked = PICKED.with(|p| p.borrow().clone());
    let chosen: Vec<SourceGroup> = sources()
        .into_iter()
        .chain(picked)
        .filter(|s| group_ids.contains(&s.id))
        .collect();
    let bytes_total: u64 = chosen.iter().map(|s| s.bytes).sum();
    let files_total: u64 = chosen.iter().map(|s| s.items).sum();
    let (mut bytes_sent, mut files_sent) = (0f64, 0f64);
    let start = now();
    log::info(
        "mock",
        &format!(
            "send starting — {} group(s), {files_total} file(s), {:.2} GB",
            chosen.len(),
            bytes_total as f64 / 1e9
        ),
    );
    for g in &chosen {
        log::debug("mock", &format!("streaming '{}' ({} items)", g.label, g.items));
        const STEPS: u32 = 22;
        for i in 0..STEPS {
            if CANCELLED.with(|c| c.get()) {
                log::warn("mock", "transfer cancelled");
                on_progress(TransferProgress {
                    state: XferState::Error,
                    bytes_sent: bytes_sent as u64,
                    bytes_total,
                    files_sent: files_sent as u64,
                    files_total,
                    error: Some("Cancelled".into()),
                    ..Default::default()
                });
                return;
            }
            sleep(90).await;
            bytes_sent += g.bytes as f64 / STEPS as f64;
            files_sent += g.items as f64 / STEPS as f64;
            let secs = (now() - start) / 1000.0;
            on_progress(TransferProgress {
                state: XferState::Running,
                bytes_sent: (bytes_sent as u64).min(bytes_total),
                bytes_total,
                files_sent: (files_sent.round() as u64).min(files_total),
                files_total,
                bytes_per_sec: bytes_sent / secs,
                current_file: Some(format!(
                    "{} — item {}",
                    g.label,
                    ((i as f64 / STEPS as f64) * g.items as f64).round()
                )),
                error: None,
            });
        }
    }
    log::info("mock", &format!("send finished in {:.1}s", (now() - start) / 1000.0));
    on_progress(TransferProgress {
        state: XferState::Done,
        bytes_sent: bytes_total,
        bytes_total,
        files_sent: files_total,
        files_total,
        ..Default::default()
    });
}

pub async fn receive(on_peer: &dyn Fn(Peer), on_progress: &dyn Fn(TransferProgress)) -> String {
    log::info("mock", "waiting for a sender to connect…");
    sleep(2000).await;
    log::info("mock", "sender 'OLD-DELL-7490' verified from 169.254.42.9");
    on_peer(Peer { name: "OLD-DELL-7490".into(), ip: "169.254.42.9".into(), port: 52128 });
    let ids: Vec<String> = ["docs", "desktop", "pictures", "chrome"].map(String::from).to_vec();
    start_send(&ids, on_progress).await;
    "C:\\Users\\you\\Documents\\WINC Received\\crossing-1721000000".into()
}

fn entry(label: &str, action: &str, count: u64, detail: Option<&str>, browser: Option<&str>) -> ImportEntry {
    ImportEntry {
        label: label.into(),
        action: action.into(),
        count,
        detail: detail.map(Into::into),
        browser_label: browser.map(Into::into),
    }
}

pub async fn import_received() -> ImportReport {
    sleep(1500).await;
    ImportReport {
        entries: vec![
            entry("Documents", "imported", 12840, Some("3 kept both"), None),
            entry("Desktop", "imported", 214, None, None),
            entry("Pictures", "imported", 9021, None, None),
            entry("Tax Stuff", "imported", 96, Some("→ Documents\\Tax Stuff"), None),
            entry("Chrome (browser)", "skipped-not-fresh", 0, Some("Chrome already has data on this PC — use Overwrite to replace it with the old PC's (originals are backed up first)."), Some("Chrome")),
            entry("Edge (browser)", "skipped-not-installed", 0, Some("Edge isn't installed on this PC (or has never been opened)."), Some("Edge")),
        ],
    }
}

pub async fn import_browser_overwrite(label: &str) -> ImportEntry {
    sleep(1200).await;
    let detail = format!(
        "overwrote 9 — originals in C:\\Users\\you\\Documents\\WINC Received\\crossing-1721000000\\Backup\\{label}"
    );
    entry(&format!("{label} (browser)"), "imported", 9, Some(&detail), Some(label))
}

pub async fn import_browser_retry(label: &str) -> ImportEntry {
    sleep(900).await;
    // Simulate the user having opened the browser: profile now exists and is
    // fresh, so the retry succeeds.
    entry(&format!("{label} (browser)"), "imported", 9, None, Some(label))
}

pub async fn cancel() {
    log::warn("mock", "cancel — stopping everything in flight");
    CANCELLED.with(|c| c.set(true));
}

pub async fn allow_firewall() {
    sleep(600).await;
}
