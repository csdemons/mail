//! RRCKI Mail для Astra Linux SE 1.7 (база Debian 10: glibc 2.28, WebKitGTK 4.0).
//! Tauri 2 требует WebKitGTK 4.1, которого там нет, поэтому эта сборка — та же
//! программа на Tauri 1: модули почты, календаря, контактов, Nextcloud и команды
//! берутся из `src-tauri/src` без изменений, своё здесь только окно и `host`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Часть общих функций используется только примерами и тестами основной сборки.
#![allow(dead_code)]

#[path = "../../src-tauri/src/commands.rs"]
mod commands;
#[path = "../../src-tauri/src/dav.rs"]
mod dav;
#[path = "../../src-tauri/src/error.rs"]
mod error;
mod host;
#[path = "../../src-tauri/src/html.rs"]
mod html;
#[path = "../../src-tauri/src/policy.rs"]
mod policy;
#[path = "../../src-tauri/src/imap.rs"]
mod imap;
#[path = "../../src-tauri/src/nextcloud.rs"]
mod nextcloud;
#[path = "../../src-tauri/src/settings.rs"]
mod settings;
#[path = "../../src-tauri/src/discover.rs"]
mod discover;
#[path = "../../src-tauri/src/desktop.rs"]
mod desktop;
#[path = "../../src-tauri/src/store.rs"]
mod store;
#[path = "../../src-tauri/src/mbox.rs"]
mod mbox;
#[path = "../../src-tauri/src/pst.rs"]
mod pst;
#[path = "../../src-tauri/src/transfer.rs"]
mod transfer;
#[path = "../../src-tauri/src/winacl.rs"]
mod winacl;
#[path = "../../src-tauri/src/netio.rs"]
mod netio;
#[path = "../../src-tauri/src/totp.rs"]
mod totp;
#[path = "../../src-tauri/src/smtp.rs"]
mod smtp;
#[path = "../../src-tauri/src/sogo.rs"]
mod sogo;
#[path = "../../src-tauri/src/updater.rs"]
mod updater;

/// Команды, разрешённые окнам писем (тот же список, что в capabilities сборки на Tauri 2).
fn message_window_commands() -> &'static std::collections::HashSet<String> {
    static SET: std::sync::OnceLock<std::collections::HashSet<String>> = std::sync::OnceLock::new();
    SET.get_or_init(|| {
        let caps: serde_json::Value = serde_json::from_str(include_str!("../../src-tauri/capabilities/message-window.json")).expect("message-window.json");
        caps["permissions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_str()?.strip_prefix("allow-").map(|c| c.replace('-', "_")))
            .collect()
    })
}

fn window_may_call(label: &str, command: &str) -> bool {
    label == "main" || (label.starts_with("msg-") && message_window_commands().contains(command))
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    tauri::Builder::default()
        .manage(commands::AppState::default())
        .setup(|app| {
            let handle = app.handle();
            host::enable_spellcheck(&handle);
            commands::setup_desktop(&handle);
            Ok(())
        })
        .invoke_handler({
            // В Tauri 1 нет разрешений по окнам (capabilities), поэтому то же правило,
            // что в сборке на Tauri 2, проверяется здесь: главное окно — все команды,
            // окна писем (msg-*) — только список из message-window.json, остальные
            // окна (редактор документов doc-*) — ни одной.
            let handler = typed_handler(commands::app_commands!());
            move |invoke: tauri::Invoke<tauri::Wry>| {
                let label = invoke.message.window().label().to_string();
                let command = invoke.message.command().to_string();
                if window_may_call(&label, &command) {
                    handler(invoke)
                } else {
                    log::warn!("command {command} refused for window {label}");
                    invoke.resolver.reject(format!("Команда «{command}» недоступна в этом окне"));
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while running RRCKI Mail")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                commands::on_exit_requested(app, || api.prevent_exit());
            }
        });
}

/// Задаёт тип замыкания из `generate_handler!` (сам макрос рассчитан на передачу
/// прямо в `invoke_handler`, где тип выводится из сигнатуры).
fn typed_handler<F: Fn(tauri::Invoke<tauri::Wry>) + Send + Sync + 'static>(f: F) -> F {
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_get_only_their_commands() {
        assert!(window_may_call("main", "logout"));
        assert!(window_may_call("msg-1", "get_message"));
        assert!(!window_may_call("msg-1", "logout"));
        assert!(!window_may_call("msg-1", "account_switch"));
        assert!(!window_may_call("doc-1", "get_message"));
        assert!(!window_may_call("other", "list_folders"));
    }
}
