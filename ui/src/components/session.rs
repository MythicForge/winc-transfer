use crate::api;
use crate::log;
use crate::store::{use_store, Step};
use crate::types::{Role, TransferMode, TransferProgress, XferState};
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::cell::Cell;
use std::rc::Rc;

/// Long-lived backend driver. Renders nothing.
/// Owns the send/receive calls so listeners survive step changes.
#[component]
pub fn SessionController() -> impl IntoView {
    let store = use_store();
    let started = StoredValue::new(None::<Role>);
    let last_state = Rc::new(Cell::new(XferState::Idle));

    let on_progress = move |p: TransferProgress| {
        // Log transitions only — progress ticks arrive several times a second.
        if p.state != last_state.get() {
            last_state.set(p.state);
            let detail = match p.state {
                XferState::Error => format!(" — {}", p.error.as_deref().unwrap_or("no reason given")),
                _ => format!(" — {}/{} files", p.files_sent, p.files_total),
            };
            let msg = format!("progress: {}{detail}", p.state.as_str());
            if p.state == XferState::Error {
                log::error("ui::transfer", &msg);
            } else {
                log::info("ui::transfer", &msg);
            }
        }
        let done = p.state == XferState::Done;
        store.progress.set(p);
        if done {
            store.step.set(Step::Done);
        }
    };

    Effect::new(move |_| {
        let role = store.role.get();
        let step = store.step.get();

        // SENDER: fire the stream once we reach the transfer step
        if role == Some(Role::Send) && step == Step::Transfer && started.get_value() != Some(Role::Send) {
            started.set_value(Some(Role::Send));
            let mode = store.mode.get_untracked();
            let ids: Vec<String> = match mode {
                TransferMode::Custom => store.picks.with_untracked(|p| p.iter().map(|g| g.id.clone()).collect()),
                TransferMode::User => store.sources.with_untracked(|s| {
                    s.iter().filter(|g| g.selected).map(|g| g.id.clone()).collect()
                }),
            };
            if store.peer.with_untracked(|p| p.is_some()) {
                let kind = if mode == TransferMode::Custom { "custom" } else { "user" };
                log::info("ui::transfer", &format!("starting {kind} send of {} group(s)", ids.len()));
                let cb = on_progress.clone();
                spawn_local(async move {
                    if let Err(e) = api::start_send(ids, cb).await {
                        log::error("ui::transfer", &format!("send failed: {e}"));
                    }
                });
            } else {
                log::error("ui::transfer", "reached the transfer step with no paired peer");
            }
        }

        // RECEIVER: open the listener + code as soon as the code step shows
        if role == Some(Role::Receive) && step == Step::Code && started.get_value() != Some(Role::Receive) {
            started.set_value(Some(Role::Receive));
            spawn_local(async move {
                match api::start_receiver("This PC").await {
                    Ok(info) => {
                        store.code.set(info.code);
                        store.self_peer.set(Some(info.peer));
                    }
                    Err(e) => log::error("ui::receive", &format!("could not start the receiver: {e}")),
                }
            });
            let cb = on_progress.clone();
            spawn_local(async move {
                let on_peer = move |peer: crate::types::Peer| {
                    log::info("ui::receive", &format!("sender {} ({}) connected", peer.name, peer.ip));
                    store.peer.set(Some(peer));
                    store.step.set(Step::Transfer);
                };
                match api::receive(on_peer, cb).await {
                    Ok(dir) => store.received_dir.set(Some(dir)),
                    Err(e) => log::error("ui::receive", &format!("receive failed: {e}")),
                }
            });
        }
    });
}
