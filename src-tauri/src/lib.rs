mod commands;
mod model;
mod net;
mod sources;

use commands::Session;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Session::default())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_link_status,
            commands::start_receiver,
            commands::discover_peer,
            commands::list_local_ips,
            commands::pair,
            commands::list_sources,
            commands::start_send,
            commands::receive,
            commands::cancel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
