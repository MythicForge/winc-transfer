//! "Import into place" — move a finished crossing out of
//! Documents\WINC Received\crossing-<ts> into the real user folders and, when
//! safe, into installed browsers' profiles.
//!
//! Policy (user-confirmed):
//! - Folder conflicts keep both: the incoming file is renamed
//!   "name (from old PC).ext" (then "(from old PC 2)", ...). Nothing on the
//!   new PC is ever overwritten.
//! - Browser data imports only into a fresh profile — if any incoming file
//!   name already exists at its target, the whole browser is skipped (the user
//!   should sign in and sync instead).
//! - Per-group failures never abort the rest of the import.

use crate::net::safe_join;
use crate::sources::{env_dir, gecko_profile, CHROMIUM_BROWSERS, GECKO_BROWSERS, OPERA_BROWSERS};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEntry {
    pub label: String,
    /// "imported" | "skipped-not-fresh" | "skipped-not-installed" | "error"
    pub action: String,
    pub count: u64,
    pub detail: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub entries: Vec<ImportEntry>,
}

/// Where a browser's incoming files land. `root` differs from `profile` only
/// for Chromium-family browsers, where "Local State" lives in the User Data
/// root while everything else lives in the profile (Default\).
struct BrowserTarget {
    root: PathBuf,
    profile: PathBuf,
}

/// Resolve a browser by its display label ("Chrome", "Opera GX", ...) — the
/// wire rel paths carry the label, not the group id. None = not installed
/// (or never opened, so no profile dir exists yet).
fn browser_target_by_label(label: &str) -> Option<BrowserTarget> {
    let chromium_like = CHROMIUM_BROWSERS
        .iter()
        .find(|(_, l, _)| *l == label)
        .and_then(|(_, _, dir)| env_dir("LOCALAPPDATA").map(|p| p.join(dir)))
        .or_else(|| {
            OPERA_BROWSERS
                .iter()
                .find(|(_, l, _)| *l == label)
                .and_then(|(_, _, dir)| env_dir("APPDATA").map(|p| p.join(dir)))
        });
    if let Some(root) = chromium_like {
        if !root.is_dir() {
            return None;
        }
        let profile = if root.join("Default").is_dir() {
            root.join("Default")
        } else {
            root.clone()
        };
        return Some(BrowserTarget { root, profile });
    }
    if let Some((_, _, dir)) = GECKO_BROWSERS.iter().find(|(_, l, _)| *l == label) {
        let profile = gecko_profile(&env_dir("APPDATA")?.join(dir))?;
        return Some(BrowserTarget {
            root: profile.clone(),
            profile,
        });
    }
    None
}

/// Next free "name (from old PC).ext" variant beside `dest`, or None if ~100
/// prior imports already claimed every variant.
fn keep_both_path(dest: &Path) -> Option<PathBuf> {
    let stem = dest.file_stem()?.to_string_lossy().to_string();
    let ext = dest
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 1..100u32 {
        let name = if n == 1 {
            format!("{stem} (from old PC){ext}")
        } else {
            format!("{stem} (from old PC {n}){ext}")
        };
        let cand = dest.with_file_name(name);
        if !cand.exists() {
            return Some(cand);
        }
    }
    None
}

/// Copy one folder group's tree into `target` with keep-both conflict handling.
fn import_folder(src: &Path, target: &Path, label: &str, note: Option<String>) -> ImportEntry {
    let mut copied = 0u64;
    let mut kept_both = 0u64;
    let mut errors = 0u64;
    let mut first_err: Option<String> = None;

    for e in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        if !e.file_type().is_file() {
            continue;
        }
        let rel = match e.path().strip_prefix(src) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let mut dest = safe_join(target, &rel);
        if dest.exists() {
            match keep_both_path(&dest) {
                Some(d) => {
                    kept_both += 1;
                    dest = d;
                }
                None => {
                    errors += 1;
                    continue;
                }
            }
        }
        let ok = dest
            .parent()
            .map(|p| fs::create_dir_all(p).is_ok())
            .unwrap_or(false)
            && match fs::copy(e.path(), &dest) {
                Ok(_) => true,
                Err(err) => {
                    if first_err.is_none() {
                        first_err = Some(err.to_string());
                    }
                    false
                }
            };
        if ok {
            copied += 1;
        } else {
            errors += 1;
        }
    }

    let mut details: Vec<String> = Vec::new();
    if let Some(n) = note {
        details.push(n);
    }
    if kept_both > 0 {
        details.push(format!("{kept_both} kept both"));
    }
    if errors > 0 {
        details.push(format!(
            "{errors} failed{}",
            first_err.map(|e| format!(" ({e})")).unwrap_or_default()
        ));
    }
    ImportEntry {
        label: label.to_string(),
        action: if copied == 0 && errors > 0 { "error" } else { "imported" }.into(),
        count: copied,
        detail: if details.is_empty() {
            None
        } else {
            Some(details.join(" · "))
        },
    }
}

/// Import one received Browser/<Label> dir — only into a fresh profile.
fn import_browser(src: &Path, label: &str) -> ImportEntry {
    let entry = |action: &str, count: u64, detail: Option<String>| ImportEntry {
        label: format!("{label} (browser)"),
        action: action.into(),
        count,
        detail,
    };

    let target = match browser_target_by_label(label) {
        Some(t) => t,
        None => {
            return entry(
                "skipped-not-installed",
                0,
                Some(format!(
                    "{label} isn't installed on this PC (or has never been opened)."
                )),
            )
        }
    };

    // incoming files are a flat dir; Local State targets the root, rest the profile
    let files: Vec<(PathBuf, String)> = match fs::read_dir(src) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                Some((e.path(), name))
            })
            .collect(),
        Err(e) => return entry("error", 0, Some(e.to_string())),
    };
    let dest_for = |name: &str| {
        if name == "Local State" {
            target.root.join(name)
        } else {
            target.profile.join(name)
        }
    };

    // fresh check: any incoming name already present ⇒ skip the whole browser
    if files.iter().any(|(_, name)| dest_for(name).exists()) {
        return entry(
            "skipped-not-fresh",
            0,
            Some(format!(
                "{label} already has data on this PC — sign in and sync instead."
            )),
        );
    }

    let mut copied = 0u64;
    for (path, name) in &files {
        let dest = dest_for(name);
        if let Some(p) = dest.parent() {
            let _ = fs::create_dir_all(p);
        }
        if let Err(e) = fs::copy(path, &dest) {
            return entry(
                "error",
                copied,
                Some(format!("close {label} and retry — {e}")),
            );
        }
        copied += 1;
    }
    entry("imported", copied, None)
}

/// Walk the top level of a crossing dump and route each group into place.
pub fn import_received(dump: &Path) -> ImportReport {
    let mut entries: Vec<ImportEntry> = Vec::new();

    let known: [(&str, Option<PathBuf>); 6] = [
        ("Documents", dirs::document_dir()),
        ("Desktop", dirs::desktop_dir()),
        ("Pictures", dirs::picture_dir()),
        ("Downloads", dirs::download_dir()),
        ("Music", dirs::audio_dir()),
        ("Videos", dirs::video_dir()),
    ];

    let rd = match fs::read_dir(dump) {
        Ok(r) => r,
        Err(e) => {
            return ImportReport {
                entries: vec![ImportEntry {
                    label: "Received folder".into(),
                    action: "error".into(),
                    count: 0,
                    detail: Some(e.to_string()),
                }],
            }
        }
    };

    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();

        if name == "Browser" {
            let Ok(browsers) = fs::read_dir(&p) else { continue };
            for b in browsers.filter_map(|b| b.ok()).filter(|b| b.path().is_dir()) {
                let label = b.file_name().to_string_lossy().to_string();
                entries.push(import_browser(&b.path(), &label));
            }
            continue;
        }

        match known.iter().find(|(l, _)| *l == name) {
            Some((label, Some(target))) => {
                entries.push(import_folder(&p, target, label, None));
            }
            Some((label, None)) => entries.push(ImportEntry {
                label: label.to_string(),
                action: "error".into(),
                count: 0,
                detail: Some("couldn't resolve this folder on the new PC".into()),
            }),
            // custom folder from the sender's picker → Documents\<name>
            None => {
                let docs = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
                let target = docs.join(&name);
                entries.push(import_folder(
                    &p,
                    &target,
                    &name,
                    Some(format!("→ Documents\\{name}")),
                ));
            }
        }
    }

    ImportReport { entries }
}
