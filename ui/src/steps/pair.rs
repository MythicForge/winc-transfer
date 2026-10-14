use crate::api::{self, TRANSFER_PORT};
use crate::log;
use crate::store::{use_store, Step};
use crate::types::Peer;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Auto,
    Manual,
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Finding,
    Found,
    Pairing,
    Error,
}

/// Manual mode: build the peer from a typed address ("ip" or "ip:port").
fn parse_addr(input: &str) -> Option<Peer> {
    let t = input.trim();
    if t.is_empty() {
        return None;
    }
    let (ip, port) = match t.split_once(':') {
        Some((ip, p)) => (ip, p.parse::<u16>().ok()?),
        None => (t, TRANSFER_PORT),
    };
    let octets: Vec<&str> = ip.split('.').collect();
    let valid = octets.len() == 4
        && octets
            .iter()
            .all(|o| (1..=3).contains(&o.len()) && o.chars().all(|c| c.is_ascii_digit()));
    valid.then(|| Peer { name: ip.into(), ip: ip.into(), port })
}

#[component]
pub fn PairStep() -> impl IntoView {
    let store = use_store();
    let mode = RwSignal::new(Mode::Auto);
    let entered = RwSignal::new(String::new());
    let addr = RwSignal::new(String::new());
    let status = RwSignal::new(Status::Finding);
    let err = RwSignal::new(String::new());
    let code_ref = NodeRef::<html::Input>::new();
    // bumped per discovery run, so a stale result from a left-behind run is ignored
    let run = StoredValue::new(0u32);

    // Auto mode: listen for the receiver beacon on the link.
    Effect::new(move |_| {
        if mode.get() != Mode::Auto {
            return;
        }
        let me = run.get_value() + 1;
        run.set_value(me);
        status.set(Status::Finding);
        spawn_local(async move {
            let out = api::discover_peer().await;
            if run.try_get_value() != Some(me) || mode.get_untracked() != Mode::Auto {
                return;
            }
            match out {
                Ok(Some(peer)) => {
                    store.peer.set(Some(peer));
                    status.set(Status::Found);
                    if let Some(el) = code_ref.get_untracked() {
                        let _ = el.focus();
                    }
                }
                Ok(None) => {
                    status.set(Status::Error);
                    err.set("No PC found on the cable. Switch to “Enter IP” if you’re on Wi-Fi or Ethernet.".into());
                }
                Err(e) => {
                    log::error("ui::pair", &format!("discovery failed: {e}"));
                    status.set(Status::Error);
                }
            }
        });
    });
    on_cleanup(move || {
        // invalidate any discovery still in flight
        if let Some(n) = run.try_get_value() {
            run.set_value(n + 1);
        }
    });

    let target = Memo::new(move |_| match mode.get() {
        Mode::Manual => parse_addr(&addr.get()),
        Mode::Auto => store.peer.get(),
    });
    let digits = Memo::new(move |_| {
        entered.with(|s| s.chars().filter(|c| c.is_ascii_digit()).take(6).collect::<String>())
    });
    let ready = move || digits.with(|d| d.len() == 6) && target.with(|t| t.is_some());

    let submit = move || {
        if !ready() || status.get_untracked() == Status::Pairing {
            return;
        }
        let Some(peer) = target.get_untracked() else { return };
        let code = digits.get_untracked();
        status.set(Status::Pairing);
        err.set(String::new());
        spawn_local(async move {
            if mode.get_untracked() == Mode::Manual {
                store.peer.set(Some(peer.clone()));
            }
            let out = async {
                api::pair(&peer, &code).await?;
                api::list_sources().await
            }
            .await;
            match out {
                Ok(sources) => {
                    store.sources.set(sources);
                    store.step.set(Step::Select);
                }
                Err(e) => {
                    log::error("ui::pair", &format!("pairing with {}:{} failed: {e}", peer.ip, peer.port));
                    status.set(Status::Error);
                    let lower = e.to_lowercase();
                    err.set(
                        if lower.contains("wrong code") || lower.contains("mismatch") {
                            "That code didn’t match. Re-read it on the new PC."
                        } else {
                            "Couldn’t reach that PC. Check the IP, that WINCI is receiving, and the firewall."
                        }
                        .into(),
                    );
                }
            }
        });
    };

    let tab = move |m: Mode, label: &'static str| {
        view! {
            <button
                role="tab"
                aria-selected=move || (mode.get() == m).to_string()
                class=move || if mode.get() == m { "segmented__btn is-on" } else { "segmented__btn" }
                on:click=move |_| mode.set(m)
            >
                {label}
            </button>
        }
    };

    view! {
        <div>
            <div class="panel-head">
                <div class="eyebrow">"Step 2 · Handshake"</div>
                <h2>"Pair with the new PC"</h2>
                <p>
                    "Confirm the two machines with the 6-digit code shown on the new PC. Auto-find works over "
                    "the direct cable; use “Enter IP” for Wi-Fi or Ethernet."
                </p>
            </div>

            <div class="segmented" role="tablist">
                {tab(Mode::Auto, "Find automatically")}
                {tab(Mode::Manual, "Enter IP address")}
            </div>

            {move || {
                if mode.get() == Mode::Auto {
                    view! {
                        <div class="card" style="padding: 22px; margin-bottom: 18px">
                            <div class="waiting">
                                {move || {
                                    if status.get() == Status::Finding {
                                        view! {
                                            <span class="spinner"></span>
                                            " "
                                            <span>"Looking for the new PC on the cable…"</span>
                                        }
                                            .into_any()
                                    } else if let Some(p) = store.peer.get() {
                                        view! {
                                            <span class="readout__dot readout__dot--linked"></span>
                                            <span>
                                                "Found " <b>{p.name}</b> " at " <span class="mono">{p.ip}</span>
                                            </span>
                                        }
                                            .into_any()
                                    } else {
                                        view! {
                                            <span class="readout__dot"></span>
                                            " "
                                            <span>"No peer yet."</span>
                                        }
                                            .into_any()
                                    }
                                }}
                            </div>
                        </div>
                    }
                        .into_any()
                } else {
                    view! {
                        <div style="margin-bottom: 18px">
                            <label class="field-label" for="ipaddr">
                                "New PC address " <span class="mono">"(shown on its screen)"</span>
                            </label>
                            <input
                                id="ipaddr"
                                class="text-input mono"
                                placeholder=format!("192.168.1.24   ·   or  169.254.42.9:{TRANSFER_PORT}")
                                prop:value=move || addr.get()
                                on:input=move |e| addr.set(event_target_value(&e))
                                on:keydown=move |e| {
                                    if e.key() == "Enter" {
                                        if let Some(el) = code_ref.get_untracked() {
                                            let _ = el.focus();
                                        }
                                    }
                                }
                                autofocus
                            />
                            <Show when=move || !addr.with(|a| a.is_empty()) && target.with(|t| t.is_none())>
                                <p class="field-warn">"Enter a valid IPv4 address, e.g. 192.168.1.24"</p>
                            </Show>
                        </div>
                    }
                        .into_any()
                }
            }}

            <label class="field-label" for="paircode">"6-digit code"</label>
            <input
                id="paircode"
                node_ref=code_ref
                class="code-input"
                inputmode="numeric"
                placeholder="000000"
                prop:value=move || digits.get()
                disabled=move || status.get() == Status::Pairing
                on:input=move |e| {
                    entered.set(event_target_value(&e));
                    // keep the field showing only the cleaned digits
                    if let Some(el) = code_ref.get_untracked() {
                        el.set_value(&digits.get_untracked());
                    }
                }
                on:keydown=move |e| {
                    if e.key() == "Enter" {
                        submit();
                    }
                }
                aria-label="Pairing code"
            />
            <Show when=move || status.get() == Status::Error && !err.with(|e| e.is_empty())>
                <p class="field-warn">{move || err.get()}</p>
            </Show>

            <div class="actions">
                <button class="btn btn--ghost" on:click=move |_| store.step.set(Step::Connect)>
                    "← Back"
                </button>
                <button
                    class="btn btn--primary"
                    disabled=move || !ready() || status.get() == Status::Pairing
                    on:click=move |_| submit()
                >
                    {move || if status.get() == Status::Pairing { "Pairing…" } else { "Pair →" }}
                </button>
            </div>
        </div>
    }
}
