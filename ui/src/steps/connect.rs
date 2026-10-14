use crate::api;
use crate::log;
use crate::store::{use_store, Step};
use crate::types::{AdapterInfo, Role};
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Fw {
    Idle,
    Working,
    Done,
    Error,
}

#[component]
pub fn ConnectStep() -> impl IntoView {
    let store = use_store();
    let next = move || if store.role.get() == Some(Role::Receive) { Step::Code } else { Step::Pair };
    let adapters = RwSignal::new(Vec::<AdapterInfo>::new());
    let fw = RwSignal::new(Fw::Idle);

    let allow_firewall = move |_| {
        fw.set(Fw::Working);
        spawn_local(async move {
            match api::allow_firewall().await {
                Ok(()) => fw.set(Fw::Done),
                Err(e) => {
                    log::error("ui::connect", &format!("firewall rule not added: {e}"));
                    fw.set(Fw::Error);
                }
            }
        });
    };

    // APIPA (169.254) can take several seconds to self-assign after the cable
    // links, so poll — a one-shot check misses the cable and latches onto Wi-Fi.
    let tick_link = move || {
        spawn_local(async move {
            if let Ok(l) = api::link_status().await {
                store.link.set(l);
            }
        })
    };
    let refresh_adapters = move || {
        spawn_local(async move {
            match api::list_adapters().await {
                Ok(a) => adapters.set(a),
                Err(e) => log::error("ui::connect", &format!("adapter list failed: {e}")),
            }
        })
    };
    tick_link();
    refresh_adapters();
    let link_poll = set_interval_with_handle(tick_link, Duration::from_millis(1500)).ok();
    // refresh the adapter list every 2s while waiting
    let adapter_poll = set_interval_with_handle(refresh_adapters, Duration::from_millis(2000)).ok();
    on_cleanup(move || {
        link_poll.map(|h| h.clear());
        adapter_poll.map(|h| h.clear());
    });

    let up = move || store.link.with(|l| l.up);
    let no_cable_adapter = move || adapters.with(|a| !a.is_empty() && !a.iter().any(|a| a.cable));
    let cable_without_ip = move || adapters.with(|a| a.iter().any(|a| a.cable && a.ip.is_empty()));

    view! {
        <div>
            <div class="panel-head">
                <div class="eyebrow">"Step 1 · Physical link"</div>
                <h2>"Connect the cable — or use your network"</h2>
                <p>
                    "Join the two PCs with a Thunderbolt or USB4 cable. Windows brings up a direct network "
                    "bridge automatically — no router, no Wi-Fi. WINCI watches for the link. No cable? Skip "
                    "this step and connect over any Wi-Fi / Ethernet network by IP address instead."
                </p>
            </div>

            <div class="card" style="padding: 22px">
                <div class="waiting">
                    {move || {
                        let l = store.link.get();
                        if l.up {
                            view! {
                                <span class="readout__dot readout__dot--linked"></span>
                                <span>
                                    "Link up on " <b>{l.adapter.clone()}</b> " · this PC is "
                                    <span class="mono">{l.local_ip.clone()}</span>
                                </span>
                            }
                                .into_any()
                        } else if let Some(a) = l.adapter {
                            // cable adapter is up, but Windows hasn't assigned its IPv4 yet
                            view! {
                                <span class="spinner"></span>
                                <span>
                                    "Cable detected on " <b>{a}</b> " — waiting for Windows to assign its "
                                    <span class="mono">"169.254.x.x"</span>
                                    " address (DHCP timeout, up to ~30 s)…"
                                </span>
                            }
                                .into_any()
                        } else {
                            view! {
                                <span class="spinner"></span>
                                <span>"Waiting for a direct cable link… (the address can take a few seconds)"</span>
                            }
                                .into_any()
                        }
                    }}
                </div>

                <Show when=move || !up()>
                    <ul style="color: var(--ink-2); margin-top: 16px; line-height: 1.7; padding-left: 18px">
                        <li>"Use a cable rated for data (Thunderbolt 3/4 or USB4), not charge-only."</li>
                        <li>"On the other PC, launch WINCI too and choose the opposite role."</li>
                        <li>
                            "The cable shows up as an " <b>"Ethernet"</b> " adapter with a "
                            <span class="mono">"169.254.x.x"</span> " address — that's normal."
                        </li>
                    </ul>
                </Show>
            </div>

            // live diagnostics: what the app actually sees
            <div class="card ipcard">
                <div class="diag-head">
                    <div class="eyebrow">"Detected adapters"</div>
                    <button class="btn btn--ghost" on:click=move |_| refresh_adapters()>
                        "↻ Refresh"
                    </button>
                </div>
                <Show
                    when=move || adapters.with(|a| !a.is_empty())
                    fallback=|| {
                        view! {
                            <p class="mono" style="color: var(--slate); margin: 0">
                                "No IPv4 adapters found yet…"
                            </p>
                        }
                    }
                >
                    <div class="adapters">
                        <For each=move || adapters.get() key=|a| format!("{}{}", a.name, a.ip) let:a>
                            <div class=if a.cable { "adapter adapter--cable" } else { "adapter" }>
                                <span class="adapter__name">{a.name.clone()}</span>
                                <span class="mono adapter__ip">
                                    {if a.ip.is_empty() { "no IPv4 yet".to_string() } else { a.ip.clone() }}
                                </span>
                                <span class=if a.cable { "adapter__tag adapter__tag--cable" } else { "adapter__tag" }>
                                    {if a.link_local {
                                        "direct cable"
                                    } else if a.cable {
                                        "cable"
                                    } else {
                                        "network"
                                    }}
                                </span>
                            </div>
                        </For>
                    </div>
                </Show>
                <Show when=move || !up() && no_cable_adapter()>
                    <p class="field-warn" style="margin-top: 12px">
                        "No cable adapter yet. If the cable is plugged in, wait a moment for its "
                        <span class="mono">"169.254"</span>
                        " address, or enable “Thunderbolt Networking” in Windows settings."
                    </p>
                </Show>
                <Show when=move || !up() && cable_without_ip()>
                    <p class="field-warn" style="margin-top: 12px">
                        "Cable adapter is up but has no IPv4 address yet. If it stays that way past a minute, "
                        "give it one by hand: Settings → Network → the cable adapter → “Edit IP assignment” → "
                        "Manual, IPv4 on — use " <span class="mono">"169.254.1.1"</span> " on one PC and "
                        <span class="mono">"169.254.1.2"</span> " on the other, mask "
                        <span class="mono">"255.255.0.0"</span> ", no gateway."
                    </p>
                </Show>

                // Windows treats the cable link as an "Unidentified network" (Public
                // profile) and silently blocks WINC's discovery + listener there —
                // the standard firewall prompt only covers Private networks.
                <div style="margin-top: 16px">
                    <p style="color: var(--ink-2); margin: 0 0 8px">
                        "If the PCs never find each other, Windows Firewall is usually blocking WINCI on "
                        "“Public” networks — the cable link counts as one."
                    </p>
                    <button
                        class="btn btn--ghost"
                        on:click=allow_firewall
                        disabled=move || fw.get() == Fw::Working
                    >
                        {move || match fw.get() {
                            Fw::Working => "Waiting for admin approval…",
                            Fw::Done => "✓ Firewall rule added",
                            _ => "Allow WINCI through the firewall",
                        }}
                    </button>
                    <Show when=move || fw.get() == Fw::Error>
                        <p class="field-warn" style="margin-top: 8px">
                            "Couldn’t add the rule — the admin prompt was declined or blocked. You can also allow "
                            "WINCI manually in “Windows Security → Firewall → Allow an app”."
                        </p>
                    </Show>
                </div>
            </div>

            <div class="actions">
                <button
                    class="btn btn--primary"
                    disabled=move || !up()
                    on:click=move |_| store.step.set(next())
                >
                    "Cable is connected →"
                </button>
                <button class="btn btn--ghost" on:click=move |_| store.step.set(next())>
                    {move || {
                        if store.role.get() == Some(Role::Receive) {
                            "No cable — receive over Wi-Fi / Ethernet →"
                        } else {
                            "No cable — connect by IP over Wi-Fi / Ethernet →"
                        }
                    }}
                </button>
            </div>
        </div>
    }
}
