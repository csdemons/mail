//! Проверка и установка обновлений.
//!
//! К каждому релизу CI прикладывает `latest.json`: версия, список изменений и для
//! каждой сборки — имя файла, ссылка и SHA-256. Клиент читает его по постоянной
//! ссылке `releases/latest/download/latest.json` (это не API GitHub, лимита
//! 60 запросов в час на IP нет), выбирает файл своей сборки, скачивает,
//! сверяет SHA-256 и ставит:
//! - Windows — установщик NSIS в тихом режиме, программа закрывается;
//! - Linux .deb/.rpm — через `pkexec` (пароль администратора); не вышло —
//!   пользователь получает путь к пакету и команду;
//! - AppImage — файл заменяется на месте, без прав администратора.
//!
//! Администратор может переопределить адрес (внутреннее зеркало) или отключить
//! проверку файлом `update.json`: `{"url": "https://…/latest.json", "disabled": false}`
//! в `/etc/rrcki-mail/`, `%ProgramData%\RRCKI Mail\` или в каталоге настроек.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};

pub const DEFAULT_MANIFEST_URL: &str = "https://github.com/csdemons/mail/releases/latest/download/latest.json";

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestAsset {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub pub_date: String,
    #[serde(default)]
    pub assets: HashMap<String, ManifestAsset>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateConfig {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub version: String,
    pub notes: String,
    pub size: u64,
    pub file_name: String,
    /// Сборка, для которой выбран файл (win, win8, deb, astra17, rpm, appimage).
    pub variant: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    /// Программа сейчас закроется (установщик запущен / будет перезапуск).
    pub restarting: bool,
    /// Если поставить автоматически не удалось — что сделать вручную.
    pub manual: Option<String>,
}

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
}

fn http(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(15))
        .user_agent(concat!("RRCKI Mail/", env!("CARGO_PKG_VERSION"), " (updater)"))
        .build()
        .map_err(|e| AppError::Other(format!("HTTP-клиент: {e}")))
}

fn net(e: reqwest::Error) -> AppError {
    let mut detail = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(cause) = src {
        detail.push_str(&format!(": {cause}"));
        src = cause.source();
    }
    AppError::Other(format!("Сервер обновлений недоступен: {detail}"))
}

/// Системный, затем пользовательский `update.json` (пользовательский важнее).
pub fn load_config(user_config_dir: Option<&Path>) -> UpdateConfig {
    let mut paths: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        if let Ok(pd) = std::env::var("ProgramData") {
            paths.push(Path::new(&pd).join("RRCKI Mail").join("update.json"));
        }
    } else {
        paths.push(PathBuf::from("/etc/rrcki-mail/update.json"));
    }
    if let Some(d) = user_config_dir {
        paths.push(d.join("update.json"));
    }
    let mut cfg = UpdateConfig::default();
    for p in paths {
        if let Ok(raw) = std::fs::read(&p) {
            if let Ok(c) = serde_json::from_slice::<UpdateConfig>(&raw) {
                if c.url.as_deref().is_some_and(|u| !u.trim().is_empty()) {
                    cfg.url = c.url;
                }
                cfg.disabled |= c.disabled;
            }
        }
    }
    cfg
}

/// 0.3.10 > 0.3.9; суффиксы (`-beta`) отбрасываются.
pub fn parse_version(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches('v')
        .split(['-', '+', '~'])
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (mut a, mut b) = (parse_version(candidate), parse_version(current));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// Какая сборка запущена. `forced` — сборка задана при компиляции (Astra 1.7).
pub fn detect_variant(forced: Option<&str>) -> String {
    // E2E: сборка без установки, «вариант» задаёт тест. В релизе этого кода нет.
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("RRCKI_E2E_VARIANT") {
        return v;
    }
    if let Some(v) = forced {
        return v.to_string();
    }
    if cfg!(windows) {
        // Сборка под Windows 8 — цель x86_64-win7-windows-*: target_vendor = "win7".
        return if cfg!(target_vendor = "win7") { "win8".into() } else { "win".into() };
    }
    if std::env::var_os("APPIMAGE").is_some() {
        return "appimage".into();
    }
    if Path::new("/var/lib/dpkg/info/rrcki-mail.list").exists() {
        return "deb".into();
    }
    if std::process::Command::new("rpm").args(["-q", "rrcki-mail"]).output().map(|o| o.status.success()).unwrap_or(false) {
        return "rpm".into();
    }
    // Запуск из каталога сборки и т.п. — автообновления нет.
    "unknown".into()
}

pub async fn fetch_manifest(url: &str) -> Result<Manifest> {
    let resp = http(Duration::from_secs(30))?.get(url).send().await.map_err(net)?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!("Сервер обновлений ответил HTTP {}", resp.status())));
    }
    resp.json::<Manifest>().await.map_err(|e| AppError::Other(format!("Некорректный latest.json: {e}")))
}

pub fn pick(manifest: &Manifest, variant: &str) -> Option<ManifestAsset> {
    manifest.assets.get(variant).cloned()
}

/// Только https и только файлы с ожидаемым расширением.
fn check_asset(asset: &ManifestAsset) -> Result<()> {
    let url = url::Url::parse(&asset.url).map_err(|_| AppError::Other("Некорректная ссылка на обновление".into()))?;
    if url.scheme() != "https" {
        return Err(AppError::Other("Обновление должно скачиваться по https".into()));
    }
    let ok_ext = [".exe", ".deb", ".rpm", ".AppImage"].iter().any(|e| asset.name.ends_with(e));
    if !ok_ext || asset.name.contains(['/', '\\']) || asset.name.starts_with('.') {
        return Err(AppError::Other("Неожиданный файл обновления".into()));
    }
    Ok(())
}

/// Скачивает файл в `dir`, проверяет SHA-256 (если он указан в latest.json).
pub async fn download(asset: &ManifestAsset, dir: &Path, progress: &Progress) -> Result<PathBuf> {
    check_asset(asset)?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(&asset.name);
    let part = dir.join(format!("{}.part", asset.name));
    let mut resp = http(Duration::from_secs(60 * 60))?.get(&asset.url).send().await.map_err(net)?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!("Скачивание обновления: HTTP {}", resp.status())));
    }
    progress.done.store(0, Ordering::Relaxed);
    progress.total.store(resp.content_length().unwrap_or(asset.size), Ordering::Relaxed);
    let mut hasher = Sha256::new();
    let mut file = tokio::fs::File::create(&part).await?;
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = resp.chunk().await.map_err(net)? {
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        progress.done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    file.flush().await?;
    drop(file);
    let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let want = asset.sha256.trim().trim_start_matches("sha256:").to_ascii_lowercase();
    if !want.is_empty() && got != want {
        let _ = std::fs::remove_file(&part);
        return Err(AppError::Other("Контрольная сумма обновления не совпала — файл повреждён или подменён".into()));
    }
    std::fs::rename(&part, &path)?;
    Ok(path)
}

/// Ставит скачанный файл. На Windows и для AppImage программа после этого
/// должна закрыться (`restarting: true`).
pub fn install(variant: &str, file: &Path) -> Result<InstallResult> {
    let manual = |cmd: String| Ok(InstallResult { restarting: false, manual: Some(cmd) });
    match variant {
        "win" | "win8" => {
            // /P — без вопросов, только окно прогресса; /R — запустить программу после установки.
            match std::process::Command::new(file).args(["/P", "/R"]).spawn() {
                Ok(_) => Ok(InstallResult { restarting: true, manual: None }),
                // 740 — программа установлена «для всех пользователей» (Program Files),
                // установщику нужны права администратора. CreateProcess их не
                // запрашивает; запуск через оболочку (как двойной щелчок) покажет UAC.
                Err(e) if e.raw_os_error() == Some(740) => {
                    run_elevated(file)?;
                    Ok(InstallResult { restarting: true, manual: None })
                }
                Err(e) => Err(AppError::Other(format!("Не удалось запустить установщик: {e}"))),
            }
        }
        "appimage" => {
            let target = std::env::var_os("APPIMAGE").map(PathBuf::from).ok_or_else(|| AppError::Other("Путь к AppImage неизвестен".into()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))?;
            }
            // rename в пределах одного каталога атомарен; иначе — копирование.
            if std::fs::rename(file, &target).is_err() {
                std::fs::copy(file, &target)?;
            }
            std::process::Command::new(&target).spawn().map_err(|e| AppError::Other(format!("Перезапуск: {e}")))?;
            Ok(InstallResult { restarting: true, manual: None })
        }
        "deb" | "astra17" | "rpm" => {
            let path = file.display().to_string();
            let (tool, args): (&str, Vec<&str>) = if variant == "rpm" {
                ("dnf", vec!["install", "-y", &path])
            } else {
                ("apt-get", vec!["install", "-y", "--allow-downgrades", &path])
            };
            let hint = format!("sudo {tool} {}", args.iter().map(|a| if a.contains(' ') { format!("'{a}'") } else { a.to_string() }).collect::<Vec<_>>().join(" "));
            let status = std::process::Command::new("pkexec").arg(tool).args(&args).status();
            match status {
                Ok(s) if s.success() => {
                    // Пакет обновил /usr/bin/rrcki-mail — запускаем новую версию.
                    let _ = std::process::Command::new("/usr/bin/rrcki-mail").spawn();
                    Ok(InstallResult { restarting: true, manual: None })
                }
                _ => manual(hint),
            }
        }
        _ => manual(format!("Скачанный файл: {}", file.display())),
    }
}

/// Запуск через оболочку Windows (`start`): для программ, которым нужны права
/// администратора, Windows сама покажет запрос UAC.
#[cfg(windows)]
fn run_elevated(file: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(file)
        .args(["/P", "/R"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| AppError::Other(format!("Не удалось запустить установщик с правами администратора: {e}")))?;
    Ok(())
}

#[cfg(not(windows))]
fn run_elevated(_file: &Path) -> Result<()> {
    Err(AppError::Other("Требуются права администратора".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.3.10", "0.3.9"));
        assert!(is_newer("v1.0.0", "0.9.99"));
        assert!(!is_newer("0.3.6", "0.3.6"));
        assert!(!is_newer("0.3.6", "0.3.6~astra1.7"));
        assert!(is_newer("0.4", "0.3.99"));
        assert!(!is_newer("0.3.5", "0.3.6"));
    }

    #[test]
    fn parses_manifest_and_rejects_bad_assets() {
        let m: Manifest = serde_json::from_str(r#"{"version":"0.3.7","notes":"- новое","assets":{
            "win":{"name":"rrcki-mail_0.3.7_x64-setup.exe","url":"https://github.com/x/y/releases/download/v0.3.7/rrcki-mail_0.3.7_x64-setup.exe","sha256":"ab","size":5},
            "deb":{"name":"../evil.deb","url":"https://x/evil.deb"},
            "rpm":{"name":"a.rpm","url":"http://x/a.rpm"}}}"#).unwrap();
        assert_eq!(m.version, "0.3.7");
        assert!(check_asset(&pick(&m, "win").unwrap()).is_ok());
        assert!(check_asset(&pick(&m, "deb").unwrap()).is_err());
        assert!(check_asset(&pick(&m, "rpm").unwrap()).is_err());
        assert!(pick(&m, "astra17").is_none());
    }

    #[test]
    fn reads_admin_config() {
        let dir = std::env::temp_dir().join(format!("rrcki-upd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("update.json"), r#"{"url":"https://mirror.local/latest.json","disabled":true}"#).unwrap();
        let c = load_config(Some(&dir));
        assert_eq!(c.url.as_deref(), Some("https://mirror.local/latest.json"));
        assert!(c.disabled);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
