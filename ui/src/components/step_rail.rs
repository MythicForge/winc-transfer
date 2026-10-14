use crate::api;
use crate::store::{use_store, Step};
use crate::types::Role;
use leptos::prelude::*;

const SEND: &[(Step, &str, &str)] = &[
    (Step::Connect, "Connect cable", "Thunderbolt / USB4"),
    (Step::Pair, "Pair devices", "Enter 6-digit code"),
    (Step::Select, "Choose data", "Files & browser"),
    (Step::Transfer, "Transfer", "Stream to new PC"),
    (Step::Done, "Done", "Verify & finish"),
];

const RECEIVE: &[(Step, &str, &str)] = &[
    (Step::Connect, "Connect cable", "Thunderbolt / USB4"),
    (Step::Code, "Show code", "Read it to old PC"),
    (Step::Transfer, "Receive", "Incoming data"),
    (Step::Done, "Done", "Everything landed"),
];

#[component]
pub fn StepRail() -> impl IntoView {
    let store = use_store();
    let steps = move || match store.role.get() {
        Some(Role::Receive) => RECEIVE,
        _ => SEND,
    };
    let active = Memo::new(move |_| {
        let step = store.step.get();
        steps().iter().position(|(id, ..)| *id == step)
    });

    view! {
        <nav class="rail" aria-label="Progress">
            {move || {
                let active = active.get();
                steps()
                    .iter()
                    .enumerate()
                    .map(|(i, (_, label, sub))| {
                        let (cls, num) = match active {
                            Some(a) if i == a => ("rail__step rail__step--active", (i + 1).to_string()),
                            Some(a) if i < a => ("rail__step rail__step--done", "✓".to_string()),
                            _ => ("rail__step", (i + 1).to_string()),
                        };
                        view! {
                            <div class=cls aria-current=(Some(i) == active).then_some("step")>
                                <div class="rail__num">{num}</div>
                                <div>
                                    <div class="rail__label">{*label}</div>
                                    <div class="rail__sub">{*sub}</div>
                                </div>
                            </div>
                        }
                    })
                    .collect_view()
            }}
            <div class="rail__spacer"></div>
            <div class="rail__foot">
                {move || {
                    store.link.with(|l| match &l.adapter {
                        Some(a) => format!("LINK · {a}"),
                        None => "LINK · none".into(),
                    })
                }}
                <br />
                {move || store.link.with(|l| l.local_ip.clone().unwrap_or_else(|| "no address".into()))}
                {api::mock().then(|| view! { <br /> "DEMO DATA" })}
            </div>
        </nav>
    }
}
