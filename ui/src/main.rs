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
    // panics also go to the page's boot reporter (index.html) and winc.log
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        let report = js_sys::Reflect::get(&js_sys::global(), &"__wincBootError".into())
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
        if let Some(f) = report {
            let _ = f.call1(&wasm_bindgen::JsValue::NULL, &format!("panic: {info}").into());
        }
    }));

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

    // After mount: mounting sets up the async executor that log::init's
    // spawn_local needs — calling it earlier panics in the packaged app.
    log::init();
}
