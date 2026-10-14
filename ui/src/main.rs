mod api;
mod app;
mod components;
mod format;
mod log;
mod steps;
mod store;
mod types;

use leptos::prelude::*;
use wasm_bindgen::JsCast;

fn main() {
    console_error_panic_hook::set_once();
    log::init();

    let root = document()
        .get_element_by_id("root")
        .expect("index.html has #root")
        .unchecked_into::<web_sys::HtmlElement>();

    // One bundle, two roots: the popout loads the same index.html with ?view=log.
    if log::is_log_window() {
        leptos::mount::mount_to(root, components::log_panel::LogWindow).forget();
    } else {
        leptos::mount::mount_to(root, || {
            store::provide_store();
            view! { <app::App /> }
        })
        .forget();
    }
}
