use crate::api;
use crate::components::{
    cable::Cable, log_drawer::LogDrawer, role_picker::RolePicker, session::SessionController,
    step_rail::StepRail,
};
use crate::log;
use crate::steps::{
    code::CodeStep, connect::ConnectStep, done::DoneStep, pair::PairStep, select::SelectStep,
    transfer::TransferStep,
};
use crate::store::{use_store, Step};
use crate::types::Role;
use leptos::ev;
use leptos::prelude::*;

#[component]
fn StepView() -> impl IntoView {
    let store = use_store();
    // memo so a same-value set() doesn't remount the step
    let at = Memo::new(move |_| (store.role.get(), store.step.get()));
    move || match at.get() {
        (_, Step::Connect) => view! { <ConnectStep /> }.into_any(),
        (_, Step::Transfer) => view! { <TransferStep /> }.into_any(),
        (_, Step::Done) => view! { <DoneStep /> }.into_any(),
        (Some(Role::Send), Step::Pair) => view! { <PairStep /> }.into_any(),
        (Some(Role::Send), Step::Select) => view! { <SelectStep /> }.into_any(),
        (Some(Role::Receive), Step::Code) => view! { <CodeStep /> }.into_any(),
        _ => ().into_any(),
    }
}

#[component]
pub fn App() -> impl IntoView {
    let store = use_store();
    let has_role = Memo::new(move |_| store.role.get().is_some());
    let logs_open = RwSignal::new(false);
    let errors = RwSignal::new(0usize);

    // Ctrl+L (or Cmd+L) toggles the log drawer
    let hotkey = window_event_listener(ev::keydown, move |e| {
        if (e.ctrl_key() || e.meta_key()) && e.key().to_lowercase() == "l" {
            e.prevent_default();
            logs_open.update(|o| *o = !*o);
        }
    });
    on_cleanup(move || hotkey.remove());

    // Badge the toggle so a failure that the flow swallowed is still visible.
    let entries = log::entries();
    Effect::new(move |_| {
        errors.set(entries.with(|es| es.iter().filter(|e| e.level == "error").count()));
    });
    Effect::new(move |_| {
        if logs_open.get() {
            errors.set(0);
        }
    });

    view! {
        <div class="app">
            <header class="topbar">
                <div class="brand">
                    <span class="brand__mark">"WIN" <b>"CI"</b></span>
                    <span class="brand__tag">"Cable or IP data crossing"</span>
                </div>
                <div class="topbar__actions">
                    <button
                        class="btn btn--ghost"
                        on:click=move |_| logs_open.update(|o| *o = !*o)
                        title="Activity log (Ctrl+L)"
                    >
                        "Logs"
                        <Show when=move || { !logs_open.get() && errors.get() > 0 }>
                            <span class="logbtn__dot">{move || errors.get()}</span>
                        </Show>
                    </button>
                    <Show when=move || has_role.get()>
                        <button class="btn btn--ghost" on:click=move |_| store.reset()>
                            "Start over"
                        </button>
                    </Show>
                </div>
            </header>

            <Show when=move || has_role.get() fallback=|| view! { <RolePicker /> }>
                <Cable />
                <div class="body">
                    <StepRail />
                    <main class="main">
                        <StepView />
                    </main>
                </div>
                <SessionController />
            </Show>

            <LogDrawer open=logs_open />

            {api::mock().then(|| view! { <div class="mock-flag">"Demo mode · no cable"</div> })}
        </div>
    }
}
