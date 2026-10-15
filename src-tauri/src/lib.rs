mod commands;
mod crypto;
mod import;
mod logbus;
mod model;
mod net;
mod settings;
mod sources;
mod transfer;

use commands::Session;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Session::default())
        .setup(|app| {
            // Always on, release included: the log view and the on-disk log are
            // the only way a fault on someone else's PC can be diagnosed.
            logbus::init(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_link_status,
            commands::list_adapters,
            commands::start_receiver,
            commands::discover_peer,
            commands::list_local_ips,
            commands::pair,
            commands::list_sources,
            commands::measure_paths,
            commands::start_send,
            commands::link_benchmark,
            commands::receive,
            commands::cancel,
            commands::allow_firewall,
            commands::import_received,
            commands::get_drop_settings,
            commands::set_receive_root,
            commands::free_space,
            commands::import_browser_overwrite,
            commands::import_browser_retry,
            logbus::get_logs,
            logbus::clear_logs,
            logbus::ui_log,
            logbus::set_log_level,
            logbus::get_log_level,
            logbus::log_file_path,
            logbus::open_log_dir,
            logbus::open_log_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
