use crate::format::rate;
use crate::store::use_store;
use crate::types::{Role, XferState};
use leptos::prelude::*;

/// The signature element: a live cable between OLD and NEW.
/// idle -> slack gray wire · linked -> taut copper · flowing -> cobalt pulses.
#[component]
pub fn Cable() -> impl IntoView {
    let store = use_store();

    let flowing = Memo::new(move |_| store.progress.with(|p| p.state == XferState::Running));
    let linked = Memo::new(move |_| {
        store.link.with(|l| l.up) && (store.peer.with(|p| p.is_some()) || flowing.get())
    });

    let wire_class = move || {
        format!(
            "wire{}{}",
            if linked.get() { " wire--linked" } else { "" },
            if flowing.get() { " wire--flowing" } else { "" }
        )
    };

    // Memo, so progress ticks don't rewrite the SVG and restart the pulses.
    let svg = Memo::new(move |_| {
        // straight when linked, gentle sag when idle
        let d = if linked.get() { "M 2 20 L 98 20" } else { "M 2 20 Q 50 32 98 20" };
        let pulses: String = if flowing.get() {
            [0.0, 0.33, 0.66]
                .iter()
                .map(|delay| {
                    format!(
                        r##"<circle class="wire__pulse" r="2.4"><animateMotion dur="1.1s" begin="{:.3}s" repeatCount="indefinite"><mpath href="#winc-wire"/></animateMotion></circle>"##,
                        delay * 1.1
                    )
                })
                .collect()
        } else {
            String::new()
        };
        format!(
            r#"<path id="winc-wire" class="wire__base" d="{d}" vector-effect="non-scaling-stroke"/><path class="wire__flow" d="{d}" vector-effect="non-scaling-stroke"/>{pulses}"#
        )
    });

    let this_role = move || match store.role.get() {
        Some(Role::Receive) => "New device",
        _ => "Old device",
    };
    let peer_role = move || match store.role.get() {
        Some(Role::Receive) => "Old device",
        _ => "New device",
    };

    let readout = move || {
        if flowing.get() {
            let bps = store.progress.with(|p| p.bytes_per_sec);
            view! { <span class="mono">{rate(bps)}</span> }.into_any()
        } else if linked.get() {
            view! { <span class="mono">"Linked"</span> }.into_any()
        } else if store.link.with(|l| l.up) {
            view! { <span class="mono">"Cable up · finding peer"</span> }.into_any()
        } else {
            "No link".into_any()
        }
    };
    let dot_class = move || {
        if flowing.get() {
            "readout__dot readout__dot--live"
        } else if linked.get() {
            "readout__dot readout__dot--linked"
        } else {
            "readout__dot"
        }
    };

    view! {
        <div class="cable">
            <div class="cable__stage">
                <div class=move || if store.role.with(|r| r.is_some()) { "node node--active" } else { "node" }>
                    <div class="node__role">{this_role}</div>
                    <div class="node__name">"This PC"</div>
                    <div class="node__ip">
                        {move || store.link.with(|l| l.local_ip.clone().unwrap_or_else(|| "—".into()))}
                    </div>
                </div>

                <div class=wire_class>
                    <svg
                        viewBox="0 0 100 40"
                        preserveAspectRatio="none"
                        aria-hidden="true"
                        inner_html=move || svg.get()
                    ></svg>
                    <div class="readout">
                        <span class=dot_class></span>
                        {readout}
                    </div>
                </div>

                <div class=move || if store.peer.with(|p| p.is_some()) { "node node--active" } else { "node" }>
                    <div class="node__role">{peer_role}</div>
                    <div class="node__name">
                        {move || store.peer.with(|p| p.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Waiting…".into()))}
                    </div>
                    <div class="node__ip">
                        {move || store.peer.with(|p| p.as_ref().map(|p| p.ip.clone()).unwrap_or_else(|| "—".into()))}
                    </div>
                </div>
            </div>
        </div>
    }
}
