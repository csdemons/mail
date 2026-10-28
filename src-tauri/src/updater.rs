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
//! Подлинность: `latest.json` подписан ключом Ed25519 (файл `latest.json.sig`
//! рядом). Закрытый ключ есть только у сборки релизов (секрет CI), открытый
//! встроен в программу. Без верной подписи обновление не предлагается; без
//! SHA-256 у файла сборки — не скачивается. Версия ниже или равная текущей
//! не ставится (защита от отката на старую уязвимую версию).
//!
//! Администратор может переопределить адрес (внутреннее зеркало: туда копируются
//! `latest.json`, `latest.json.sig` и пакеты без изменений) или отключить проверку
//! файлом `update.json`: `{"url": "https://…/latest.json", "disabled": false}` —
//! только в `/etc/rrcki-mail/` или `%ProgramData%\RRCKI Mail\` (каталоги, куда
//! пишет только администратор).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};

pub const DEFAULT_MANIFEST_URL: &str = "https://github.com/csdemons/mail/releases/latest/download/latest.json";

/// Открытые ключи подписи `latest.json` (Ed25519, base64 32 байт). Несколько —
/// чтобы сменить ключ: новый добавляется заранее, старый убирается позже.
const UPDATE_PUBLIC_KEYS: &[&str] = &["UhcIkgOlNtHqJjtYlNf4Ij6HHuRbEgFNh+zptMcVBaY="];

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

fn decode_key(b64: &str) -> Option<ed25519_dalek::VerifyingKey> {
    use base64::Engine as _;
    let raw = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    ed25519_dalek::VerifyingKey::from_bytes(&raw.try_into().ok()?).ok()
}

fn trusted_keys() -> Vec<ed25519_dalek::VerifyingKey> {
    #[allow(unused_mut)]
    let mut keys: Vec<_> = UPDATE_PUBLIC_KEYS.iter().filter_map(|k| decode_key(k)).collect();
    // E2E: мок-сервер подписывает тестовым ключом. В релизной сборке этого кода нет.
    #[cfg(debug_assertions)]
    if let Ok(k) = std::env::var("RRCKI_E2E_UPDATE_PUBKEY") {
        keys.extend(decode_key(&k));
    }
    keys
}

/// Проверяет подпись (base64 64 байт) ровно над байтами `latest.json`.
pub fn verify_manifest(body: &[u8], signature_b64: &str, keys: &[ed25519_dalek::VerifyingKey]) -> Result<()> {
    use base64::Engine as _;
    let bad = || AppError::Other("Подпись обновления не прошла проверку — обновление не предлагается".into());
    let raw = base64::engine::general_purpose::STANDARD.decode(signature_b64.trim()).map_err(|_| bad())?;
    let sig = ed25519_dalek::Signature::from_slice(&raw).map_err(|_| bad())?;
    if keys.iter().any(|k| k.verify_strict(body, &sig).is_ok()) {
        Ok(())
    } else {
        Err(bad())
    }
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Разбор подписанного манифеста: версия и у каждого файла — SHA-256.
pub fn parse_manifest(body: &[u8]) -> Result<Manifest> {
    let m: Manifest = serde_json::from_slice(body).map_err(|e| AppError::Other(format!("Некорректный latest.json: {e}")))?;
    if parse_version(&m.version).iter().all(|&n| n == 0) {
        return Err(AppError::Other("latest.json: нет версии".into()));
    }
    for (variant, a) in &m.assets {
        if !is_sha256_hex(a.sha256.trim().trim_start_matches("sha256:")) {
            return Err(AppError::Other(format!("latest.json: у файла «{variant}» нет корректной SHA-256")));
        }
    }
    Ok(m)
}

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

/// `update.json` администратора. Пользовательский каталог читается только в
/// отладочной сборке (E2E) — в релизе пользователь не может подменить источник.
pub fn load_config(user_config_dir: Option<&Path>) -> UpdateConfig {
    #[cfg(not(debug_assertions))]
    let user_config_dir: Option<&Path> = {
        let _ = user_config_dir;
        None
    };
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

async fn fetch_small(client: &reqwest::Client, url: &str, what: &str) -> Result<Vec<u8>> {
    let mut resp = client.get(url).send().await.map_err(net)?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!("Сервер обновлений: {what} — HTTP {}", resp.status())));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(net)? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_MANIFEST_BYTES {
            return Err(AppError::Other(format!("Сервер обновлений: {what} слишком большой")));
        }
    }
    Ok(body)
}

/// `latest.json` и его подпись `latest.json.sig`; без верной подписи — ошибка.
pub async fn fetch_manifest(url: &str) -> Result<Manifest> {
    let parsed = url::Url::parse(url).map_err(|_| AppError::Other("Некорректный адрес обновлений".into()))?;
    if parsed.scheme() != "https" {
        return Err(AppError::Other("Обновления проверяются только по https".into()));
    }
    let client = http(Duration::from_secs(30))?;
    let body = fetch_small(&client, url, "latest.json").await?;
    let sig = fetch_small(&client, &format!("{url}.sig"), "подпись latest.json.sig").await?;
    verify_manifest(&body, &String::from_utf8_lossy(&sig), &trusted_keys())?;
    parse_manifest(&body)
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
    // SHA-256 обязательна: манифест подписан, сумма в нём гарантирует целостность файла.
    if !is_sha256_hex(&want) || got != want {
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

    fn test_key() -> (ed25519_dalek::SigningKey, ed25519_dalek::VerifyingKey) {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let vk = sk.verifying_key();
        (sk, vk)
    }

    fn sign(sk: &ed25519_dalek::SigningKey, body: &[u8]) -> String {
        use base64::Engine as _;
        use ed25519_dalek::Signer as _;
        base64::engine::general_purpose::STANDARD.encode(sk.sign(body).to_bytes())
    }

    #[test]
    fn manifest_signature_is_required_and_checked() {
        let (sk, vk) = test_key();
        let body = br#"{"version":"1.2.3","assets":{}}"#;
        let sig = sign(&sk, body);
        assert!(verify_manifest(body, &sig, &[vk]).is_ok());
        // Подменён один байт манифеста.
        assert!(verify_manifest(br#"{"version":"9.2.3","assets":{}}"#, &sig, &[vk]).is_err());
        // Подписан другим ключом.
        let other = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        assert!(verify_manifest(body, &sign(&other, body), &[vk]).is_err());
        // Пустая, мусорная, обрезанная подпись; нет доверенных ключей.
        for bad in ["", "not base64!", &sig[..20]] {
            assert!(verify_manifest(body, bad, &[vk]).is_err(), "{bad}");
        }
        assert!(verify_manifest(body, &sig, &[]).is_err());
        // Встроенный ключ корректен.
        assert!(trusted_keys().len() >= UPDATE_PUBLIC_KEYS.len());
        assert!(UPDATE_PUBLIC_KEYS.iter().all(|k| decode_key(k).is_some()));
    }

    #[test]
    fn accepts_signature_made_by_openssl_in_ci() {
        // Подпись сделана так же, как в CI: `openssl pkeyutl -sign -rawin` (ключ tests/servers/update-test-key.pem).
        let vk = decode_key("4AF+AFb4f3ka6BZU5jOqcgMUv3K0MRcK7SP/z57l4u0=").unwrap();
        let body = br#"{"version":"1.0.0","assets":{}}"#;
        assert!(verify_manifest(body, "FEvwJwJW++adoB5II/MP/SAUkvZmW7bvmRXPklJqso4JcpF+GK7COiTPb8K4L1OiDfp+xvMPHyKhcSAPqRChBg==\n", &[vk]).is_ok());
    }

    #[test]
    fn manifest_requires_sha256_for_every_file() {
        let ok = format!(r#"{{"version":"1.0.0","assets":{{"win":{{"name":"a.exe","url":"https://x/a.exe","sha256":"{}"}}}}}}"#, "a".repeat(64));
        assert!(parse_manifest(ok.as_bytes()).is_ok());
        for sha in ["", "00", &"g".repeat(64), &"a".repeat(63)] {
            let m = format!(r#"{{"version":"1.0.0","assets":{{"win":{{"name":"a.exe","url":"https://x/a.exe","sha256":"{sha}"}}}}}}"#);
            assert!(parse_manifest(m.as_bytes()).is_err(), "sha {sha:?}");
        }
        let no_sha = r#"{"version":"1.0.0","assets":{"win":{"name":"a.exe","url":"https://x/a.exe"}}}"#;
        assert!(parse_manifest(no_sha.as_bytes()).is_err());
        assert!(parse_manifest(br#"{"version":"","assets":{}}"#).is_err());
        // Откат: подписанный старый манифест не предлагается.
        assert!(!is_newer("0.9.0", "0.9.2"));
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
