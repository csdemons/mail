//! Настройки подключения (без пароля) и хранение пароля в системном хранилище:
//! Windows Credential Manager / Secret Service (GNOME Keyring, KWallet) в Linux.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::{AppError, Result};

const KEYRING_SERVICE: &str = "ru.rrcki.mail";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub username: String,
}

fn settings_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::Other(format!("Каталог настроек недоступен: {e}")))?;
    Ok(dir.join("settings.json"))
}

pub fn load(app: &AppHandle) -> Option<Settings> {
    let path = settings_path(app).ok()?;
    let raw = std::fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

pub fn save(app: &AppHandle, settings: &Settings) -> Result<()> {
    let path = settings_path(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let raw = serde_json::to_vec_pretty(settings).map_err(|e| AppError::Other(e.to_string()))?;
    std::fs::write(path, raw)?;
    Ok(())
}

fn entry(settings: &Settings) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("{}@{}", settings.username, settings.host),
    )
}

/// Пароль из системного хранилища. `None` — пароль не сохранён или хранилище недоступно.
pub fn load_password(settings: &Settings) -> Option<String> {
    entry(settings).ok()?.get_password().ok()
}

/// Сохраняет пароль. Ошибка возвращается текстом — это не повод прерывать вход
/// (например, на терминальном сервере без GNOME Keyring).
pub fn store_password(settings: &Settings, password: &str) -> std::result::Result<(), String> {
    entry(settings)
        .and_then(|e| e.set_password(password))
        .map_err(|e| e.to_string())
}

pub fn forget_password(settings: &Settings) {
    if let Ok(e) = entry(settings) {
        let _ = e.delete_credential();
    }
}
