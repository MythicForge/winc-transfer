use crate::format::clock;
use crate::log::{self, at_least, LEVELS};
use gloo_timers::callback::Timeout;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;

/// Rendering every one of 3000 rows on each burst is what makes a log view
/// janky; the tail is what anyone actually reads.
const MAX_ROWS: usize = 800;

#[component]
pub fn LogPanel(#[prop(optional)] popout: bool) -> impl IntoView {
    let entries = log::entries();
    let floor = RwSignal::new("info".to_string());
    let needle = RwSignal::new(String::new());
    let follow = RwSignal::new(true);
    let path = RwSignal::new(String::new());
    let copied = RwSignal::new(false);
    let body = NodeRef::<html::Div>::new();

    spawn_local(async move {
        floor.set(log::get_level().await);
        path.set(log::file_path().await);
    });

    let shown = Memo::new(move |_| {
        let q = needle.get().trim().to_lowercase();
        let floor = floor.get();
        entries.with(|es| {
            let hits: Vec<_> = es
                .iter()
                .filter(|e| {
                    at_least(&e.level, &floor)
                        && (q.is_empty()
                            || e.msg.to_lowercase().contains(&q)
                            || e.target.to_lowercase().contains(&q))
                })
                .cloned()
                .collect();
            let skip = hits.len().saturating_sub(MAX_ROWS);
            hits.into_iter().skip(skip).collect::<Vec<_>>()
        })
    });

    let errors = Memo::new(move |_| entries.with(|es| es.iter().filter(|e| e.level == "error").count()));

    // Stick to the bottom while following; the scroll handler below drops
    // follow the moment the user scrolls up to read something.
    Effect::new(move |_| {
        shown.track();
        if !follow.get() {
            return;
        }
        if let Some(el) = body.get() {
            // after the rows render
            request_animation_frame(move || el.set_scroll_top(el.scroll_height()));
        }
    });

    let on_scroll = move |_| {
        if let Some(el) = body.get_untracked() {
            let at_bottom = el.scroll_height() - el.scroll_top() - el.client_height() < 24;
            follow.set(at_bottom);
        }
    };

    let on_level = move |v: String| {
        floor.set(v.clone());
        // Debug/trace must also be raised in Rust, or those records are never sent.
        spawn_local(async move { log::set_level(&v).await });
    };

    let copy = move |_| {
        let text = shown.with_untracked(|rows| {
            rows.iter()
                .map(|e| format!("{} {:<5} [{}] {}", clock(e.ts), e.level.to_uppercase(), e.target, e.msg))
                .collect::<Vec<_>>()
                .join("\n")
        });
        let clip = window().navigator().clipboard();
        spawn_local(async move {
            let ok = wasm_bindgen_futures::JsFuture::from(clip.write_text(&text)).await.is_ok();
            copied.set(ok);
            if ok {
                Timeout::new(1600, move || copied.set(false)).forget();
            }
        });
    };

    view! {
        <div class="logs">
            <div class="logs__bar">
                <select
                    class="logs__select"
                    prop:value=move || floor.get()
                    on:change=move |e| on_level(event_target_value(&e))
                    aria-label="Minimum level"
                >
                    {LEVELS
                        .iter()
                        .map(|(value, label)| {
                            view! {
                                <option value=*value selected=move || floor.get() == *value>
                                    {*label}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>

                <input
                    class="logs__find"
                    placeholder="Filter…"
                    prop:value=move || needle.get()
                    on:input=move |e| needle.set(event_target_value(&e))
                />

                <span class="logs__count">
                    {move || shown.with(|s| s.len())}
                    " shown"
                    {move || {
                        let n = errors.get();
                        (n > 0)
                            .then(|| {
                                view! {
                                    <b class="logs__errs">
                                        {format!(" · {n} error{}", if n > 1 { "s" } else { "" })}
                                    </b>
                                }
                            })
                    }}
                </span>

                <span class="logs__spacer"></span>

                <button
                    class=move || if follow.get() { "btn btn--ghost logs__btn is-on" } else { "btn btn--ghost logs__btn" }
                    on:click=move |_| follow.update(|f| *f = !*f)
                    title="Keep the newest line in view"
                >
                    {move || if follow.get() { "Following" } else { "Follow" }}
                </button>
                <button class="btn btn--ghost logs__btn" on:click=copy>
                    {move || if copied.get() { "Copied" } else { "Copy" }}
                </button>
                <button class="btn btn--ghost logs__btn" on:click=move |_| spawn_local(log::clear())>
                    "Clear"
                </button>
                <button
                    class="btn btn--ghost logs__btn"
                    on:click=move |_| spawn_local(log::open_dir())
                    title=move || path.get()
                >
                    "Log file"
                </button>
                {(!popout)
                    .then(|| {
                        view! {
                            <button class="btn btn--ghost logs__btn" on:click=move |_| spawn_local(log::open_window())>
                                "Pop out ↗"
                            </button>
                        }
                    })}
            </div>

            <div class="logs__body" node_ref=body on:scroll=on_scroll>
                <Show
                    when=move || shown.with(|s| !s.is_empty())
                    fallback=move || {
                        view! {
                            <p class="logs__empty">
                                "Nothing at this level yet"
                                {move || (!needle.get().is_empty()).then_some(" matching that filter")}
                                "."
                            </p>
                        }
                    }
                >
                    <For each=move || shown.get() key=|e| e.seq let:e>
                        <div class=format!("logrow logrow--{}", e.level)>
                            <span class="logrow__t">{clock(e.ts)}</span>
                            <span class="logrow__lvl">{e.level.clone()}</span>
                            <span class="logrow__src">{e.target.clone()}</span>
                            <span class="logrow__msg">{e.msg.clone()}</span>
                        </div>
                    </For>
                </Show>
            </div>
        </div>
    }
}

/// Root of the detached log window (index.html?view=log). It hydrates from the
/// Rust ring buffer, so opening it mid-transfer still shows how things got here.
#[component]
pub fn LogWindow() -> impl IntoView {
    view! {
        <div class="logwin">
            <header class="logwin__head">
                <span class="brand__mark">"WIN" <b>"CI"</b></span>
                <span class="brand__tag">"Activity log"</span>
            </header>
            <LogPanel popout=true />
        </div>
    }
}
