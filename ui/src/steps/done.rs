use crate::api;
use crate::format::{bytes, count};
use crate::store::use_store;
use crate::types::{ImportEntry, ImportReport, Role};
use leptos::prelude::*;
use leptos::task::spawn_local;

fn badge(action: &str) -> (&'static str, &'static str) {
    match action {
        "imported" => ("✓ imported", "import__badge--ok"),
        "skipped-not-fresh" => ("skipped", "import__badge--warn"),
        "skipped-not-installed" => ("not installed", "import__badge--warn"),
        _ => ("failed", "import__badge--err"),
    }
}

#[derive(Clone, Copy)]
enum BrowserAction {
    Overwrite,
    Retry,
}

#[component]
pub fn DoneStep() -> impl IntoView {
    let store = use_store();
    let p = store.progress;
    let sending = store.role.get_untracked() == Some(Role::Send);
    let report = RwSignal::new(None::<ImportReport>);
    let busy = RwSignal::new(false);
    let err = RwSignal::new(None::<String>);
    let pending = RwSignal::new(None::<String>);

    let run_import = move |_| {
        let Some(dir) = store.received_dir.get_untracked() else { return };
        busy.set(true);
        err.set(None);
        spawn_local(async move {
            match api::import_received(&dir).await {
                Ok(r) => report.set(Some(r)),
                Err(e) => err.set(Some(e)),
            }
            busy.set(false);
        });
    };

    // Re-run one browser's import (Overwrite? or Open First) and swap its row in
    // place with whatever the backend now reports.
    let run_browser_action = move |label: String, action: BrowserAction| {
        let Some(dir) = store.received_dir.get_untracked() else { return };
        if pending.with_untracked(|p| p.is_some()) {
            return;
        }
        pending.set(Some(label.clone()));
        err.set(None);
        spawn_local(async move {
            let out = match action {
                BrowserAction::Overwrite => api::import_browser_overwrite(&dir, &label).await,
                BrowserAction::Retry => api::import_browser_retry(&dir, &label).await,
            };
            match out {
                Ok(updated) => report.update(|r| {
                    if let Some(r) = r {
                        for en in r.entries.iter_mut() {
                            if en.browser_label.as_deref() == Some(label.as_str()) {
                                *en = updated.clone();
                            }
                        }
                    }
                }),
                Err(e) => err.set(Some(e)),
            }
            pending.set(None);
        });
    };

    let row = move |en: ImportEntry| {
        let action_view = match (en.action.as_str(), en.browser_label.clone()) {
            ("skipped-not-fresh", Some(bl)) => {
                let title = format!(
                    "Replace {bl}'s data with the old PC's. Current files are backed up to the WINC Received folder first."
                );
                let shown = bl.clone();
                view! {
                    <button
                        class="btn btn--ghost import__overwrite"
                        disabled=move || pending.with(|p| p.is_some())
                        on:click=move |_| run_browser_action(bl.clone(), BrowserAction::Overwrite)
                        title=title
                    >
                        {move || {
                            if pending.with(|p| p.as_deref() == Some(shown.as_str())) { "Overwriting…" } else { "Overwrite?" }
                        }}
                    </button>
                }
                    .into_any()
            }
            ("skipped-not-installed", Some(bl)) => {
                let title = format!("Open {bl} once so it creates its profile, then click to finish importing.");
                let shown = bl.clone();
                view! {
                    <button
                        class="btn btn--ghost import__overwrite"
                        disabled=move || pending.with(|p| p.is_some())
                        on:click=move |_| run_browser_action(bl.clone(), BrowserAction::Retry)
                        title=title
                    >
                        {move || {
                            if pending.with(|p| p.as_deref() == Some(shown.as_str())) { "Checking…" } else { "Open First" }
                        }}
                    </button>
                }
                    .into_any()
            }
            (action, _) => {
                let (text, cls) = badge(action);
                view! { <span class=format!("import__badge {cls}")>{text}</span> }.into_any()
            }
        };
        view! {
            <div style="display: flex; gap: 10px; align-items: baseline; padding: 6px 0; border-bottom: 1px solid var(--line, rgba(0,0,0,.06))">
                <b style="min-width: 140px">{en.label.clone()}</b>
                {action_view}
                {(en.count > 0).then(|| view! { <span style="color: var(--ink-2)">{format!("{} files", count(en.count))}</span> })}
                {en.detail.clone().map(|d| view! { <span style="color: var(--ink-2); font-size: 13px">{d}</span> })}
            </div>
        }
    };

    let import_card = move || {
        view! {
            <div class="card" style="padding: 20px; margin-top: 14px">
                <div class="eyebrow" style="margin-bottom: 10px">"Import into place"</div>
                {move || match report.get() {
                    None => view! {
                        <p style="color: var(--ink-2); margin: 0 0 12px; line-height: 1.6">
                            "Files landed in "
                            <span class="mono">
                                {move || store.received_dir.get().unwrap_or_else(|| "Documents\\WINC Received".into())}
                            </span>
                            ". Import moves them into your real folders — nothing on this PC is overwritten; "
                            "incoming duplicates are kept as “name (from old PC)”. Browser data imports "
                            "only into browsers with no existing data; ones that already have data show "
                            "an " <b>"Overwrite?"</b> " option, and ones not yet opened show " <b>"Open First"</b> "."
                        </p>
                        <button
                            class="btn btn--primary"
                            disabled=move || busy.get() || store.received_dir.with(|d| d.is_none())
                            on:click=run_import
                        >
                            {move || if busy.get() { "Importing…" } else { "Import into place" }}
                        </button>
                        {move || err.get().map(|e| view! {
                            <p class="field-warn" style="margin-top: 8px">"Import failed — " {e}</p>
                        })}
                    }
                        .into_any(),
                    Some(r) => view! {
                        <div class="import-report">
                            {r.entries.is_empty().then(|| view! {
                                <p style="color: var(--ink-2); margin: 0">"Nothing to import."</p>
                            })}
                            {r.entries.into_iter().map(row).collect_view()}
                            <p style="color: var(--slate, var(--ink-2)); font-size: 12px; margin: 10px 0 0">
                                "A snapshot log of every file (source → destination) was saved as "
                                <span class="mono">"import-log-*.json"</span>
                                " in the WINC Received folder."
                            </p>
                            {move || err.get().map(|e| view! {
                                <p class="field-warn" style="margin-top: 8px">{e}</p>
                            })}
                        </div>
                    }
                        .into_any(),
                }}
            </div>
        }
    };

    view! {
        <div>
            <div class="done-hero">
                <div class="done-hero__seal" aria-hidden="true">
                    <svg width="30" height="30" viewBox="0 0 24 24" fill="none">
                        <path
                            d="M5 13l4 4L19 7"
                            stroke="currentColor"
                            stroke-width="2.5"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                        />
                    </svg>
                </div>
                <h2>{if sending { "Everything sent" } else { "Everything landed" }}</h2>
                <p style="color: var(--ink-2); margin-top: 8px">
                    {move || {
                        p.with(|p| format!(
                            "{} across {} items crossed the cable.",
                            bytes(p.bytes_total as f64),
                            count(p.files_total)
                        ))
                    }}
                </p>
            </div>

            <div class="card" style="padding: 20px">
                <div class="eyebrow" style="margin-bottom: 10px">"What next"</div>
                <ul style="color: var(--ink-2); line-height: 1.8; margin: 0; padding-left: 18px">
                    <li>"Open the new PC and check a few folders landed where you expect."</li>
                    <li>"Sign into your browser to finish syncing bookmarks and history."</li>
                    <li>"Saved passwords only unlock under the same Windows / Microsoft account."</li>
                    <li>"You can now safely unplug the cable."</li>
                </ul>
            </div>

            {(!sending).then(import_card)}

            <div class="actions">
                <button class="btn" on:click=move |_| store.reset()>
                    "Transfer something else"
                </button>
            </div>
        </div>
    }
}
