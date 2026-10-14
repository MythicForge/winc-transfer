use crate::api;
use crate::format::bytes;
use crate::log;
use crate::store::use_store;
use crate::types::DropSettings;
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

    // Where incoming data lands — changeable until the sender connects (this
    // step unmounts then), remembered by the backend for next time.
    let drop = RwSignal::new(None::<DropSettings>);
    let free = RwSignal::new(None::<u64>);
    let show = move |d: DropSettings| {
        let root = d.receive_root.clone();
        drop.set(Some(d));
        spawn_local(async move { free.set(api::free_space(&root).await.ok().flatten()) });
    };
    spawn_local(async move {
        match api::get_drop_settings().await {
            Ok(d) => show(d),
            Err(e) => log::error("ui::code", &format!("could not read the drop folder: {e}")),
        }
    });
    let set_root = move |root: Option<String>| {
        spawn_local(async move {
            match api::set_receive_root(root).await {
                Ok(d) => show(d),
                Err(e) => log::error("ui::code", &format!("could not change the drop folder: {e}")),
            }
        })
    };
    let change = move |_| {
        spawn_local(async move {
            match api::pick_folder().await {
                Ok(Some(dir)) => set_root(Some(dir)),
                Ok(None) => {}
                Err(e) => log::error("ui::code", &format!("folder picker failed: {e}")),
            }
        })
    };
    let landing = move || {
        drop.with(|d| match d {
            Some(d) => format!("{}\\WINC Received", d.receive_root.trim_end_matches('\\')),
            None => "…".into(),
        })
    };

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

            <div class="card ipcard">
                <div class="diag-head">
                    <div class="eyebrow">"Where incoming data lands"</div>
                    <div class="pick-actions" style="margin-top: 0">
                        <Show when=move || drop.with(|d| d.as_ref().is_some_and(|d| !d.receive_is_default))>
                            <button class="btn btn--ghost" on:click=move |_| set_root(None)>
                                "Use Documents"
                            </button>
                        </Show>
                        <button class="btn btn--ghost" on:click=change>"Change folder…"</button>
                    </div>
                </div>
                <p style="color: var(--ink-2); margin: 0">
                    "Received files go to " <span class="mono">{landing}</span>
                    {move || free.get().map(|f| format!(" — {} free.", bytes(f as f64)))}
                </p>
                <p style="color: var(--slate); margin: 8px 0 0; font-size: var(--step--1)">
                    "If this drive is short on space, pick a bigger one before the old PC connects. "
                    "WINCI remembers the choice."
                </p>
            </div>

            <div class="waiting" style="margin-top: 22px">
                <span class="spinner"></span>
                <span>"Listening for the old PC to connect…"</span>
            </div>
        </div>
    }
}
