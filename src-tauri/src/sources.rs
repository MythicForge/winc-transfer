use crate::model::SourceGroup;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const PW_CAVEAT: &str = "Saved passwords are locked to this Windows account (DPAPI). They copy across but only unlock if you sign into the new PC with the same Microsoft account.";

/// Concrete file to move: absolute source + destination-relative path.
pub struct Item {
    pub abs: PathBuf,
    pub rel: String,
}

fn env_dir(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from).filter(|p| p.exists())
}

fn safe(name: &str) -> String {
    name.chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect()
}

/// (bytes, files) for a directory tree.
fn measure_dir(root: &Path) -> (u64, u64) {
    let mut bytes = 0u64;
    let mut files = 0u64;
    for e in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if e.file_type().is_file() {
            if let Ok(m) = e.metadata() {
                bytes += m.len();
                files += 1;
            }
        }
    }
    (bytes, files)
}

/// The specific files that make up a browser group, as (abs, rel).
fn browser_files(id: &str) -> Vec<Item> {
    let local = env_dir("LOCALAPPDATA");
    let appdata = env_dir("APPDATA");
    let mut out = Vec::new();
    let mut add = |base: &Path, rel_root: &str, names: &[&str]| {
        for n in names {
            let abs = base.join(n);
            if abs.exists() {
                out.push(Item {
                    abs,
                    rel: format!("Browser/{}/{}", rel_root, n),
                });
            }
        }
    };
    let history = &["Bookmarks", "History", "Favicons", "Top Sites", "Shortcuts", "Preferences"];
    let passwords = &["Login Data", "Login Data For Account", "Local State"];

    match id {
        "chrome" => {
            if let Some(l) = &local {
                add(&l.join("Google/Chrome/User Data/Default"), "Chrome", history);
            }
        }
        "chrome-pw" => {
            if let Some(l) = &local {
                let ud = l.join("Google/Chrome/User Data");
                add(&ud.join("Default"), "Chrome", &["Login Data", "Login Data For Account"]);
                add(&ud, "Chrome", &["Local State"]);
            }
        }
        "edge" => {
            if let Some(l) = &local {
                add(&l.join("Microsoft/Edge/User Data/Default"), "Edge", history);
            }
        }
        "edge-pw" => {
            if let Some(l) = &local {
                let ud = l.join("Microsoft/Edge/User Data");
                add(&ud.join("Default"), "Edge", passwords);
            }
        }
        "firefox" => {
            if let Some(a) = &appdata {
                if let Some(profile) = firefox_profile(a) {
                    add(&profile, "Firefox", &["places.sqlite", "favicons.sqlite", "logins.json", "key4.db"]);
                }
            }
        }
        _ => {}
    }
    out
}

fn firefox_profile(appdata: &Path) -> Option<PathBuf> {
    let base = appdata.join("Mozilla/Firefox/Profiles");
    let mut best: Option<PathBuf> = None;
    for e in std::fs::read_dir(&base).ok()?.filter_map(|e| e.ok()) {
        let p = e.path();
        let name = p.file_name()?.to_string_lossy().to_string();
        if name.ends_with(".default-release") {
            return Some(p);
        }
        if name.contains("default") {
            best = Some(p);
        }
    }
    best
}

/// Build the list shown in the UI. Only groups that exist on this PC appear.
pub fn list_sources() -> Vec<SourceGroup> {
    let mut groups: Vec<SourceGroup> = Vec::new();

    let folders: [(&str, Option<PathBuf>); 6] = [
        ("Documents", dirs::document_dir()),
        ("Desktop", dirs::desktop_dir()),
        ("Pictures", dirs::picture_dir()),
        ("Downloads", dirs::download_dir()),
        ("Music", dirs::audio_dir()),
        ("Videos", dirs::video_dir()),
    ];
    for (label, path) in folders {
        if let Some(p) = path.filter(|p| p.exists()) {
            let (bytes, items) = measure_dir(&p);
            groups.push(SourceGroup {
                id: label.to_lowercase(),
                label: label.to_string(),
                hint: p.display().to_string(),
                kind: "folder".into(),
                path: Some(p.display().to_string()),
                bytes,
                items,
                caveat: None,
                selected: matches!(label, "Documents" | "Desktop" | "Pictures"),
            });
        }
    }

    let browsers: [(&str, &str, Option<&str>, bool); 5] = [
        ("chrome", "Chrome — bookmarks & history", None, true),
        ("edge", "Edge — bookmarks & history", None, false),
        ("firefox", "Firefox — bookmarks & history", None, false),
        ("chrome-pw", "Chrome — saved passwords", Some(PW_CAVEAT), false),
        ("edge-pw", "Edge — saved passwords", Some(PW_CAVEAT), false),
    ];
    for (id, label, caveat, on) in browsers {
        let files = browser_files(id);
        if files.is_empty() {
            continue;
        }
        let bytes: u64 = files
            .iter()
            .filter_map(|i| std::fs::metadata(&i.abs).ok().map(|m| m.len()))
            .sum();
        groups.push(SourceGroup {
            id: id.to_string(),
            label: label.to_string(),
            hint: "Default profile".into(),
            kind: "browser".into(),
            path: None,
            bytes,
            items: files.len() as u64,
            caveat: caveat.map(|c| c.to_string()),
            selected: on,
        });
    }

    groups
}

/// Expand a selected group id into concrete files to stream.
pub fn expand(id: &str) -> Vec<Item> {
    // custom folder added via the picker: id = "custom-<absolute path>"
    if let Some(path) = id.strip_prefix("custom-") {
        return expand_folder(Path::new(path), &safe(Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default().as_str()));
    }
    let folder = match id {
        "documents" => dirs::document_dir().map(|p| (p, "Documents")),
        "desktop" => dirs::desktop_dir().map(|p| (p, "Desktop")),
        "pictures" => dirs::picture_dir().map(|p| (p, "Pictures")),
        "downloads" => dirs::download_dir().map(|p| (p, "Downloads")),
        "music" => dirs::audio_dir().map(|p| (p, "Music")),
        "videos" => dirs::video_dir().map(|p| (p, "Videos")),
        _ => None,
    };
    if let Some((root, label)) = folder {
        return expand_folder(&root, &safe(label));
    }
    // otherwise a browser group
    browser_files(id)
}

fn expand_folder(root: &Path, label: &str) -> Vec<Item> {
    let mut out = Vec::new();
    for e in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if e.file_type().is_file() {
            if let Ok(rel) = e.path().strip_prefix(root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                out.push(Item {
                    abs: e.path().to_path_buf(),
                    rel: format!("{}/{}", label, rel),
                });
            }
        }
    }
    out
}
