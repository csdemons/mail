#[doc(hidden)]
pub mod dav;
mod error;
#[doc(hidden)]
pub mod nextcloud;
#[doc(hidden)]
pub mod imap;
mod settings;
#[doc(hidden)]
pub mod smtp;
pub mod sogo;
mod commands;
mod host;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                })
                .build(),
        )
        .manage(commands::AppState::default())
        .invoke_handler(commands::app_commands!())
        .run(tauri::generate_context!())
        .expect("error while running RRCKI Mail");
}
