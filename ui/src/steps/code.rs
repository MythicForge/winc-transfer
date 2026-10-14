use crate::api;
use crate::log;
use crate::store::use_store;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn CodeStep() -> impl IntoView {
    let store = use_store();
    let ips = RwSignal::new(Vec::<String>::new());

    spawn_local(async move {
        match api::list_local_ips().await {
            Ok(v) => ips.set(v),
            Err(e) => log::error("ui::code", &format!("could not list local IPs: {e}")),
        }
    });

    let code = move || {
        let c = store.code.get();
        if c.is_empty() { "······".to_string() } else { c }
    };
    let port = move || {
        store
            .self_peer
            .with(|p| p.as_ref().map(|p| p.port.to_string()).unwrap_or_else(|| "—".into()))
    };

    view! {
        <div>
            <div class="panel-head">
                <div class="eyebrow">"Step 2 · Handshake"</div>
                <h2>"Read this code to the old PC"</h2>
                <p>
                    "On the old computer, WINCI is asking for a 6-digit code. Type the numbers below over "
                    "there to confirm the two machines are paired."
                </p>
            </div>

            <div class="codebox">
                <div class="codebox__digits">{code}</div>
                <div class="codebox__cap">"Enter this on the old PC to pair"</div>
            </div>

            <div class="card ipcard">
                <div class="eyebrow" style="margin-bottom: 10px">"No cable? Pair by IP address"</div>
                <p style="color: var(--ink-2); margin: 0 0 14px; font-size: var(--step-0)">
                    "If the old PC can’t find this one automatically, choose “Enter IP” over there and type one "
                    "of these addresses. Port is " <span class="mono">{port}</span> "."
                </p>
                <div class="iplist">
                    {move || {
                        let list = ips.get();
                        if list.is_empty() {
                            view! {
                                <span class="mono" style="color: var(--slate)">"Reading addresses…"</span>
                            }
                                .into_any()
                        } else {
                            list.into_iter()
                                .enumerate()
                                .map(|(i, ip)| {
                                    view! {
                                        <span class=if i == 0 { "ipchip ipchip--primary" } else { "ipchip" }>
                                            <span class="mono">{ip}</span>
                                            {(i == 0).then(|| view! { <em>"direct link"</em> })}
                                        </span>
                                    }
                                })
                                .collect_view()
                                .into_any()
                        }
                    }}
                </div>
            </div>

            <div class="waiting" style="margin-top: 22px">
                <span class="spinner"></span>
                <span>"Listening for the old PC to connect…"</span>
            </div>
        </div>
    }
}
