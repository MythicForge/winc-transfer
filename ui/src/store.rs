//! App state: one signal per field, shared through context. Mirrors the old
//! React reducer — every action is a method here.

use crate::api;
use crate::types::*;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Connect,
    Pair,
    Select,
    Code,
    Transfer,
    Done,
}

#[derive(Clone, Copy)]
pub struct Store {
    pub role: RwSignal<Option<Role>>,
    pub step: RwSignal<Step>,
    pub link: RwSignal<LinkStatus>,
    pub peer: RwSignal<Option<Peer>>,
    /// receiver's own address, shown for manual pairing
    pub self_peer: RwSignal<Option<Peer>>,
    /// shown (receive) or entered (send)
    pub code: RwSignal<String>,
    /// sender: preset "User Transfer" or explorer "Custom Transfer"
    pub mode: RwSignal<TransferMode>,
    pub sources: RwSignal<Vec<SourceGroup>>,
    /// Custom Transfer list, all sent (no checkboxes)
    pub picks: RwSignal<Vec<SourceGroup>>,
    pub progress: RwSignal<TransferProgress>,
    /// where the crossing landed, for "Import into place"
    pub received_dir: RwSignal<Option<String>>,
}

impl Store {
    fn new() -> Self {
        Self {
            role: RwSignal::new(None),
            step: RwSignal::new(Step::Connect),
            link: RwSignal::new(LinkStatus::default()),
            peer: RwSignal::new(None),
            self_peer: RwSignal::new(None),
            code: RwSignal::new(String::new()),
            mode: RwSignal::new(TransferMode::User),
            sources: RwSignal::new(Vec::new()),
            picks: RwSignal::new(Vec::new()),
            progress: RwSignal::new(TransferProgress::default()),
            received_dir: RwSignal::new(None),
        }
    }

    pub fn set_role(&self, role: Role) {
        self.role.set(Some(role));
        self.step.set(Step::Connect);
    }

    pub fn toggle(&self, id: &str) {
        self.sources.update(|s| {
            if let Some(g) = s.iter_mut().find(|g| g.id == id) {
                g.selected = !g.selected;
            }
        });
    }

    pub fn select_all(&self, on: bool) {
        self.sources.update(|s| s.iter_mut().for_each(|g| g.selected = on));
    }

    pub fn add_picks(&self, add: Vec<SourceGroup>) {
        self.picks.update(|cur| merge_picks(cur, add));
    }

    /// Reset the UI *and* stop any in-flight backend session (unblocks the
    /// receiver's accept loop, ends transfers, stops the beacon). Use for every
    /// "Start over" so the backend doesn't stay stuck listening.
    pub fn reset(&self) {
        spawn_local(async {
            let _ = api::cancel().await;
        });
        self.role.set(None);
        self.step.set(Step::Connect);
        self.link.set(LinkStatus::default());
        self.peer.set(None);
        self.self_peer.set(None);
        self.code.set(String::new());
        self.mode.set(TransferMode::User);
        self.sources.set(Vec::new());
        self.picks.set(Vec::new());
        self.progress.set(TransferProgress::default());
        self.received_dir.set(None);
    }
}

/// Windows paths: case-insensitive, either slash.
fn norm(p: &str) -> String {
    p.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn inside(child: &str, parent: &str) -> bool {
    norm(child).starts_with(&(norm(parent) + "\\"))
}

/// Add explorer picks without sending anything twice: skip exact repeats and
/// anything already covered by a picked folder; a new folder swallows earlier
/// picks that live inside it.
fn merge_picks(cur: &mut Vec<SourceGroup>, add: Vec<SourceGroup>) {
    for p in add {
        let path = p.path.clone().unwrap_or_default();
        let covered = cur.iter().any(|o| {
            let op = o.path.as_deref().unwrap_or("");
            norm(op) == norm(&path) || (o.kind == "folder" && inside(&path, op))
        });
        if covered {
            continue;
        }
        if p.kind == "folder" {
            cur.retain(|o| !inside(o.path.as_deref().unwrap_or(""), &path));
        }
        cur.push(p);
    }
}

pub fn provide_store() {
    provide_context(Store::new());
}

pub fn use_store() -> Store {
    expect_context::<Store>()
}
