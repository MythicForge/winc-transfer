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

/* ---------------- browser catalog ----------------
 * Three families, each data-driven so adding a browser is one table row.
 * Chromium family: profile lives in %LOCALAPPDATA%\<dir>\Default.
 * Opera family:    Chromium-based but under %APPDATA%, and older installs keep
 *                  the profile files directly in the dir (no Default\).
 * Gecko family:    Firefox-style %APPDATA%\<dir>\Profiles\<xxx.default*>\.
 */

/// (group id, label, %LOCALAPPDATA%-relative "User Data" dir)
const CHROMIUM_BROWSERS: &[(&str, &str, &str)] = &[
    ("chrome", "Chrome", "Google/Chrome/User Data"),
    ("edge", "Edge", "Microsoft/Edge/User Data"),
    ("brave", "Brave", "BraveSoftware/Brave-Browser/User Data"),
    ("vivaldi", "Vivaldi", "Vivaldi/User Data"),
    ("chromium", "Chromium", "Chromium/User Data"),
];

/// (group id, label, %APPDATA%-relative profile dir)
const OPERA_BROWSERS: &[(&str, &str, &str)] = &[
    ("opera", "Opera", "Opera Software/Opera Stable"),
    ("opera-gx", "Opera GX", "Opera Software/Opera GX Stable"),
];

/// (group id, label, %APPDATA%-relative Profiles root)
const GECKO_BROWSERS: &[(&str, &str, &str)] = &[
    ("firefox", "Firefox", "Mozilla/Firefox/Profiles"),
    ("zen", "Zen", "zen/Profiles"),
    ("librewolf", "LibreWolf", "librewolf/Profiles"),
    ("waterfox", "Waterfox", "Waterfox/Profiles"),
    ("floorp", "Floorp", "Floorp/Profiles"),
];

const CHROMIUM_HISTORY: &[&str] =
    &["Bookmarks", "History", "Favicons", "Top Sites", "Shortcuts", "Preferences"];
const CHROMIUM_PASSWORDS: &[&str] = &["Login Data", "Login Data For Account"];
/// Gecko keeps logins portable (logins.json + key4.db), so they travel with the
/// main group instead of a separate DPAPI-caveat group.
const GECKO_FILES: &[&str] = &["places.sqlite", "favicons.sqlite", "logins.json", "key4.db"];

/// The specific files that make up a browser group, as (abs, rel).
/// Ids: the table id for bookmarks/history, or "<id>-pw" for saved passwords.
fn browser_files(id: &str) -> Vec<Item> {
    let (base_id, pw) = match id.strip_suffix("-pw") {
        Some(b) => (b, true),
        None => (id, false),
    };
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

    // Chromium + Opera share the same file layout; they differ only in where
    // the "User Data"-style dir lives and whether a Default\ subdir exists.
    let chromium_like = CHROMIUM_BROWSERS
        .iter()
        .find(|(i, ..)| *i == base_id)
        .and_then(|(_, label, dir)| env_dir("LOCALAPPDATA").map(|l| (*label, l.join(dir))))
        .or_else(|| {
            OPERA_BROWSERS
                .iter()
                .find(|(i, ..)| *i == base_id)
                .and_then(|(_, label, dir)| env_dir("APPDATA").map(|a| (*label, a.join(dir))))
        });
    if let Some((label, root)) = chromium_like {
        let profile = if root.join("Default").is_dir() {
            root.join("Default")
        } else {
            root.clone()
        };
        if pw {
            add(&profile, label, CHROMIUM_PASSWORDS);
            add(&root, label, &["Local State"]); // holds the DPAPI-wrapped key
        } else {
            add(&profile, label, CHROMIUM_HISTORY);
        }
        return out;
    }

    if let Some((_, label, dir)) = GECKO_BROWSERS.iter().find(|(i, ..)| *i == base_id) {
        if let Some(a) = env_dir("APPDATA") {
            if let Some(profile) = gecko_profile(&a.join(dir)) {
                add(&profile, label, GECKO_FILES);
            }
        }
    }
    out
}

/// Pick the default profile dir under a Firefox-style Profiles root.
fn gecko_profile(base: &Path) -> Option<PathBuf> {
    let mut best: Option<PathBuf> = None;
    for e in std::fs::read_dir(base).ok()?.filter_map(|e| e.ok()) {
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

    // Every browser from the catalog that actually exists on this PC:
    // bookmarks & history for all three families, then saved-password groups
    // for the Chromium-family ones (Gecko logins travel with the main group).
    let mut browsers: Vec<(String, String, Option<&str>, bool)> = Vec::new();
    for (id, label, _) in CHROMIUM_BROWSERS.iter().chain(OPERA_BROWSERS).chain(GECKO_BROWSERS) {
        browsers.push((
            id.to_string(),
            format!("{label} — bookmarks & history"),
            None,
            *id == "chrome",
        ));
    }
    for (id, label, _) in CHROMIUM_BROWSERS.iter().chain(OPERA_BROWSERS) {
        browsers.push((
            format!("{id}-pw"),
            format!("{label} — saved passwords"),
            Some(PW_CAVEAT),
            false,
        ));
    }
    for (id, label, caveat, on) in browsers {
        let files = browser_files(&id);
        if files.is_empty() {
            continue;
        }
        let bytes: u64 = files
            .iter()
            .filter_map(|i| std::fs::metadata(&i.abs).ok().map(|m| m.len()))
            .sum();
        groups.push(SourceGroup {
            id,
            label,
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
