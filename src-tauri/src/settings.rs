//! Настройки подключения (без пароля) и хранение пароля в системном хранилище:
//! Windows Credential Manager / Secret Service (GNOME Keyring, KWallet) в Linux.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::{AppError, Result};

const KEYRING_SERVICE: &str = "ru.rrcki.mail";

pub const DEFAULT_SMTP_HOST: &str = "rp.nrcki.ru";
/// В 0.2.0 по умолчанию стоял несуществующий rp.rrcki.ru (опечатка) — исправляем при загрузке.
const BROKEN_SMTP_HOST: &str = "rp.rrcki.ru";
pub const DEFAULT_SMTP_PORT: u16 = 465;

fn default_smtp_host() -> String {
    DEFAULT_SMTP_HOST.to_string()
}
fn default_smtp_port() -> u16 {
    DEFAULT_SMTP_PORT
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Исходящий сервер (SMTPS 465 или STARTTLS 587), тот же логин/пароль.
    #[serde(default = "default_smtp_host")]
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    /// Адрес сервера календаря/контактов (CalDAV/CardDAV). Пусто — https://<IMAP-сервер>.
    #[serde(default)]
    pub dav_url: String,
    /// Имя отправителя в поле «От».
    #[serde(default)]
    pub display_name: String,
}

impl Settings {
    /// Адрес отправителя: логин, если это e-mail.
    pub fn email(&self) -> String {
        if self.username.contains('@') {
            self.username.clone()
        } else {
            let domain = self.host.split_once('.').map(|(_, d)| d).unwrap_or(&self.host);
            format!("{}@{}", self.username, domain)
        }
    }

    pub fn dav_base(&self) -> String {
        let url = self.dav_url.trim();
        if url.is_empty() {
            format!("https://{}/", self.host)
        } else if url.contains("://") {
            url.to_string()
        } else {
            format!("https://{url}/")
        }
    }
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
    let mut s: Settings = serde_json::from_slice(&raw).ok()?;
    if s.smtp_host.eq_ignore_ascii_case(BROKEN_SMTP_HOST) {
        s.smtp_host = s.host.clone();
    }
    Some(s)
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
