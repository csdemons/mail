#[doc(hidden)]
pub mod dav;
mod error;
mod html;
mod policy;
#[doc(hidden)]
pub mod nextcloud;
#[doc(hidden)]
pub mod imap;
mod settings;
mod discover;
mod desktop;
mod store;
mod totp;
mod netio;
mod winacl;
mod mbox;
mod pst;
mod transfer;
#[doc(hidden)]
pub mod smtp;
pub mod sogo;
pub mod updater;
mod commands;
mod host;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
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
        .setup(|app| {
            let handle = app.handle().clone();
            host::enable_spellcheck(&handle);
            commands::setup_desktop(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            use tauri::Manager;
            // Закрытие главного окна при включённом трее — свернуть, а не выйти:
            // почта, уведомления и отложенная отправка продолжают работать.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" && window.app_handle().state::<commands::AppState>().keeps_running() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(commands::app_commands!())
        .build(tauri::generate_context!())
        .expect("error while running RRCKI Mail")
        .run(|app, event| {
            match event {
                tauri::RunEvent::ExitRequested { api, .. } => commands::on_exit_requested(app, || api.prevent_exit()),
                // macOS: щелчок по значку в Dock, когда окно свёрнуто в трей (строку меню).
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => host::show_main(app),
                _ => {}
            }
        });
}
