use super::log_panel::LogPanel;
use leptos::ev;
use leptos::prelude::*;

const MIN_H: f64 = 160.0;
const MAX_FRAC: f64 = 0.75;

/// In-app activity log: a resizable bottom drawer. Ctrl+L toggles, Esc closes.
/// The same panel also runs detached — see LogWindow.
#[component]
pub fn LogDrawer(open: RwSignal<bool>) -> impl IntoView {
    let h = RwSignal::new(300.0);
    // (pointer y, height) at drag start
    let drag = StoredValue::new(None::<(f64, f64)>);

    let on_move = window_event_listener(ev::pointermove, move |e| {
        if let Some((y, start_h)) = drag.get_value() {
            let max = window().inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(800.0) * MAX_FRAC;
            h.set((start_h + (y - e.client_y() as f64)).clamp(MIN_H, max.max(MIN_H)));
        }
    });
    let on_up = window_event_listener(ev::pointerup, move |_| drag.set_value(None));
    let on_key = window_event_listener(ev::keydown, move |e| {
        if open.get_untracked() && crate::format::key_of(&e) == "Escape" {
            open.set(false);
        }
    });
    on_cleanup(move || {
        on_move.remove();
        on_up.remove();
        on_key.remove();
    });

    view! {
        <Show when=move || open.get()>
            <aside class="logdrawer" style:height=move || format!("{}px", h.get())>
                <div
                    class="logdrawer__grip"
                    on:pointerdown=move |e| drag.set_value(Some((e.client_y() as f64, h.get_untracked())))
                    role="separator"
                    aria-orientation="horizontal"
                    title="Drag to resize"
                ></div>
                <div class="logdrawer__head">
                    <span class="eyebrow">"Activity log"</span>
                    <button class="btn btn--ghost logs__btn" on:click=move |_| open.set(false)>
                        "Close"
                    </button>
                </div>
                <LogPanel popout=false />
            </aside>
        </Show>
    }
}
