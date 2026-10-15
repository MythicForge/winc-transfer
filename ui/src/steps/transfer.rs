use crate::api;
use crate::format::{bytes, count, eta, rate};
use crate::store::use_store;
use crate::types::{Role, XferState};
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn TransferStep() -> impl IntoView {
    let store = use_store();
    let p = store.progress;
    let sending = move || store.role.get() == Some(Role::Send);
    // memo: progress ticks several times a second must not rebuild the button
    // in flight (stoppable): streaming, or re-pairing after the link broke
    let running = Memo::new(move |_| {
        p.with(|p| matches!(p.state, XferState::Running | XferState::Reconnecting))
    });
    let reconnecting = Memo::new(move |_| p.with(|p| p.state == XferState::Reconnecting));
    // Stop gives instant feedback; otherwise a slow stop reads as a dead button
    let stopping = RwSignal::new(false);
    let pct = move || {
        p.with(|p| {
            if p.bytes_total > 0 {
                (p.bytes_sent as f64 / p.bytes_total as f64 * 100.0).min(100.0)
            } else {
                0.0
            }
        })
    };

    view! {
        <div>
            <div class="panel-head">
                <div class="eyebrow">{move || format!("Step {} · Crossing", if sending() { 4 } else { 3 })}</div>
                <h2>{move || if sending() { "Sending to the new PC" } else { "Receiving from the old PC" }}</h2>
                <p>
                    "Keep the cable connected. Data moves directly between the two machines over the wire "
                    "above — nothing touches the internet."
                </p>
            </div>

            <div class="meters">
                <div class="meter">
                    <div class="meter__k">"Throughput"</div>
                    <div class="meter__v mono">{move || rate(p.with(|p| p.bytes_per_sec))}</div>
                </div>
                <div class="meter">
                    <div class="meter__k">"Moved"</div>
                    <div class="meter__v mono">
                        {move || bytes(p.with(|p| p.bytes_sent) as f64)} " "
                        <small>{move || format!("/ {}", bytes(p.with(|p| p.bytes_total) as f64))}</small>
                    </div>
                </div>
                <div class="meter">
                    <div class="meter__k">"Items"</div>
                    <div class="meter__v mono">
                        {move || count(p.with(|p| p.files_sent))} " "
                        <small>{move || format!("/ {}", count(p.with(|p| p.files_total)))}</small>
                    </div>
                </div>
                <div class="meter">
                    <div class="meter__k">"Time left"</div>
                    <div class="meter__v mono">
                        {move || {
                            p.with(|p| eta(p.bytes_total.saturating_sub(p.bytes_sent) as f64, p.bytes_per_sec))
                        }}
                    </div>
                </div>
            </div>

            <div class="progressbar">
                <div class="progressbar__fill" style:width=move || format!("{}%", pct())></div>
            </div>
            <Show when=move || reconnecting.get()>
                <p class="field-warn" style="margin: 10px 0 8px">
                    "The connection was interrupted or data arrived damaged. WINCI is reconnecting "
                    "and will pick up from the file that was in flight — nothing already received "
                    "is sent again."
                </p>
            </Show>
            <div class="nowfile">
                {move || {
                    p.with(|p| match (&p.current_file, p.state) {
                        (Some(f), _) => f.clone(),
                        (None, XferState::Idle) => "Preparing…".into(),
                        _ => " ".into(),
                    })
                }}
            </div>

            <Show when=move || p.with(|p| p.state == XferState::Error)>
                <p style="color: var(--bad); font-family: var(--mono); font-size: var(--step--1); margin-top: 12px">
                    {move || p.with(|p| p.error.clone().unwrap_or_else(|| "Transfer stopped.".into()))}
                </p>
            </Show>

            <div class="actions">
                {move || {
                    if running.get() {
                        view! {
                            <button
                                class="btn"
                                disabled=move || stopping.get()
                                on:click=move |_| {
                                    stopping.set(true);
                                    spawn_local(async {
                                        let _ = api::cancel().await;
                                    })
                                }
                            >
                                {move || if stopping.get() { "Stopping…" } else { "Stop transfer" }}
                            </button>
                        }
                            .into_any()
                    } else {
                        view! {
                            <button class="btn btn--ghost" on:click=move |_| store.reset()>
                                "← Start over"
                            </button>
                        }
                            .into_any()
                    }
                }}
            </div>
        </div>
    }
}
