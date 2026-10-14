//! Raw bridge to the Tauri runtime via `window.__TAURI__` (needs
//! `app.withGlobalTauri` in tauri.conf.json). Everything above this file is
//! plain typed Rust.

use serde::{de::DeserializeOwned, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn js_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen, catch)]
    async fn js_listen(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> Result<JsValue, JsValue>;
}

/// True inside the packaged app; false in a plain browser (mock mode).
pub fn is_tauri() -> bool {
    js_sys::Reflect::has(&js_sys::global(), &"__TAURI_INTERNALS__".into()).unwrap_or(false)
}

/// Normalize whatever a rejected promise carried into one line.
pub fn err_text(e: &JsValue) -> String {
    if let Some(s) = e.as_string() {
        return s;
    }
    if let Some(err) = e.dyn_ref::<js_sys::Error>() {
        return err.message().into();
    }
    js_sys::JSON::stringify(e)
        .ok()
        .and_then(|s| s.as_string())
        .unwrap_or_else(|| format!("{e:?}"))
}

/// Invoke a command with no logging — the log bus itself uses this, so it
/// can't recurse through the logged wrapper in api::mod.
pub async fn invoke_raw<T: DeserializeOwned>(cmd: &str, args: impl Serialize) -> Result<T, String> {
    // json_compatible: maps become plain objects, which is what Tauri expects
    let ser = serde_wasm_bindgen::Serializer::json_compatible();
    let args = args.serialize(&ser).map_err(|e| e.to_string())?;
    let out = js_invoke(cmd, args).await.map_err(|e| err_text(&e))?;
    serde_wasm_bindgen::from_value(out).map_err(|e| format!("{cmd}: bad reply — {e}"))
}

/// A live event subscription; dropping it unsubscribes.
pub struct Listener {
    unlisten: js_sys::Function,
    _handler: Closure<dyn FnMut(JsValue)>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.unlisten.call0(&JsValue::NULL);
    }
}

/// Subscribe to a backend event, decoding each payload as `T`.
pub async fn listen<T: DeserializeOwned + 'static>(
    event: &str,
    mut cb: impl FnMut(T) + 'static,
) -> Result<Listener, String> {
    let name = event.to_string();
    let handler = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        let payload = js_sys::Reflect::get(&e, &"payload".into()).unwrap_or(JsValue::NULL);
        match serde_wasm_bindgen::from_value::<T>(payload) {
            Ok(p) => cb(p),
            Err(err) => web_sys::console::warn_1(&format!("{name}: bad payload — {err}").into()),
        }
    });
    let unlisten = js_listen(event, &handler).await.map_err(|e| err_text(&e))?;
    Ok(Listener {
        unlisten: unlisten.unchecked_into(),
        _handler: handler,
    })
}
