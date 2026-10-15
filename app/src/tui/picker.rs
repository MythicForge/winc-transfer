//! The TUI's file browser — what Explorer's pickers are in the window. Over
//! SSH there's no dialog to open, so folders and files are picked in the
//! terminal: arrows / click to move, Enter / double-click to open, Space /
//! click on the box to mark, or type a path (Tab completes it).

use super::input::LineEdit;
use crate::ui::PickInto;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What the browser picks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickKind {
    /// one folder: the one you're in
    Folder,
    /// any number of marked entries; `dirs_only` hides files
    Many { dirs_only: bool },
}

/// Where the result goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickFor {
    /// the receiver's drop folder
    Drop,
    /// the Done screen's "A folder you choose"
    ImportFolder,
    /// Custom Transfer picks / an extra User Transfer folder
    Into(PickInto),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub dir: bool,
    pub size: u64,
    /// the ".." row
    pub up: bool,
}

#[derive(Clone, Debug)]
pub struct Picker {
    pub kind: PickKind,
    pub purpose: PickFor,
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    /// first list row shown
    pub scroll: usize,
    /// list rows that fit, set by the last draw
    pub page: usize,
    pub marked: Vec<PathBuf>,
    pub hidden: bool,
    pub path: LineEdit,
    pub err: Option<String>,
    last_click: Option<(usize, Instant)>,
}

const DOUBLE_CLICK: Duration = Duration::from_millis(450);

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// "~/x" → "$HOME/x"; relative paths are taken from `base`.
pub fn expand(s: &str, base: &Path) -> PathBuf {
    let s = s.trim();
    if s == "~" {
        return home();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return home().join(rest);
    }
    let p = PathBuf::from(s);
    if p.is_absolute() {
        p
    } else {
        base.join(p)
    }
}

/// Entries of `dir`: ".." first (unless at the root), then folders, then
/// files, each A→Z ignoring case.
pub fn list(dir: &Path, hidden: bool, dirs_only: bool) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("can't open {}: {e}", dir.display()))?;
    let mut out: Vec<Entry> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                return None;
            }
            let path = e.path();
            // follow symlinks, so a linked folder opens like a folder
            let meta = std::fs::metadata(&path).ok();
            let dir = meta.as_ref().is_some_and(|m| m.is_dir());
            if dirs_only && !dir {
                return None;
            }
            Some(Entry {
                size: meta.filter(|m| m.is_file()).map_or(0, |m| m.len()),
                name,
                path,
                dir,
                up: false,
            })
        })
        .collect();
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    if let Some(parent) = dir.parent() {
        out.insert(
            0,
            Entry {
                name: "..".into(),
                path: parent.to_path_buf(),
                dir: true,
                size: 0,
                up: true,
            },
        );
    }
    Ok(out)
}

/// Tab completion for a typed path: the one match in full (folders get a
/// trailing '/'), or as far as several matches agree. None = nothing to add.
pub fn complete(input: &str, base: &Path, hidden: bool) -> Option<String> {
    let (head, prefix) = match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None => ("", input),
    };
    let dir = if head.is_empty() { base.to_path_buf() } else { expand(head, base) };
    let mut names: Vec<(String, bool)> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().is_dir()))
        .filter(|(n, _)| n.starts_with(prefix) && (hidden || prefix.starts_with('.') || !n.starts_with('.')))
        .collect();
    names.sort();
    let out = match names.as_slice() {
        [] => return None,
        [(n, d)] => format!("{head}{n}{}", if *d { "/" } else { "" }),
        many => {
            let first = &many[0].0;
            let mut common = first.len();
            for (n, _) in &many[1..] {
                common = common.min(
                    first
                        .char_indices()
                        .zip(n.chars())
                        .take_while(|((_, a), b)| a == b)
                        .last()
                        .map_or(0, |((i, a), _)| i + a.len_utf8()),
                );
            }
            format!("{head}{}", &first[..common])
        }
    };
    (out != input).then_some(out)
}

impl Picker {
    pub fn open(kind: PickKind, purpose: PickFor, start: Option<&str>) -> Self {
        let dir = start.map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(home);
        let mut p = Picker {
            kind,
            purpose,
            dir: PathBuf::new(),
            entries: Vec::new(),
            cursor: 0,
            scroll: 0,
            page: 10,
            marked: Vec::new(),
            hidden: false,
            path: LineEdit::default(),
            err: None,
            last_click: None,
        };
        p.cd(dir);
        p
    }

    fn dirs_only(&self) -> bool {
        match self.kind {
            PickKind::Folder => true,
            PickKind::Many { dirs_only } => dirs_only,
        }
    }

    pub fn multi(&self) -> bool {
        matches!(self.kind, PickKind::Many { .. })
    }

    /// Go to `dir` (stays put, with an error line, if it can't be read).
    pub fn cd(&mut self, dir: PathBuf) {
        let from = std::mem::take(&mut self.dir);
        match list(&dir, self.hidden, self.dirs_only()) {
            Ok(entries) => {
                // coming back up: land on the folder we came out of
                self.cursor = entries.iter().position(|e| !e.up && e.path == from).unwrap_or(0);
                self.entries = entries;
                self.dir = dir;
                self.scroll = 0;
                self.err = None;
                self.path.set(format!("{}/", self.dir.display()).replace("//", "/"));
            }
            Err(e) => {
                self.dir = from;
                self.err = Some(e);
            }
        }
        self.keep_in_view();
    }

    pub fn reload(&mut self) {
        let keep = self.entries.get(self.cursor).map(|e| e.path.clone());
        let dir = self.dir.clone();
        self.cd(dir);
        if let Some(k) = keep {
            if let Some(i) = self.entries.iter().position(|e| e.path == k) {
                self.cursor = i;
            }
        }
        self.keep_in_view();
    }

    pub fn toggle_hidden(&mut self) {
        self.hidden = !self.hidden;
        self.reload();
    }

    pub fn parent(&mut self) {
        if let Some(p) = self.dir.parent() {
            self.cd(p.to_path_buf());
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
        self.keep_in_view();
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let max = self.entries.len().saturating_sub(self.page) as isize;
        self.scroll = (self.scroll as isize + delta).clamp(0, max.max(0)) as usize;
    }

    pub fn keep_in_view(&mut self) {
        let page = self.page.max(1);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + page {
            self.scroll = self.cursor + 1 - page;
        }
    }

    /// Enter on the highlighted row: open a folder; mark a file.
    pub fn activate(&mut self) {
        let Some(e) = self.entries.get(self.cursor).cloned() else { return };
        if e.dir {
            self.cd(e.path);
        } else if self.multi() {
            self.toggle_mark();
        }
    }

    pub fn toggle_mark(&mut self) {
        if !self.multi() {
            return;
        }
        let Some(e) = self.entries.get(self.cursor) else { return };
        if e.up {
            return;
        }
        match self.marked.iter().position(|m| *m == e.path) {
            Some(i) => {
                self.marked.remove(i);
            }
            None => self.marked.push(e.path.clone()),
        }
    }

    pub fn is_marked(&self, p: &Path) -> bool {
        self.marked.iter().any(|m| m == p)
    }

    /// A click on list row `i`; a second click soon after on the same row opens it.
    pub fn click(&mut self, i: usize) {
        if i >= self.entries.len() {
            return;
        }
        let double = self
            .last_click
            .is_some_and(|(j, t)| j == i && t.elapsed() < DOUBLE_CLICK);
        self.cursor = i;
        if double {
            self.last_click = None;
            self.activate();
        } else {
            self.last_click = Some((i, Instant::now()));
        }
    }

    /// A click on row `i`'s checkbox.
    pub fn click_mark(&mut self, i: usize) {
        if i < self.entries.len() {
            self.cursor = i;
            self.toggle_mark();
        }
    }

    /// Tab in the path field.
    pub fn complete_path(&mut self) -> bool {
        match complete(&self.path.text, &self.dir, self.hidden) {
            Some(s) => {
                self.path.set(s);
                true
            }
            None => false,
        }
    }

    /// Enter in the path field: go to a folder, or mark a file.
    pub fn submit_path(&mut self) {
        let p = expand(&self.path.text, &self.dir);
        if p.is_dir() {
            self.cd(p);
        } else if p.is_file() && self.multi() && !self.dirs_only() {
            if !self.is_marked(&p) {
                self.marked.push(p.clone());
            }
            if let Some(parent) = p.parent() {
                self.cd(parent.to_path_buf());
            }
            if let Some(i) = self.entries.iter().position(|e| e.path == p) {
                self.cursor = i;
                self.keep_in_view();
            }
        } else {
            self.err = Some(format!("{} — no such folder", p.display()));
        }
    }

    /// What the confirm button would hand back, and its label. None = nothing to pick yet.
    pub fn result(&self) -> Option<(Vec<String>, String)> {
        match self.kind {
            PickKind::Folder => Some((vec![self.dir.display().to_string()], format!("Use {}", self.dir.display()))),
            PickKind::Many { .. } if !self.marked.is_empty() => {
                let n = self.marked.len();
                Some((
                    self.marked.iter().map(|p| p.display().to_string()).collect(),
                    format!("Add {n} marked item{}", if n == 1 { "" } else { "s" }),
                ))
            }
            PickKind::Many { .. } => {
                let e = self.entries.get(self.cursor).filter(|e| !e.up)?;
                Some((vec![e.path.display().to_string()], format!("Add {}", e.name)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> PathBuf {
        let root = std::env::temp_dir().join(format!("winci-picker-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Documents/inner")).unwrap();
        std::fs::create_dir_all(root.join("Downloads")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("notes.txt"), b"12345").unwrap();
        std::fs::write(root.join("Documents/a.txt"), b"x").unwrap();
        root
    }

    #[test]
    fn listing_order_and_hidden() {
        let root = tree();
        let names: Vec<String> = list(&root, false, false).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["..", "Documents", "Downloads", "notes.txt"]);
        let names: Vec<String> = list(&root, true, true).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["..", ".hidden", "Documents", "Downloads"]);
        let notes = list(&root, false, false).unwrap().pop().unwrap();
        assert_eq!((notes.dir, notes.size), (false, 5));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tab_completion() {
        let root = tree();
        let r = root.display().to_string();
        // unique: full name, folders get a slash
        assert_eq!(complete(&format!("{r}/Doc"), &root, false), Some(format!("{r}/Documents/")));
        // shared prefix only
        assert_eq!(complete(&format!("{r}/D"), &root, false), Some(format!("{r}/Do")));
        assert_eq!(complete(&format!("{r}/Do"), &root, false), None);
        // relative to the folder being browsed
        assert_eq!(complete("no", &root, false), Some("notes.txt".into()));
        // hidden only when asked for
        assert_eq!(complete(&format!("{r}/.h"), &root, false), Some(format!("{r}/.hidden/")));
        assert_eq!(complete(&format!("{r}/zz"), &root, false), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn browse_mark_and_pick() {
        let root = tree();
        let mut p = Picker::open(PickKind::Many { dirs_only: false }, PickFor::Into(PickInto::Picks), Some(&root.display().to_string()));
        assert_eq!(p.entries[p.cursor].name, "..");
        // nothing marked: the highlighted row is the pick; ".." isn't pickable
        assert!(p.result().is_none());
        p.move_by(1);
        assert_eq!(p.result().unwrap().1, "Add Documents");
        p.toggle_mark();
        p.move_by(2);
        p.activate(); // a file: marks it
        assert_eq!(p.result().unwrap().0.len(), 2);
        // into Documents and back out lands on Documents
        p.move_by(-2);
        p.activate();
        assert!(p.dir.ends_with("Documents"));
        p.parent();
        assert_eq!(p.entries[p.cursor].name, "Documents");
        // typed path to a file marks it once
        let typed = format!("{}/Documents/a.txt", root.display());
        p.path.set(typed.clone());
        p.submit_path();
        p.path.set(typed);
        p.submit_path();
        assert_eq!(p.marked.len(), 3);
        assert!(p.dir.ends_with("Documents"));
        assert_eq!(p.entries[p.cursor].name, "a.txt");

        let mut f = Picker::open(PickKind::Folder, PickFor::Drop, Some(&root.display().to_string()));
        assert!(f.entries.iter().all(|e| e.dir));
        f.path.set("Downloads");
        f.submit_path();
        assert_eq!(f.result().unwrap().0, vec![root.join("Downloads").display().to_string()]);
        f.path.set("nope");
        f.submit_path();
        assert!(f.err.is_some());
        let _ = std::fs::remove_dir_all(&root);
    }
}
