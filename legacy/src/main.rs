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
#[path = "../../src-tauri/src/imap.rs"]
mod imap;
#[path = "../../src-tauri/src/nextcloud.rs"]
mod nextcloud;
#[path = "../../src-tauri/src/settings.rs"]
mod settings;
#[path = "../../src-tauri/src/smtp.rs"]
mod smtp;
#[path = "../../src-tauri/src/sogo.rs"]
mod sogo;
#[path = "../../src-tauri/src/updater.rs"]
mod updater;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    tauri::Builder::default()
        .manage(commands::AppState::default())
        .invoke_handler(commands::app_commands!())
        .run(tauri::generate_context!())
        .expect("error while running RRCKI Mail");
}
