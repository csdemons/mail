//! Настройки подключения (без пароля) и хранение пароля в системном хранилище:
//! Windows Credential Manager / Secret Service (GNOME Keyring, KWallet) в Linux.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

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
    /// Nextcloud (Deck, Talk): адрес, логин и ID пользователя; пароль приложения — в keyring.
    #[serde(default = "default_nextcloud_url")]
    pub nextcloud_url: String,
    #[serde(default)]
    pub nextcloud_login: String,
    #[serde(default)]
    pub nextcloud_uid: String,
}

pub const DEFAULT_NEXTCLOUD_URL: &str = "https://next.nrcki.ru/";

fn default_nextcloud_url() -> String {
    DEFAULT_NEXTCLOUD_URL.to_string()
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

/// Адрес календаря/контактов: пусто, имя сервера или https://… (http запрещён —
/// туда уходит пароль почты).
pub fn validate_dav_url(url: &str) -> Result<()> {
    let u = url.trim();
    if u.to_ascii_lowercase().starts_with("http://") {
        return Err(AppError::Other("Адрес календаря и контактов: только https:// (по http пароль передавался бы открытым текстом)".into()));
    }
    if u.contains("://") && !u.to_ascii_lowercase().starts_with("https://") {
        return Err(AppError::Other("Адрес календаря и контактов должен начинаться с https://".into()));
    }
    Ok(())
}

fn settings_path(app: &AppHandle) -> Result<PathBuf> {
    Ok(crate::host::config_dir(app)?.join("settings.json"))
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
    // Атомарно (временный файл + rename) и, в Linux, только для владельца:
    // каталог 0700, файл 0600 — другие пользователи компьютера не читают и не меняют настройки.
    let tmp = path.with_extension("json.tmp");
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        if let Some(dir) = path.parent() {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(&raw)?;
        f.sync_all()?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, raw)?;
    std::fs::rename(&tmp, &path)?;
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

const NC_KEYRING_SERVICE: &str = "ru.rrcki.mail.nextcloud";

fn nc_entry(base: &str, login: &str) -> keyring::Result<keyring::Entry> {
    let host = url::Url::parse(base).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    keyring::Entry::new(NC_KEYRING_SERVICE, &format!("{login}@{host}"))
}

pub fn nc_load_password(base: &str, login: &str) -> Option<String> {
    nc_entry(base, login).ok()?.get_password().ok()
}

pub fn nc_store_password(base: &str, login: &str, password: &str) -> std::result::Result<(), String> {
    nc_entry(base, login).and_then(|e| e.set_password(password)).map_err(|e| e.to_string())
}

pub fn nc_forget_password(base: &str, login: &str) {
    if let Ok(e) = nc_entry(base, login) {
        let _ = e.delete_credential();
    }
}
