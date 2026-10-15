use crate::model::SourceGroup;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const PW_CAVEAT: &str = "Saved passwords are locked to this Windows account (DPAPI). They copy across but only unlock if you sign into the new PC with the same Microsoft account.";

/// Concrete file to move: absolute source + destination-relative path.
pub struct Item {
    pub abs: PathBuf,
    pub rel: String,
    /// Taken during the walk, so the manifest needs no second stat per file —
    /// on a few hundred thousand files that pass alone took minutes.
    pub size: u64,
}

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

pub(crate) fn env_dir(key: &str) -> Option<PathBuf> {
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
pub(crate) const CHROMIUM_BROWSERS: &[(&str, &str, &str)] = &[
    ("chrome", "Chrome", "Google/Chrome/User Data"),
    ("edge", "Edge", "Microsoft/Edge/User Data"),
    ("brave", "Brave", "BraveSoftware/Brave-Browser/User Data"),
    ("vivaldi", "Vivaldi", "Vivaldi/User Data"),
    ("chromium", "Chromium", "Chromium/User Data"),
];

/// (group id, label, %APPDATA%-relative profile dir)
pub(crate) const OPERA_BROWSERS: &[(&str, &str, &str)] = &[
    ("opera", "Opera", "Opera Software/Opera Stable"),
    ("opera-gx", "Opera GX", "Opera Software/Opera GX Stable"),
];

/// (group id, label, %APPDATA%-relative Profiles root)
pub(crate) const GECKO_BROWSERS: &[(&str, &str, &str)] = &[
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

/// The specific files that make up a browser group, as (abs, rel). Each group
/// carries bookmarks/history AND saved logins: Chromium/Opera add Login Data +
/// Local State, Gecko bundles logins.json + key4.db via GECKO_FILES.
fn browser_files(id: &str) -> Vec<Item> {
    let base_id = id;
    let mut out = Vec::new();
    let mut add = |base: &Path, rel_root: &str, names: &[&str]| {
        for n in names {
            let abs = base.join(n);
            if abs.exists() {
                out.push(Item {
                    size: file_size(&abs),
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
        add(&profile, label, CHROMIUM_HISTORY);
        add(&profile, label, CHROMIUM_PASSWORDS);
        add(&root, label, &["Local State"]); // holds the DPAPI-wrapped key
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
pub(crate) fn gecko_profile(base: &Path) -> Option<PathBuf> {
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
        let Some(p) = path.filter(|p| p.exists()) else {
            log::debug!("source '{label}' not present on this PC — skipped");
            continue;
        };
        let (bytes, items) = measure_dir(&p);
        log::debug!(
            "source '{label}': {items} file(s), {:.2} GB at {}",
            bytes as f64 / 1e9,
            p.display()
        );
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

    // Every browser from the catalog that actually exists on this PC. Each
    // group now carries bookmarks, history AND saved logins in one checkbox
    // (matching how Gecko already bundled its logins), so passwords ride along
    // instead of sitting in a separate opt-in group the user had to find.
    // Chromium/Opera keep the DPAPI caveat; Gecko logins are portable.
    let mut browsers: Vec<(String, String, Option<&str>, bool)> = Vec::new();
    for (id, label, _) in CHROMIUM_BROWSERS.iter().chain(OPERA_BROWSERS) {
        browsers.push((
            id.to_string(),
            format!("{label} — bookmarks, history & passwords"),
            Some(PW_CAVEAT),
            *id == "chrome",
        ));
    }
    for (id, label, _) in GECKO_BROWSERS.iter() {
        browsers.push((
            id.to_string(),
            format!("{label} — bookmarks, history & passwords"),
            None,
            false,
        ));
    }
    for (id, label, caveat, on) in browsers {
        let files = browser_files(&id);
        if files.is_empty() {
            log::debug!("browser '{id}': no profile files found — not offered");
            continue;
        }
        log::debug!("browser '{id}': {} file(s) found", files.len());
        let bytes: u64 = files.iter().map(|i| i.size).sum();
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

/// Size up paths picked in the explorer (Custom Transfer, or "+ Add a folder")
/// so the UI can show real totals. Paths that vanished since picking are dropped.
pub fn measure_paths(paths: &[String]) -> Vec<SourceGroup> {
    let mut out = Vec::new();
    for raw in paths {
        let p = Path::new(raw);
        let (kind, bytes, items) = if p.is_file() {
            ("file", std::fs::metadata(p).map(|m| m.len()).unwrap_or(0), 1)
        } else if p.is_dir() {
            let (b, n) = measure_dir(p);
            ("folder", b, n)
        } else {
            log::warn!("picked path no longer exists — skipped: {raw}");
            continue;
        };
        log::debug!("picked {kind} '{raw}': {items} file(s), {bytes} bytes");
        out.push(SourceGroup {
            id: format!("custom-{raw}"),
            label: p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| raw.clone()),
            hint: raw.clone(),
            kind: kind.into(),
            path: Some(raw.clone()),
            bytes,
            items,
            caveat: None,
            selected: true,
        });
    }
    out
}

/// Expand every selected group id into concrete files to stream, as
/// (id, files) so the caller can log each group.
///
/// Explorer picks (id = "custom-<absolute path>", a file or a folder) land
/// under Custom/<name> on the wire, so a pick named "Documents" or "Browser"
/// can't collide with the preset groups. Picks sharing a name get " (2)",
/// " (3)"... so they stay separate on the receiver instead of merging.
pub fn expand_all(ids: &[String]) -> Vec<(String, Vec<Item>)> {
    let mut used: HashSet<String> = HashSet::new();
    ids.iter()
        .map(|id| {
            let items = match id.strip_prefix("custom-") {
                Some(path) => {
                    let path = Path::new(path);
                    let name = safe(
                        &path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "Picked".into()),
                    );
                    expand_custom(path, &unique_name(&name, path.is_file(), &mut used))
                }
                None => expand(id),
            };
            (id.clone(), items)
        })
        .collect()
}

/// `name`, or the first free "name (n)" — before the extension for files.
fn unique_name(name: &str, is_file: bool, used: &mut HashSet<String>) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if is_file && i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut cand = name.to_string();
    let mut n = 2;
    // Windows paths are case-insensitive, so "Notes" and "notes" collide
    while !used.insert(cand.to_lowercase()) {
        cand = format!("{stem} ({n}){ext}");
        n += 1;
    }
    cand
}

fn expand_custom(path: &Path, name: &str) -> Vec<Item> {
    if path.is_file() {
        return vec![Item {
            abs: path.to_path_buf(),
            rel: format!("Custom/{name}"),
            size: file_size(path),
        }];
    }
    expand_folder(path, &format!("Custom/{name}"))
}

/// Expand a preset group id (user folder or browser) into concrete files.
fn expand(id: &str) -> Vec<Item> {
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
                    // walkdir's metadata comes from the directory listing on Windows — no extra syscall
                    size: e.metadata().map(|m| m.len()).unwrap_or(0),
                });
            }
        }
    }
    out
}
