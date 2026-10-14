use crate::api;
use crate::format::{bytes, count};
use crate::log;
use crate::store::{use_store, Step};
use crate::types::{SourceGroup, TransferMode};
use leptos::prelude::*;
use leptos::task::spawn_local;

const MODES: [(TransferMode, &str, &str); 2] = [
    (TransferMode::User, "User Transfer", "Your folders & browser data, found for you"),
    (TransferMode::Custom, "Custom Transfer", "Pick any files and folders in Explorer"),
];

/// Which list an Explorer pick lands in.
#[derive(Clone, Copy)]
enum Into_ {
    Picks,
    Sources,
}

/// Open Explorer and size up whatever comes back. None = cancelled.
async fn pick(directory: bool) -> Result<Option<Vec<SourceGroup>>, String> {
    let paths = api::pick_paths(directory).await?;
    if paths.is_empty() {
        return Ok(None);
    }
    api::measure_paths(paths).await.map(Some)
}

#[component]
pub fn SelectStep() -> impl IntoView {
    let store = use_store();
    let custom = move || store.mode.get() == TransferMode::Custom;
    let busy = RwSignal::new(false);

    // (groups chosen, bytes, items) for the current mode
    let chosen = Memo::new(move |_| {
        let list: Vec<SourceGroup> = if custom() {
            store.picks.get()
        } else {
            store.sources.with(|s| s.iter().filter(|g| g.selected).cloned().collect())
        };
        let b: u64 = list.iter().map(|s| s.bytes).sum();
        let n: u64 = list.iter().map(|s| s.items).sum();
        (list.len(), b, n)
    });
    let all_on = move || store.sources.with(|s| !s.is_empty() && s.iter().all(|g| g.selected));

    let run = move |directory: bool, into: Into_| {
        busy.set(true);
        spawn_local(async move {
            match pick(directory).await {
                Ok(Some(found)) => match into {
                    Into_::Picks => store.add_picks(found),
                    Into_::Sources => store.sources.update(|s| {
                        for f in found {
                            if !s.iter().any(|g| g.id == f.id) {
                                s.push(f);
                            }
                        }
                    }),
                },
                Ok(None) => {}
                Err(e) => log::error("ui::select", &format!("could not add from Explorer: {e}")),
            }
            busy.set(false);
        });
    };

    let busy_note = move || busy.get().then(|| view! { <span class="pick-busy">"Measuring…"</span> });

    let custom_list = move || {
        view! {
            <Show
                when=move || store.picks.with(|p| !p.is_empty())
                fallback=|| {
                    view! {
                        <div class="picks-empty">
                            "Nothing picked yet. Add folders or files from anywhere on this PC — other drives "
                            "included. They land in " <span class="mono">"Documents"</span> " on the new PC."
                        </div>
                    }
                }
            >
                <div class="sources">
                    <For each=move || store.picks.get() key=|s| s.id.clone() let:s>
                        <div class="source source--on source--pick">
                            <span class="source__glyph" aria-hidden="true">
                                {if s.kind == "file" { "▤" } else { "▣" }}
                            </span>
                            <span>
                                <span class="source__label">{s.label.clone()}</span>
                                <span class="source__hint">{s.hint.clone()}</span>
                            </span>
                            <span class="source__meta">
                                <b>{bytes(s.bytes as f64)}</b>
                                <br />
                                {if s.kind == "file" { "file".to_string() } else { format!("{} items", count(s.items)) }}
                            </span>
                            <button
                                class="source__remove"
                                title=format!("Remove {}", s.label)
                                aria-label=format!("Remove {}", s.label)
                                on:click={
                                    let id = s.id.clone();
                                    move |_| store.picks.update(|p| p.retain(|g| g.id != id))
                                }
                            >
                                "✕"
                            </button>
                        </div>
                    </For>
                </div>
            </Show>
            <div class="pick-actions">
                <button class="btn btn--ghost" disabled=move || busy.get() on:click=move |_| run(true, Into_::Picks)>
                    "+ Add folders"
                </button>
                <button class="btn btn--ghost" disabled=move || busy.get() on:click=move |_| run(false, Into_::Picks)>
                    "+ Add files"
                </button>
                {busy_note}
            </div>
        }
    };

    let user_list = move || {
        view! {
            <div class="sources">
                <For each=move || store.sources.get() key=|s| s.id.clone() let:s>
                    {
                        let id = s.id.clone();
                        let on = {
                            let id = id.clone();
                            Memo::new(move |_| store.sources.with(|v| v.iter().any(|g| g.id == id && g.selected)))
                        };
                        let caveat = s.caveat.clone();
                        view! {
                            <label class=move || if on.get() { "source source--on" } else { "source" }>
                                <input
                                    type="checkbox"
                                    prop:checked=move || on.get()
                                    on:change=move |_| store.toggle(&id)
                                    style="position: absolute; opacity: 0; width: 0; height: 0"
                                />
                                <span class="source__check" aria-hidden="true">
                                    {move || if on.get() { "✓" } else { "" }}
                                </span>
                                <span>
                                    <span class="source__label">
                                        {s.label.clone()}
                                        {(s.kind == "browser").then(|| view! { <span class="badge-copper">"browser"</span> })}
                                    </span>
                                    <span class="source__hint">{s.hint.clone()}</span>
                                </span>
                                <span class="source__meta">
                                    <b>{bytes(s.bytes as f64)}</b>
                                    <br />
                                    {format!("{} items", count(s.items))}
                                </span>
                                {move || {
                                    caveat
                                        .clone()
                                        .filter(|_| on.get())
                                        .map(|c| view! { <span class="source__caveat">"⚠ " {c}</span> })
                                }}
                            </label>
                        }
                    }
                </For>
            </div>

            <div class="pick-actions">
                <button class="btn btn--ghost" disabled=move || busy.get() on:click=move |_| run(true, Into_::Sources)>
                    "+ Add a folder"
                </button>
                {busy_note}
            </div>
        }
    };

    view! {
        <div>
            <div class="panel-head">
                <div class="eyebrow">"Step 3 · Payload"</div>
                <h2>"Choose what to carry over"</h2>
                <p>
                    {move || {
                        if custom() {
                            "Pick exactly what to send. Everything in the list crosses the cable."
                        } else {
                            "WINCI found these on the old PC. Tick what should land on the new machine."
                        }
                    }}
                </p>
            </div>

            <div class="modes" role="radiogroup" aria-label="Transfer type">
                {MODES
                    .iter()
                    .map(|&(m, title, sub)| {
                        view! {
                            <button
                                role="radio"
                                aria-checked=move || (store.mode.get() == m).to_string()
                                class=move || if store.mode.get() == m { "mode mode--on" } else { "mode" }
                                on:click=move |_| store.mode.set(m)
                            >
                                <span class="mode__title">{title}</span>
                                <span class="mode__sub">{sub}</span>
                            </button>
                        }
                    })
                    .collect_view()}
            </div>

            {move || if custom() { custom_list().into_any() } else { user_list().into_any() }}

            <div class="totals">
                {move || {
                    if custom() {
                        view! {
                            <button
                                class="btn btn--ghost"
                                disabled=move || store.picks.with(|p| p.is_empty())
                                on:click=move |_| store.picks.set(Vec::new())
                            >
                                "Clear list"
                            </button>
                        }
                            .into_any()
                    } else {
                        view! {
                            <button
                                class="btn btn--ghost"
                                disabled=move || store.sources.with(|s| s.is_empty())
                                on:click=move |_| store.select_all(!all_on())
                            >
                                {move || if all_on() { "Deselect all" } else { "Select all" }}
                            </button>
                        }
                            .into_any()
                    }
                }}
                <div class="totals__n">
                    <b>{move || bytes(chosen.get().1 as f64)}</b>
                    " across "
                    {move || count(chosen.get().2)}
                    " items"
                </div>
                <button
                    class="btn btn--primary"
                    disabled=move || chosen.get().0 == 0 || busy.get()
                    on:click=move |_| store.step.set(Step::Transfer)
                >
                    "Start transfer →"
                </button>
            </div>
        </div>
    }
}
