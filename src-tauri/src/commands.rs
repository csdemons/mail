//! Команды приложения (вызываются из интерфейса через `invoke`). Код общий для
//! обычной сборки (Tauri 2) и сборки для старых систем (Tauri 1, Astra 1.7):
//! всё, что отличается между версиями Tauri, — в модуле `host`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;

use crate::error::{AppError, Result};
use crate::host;
use crate::{dav, imap, nextcloud, settings, smtp, sogo, updater};
use crate::imap::{Connection, Credentials, FlagKind, Folder, MessageDetail, MessagePage};
use crate::settings::Settings;

const OP_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Default)]
pub struct AppState {
    /// Учётные данные текущей сессии (пароль только в памяти процесса).
    creds: Mutex<Option<Credentials>>,
    /// Открытое IMAP-соединение; переиспользуется между командами.
    conn: Mutex<Option<Connection>>,
    /// Настройки текущей сессии (SMTP, DAV, имя отправителя).
    profile: Mutex<Option<Settings>>,
    /// Файлы, выбранные пользователем для вложения: id → путь. Пути не
    /// передаются во frontend и не принимаются от него.
    picked: Mutex<HashMap<String, PathBuf>>,
    next_pick: AtomicU64,
    /// Найденные календари и адресные книги (автообнаружение один раз за сессию).
    calendars: Mutex<Option<Vec<dav::Collection>>>,
    books: Mutex<Option<Vec<dav::Collection>>>,
    /// Подключение к Nextcloud: учётные данные и ID пользователя.
    nc: Mutex<Option<(nextcloud::NcCreds, String)>>,
    /// Незавершённый вход через браузер (Login Flow v2).
    nc_flow: Mutex<Option<(url::Url, nextcloud::LoginFlow)>>,
    /// Сессия веб-API SOGo для поиска адресатов (или текст ошибки входа,
    /// чтобы не повторять неудачный вход на каждое нажатие клавиши).
    sogo: Mutex<Option<std::result::Result<sogo::SogoSession, String>>>,
    /// Найденное обновление и ход его скачивания.
    update: Mutex<Option<(updater::UpdateInfo, updater::ManifestAsset)>>,
    update_progress: updater::Progress,
}

/// Выполняет операцию на открытом соединении. Если соединение оборвалось
/// (сервер закрыл его по таймауту простоя и т.п.) — переподключается и
/// повторяет операцию один раз.
macro_rules! with_conn {
    ($state:expr, $c:ident => $body:expr) => {{
        let creds = $state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
        let mut guard = $state.conn.lock().await;
        let mut attempt = 0;
        loop {
            if guard.is_none() {
                *guard = Some(imap::connect(&creds).await?);
            }
            let $c = guard.as_mut().expect("connection present");
            let result = match tokio::time::timeout(OP_TIMEOUT, $body).await {
                Ok(r) => r,
                Err(_) => Err(AppError::Timeout),
            };
            match result {
                Err(e) if e.is_connection() => {
                    *guard = None;
                    if attempt == 0 {
                        attempt += 1;
                        log::warn!("IMAP connection lost, reconnecting: {e}");
                        continue;
                    }
                    break Err(e);
                }
                other => break other,
            }
        }
    }};
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    username: String,
    host: String,
    port: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    account: Account,
    /// Предупреждение, если пароль не удалось сохранить в системное хранилище.
    warning: Option<String>,
}

async fn start_session(state: &AppState, creds: Credentials, profile: Settings) -> Result<()> {
    let conn = imap::connect(&creds).await?;
    if let Some(old) = state.conn.lock().await.replace(conn) {
        old.logout().await;
    }
    *state.creds.lock().await = Some(creds);
    *state.nc.lock().await = load_nc(&profile);
    *state.profile.lock().await = Some(profile);
    *state.calendars.lock().await = None;
    *state.books.lock().await = None;
    // Сессия веб-API SOGo и выбранные файлы принадлежали прежнему входу.
    if let Some(Ok(session)) = state.sogo.lock().await.take() {
        session.logout().await;
    }
    state.picked.lock().await.clear();
    Ok(())
}

fn load_nc(p: &Settings) -> Option<(nextcloud::NcCreds, String)> {
    if p.nextcloud_login.is_empty() {
        return None;
    }
    let base = nextcloud::normalize_base(&p.nextcloud_url).ok()?;
    let password = settings::nc_load_password(base.as_str(), &p.nextcloud_login)?;
    let uid = if p.nextcloud_uid.is_empty() { p.nextcloud_login.clone() } else { p.nextcloud_uid.clone() };
    Some((nextcloud::NcCreds { base, login: p.nextcloud_login.clone(), app_password: password }, uid))
}

fn account_of(s: &Settings) -> Account {
    Account {
        username: s.username.clone(),
        host: s.host.clone(),
        port: s.port,
    }
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Option<Settings> {
    settings::load(&app)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn login(
    app: AppHandle,
    state: State<'_, AppState>,
    host: String,
    port: u16,
    username: String,
    password: String,
    remember: bool,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    dav_url: Option<String>,
) -> Result<LoginResult> {
    let host = host.trim().to_string();
    let username = username.trim().to_string();
    if host.is_empty() || username.is_empty() || password.is_empty() {
        return Err(AppError::Other("Укажите сервер, логин и пароль".into()));
    }

    let previous = settings::load(&app);
    // Другой сотрудник на том же компьютере не должен унаследовать чужой Nextcloud
    // (Deck, Talk, Файлы) и имя отправителя: эти поля берём, только если учётка та же.
    let same_user = previous
        .as_ref()
        .is_some_and(|p| p.username.eq_ignore_ascii_case(&username) && p.host.eq_ignore_ascii_case(&host));
    let mine = previous.as_ref().filter(|_| same_user);
    if let Some(p) = previous.as_ref().filter(|_| !same_user) {
        if !p.nextcloud_login.is_empty() {
            if let Ok(base) = nextcloud::normalize_base(&p.nextcloud_url) {
                settings::nc_forget_password(base.as_str(), &p.nextcloud_login);
            }
        }
    }
    let s = Settings {
        host: host.clone(),
        port,
        username: username.clone(),
        smtp_host: smtp_host
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty())
            .or_else(|| previous.as_ref().map(|p| p.smtp_host.clone()))
            .unwrap_or_else(|| settings::DEFAULT_SMTP_HOST.to_string()),
        smtp_port: smtp_port
            .or_else(|| previous.as_ref().map(|p| p.smtp_port))
            .unwrap_or(settings::DEFAULT_SMTP_PORT),
        dav_url: dav_url
            .map(|u| u.trim().to_string())
            .or_else(|| previous.as_ref().map(|p| p.dav_url.clone()))
            .unwrap_or_default(),
        display_name: mine.map(|p| p.display_name.clone()).unwrap_or_default(),
        nextcloud_url: previous
            .as_ref()
            .map(|p| p.nextcloud_url.clone())
            .unwrap_or_else(|| settings::DEFAULT_NEXTCLOUD_URL.to_string()),
        nextcloud_login: mine.map(|p| p.nextcloud_login.clone()).unwrap_or_default(),
        nextcloud_uid: mine.map(|p| p.nextcloud_uid.clone()).unwrap_or_default(),
    };

    let creds = Credentials {
        host,
        port,
        username,
        password: password.clone(),
    };
    start_session(&state, creds, s.clone()).await?;

    // Вход уже выполнен: ошибки сохранения настроек/пароля не отменяют его,
    // а показываются пользователю предупреждением.
    let mut warnings = Vec::new();
    if let Err(e) = settings::save(&app, &s) {
        log::warn!("settings not saved: {e}");
        warnings.push(format!("Не удалось сохранить настройки подключения: {e}."));
    }
    if remember {
        if let Err(e) = settings::store_password(&s, &password) {
            log::warn!("keyring unavailable: {e}");
            warnings.push("Не удалось сохранить пароль в системном хранилище — при следующем запуске его нужно будет ввести снова.".to_string());
        }
    } else {
        settings::forget_password(&s);
    }
    let warning = (!warnings.is_empty()).then(|| warnings.join(" "));

    Ok(LoginResult {
        account: account_of(&s),
        warning,
    })
}

/// Вход с сохранённым паролем. `None` — пароль не сохранён, нужен экран входа.
#[tauri::command]
pub async fn auto_login(app: AppHandle, state: State<'_, AppState>) -> Result<Option<Account>> {
    let Some(s) = settings::load(&app) else {
        return Ok(None);
    };
    let Some(password) = settings::load_password(&s) else {
        return Ok(None);
    };
    let creds = Credentials {
        host: s.host.clone(),
        port: s.port,
        username: s.username.clone(),
        password,
    };
    match start_session(&state, creds, s.clone()).await {
        Ok(()) => Ok(Some(account_of(&s))),
        Err(AppError::AuthFailed) => {
            // Пароль сменили — сохранённый больше не годится.
            settings::forget_password(&s);
            Err(AppError::AuthFailed)
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn logout(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    // Окна писем показывают данные этой учётной записи.
    host::close_message_windows(&app);
    // «Выйти и забыть пароль» забывает и подключение к Nextcloud: пароль приложения
    // отзывается на сервере и удаляется из системного хранилища.
    *state.nc_flow.lock().await = None;
    if let Some((creds, uid)) = state.nc.lock().await.take() {
        if let Ok(client) = nextcloud::Nc::new(creds.clone(), &uid) {
            client.revoke().await;
        }
        settings::nc_forget_password(creds.base.as_str(), &creds.login);
    }
    if let Some(mut s) = settings::load(&app) {
        settings::forget_password(&s);
        if !s.nextcloud_login.is_empty() {
            if let Ok(base) = nextcloud::normalize_base(&s.nextcloud_url) {
                settings::nc_forget_password(base.as_str(), &s.nextcloud_login);
            }
            s.nextcloud_login.clear();
            s.nextcloud_uid.clear();
            let _ = settings::save(&app, &s);
        }
    }
    *state.creds.lock().await = None;
    *state.profile.lock().await = None;
    *state.calendars.lock().await = None;
    *state.books.lock().await = None;
    if let Some(Ok(session)) = state.sogo.lock().await.take() {
        session.logout().await;
    }
    state.picked.lock().await.clear();
    if let Some(conn) = state.conn.lock().await.take() {
        conn.logout().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn list_folders(state: State<'_, AppState>) -> Result<Vec<Folder>> {
    with_conn!(state, c => imap::list_folders(c))
}

#[tauri::command]
pub async fn list_messages(
    state: State<'_, AppState>,
    folder: String,
    before_uid: Option<u32>,
    limit: u32,
) -> Result<MessagePage> {
    with_conn!(state, c => imap::list_messages(c, &folder, before_uid, limit))
}

/// Текстовый отчёт о том, что отвечает сервер (без пароля) — для разбора проблем.
#[tauri::command]
pub async fn diagnostics(state: State<'_, AppState>, folder: Option<String>) -> Result<String> {
    let header = state
        .creds
        .lock()
        .await
        .clone()
        .map(|c| format!("Сервер: {}:{}, пользователь: {}\n", c.host, c.port, c.username))
        .unwrap_or_default();
    // Отдельное соединение: диагностика не должна повлиять на рабочую сессию.
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let mut conn = imap::connect(&creds).await?;
    let report = tokio::time::timeout(OP_TIMEOUT * 2, imap::diagnostics(&mut conn, folder.as_deref()))
        .await
        .map_err(|_| AppError::Timeout)??;
    conn.logout().await;

    let mut extra = String::new();
    if let Some(p) = state.profile.lock().await.clone() {
        extra.push_str(&format!("\n## SMTP {}:{}\n", p.smtp_host, p.smtp_port));
        match smtp::check(&p.smtp_host, p.smtp_port, &creds.username, &creds.password).await {
            Ok(true) => extra.push_str("подключение и EHLO: ok (вход проверяется при отправке)\n"),
            Ok(false) => extra.push_str("сервер не ответил на EHLO\n"),
            Err(e) => extra.push_str(&format!("ошибка: {e}\n")),
        }
        for (title, kind) in [("CalDAV", dav::Kind::Calendar), ("CardDAV", dav::Kind::Contacts)] {
            extra.push_str(&format!("\n## {title} ({})\n", p.dav_base()));
            let base = url::Url::parse(&p.dav_base()).map_err(|e| AppError::Other(e.to_string()))?;
            match dav::DavClient::new(&creds.username, &creds.password, &base) {
                Ok(mut client) => {
                    let res = client.discover(&base, kind).await;
                    for line in &client.log {
                        extra.push_str(line);
                        extra.push('\n');
                    }
                    match res {
                        Ok(cols) => {
                            // Пробный поиск: корпоративная книга (LDAP) отвечает только на поиск.
                            let probe: String = creds.username.split('@').next().unwrap_or_default().split(['_', '.']).next().unwrap_or_default().chars().take(6).collect();
                            for c in cols {
                                extra.push_str(&format!("коллекция: {} — {}\n", c.name, c.url));
                                if matches!(kind, dav::Kind::Contacts) && probe.chars().count() >= 2 {
                                    match client.contacts(&c).await {
                                        Ok(all) => extra.push_str(&format!("  весь список: {} записей\n", all.len())),
                                        Err(e) => extra.push_str(&format!("  весь список: ошибка: {e}\n")),
                                    }
                                    match client.search_contacts(&c, &probe, 25).await {
                                        Ok(found) => extra.push_str(&format!("  поиск «{probe}»: {} записей\n", found.len())),
                                        Err(e) => extra.push_str(&format!("  поиск «{probe}»: ошибка: {e}\n")),
                                    }
                                    if !c.writable {
                                        // Подробно для корпоративной книги: что именно отвечает сервер.
                                        for (label, props) in [("FN+EMAIL+N+ORG", &["FN", "EMAIL", "N", "ORG"][..]), ("только FN", &["FN"][..]), ("только EMAIL", &["EMAIL"][..])] {
                                            let body = dav::DavClient::search_body(&probe, props, 5);
                                            match client.report_raw(&c.url, &body).await {
                                                Ok((st, text)) => {
                                                    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                                                    extra.push_str(&format!("  REPORT {label} → {st}, {} байт: {}\n", text.len(), one.chars().take(400).collect::<String>()));
                                                }
                                                Err(e) => extra.push_str(&format!("  REPORT {label} → ошибка: {e}\n")),
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => extra.push_str(&format!("ошибка: {e}\n")),
                    }
                }
                Err(e) => extra.push_str(&format!("ошибка: {e}\n")),
            }
        }
    }
    if let Some(p) = state.profile.lock().await.clone() {
        let probe: String = creds.username.split('@').next().unwrap_or_default().split(['_', '.']).next().unwrap_or_default().chars().take(6).collect();
        extra.push_str("\n## Веб-API SOGo (поиск адресатов)\n");
        match url::Url::parse(&p.dav_base()) {
            Ok(base) => {
                let mut log = Vec::new();
                let res = sogo::login(&base, &creds.username, &creds.password, &mut log).await;
                for l in &log {
                    extra.push_str(l);
                    extra.push('\n');
                }
                match res {
                    Ok(session) => {
                        match session.search_raw(&probe).await {
                            Ok((st, text)) => {
                                let parsed = serde_json::from_str::<serde_json::Value>(&text).map(|v| sogo::parse_search(&v).len());
                                let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                                extra.push_str(&format!("allContactSearch «{probe}» → {st}, найдено: {}, ответ: {}\n",
                                    parsed.map(|n| n.to_string()).unwrap_or_else(|_| "не JSON".into()), one.chars().take(400).collect::<String>()));
                            }
                            Err(e) => extra.push_str(&format!("allContactSearch: ошибка: {e}\n")),
                        }
                        match session.defaults().await {
                            Ok(d) => {
                                let f = d.get("SOGoSieveFilters").cloned().unwrap_or(serde_json::Value::Null);
                                let n = f.as_array().map(Vec::len).unwrap_or(0);
                                let sample: String = f.to_string().chars().take(400).collect();
                                extra.push_str(&format!("фильтры (SOGoSieveFilters): {n}, пример: {sample}\n"));
                                extra.push_str(&format!("настройки: {} ключей, Sieve: {}\n", d.as_object().map(|o| o.len()).unwrap_or(0),
                                    d.get("SOGoSieveScriptsEnabled").or(d.get("SOGoSieveFiltersEnabled")).map(|v| v.to_string()).unwrap_or_else(|| "—".into())));
                            }
                            Err(e) => extra.push_str(&format!("фильтры: {e}\n")),
                        }
                        let mut blog = Vec::new();
                        let _ = session.browse("gal", &mut blog).await;
                        for l in blog {
                            extra.push_str(&format!("список gal: {l}\n"));
                        }
                        // Подробности первого найденного: веб-API SOGo и vCard по CardDAV.
                        if let Ok(hit) = session.search(&probe).await.map(|l| l.into_iter().next()) {
                            if let Some((source, sid)) = hit.as_ref().and_then(|h| h.id.strip_prefix("sogo:")).and_then(|r| r.split_once(':')) {
                                match session.view_raw(source, sid).await {
                                    Ok((st, text)) => {
                                        let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                                        extra.push_str(&format!("view {source}/{sid} → {st}: {}\n", one.chars().take(500).collect::<String>()));
                                    }
                                    Err(e) => extra.push_str(&format!("view: ошибка: {e}\n")),
                                }
                                let dav_base = url::Url::parse(&p.dav_base()).map_err(|e| AppError::Other(e.to_string()))?;
                                if let Ok(mut dc) = dav::DavClient::new(&creds.username, &creds.password, &dav_base) {
                                    let vcf = format!("{}Contacts/{source}/{}", p.dav_base().trim_end_matches('/').to_string() + "/SOGo/dav/" + &creds.username + "/", urlencoding_segment(&format!("{sid}.vcf")));
                                    match dc.get(&vcf).await {
                                        Ok((text, _)) => extra.push_str(&format!("GET {vcf} → ok, {} байт, поля: {}\n", text.len(),
                                            text.lines().filter_map(|l| l.split([':', ';']).next()).filter(|k| !k.is_empty() && !k.starts_with(' ')).collect::<Vec<_>>().join(","))),
                                        Err(e) => extra.push_str(&format!("GET {vcf} → {e}\n")),
                                    }
                                }
                            }
                        }
                        session.logout().await;
                    }
                    Err(e) => extra.push_str(&format!("вход: {e}\n")),
                }
            }
            Err(e) => extra.push_str(&format!("адрес: {e}\n")),
        }
    }
    Ok(format!("{header}{report}{extra}"))
}

// ---------------------------------------------------------------- профиль

#[tauri::command]
pub async fn get_profile(state: State<'_, AppState>) -> Result<Option<Settings>> {
    Ok(state.profile.lock().await.clone())
}

#[tauri::command]
pub async fn update_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    display_name: String,
    smtp_host: String,
    smtp_port: u16,
    dav_url: String,
) -> Result<Settings> {
    let mut guard = state.profile.lock().await;
    let mut p = guard.clone().ok_or(AppError::NotLoggedIn)?;
    p.display_name = display_name.trim().to_string();
    p.smtp_host = smtp_host.trim().to_string();
    p.smtp_port = smtp_port;
    if p.dav_url != dav_url.trim() {
        p.dav_url = dav_url.trim().to_string();
        *state.calendars.lock().await = None;
        *state.books.lock().await = None;
        *state.sogo.lock().await = None;
    }
    settings::save(&app, &p)?;
    *guard = Some(p.clone());
    Ok(p)
}

// ---------------------------------------------------------------- отправка

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedFile {
    id: String,
    name: String,
    size: u64,
}

const MAX_OUTGOING_MB: u64 = 25;

#[tauri::command]
pub async fn pick_attachments(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<PickedFile>> {
    // E2E: системный диалог WebDriver не нажать, поэтому в отладочной сборке
    // файлы можно подставить переменной окружения. В релизе этого кода нет.
    #[cfg(debug_assertions)]
    let test_paths: Option<Vec<PathBuf>> = std::env::var("RRCKI_E2E_PICK").ok().map(|v| v.split(';').map(PathBuf::from).collect());
    #[cfg(not(debug_assertions))]
    let test_paths: Option<Vec<PathBuf>> = None;
    let paths = match test_paths {
        Some(p) => Some(p),
        None => host::pick_files(&app).await,
    };
    let Some(paths) = paths else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut picked = state.picked.lock().await;
    for path in paths {
        let size = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
        let id = state.next_pick.fetch_add(1, Ordering::Relaxed).to_string();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        picked.insert(id.clone(), path);
        out.push(PickedFile { id, name, size });
    }
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    /// Папка, куда сохранена копия, или `None`.
    saved_to: Option<String>,
    warning: Option<String>,
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn send_message(
    state: State<'_, AppState>,
    to: String,
    cc: String,
    subject: String,
    body: String,
    html: Option<String>,
    in_reply_to: Option<String>,
    references: Option<String>,
    attachments: Vec<String>,
) -> Result<SendResult> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;

    let mut files = Vec::new();
    let mut total = 0u64;
    {
        let picked = state.picked.lock().await;
        for id in &attachments {
            let path = picked
                .get(id)
                .ok_or_else(|| AppError::Other("Вложение не найдено, выберите файл заново".into()))?;
            let data = tokio::fs::read(path).await?;
            total += data.len() as u64;
            if total > MAX_OUTGOING_MB * 1024 * 1024 {
                return Err(AppError::Other(format!(
                    "Вложения больше {MAX_OUTGOING_MB} МБ — почтовый сервер их не примет"
                )));
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".into());
            let content_type = mime_guess::from_path(path).first_or_octet_stream().to_string();
            files.push(smtp::OutgoingAttachment { name, content_type, data });
        }
    }

    let message = smtp::build(smtp::Outgoing {
        from_name: Some(profile.display_name.clone()),
        from_address: profile.email(),
        to: smtp::split_addresses(&to),
        cc: smtp::split_addresses(&cc),
        subject,
        body,
        html,
        in_reply_to,
        references,
        attachments: files,
    })?;
    smtp::send(&profile.smtp_host, profile.smtp_port, &creds.username, &creds.password, &message).await?;

    {
        let mut picked = state.picked.lock().await;
        for id in &attachments {
            picked.remove(id);
        }
    }

    // Копия в «Отправленные»: письмо уже ушло, поэтому ошибка здесь — предупреждение.
    let raw = message.formatted();
    match with_conn!(state, c => imap::append_sent(c, &raw)) {
        Ok(folder) => Ok(SendResult { saved_to: Some(folder), warning: None }),
        Err(e) => Ok(SendResult {
            saved_to: None,
            warning: Some(format!("Письмо отправлено, но копия не сохранена в «Отправленные»: {e}")),
        }),
    }
}

// ---------------------------------------------------------------- календарь и контакты

async fn dav_client(state: &AppState) -> Result<(dav::DavClient, url::Url)> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let base = url::Url::parse(&profile.dav_base())
        .map_err(|e| AppError::Other(format!("Некорректный адрес сервера календаря: {e}")))?;
    Ok((dav::DavClient::new(&creds.username, &creds.password, &base)?, base))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsResult {
    books: Vec<dav::Collection>,
    contacts: Vec<dav::Contact>,
    errors: Vec<String>,
}

#[tauri::command]
pub async fn list_contacts(state: State<'_, AppState>) -> Result<ContactsResult> {
    let (mut client, base) = dav_client(&state).await?;
    // Кэш читаем отдельным оператором: guard в выражении `match` жил бы до
    // конца match, и повторный lock ниже заблокировал бы команду навсегда.
    let cached = state.books.lock().await.clone();
    let books = match cached {
        Some(b) => b,
        None => {
            let found = client.discover(&base, dav::Kind::Contacts).await?;
            *state.books.lock().await = Some(found.clone());
            found
        }
    };
    let mut contacts = Vec::new();
    let mut errors = Vec::new();
    for book in &books {
        match client.contacts(book).await {
            Ok(list) => contacts.extend(list),
            Err(e) => errors.push(format!("{}: {e}", book.name)),
        }
    }
    contacts.sort_by_key(|c| c.full_name.to_lowercase());
    Ok(ContactsResult { books, contacts, errors })
}

/// Поиск контактов на сервере во всех адресных книгах (в т.ч. корпоративной).
/// Сначала CardDAV; если корпоративная (только для чтения) книга ничего не
/// вернула, а сервер — SOGo, дополнительно ищем через веб-API SOGo.
#[tauri::command]
pub async fn search_contacts(state: State<'_, AppState>, query: String) -> Result<Vec<dav::Contact>> {
    let query = query.trim().to_string();
    if query.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let (mut client, base) = dav_client(&state).await?;
    let cached = state.books.lock().await.clone();
    let books = match cached {
        Some(b) => b,
        None => {
            let found = client.discover(&base, dav::Kind::Contacts).await?;
            *state.books.lock().await = Some(found.clone());
            found
        }
    };
    let mut out: Vec<dav::Contact> = Vec::new();
    let mut last_error = None;
    let mut gal_empty = books.iter().all(|b| b.writable);
    for book in &books {
        match client.search_contacts(book, &query, 25).await {
            Ok(list) => {
                if !book.writable && list.is_empty() {
                    gal_empty = true;
                }
                for c in list {
                    if !out.iter().any(|o| o.href == c.href) {
                        out.push(c);
                    }
                }
            }
            Err(e) => {
                log::warn!("поиск в «{}»: {e}", book.name);
                gal_empty = gal_empty || !book.writable;
                last_error = Some(e);
            }
        }
    }
    let is_sogo = books.iter().any(|b| b.url.contains("/SOGo/"));
    if gal_empty && is_sogo {
        match sogo_search(&state, &base, &query).await {
            Ok(list) => {
                for mut c in list {
                    let dup = out.iter().any(|o| o.emails.iter().any(|e| c.emails.iter().any(|x| x.eq_ignore_ascii_case(e))));
                    if dup {
                        continue;
                    }
                    // «sogo:<книга>:<id>» → адрес vCard в найденной книге (gal/<id>.vcf),
                    // чтобы подробности загрузить по CardDAV.
                    if let Some((source, id)) = c.id.strip_prefix("sogo:").and_then(|r| r.split_once(':')) {
                        if let Some(b) = books.iter().find(|b| b.url.trim_end_matches('/').rsplit('/').next() == Some(source)) {
                            let file = if id.ends_with(".vcf") { id.to_string() } else { format!("{id}.vcf") };
                            if let Ok(u) = url::Url::parse(&b.url).and_then(|u| u.join(&urlencoding_segment(&file))) {
                                c.href = u.to_string();
                            }
                            c.book = b.name.clone();
                        }
                    }
                    out.push(c);
                }
            }
            Err(e) => log::warn!("поиск через веб-API SOGo: {e}"),
        }
    }
    if out.is_empty() {
        if let Some(e) = last_error {
            return Err(e);
        }
    }
    Ok(out)
}

/// Кодирует имя файла для пути URL (пробелы, «@», кириллица).
fn urlencoding_segment(name: &str) -> String {
    let mut u = url::Url::parse("https://x/").expect("static");
    u.path_segments_mut().expect("base").push(name);
    u.path().trim_start_matches('/').to_string()
}

/// Корпоративная (или другая только для чтения) книга целиком — через веб-API
/// SOGo, как в веб-интерфейсе: CardDAV для LDAP-книг отдаёт пустой список.
#[tauri::command]
pub async fn browse_contacts(state: State<'_, AppState>, book_url: String) -> Result<Vec<dav::Contact>> {
    let (_, base) = dav_client(&state).await?;
    let books = state.books.lock().await.clone().unwrap_or_default();
    let book = books.iter().find(|b| b.url == book_url).cloned().ok_or_else(|| AppError::Other("Адресная книга не найдена".into()))?;
    let source = book.url.trim_end_matches('/').rsplit('/').next().unwrap_or("gal").to_string();
    let mut log = Vec::new();
    let mut list = match sogo_session(&state, &base, false).await?.browse(&source, &mut log).await {
        Ok(l) => l,
        Err(_) => sogo_session(&state, &base, true).await?.browse(&source, &mut log).await?,
    };
    for l in &log {
        log::info!("browse {source}: {l}");
    }
    for c in &mut list {
        c.book = book.name.clone();
    }
    list.sort_by_key(|c| c.full_name.to_lowercase());
    Ok(list)
}

/// Подробная карточка контакта: для корпоративной книги поиск отдаёт только
/// имя и почту — дозапрашиваем vCard по CardDAV, а если не вышло, карточку из
/// веб-API SOGo.
#[tauri::command]
pub async fn contact_details(state: State<'_, AppState>, href: String, id: String) -> Result<dav::Contact> {
    let (mut client, _) = dav_client(&state).await?;
    let books = state.books.lock().await.clone().unwrap_or_default();
    let mut found: Option<dav::Contact> = None;
    if let Ok(book) = owning_collection(&books, &href) {
        if let Ok((data, etag)) = client.get(&href).await {
            let origin = dav::Origin { href: href.clone(), etag, editable: book.writable };
            found = dav::parse_vcards(&data, &book.name, &origin).into_iter().next();
        }
    }
    let rich = |c: &dav::Contact| !c.phones.is_empty() || c.title.is_some() || c.address.is_some();
    if found.as_ref().is_some_and(rich) {
        let mut c = found.expect("checked");
        c.id = id;
        return Ok(c);
    }
    if let Some((source, sid)) = id.strip_prefix("sogo:").and_then(|r| r.split_once(':')) {
        let (_, base) = dav_client(&state).await?;
        if let Ok(session) = sogo_session(&state, &base, false).await {
            if let Ok((st, text)) = session.view_raw(source, sid).await {
                if st.is_success() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        let base_contact = found.clone().unwrap_or_else(|| dav::Contact {
                            id: id.clone(), book: "Корпоративная адресная книга".into(), href: href.clone(), etag: None, editable: false,
                            full_name: String::new(), emails: vec![], phones: vec![], org: None, title: None, note: None, address: None,
                        });
                        let mut c = sogo::parse_view(&v, base_contact);
                        c.id = id;
                        return Ok(c);
                    }
                }
            }
        }
    }
    found.map(|mut c| { c.id = id; c }).ok_or_else(|| AppError::Other("Сервер не отдал подробности контакта".into()))
}

/// Сессия веб-API SOGo из кэша или новым входом (`fresh` — войти заново).
async fn sogo_session(state: &AppState, base: &url::Url, fresh: bool) -> Result<sogo::SogoSession> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let cached = state.sogo.lock().await.clone();
    match cached {
        Some(Ok(s)) if !fresh => Ok(s),
        Some(Err(e)) if !fresh => Err(AppError::Other(e)),
        _ => {
            let mut log = Vec::new();
            match sogo::login(base, &creds.username, &creds.password, &mut log).await {
                Ok(s) => {
                    *state.sogo.lock().await = Some(Ok(s.clone()));
                    Ok(s)
                }
                Err(e) => {
                    *state.sogo.lock().await = Some(Err(e.to_string()));
                    Err(e)
                }
            }
        }
    }
}

async fn sogo_search(state: &AppState, base: &url::Url, query: &str) -> Result<Vec<dav::Contact>> {
    match sogo_session(state, base, false).await?.search(query).await {
        Ok(list) => Ok(list),
        // Сессия истекла — войти заново один раз.
        Err(_) => sogo_session(state, base, true).await?.search(query).await,
    }
}

/// Проверяет, что адрес записи лежит внутри найденной коллекции: frontend не
/// может заставить клиент писать или удалять по произвольному URL с паролем пользователя.
fn owning_collection(cols: &[dav::Collection], href: &str) -> Result<dav::Collection> {
    let target = url::Url::parse(href).map_err(|_| AppError::Other("Некорректный адрес записи".into()))?;
    cols.iter()
        .find(|c| {
            url::Url::parse(&c.url).is_ok_and(|u| {
                u.origin() == target.origin() && target.path().starts_with(u.path()) && target.path() != u.path()
            })
        })
        .cloned()
        .ok_or_else(|| AppError::Other("Запись не принадлежит ни одному известному календарю/адресной книге".into()))
}

fn require_writable(c: &dav::Collection) -> Result<()> {
    if c.writable {
        Ok(())
    } else {
        Err(AppError::Other(format!("«{}» доступен только для чтения", c.name)))
    }
}

async fn known(state: &AppState, kind: dav::Kind) -> Result<Vec<dav::Collection>> {
    let cached = match kind {
        dav::Kind::Calendar => state.calendars.lock().await.clone(),
        dav::Kind::Contacts => state.books.lock().await.clone(),
    };
    cached.ok_or_else(|| AppError::Other("Сначала откройте раздел, чтобы загрузить список".into()))
}

fn collection_child(collection_url: &str, uid: &str, ext: &str) -> Result<String> {
    let base = url::Url::parse(collection_url).map_err(|e| AppError::Other(e.to_string()))?;
    let base = if base.path().ends_with('/') { base } else { url::Url::parse(&format!("{collection_url}/")).map_err(|e| AppError::Other(e.to_string()))? };
    base.join(&format!("{uid}.{ext}"))
        .map(|u| u.to_string())
        .map_err(|e| AppError::Other(e.to_string()))
}

#[tauri::command]
pub async fn create_event(state: State<'_, AppState>, calendar_url: String, input: dav::EventInput) -> Result<()> {
    let cols = known(&state, dav::Kind::Calendar).await?;
    let cal = cols
        .iter()
        .find(|c| c.url == calendar_url)
        .ok_or_else(|| AppError::Other("Календарь не найден".into()))?;
    require_writable(cal)?;
    let uid = dav::new_uid();
    let ics = dav::build_event_ics(&uid, &input)?;
    let (mut client, _) = dav_client(&state).await?;
    client.put(&collection_child(&cal.url, &uid, "ics")?, "text/calendar; charset=utf-8", &ics, None).await
}

#[tauri::command]
pub async fn update_event(
    state: State<'_, AppState>,
    href: String,
    input: dav::EventInput,
    change_time: bool,
) -> Result<()> {
    let cal = owning_collection(&known(&state, dav::Kind::Calendar).await?, &href)?;
    require_writable(&cal)?;
    let (mut client, _) = dav_client(&state).await?;
    // Берём свежую версию с сервера: правим её, а не устаревшую копию из списка.
    let (current, etag) = client.get(&href).await?;
    let ics = dav::update_event_ics(&current, &input, change_time)?;
    client.put(&href, "text/calendar; charset=utf-8", &ics, etag.as_deref().or(Some("*"))).await
}

#[tauri::command]
pub async fn delete_event(state: State<'_, AppState>, href: String, etag: Option<String>) -> Result<()> {
    let cal = owning_collection(&known(&state, dav::Kind::Calendar).await?, &href)?;
    require_writable(&cal)?;
    let (mut client, _) = dav_client(&state).await?;
    client.delete(&href, etag.as_deref()).await
}

#[tauri::command]
pub async fn create_contact(state: State<'_, AppState>, book_url: String, input: dav::ContactInput) -> Result<()> {
    let cols = known(&state, dav::Kind::Contacts).await?;
    let book = cols
        .iter()
        .find(|c| c.url == book_url)
        .ok_or_else(|| AppError::Other("Адресная книга не найдена".into()))?;
    require_writable(book)?;
    let uid = dav::new_uid();
    let card = dav::build_vcard(&uid, &input)?;
    let (mut client, _) = dav_client(&state).await?;
    client.put(&collection_child(&book.url, &uid, "vcf")?, "text/vcard; charset=utf-8", &card, None).await
}

#[tauri::command]
pub async fn update_contact(state: State<'_, AppState>, href: String, input: dav::ContactInput) -> Result<()> {
    let book = owning_collection(&known(&state, dav::Kind::Contacts).await?, &href)?;
    require_writable(&book)?;
    let (mut client, _) = dav_client(&state).await?;
    let (current, etag) = client.get(&href).await?;
    let card = dav::update_vcard(&current, &input)?;
    client.put(&href, "text/vcard; charset=utf-8", &card, etag.as_deref().or(Some("*"))).await
}

#[tauri::command]
pub async fn delete_contact(state: State<'_, AppState>, href: String, etag: Option<String>) -> Result<()> {
    let book = owning_collection(&known(&state, dav::Kind::Contacts).await?, &href)?;
    require_writable(&book)?;
    let (mut client, _) = dav_client(&state).await?;
    client.delete(&href, etag.as_deref()).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsResult {
    calendars: Vec<dav::Collection>,
    events: Vec<dav::CalEvent>,
    errors: Vec<String>,
}

#[tauri::command]
pub async fn list_events(state: State<'_, AppState>, start: String, end: String) -> Result<EventsResult> {
    let parse = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&chrono::Utc))
            .map_err(|e| AppError::Other(format!("Некорректная дата {s}: {e}")))
    };
    let (start, end) = (parse(&start)?, parse(&end)?);
    let (mut client, base) = dav_client(&state).await?;
    let cached = state.calendars.lock().await.clone();
    let calendars = match cached {
        Some(c) => c,
        None => {
            let found = client.discover(&base, dav::Kind::Calendar).await?;
            *state.calendars.lock().await = Some(found.clone());
            found
        }
    };
    let mut events = Vec::new();
    let mut errors = Vec::new();
    for cal in &calendars {
        match client.events(cal, start, end).await {
            Ok(list) => events.extend(list),
            Err(e) => errors.push(format!("{}: {e}", cal.name)),
        }
    }
    events.sort_by(|a, b| a.start.cmp(&b.start));
    Ok(EventsResult { calendars, events, errors })
}

#[tauri::command]
pub async fn get_message(state: State<'_, AppState>, folder: String, uid: u32) -> Result<MessageDetail> {
    with_conn!(state, c => imap::get_message(c, &folder, uid))
}

#[tauri::command]
pub async fn set_flag(
    state: State<'_, AppState>,
    folder: String,
    uid: u32,
    flag: FlagKind,
    value: bool,
) -> Result<()> {
    with_conn!(state, c => imap::set_flag(c, &folder, uid, flag, value))
}

/// Сохраняет вложение: путь выбирает пользователь в системном диалоге,
/// frontend путь не передаёт. Возвращает путь или `None`, если отменили.
#[tauri::command]
pub async fn save_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: String,
    uid: u32,
    index: usize,
) -> Result<Option<String>> {
    let (name, data) = with_conn!(state, c => imap::get_attachment(c, &folder, uid, index))?;

    let Some(path) = host::save_file(&app, &name).await else {
        return Ok(None);
    };
    tokio::fs::write(&path, data).await?;
    Ok(Some(path.display().to_string()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMail {
    items: Vec<imap::MessageSummary>,
    last_uid: u32,
}

/// Новые непрочитанные письма папки после `after_uid` (для уведомлений).
#[tauri::command]
pub async fn poll_new_mail(state: State<'_, AppState>, folder: String, after_uid: u32, limit: u32) -> Result<NewMail> {
    let (items, last_uid) = with_conn!(state, c => imap::new_unseen(c, &folder, after_uid, limit))?;
    Ok(NewMail { items, last_uid })
}

/// Всплывающее уведомление ОС. Текст ограничен — уведомления не для длинных писем.
#[tauri::command]
pub async fn notify(app: AppHandle, title: String, body: String) -> Result<()> {
    let cut = |s: &str, n: usize| -> String {
        let t: String = s.chars().take(n).collect();
        if s.chars().count() > n { format!("{t}…") } else { t }
    };
    host::notify(&app, &cut(&title, 80), &cut(&body, 240))
}

/// Как часто проверять почту и чат для уведомлений (секунды).
#[tauri::command]
pub fn notify_poll_secs() -> u32 {
    #[cfg(debug_assertions)]
    if let Some(s) = std::env::var("RRCKI_E2E_POLL_SECS").ok().and_then(|v| v.parse().ok()) {
        return s;
    }
    60
}

/// Аватар Nextcloud: `kind` = "user" (ID пользователя) или "room" (токен беседы Talk).
#[tauri::command]
pub async fn nc_avatar(state: State<'_, AppState>, kind: String, id: String) -> Result<Option<String>> {
    let nc = nc(&state).await?;
    match kind.as_str() {
        "user" => nc.user_avatar(&id, 64).await,
        "room" => nc.room_avatar(&id).await,
        _ => Ok(None),
    }
}

/// HTML письма с картинками из интернета. Картинки загружает программа (не страница),
/// IMAP-соединение во время загрузки не занято.
#[tauri::command]
pub async fn message_html_remote(state: State<'_, AppState>, folder: String, uid: u32) -> Result<String> {
    let (_, urls) = with_conn!(state, c => imap::message_html_with_remote(c, &folder, uid))?;
    // Автотесты: картинка с мок-сервера на localhost. В релизной сборке — никогда.
    let allow_local = cfg!(debug_assertions) && std::env::var_os("RRCKI_E2E_ALLOW_LOCAL_IMAGES").is_some();
    let images = crate::html::fetch_remote(&urls, allow_local).await;
    with_conn!(state, c => imap::rerender_html(c, &folder, uid, &images))
}

/// Вложение для предпросмотра в окне программы (картинки, PDF, текст).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentData {
    name: String,
    /// Содержимое в base64.
    data: String,
}

/// Предпросмотр больше этого размера не загружается в интерфейс — только «Сохранить».
const MAX_PREVIEW_BYTES: usize = 30 * 1024 * 1024;

#[tauri::command]
pub async fn attachment_data(state: State<'_, AppState>, folder: String, uid: u32, index: usize) -> Result<AttachmentData> {
    use base64::Engine as _;
    let (name, data) = with_conn!(state, c => imap::get_attachment(c, &folder, uid, index))?;
    if data.len() > MAX_PREVIEW_BYTES {
        return Err(AppError::Other(format!("Файл слишком большой для просмотра ({} МБ) — сохраните его", data.len() / 1024 / 1024)));
    }
    Ok(AttachmentData { name, data: base64::engine::general_purpose::STANDARD.encode(data) })
}

// ---------------------------------------------------------------- Nextcloud

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcStatus {
    url: String,
    connected: bool,
    login: String,
    /// ID пользователя Nextcloud («Мои карточки» в Deck, свои сообщения в Talk).
    uid: String,
}

async fn nc_status_of(state: &AppState) -> NcStatus {
    let profile = state.profile.lock().await.clone();
    let nc = state.nc.lock().await.clone();
    NcStatus {
        url: profile.as_ref().map(|p| p.nextcloud_url.clone()).unwrap_or_else(|| settings::DEFAULT_NEXTCLOUD_URL.into()),
        connected: nc.is_some(),
        login: nc.as_ref().map(|(c, _)| c.login.clone()).unwrap_or_default(),
        uid: nc.map(|(_, u)| u).unwrap_or_default(),
    }
}

async fn nc(state: &AppState) -> Result<nextcloud::Nc> {
    let (creds, uid) = state
        .nc
        .lock()
        .await
        .clone()
        .ok_or_else(|| AppError::Other("Nextcloud не подключён — подключите его в Настройках".into()))?;
    nextcloud::Nc::new(creds, &uid)
}

/// Сохраняет подключение: проверяет пароль приложения и узнаёт ID пользователя.
async fn nc_save(app: &AppHandle, state: &AppState, creds: nextcloud::NcCreds) -> Result<NcStatus> {
    let probe = nextcloud::Nc::new(creds.clone(), &creds.login)?;
    let (uid, _name) = probe.user_info().await?;
    let mut guard = state.profile.lock().await;
    let mut p = guard.clone().ok_or(AppError::NotLoggedIn)?;
    p.nextcloud_url = creds.base.to_string();
    p.nextcloud_login = creds.login.clone();
    p.nextcloud_uid = uid.clone();
    settings::save(app, &p)?;
    *guard = Some(p);
    drop(guard);
    if let Err(e) = settings::nc_store_password(creds.base.as_str(), &creds.login, &creds.app_password) {
        log::warn!("nextcloud keyring: {e}");
    }
    *state.nc.lock().await = Some((creds, uid));
    Ok(nc_status_of(state).await)
}

#[tauri::command]
pub async fn nc_status(state: State<'_, AppState>) -> Result<NcStatus> {
    Ok(nc_status_of(&state).await)
}

/// Начинает вход: открывает страницу входа Nextcloud в браузере и возвращает её
/// адрес (на случай, если браузер не открылся).
#[tauri::command]
pub async fn nc_connect_start(app: AppHandle, state: State<'_, AppState>, url: String) -> Result<String> {
    let base = nextcloud::normalize_base(&url)?;
    let flow = nextcloud::login_flow_start(&base).await?;
    let login_url = flow.login_url.clone();
    *state.nc_flow.lock().await = Some((base, flow));
    if let Err(e) = host::open_url(&app, &login_url) {
        log::warn!("browser not opened: {e}");
    }
    Ok(login_url)
}

/// `None` — вход в браузере ещё не подтверждён.
#[tauri::command]
pub async fn nc_connect_poll(app: AppHandle, state: State<'_, AppState>) -> Result<Option<NcStatus>> {
    let Some((base, flow)) = state.nc_flow.lock().await.clone() else {
        return Err(AppError::Other("Вход в Nextcloud не начат".into()));
    };
    match nextcloud::login_flow_poll(&base, &flow).await? {
        None => Ok(None),
        Some(creds) => {
            *state.nc_flow.lock().await = None;
            Ok(Some(nc_save(&app, &state, creds).await?))
        }
    }
}

#[tauri::command]
pub async fn nc_connect_manual(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    login: String,
    app_password: String,
) -> Result<NcStatus> {
    let creds = nextcloud::NcCreds {
        base: nextcloud::normalize_base(&url)?,
        login: login.trim().to_string(),
        app_password: app_password.trim().to_string(),
    };
    if creds.login.is_empty() || creds.app_password.is_empty() {
        return Err(AppError::Other("Укажите логин и пароль приложения".into()));
    }
    nc_save(&app, &state, creds).await
}

#[tauri::command]
pub async fn nc_disconnect(app: AppHandle, state: State<'_, AppState>) -> Result<NcStatus> {
    *state.nc_flow.lock().await = None;
    if let Some((creds, uid)) = state.nc.lock().await.take() {
        if let Ok(client) = nextcloud::Nc::new(creds.clone(), &uid) {
            client.revoke().await;
        }
        settings::nc_forget_password(creds.base.as_str(), &creds.login);
    }
    let mut guard = state.profile.lock().await;
    if let Some(p) = guard.as_mut() {
        p.nextcloud_login.clear();
        p.nextcloud_uid.clear();
        let _ = settings::save(&app, p);
    }
    drop(guard);
    Ok(nc_status_of(&state).await)
}

/// Открывает страницу Nextcloud (звонок, доска) в браузере — только на своём сервере.
#[tauri::command]
pub async fn nc_open(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<()> {
    let client = nc(&state).await?;
    let target = client.creds.base.join(path.trim_start_matches('/')).map_err(|e| AppError::Other(e.to_string()))?;
    if target.origin() != client.creds.base.origin() {
        return Err(AppError::Other("Можно открывать только страницы своего Nextcloud".into()));
    }
    host::open_url(&app, target.as_str())
}

#[tauri::command]
pub async fn deck_boards(state: State<'_, AppState>) -> Result<Vec<nextcloud::DeckBoard>> {
    nc(&state).await?.deck_boards().await
}

#[tauri::command]
pub async fn deck_stacks(state: State<'_, AppState>, board_id: i64) -> Result<Vec<nextcloud::DeckStack>> {
    nc(&state).await?.deck_stacks(board_id).await
}

#[tauri::command]
pub async fn deck_create_card(
    state: State<'_, AppState>,
    board_id: i64,
    stack_id: i64,
    title: String,
    description: String,
    duedate: Option<String>,
) -> Result<()> {
    nc(&state).await?.deck_create_card(board_id, stack_id, &title, &description, duedate.as_deref()).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn deck_update_card(
    state: State<'_, AppState>,
    board_id: i64,
    stack_id: i64,
    card_id: i64,
    title: String,
    description: String,
    duedate: Option<String>,
    done: bool,
) -> Result<()> {
    nc(&state).await?.deck_update_card(board_id, stack_id, card_id, &title, &description, duedate.as_deref(), done).await
}

#[tauri::command]
pub async fn deck_move_card(
    state: State<'_, AppState>,
    board_id: i64,
    stack_id: i64,
    card_id: i64,
    target_stack_id: i64,
    order: i64,
) -> Result<()> {
    nc(&state).await?.deck_move_card(board_id, stack_id, card_id, target_stack_id, order).await
}

#[tauri::command]
pub async fn deck_delete_card(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64) -> Result<()> {
    nc(&state).await?.deck_delete_card(board_id, stack_id, card_id).await
}

#[tauri::command]
pub async fn deck_board_details(state: State<'_, AppState>, board_id: i64) -> Result<nextcloud::DeckBoardDetails> {
    nc(&state).await?.deck_board_details(board_id).await
}

#[tauri::command]
pub async fn deck_assign(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64, uid: String, assign: bool) -> Result<()> {
    nc(&state).await?.deck_assign(board_id, stack_id, card_id, &uid, assign).await
}

#[tauri::command]
pub async fn deck_label(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64, label_id: i64, add: bool) -> Result<()> {
    nc(&state).await?.deck_label(board_id, stack_id, card_id, label_id, add).await
}

#[tauri::command]
pub async fn deck_comments(state: State<'_, AppState>, card_id: i64) -> Result<Vec<nextcloud::DeckComment>> {
    nc(&state).await?.deck_comments(card_id).await
}

#[tauri::command]
pub async fn deck_comment_add(state: State<'_, AppState>, card_id: i64, message: String) -> Result<()> {
    nc(&state).await?.deck_comment_add(card_id, &message).await
}

#[tauri::command]
pub async fn deck_comment_delete(state: State<'_, AppState>, card_id: i64, comment_id: i64) -> Result<()> {
    nc(&state).await?.deck_comment_delete(card_id, comment_id).await
}

#[tauri::command]
pub async fn deck_attachments(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64) -> Result<Vec<nextcloud::DeckAttachment>> {
    nc(&state).await?.deck_attachments(board_id, stack_id, card_id).await
}

#[tauri::command]
pub async fn deck_create_stack(state: State<'_, AppState>, board_id: i64, title: String) -> Result<()> {
    nc(&state).await?.deck_create_stack(board_id, &title).await
}

#[tauri::command]
pub async fn talk_rooms(state: State<'_, AppState>) -> Result<Vec<nextcloud::TalkRoom>> {
    nc(&state).await?.talk_rooms().await
}

#[tauri::command]
pub async fn talk_history(state: State<'_, AppState>, token: String, before: Option<i64>) -> Result<nextcloud::TalkChunk> {
    nc(&state).await?.talk_history(&token, before).await
}

#[tauri::command]
pub async fn talk_poll(state: State<'_, AppState>, token: String, last_known: i64) -> Result<nextcloud::TalkChunk> {
    let (creds, uid) = state.nc.lock().await.clone().ok_or_else(|| AppError::Other("Nextcloud не подключён".into()))?;
    nextcloud::Nc::long_poll(creds, &uid)?.talk_poll(&token, last_known).await
}

#[tauri::command]
pub async fn talk_send(state: State<'_, AppState>, token: String, message: String, reply_to: Option<i64>) -> Result<()> {
    nc(&state).await?.talk_send(&token, &message, reply_to).await
}

#[tauri::command]
pub async fn talk_react(state: State<'_, AppState>, token: String, message_id: i64, emoji: String, add: bool) -> Result<()> {
    nc(&state).await?.talk_react(&token, message_id, &emoji, add).await
}

#[tauri::command]
pub async fn talk_edit(state: State<'_, AppState>, token: String, message_id: i64, message: String) -> Result<()> {
    nc(&state).await?.talk_edit(&token, message_id, &message).await
}

#[tauri::command]
pub async fn talk_delete(state: State<'_, AppState>, token: String, message_id: i64) -> Result<()> {
    nc(&state).await?.talk_delete(&token, message_id).await
}

#[tauri::command]
pub async fn talk_mentions(state: State<'_, AppState>, token: String, search: String) -> Result<Vec<nextcloud::TalkMention>> {
    nc(&state).await?.talk_mentions(&token, &search).await
}

#[tauri::command]
pub async fn nc_search_users(state: State<'_, AppState>, search: String) -> Result<Vec<nextcloud::NcUser>> {
    if search.trim().chars().count() < 2 {
        return Ok(Vec::new());
    }
    nc(&state).await?.search_users(search.trim()).await
}

#[tauri::command]
pub async fn talk_create(state: State<'_, AppState>, users: Vec<String>, name: Option<String>) -> Result<String> {
    nc(&state).await?.talk_create(&users, name.as_deref()).await
}

/// Открывает ссылку из сообщения во внешнем браузере (только http/https/mailto).
#[tauri::command]
pub async fn open_link(app: AppHandle, url: String) -> Result<()> {
    let parsed = url::Url::parse(url.trim()).map_err(|_| AppError::Other("Некорректная ссылка".into()))?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return Err(AppError::Other("Можно открывать только ссылки http, https и mailto".into()));
    }
    host::open_url(&app, parsed.as_str())
}

// ---------------------------------------------------------------- Файлы Nextcloud

#[tauri::command]
pub async fn nc_files_list(state: State<'_, AppState>, path: String) -> Result<Vec<nextcloud::NcFile>> {
    nc(&state).await?.files_list(&path).await
}

#[tauri::command]
pub async fn nc_files_mkdir(state: State<'_, AppState>, parent: String, name: String) -> Result<String> {
    nc(&state).await?.files_mkdir(&parent, &name).await
}

/// Читает выбранные пользователем файлы (по id из `pick_attachments`).
async fn read_picked(state: &AppState, ids: &[String]) -> Result<Vec<(String, String, Vec<u8>)>> {
    let mut out = Vec::new();
    for id in ids {
        let path = state.picked.lock().await.get(id).cloned().ok_or_else(|| AppError::Other("Файл не найден, выберите его заново".into()))?;
        let size = tokio::fs::metadata(&path).await?.len();
        if size > MAX_TALK_FILE_MB * 1024 * 1024 {
            return Err(AppError::Other(format!("Файл больше {MAX_TALK_FILE_MB} МБ")));
        }
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        out.push((id.clone(), name, tokio::fs::read(&path).await?));
    }
    Ok(out)
}

async fn nc_upload(state: &AppState) -> Result<nextcloud::Nc> {
    let (creds, uid) = state.nc.lock().await.clone().ok_or_else(|| AppError::Other("Nextcloud не подключён — подключите его в Настройках".into()))?;
    nextcloud::Nc::upload(creds, &uid)
}

/// Загружает выбранные файлы в папку Nextcloud. Возвращает пути.
#[tauri::command]
pub async fn nc_files_upload(state: State<'_, AppState>, dir: String, attachments: Vec<String>) -> Result<Vec<String>> {
    let files = read_picked(&state, &attachments).await?;
    let client = nc_upload(&state).await?;
    let mut paths = Vec::new();
    for (id, name, data) in files {
        paths.push(client.upload_unique(&dir, &name, data).await?);
        state.picked.lock().await.remove(&id);
    }
    Ok(paths)
}

/// Скачивает файл: куда сохранить, выбирает пользователь в системном диалоге.
#[tauri::command]
pub async fn nc_files_download(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<Option<String>> {
    let name = path.rsplit('/').next().unwrap_or("file").to_string();
    let Some(target) = host::save_file(&app, &name).await else {
        return Ok(None);
    };
    let data = nc_upload(&state).await?.files_download(&path).await?;
    tokio::fs::write(&target, data).await?;
    Ok(Some(target.display().to_string()))
}

#[tauri::command]
pub async fn nc_files_delete(state: State<'_, AppState>, path: String) -> Result<()> {
    nc(&state).await?.files_delete(&path).await
}

#[tauri::command]
pub async fn nc_files_rename(state: State<'_, AppState>, path: String, name: String) -> Result<String> {
    nc(&state).await?.files_rename(&path, &name).await
}

#[tauri::command]
pub async fn nc_shares(state: State<'_, AppState>, path: String) -> Result<Vec<nextcloud::NcShare>> {
    nc(&state).await?.shares_list(&path).await
}

#[tauri::command]
pub async fn nc_share_link(state: State<'_, AppState>, path: String, password: Option<String>, expire: Option<String>) -> Result<nextcloud::NcShare> {
    nc(&state).await?.share_link(&path, password.as_deref(), expire.as_deref()).await
}

#[tauri::command]
pub async fn nc_share_user(state: State<'_, AppState>, path: String, uid: String, can_edit: bool) -> Result<nextcloud::NcShare> {
    nc(&state).await?.share_user(&path, &uid, can_edit).await
}

#[tauri::command]
pub async fn nc_share_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    nc(&state).await?.share_delete(&id).await
}

/// Публичная ссылка на файл для письма (существующая без пароля или новая).
#[tauri::command]
pub async fn nc_link_for(state: State<'_, AppState>, path: String) -> Result<String> {
    nc(&state).await?.link_for(&path).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedLink {
    name: String,
    path: String,
    url: String,
}

/// Файлы с компьютера → папка в Nextcloud → публичные ссылки (для больших вложений).
#[tauri::command]
pub async fn nc_upload_links(state: State<'_, AppState>, dir: String, attachments: Vec<String>) -> Result<Vec<UploadedLink>> {
    let files = read_picked(&state, &attachments).await?;
    let client = nc_upload(&state).await?;
    let mut out = Vec::new();
    for (id, name, data) in files {
        let path = client.upload_unique(&dir, &name, data).await?;
        let url = client.link_for(&path).await?;
        state.picked.lock().await.remove(&id);
        out.push(UploadedLink { name: path.rsplit('/').next().unwrap_or(&name).to_string(), path, url });
    }
    Ok(out)
}

/// Вложение письма → папка в Nextcloud. Возвращает путь.
#[tauri::command]
pub async fn nc_save_mail_attachment(state: State<'_, AppState>, folder: String, uid: u32, index: usize, dir: String) -> Result<String> {
    let (name, data) = with_conn!(state, c => imap::get_attachment(c, &folder, uid, index))?;
    nc_upload(&state).await?.upload_unique(&dir, &name, data).await
}

const MAX_TALK_FILE_MB: u64 = 512;

/// Отправляет в беседу файл, выбранный через `pick_attachments`.
#[tauri::command]
pub async fn talk_send_file(state: State<'_, AppState>, token: String, attachment: String, caption: String, reply_to: Option<i64>) -> Result<()> {
    let (creds, uid) = state.nc.lock().await.clone().ok_or_else(|| AppError::Other("Nextcloud не подключён".into()))?;
    let path = state
        .picked
        .lock()
        .await
        .get(&attachment)
        .cloned()
        .ok_or_else(|| AppError::Other("Файл не найден, выберите его заново".into()))?;
    let size = tokio::fs::metadata(&path).await?.len();
    if size > MAX_TALK_FILE_MB * 1024 * 1024 {
        return Err(AppError::Other(format!("Файл больше {MAX_TALK_FILE_MB} МБ — отправьте ссылку из Nextcloud")));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
    let data = tokio::fs::read(&path).await?;
    nextcloud::Nc::upload(creds, &uid)?.talk_send_file(&token, &name, data, &caption, reply_to).await?;
    state.picked.lock().await.remove(&attachment);
    Ok(())
}

/// Сбрасывает выбранный, но не отправленный файл.
#[tauri::command]
pub async fn forget_attachment(state: State<'_, AppState>, attachment: String) -> Result<()> {
    state.picked.lock().await.remove(&attachment);
    Ok(())
}

// ---------------------------------------------------------------- фильтры почты (SOGo / Sieve)

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailFilters {
    filters: serde_json::Value,
}

/// Фильтры почты пользователя — те же, что в веб-почте SOGo (Настройки → Фильтры).
#[tauri::command]
pub async fn mail_filters_get(state: State<'_, AppState>) -> Result<MailFilters> {
    let (_, base) = dav_client(&state).await?;
    let (_, defaults) = sogo_defaults(&state, &base).await?;
    let filters = defaults.get("SOGoSieveFilters").cloned().filter(|v| v.is_array()).unwrap_or_else(|| serde_json::json!([]));
    Ok(MailFilters { filters })
}

/// Сессия SOGo и настройки пользователя; при ошибке (сессия истекла, прошлый вход
/// не удался) — один повтор с новым входом.
async fn sogo_defaults(state: &AppState, base: &url::Url) -> Result<(sogo::SogoSession, serde_json::Value)> {
    if let Ok(s) = sogo_session(state, base, false).await {
        if let Ok(d) = s.defaults().await {
            return Ok((s, d));
        }
    }
    let s = sogo_session(state, base, true).await?;
    let d = s.defaults().await?;
    Ok((s, d))
}

/// Сохраняет фильтры: читает свежие настройки, меняет только `SOGoSieveFilters`
/// и отправляет обратно целиком (SOGo пересобирает Sieve-скрипт).
#[tauri::command]
pub async fn mail_filters_save(state: State<'_, AppState>, filters: serde_json::Value) -> Result<()> {
    sogo::validate_filters(&filters)?;
    let (_, base) = dav_client(&state).await?;
    let (session, mut defaults) = sogo_defaults(&state, &base).await?;
    defaults["SOGoSieveFilters"] = filters;
    session.save_defaults(&defaults).await
}

// ---------------------------------------------------------------- перемещение и удаление писем

#[tauri::command]
pub async fn create_folder(state: State<'_, AppState>, name: String) -> Result<()> {
    if name.trim().is_empty() || name.len() > 400 {
        return Err(AppError::Other("Некорректное имя папки".into()));
    }
    with_conn!(state, c => imap::create_folder(c, &name))
}

/// Закрывает окно, из которого вызвана команда (окно письма после удаления/архивации).
#[tauri::command]
pub async fn close_window(window: tauri::Window) -> Result<()> {
    window.close().map_err(|e| AppError::Other(format!("Не удалось закрыть окно: {e}")))
}

/// Открывает письмо в отдельном окне (удобно читать длинные письма).
#[tauri::command]
pub async fn open_message_window(app: AppHandle, folder: String, uid: u32, title: String) -> Result<()> {
    let params = serde_json::json!({ "folder": folder, "uid": uid });
    // Одно окно на письмо: повторный вызов поднимает уже открытое.
    let label = format!("msg-{:x}", folder.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)) ^ u64::from(uid));
    let title: String = title.chars().take(120).collect();
    let title = if title.trim().is_empty() { "Письмо — Atom Mail".to_string() } else { format!("{title} — Atom Mail") };
    crate::host::open_window(&app, &label, &title, &format!("window.__RRCKI_MESSAGE = {params};"))
}

#[tauri::command]
pub async fn move_messages(state: State<'_, AppState>, folder: String, uids: Vec<u32>, target: String) -> Result<()> {
    with_conn!(state, c => imap::move_messages(c, &folder, &uids, &target))
}

/// Удалить письма: в корзину `trash` или (из корзины) насовсем. `true` — насовсем.
#[tauri::command]
pub async fn delete_messages(state: State<'_, AppState>, folder: String, uids: Vec<u32>, trash: Option<String>) -> Result<bool> {
    with_conn!(state, c => imap::delete_messages(c, &folder, &uids, trash.as_deref()))
}

// ---------------------------------------------------------------- о программе

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: String,
    /// Сборка: win, win8, deb, astra17, rpm, appimage, unknown.
    variant: String,
    os: String,
}

#[tauri::command]
pub fn app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        variant: updater::detect_variant(host::VARIANT),
        os: std::env::consts::OS.to_string(),
    }
}

// ---------------------------------------------------------------- обновления

/// Есть ли новая версия для этой сборки. `None` — обновлений нет или проверка
/// отключена администратором (`update.json`).
#[tauri::command]
pub async fn check_update(app: AppHandle, state: State<'_, AppState>) -> Result<Option<updater::UpdateInfo>> {
    let cfg = updater::load_config(host::config_dir(&app).ok().as_deref());
    if cfg.disabled {
        return Ok(None);
    }
    let url = cfg.url.unwrap_or_else(|| updater::DEFAULT_MANIFEST_URL.to_string());
    let manifest = updater::fetch_manifest(&url).await?;
    let current = env!("CARGO_PKG_VERSION").to_string();
    // Версия приложения в легаси-сборке берётся из tauri.conf.json.
    let current = app.package_info().version.to_string().split('~').next().map(str::to_string).filter(|v| v != "0.0.0").unwrap_or(current);
    if !updater::is_newer(&manifest.version, &current) {
        *state.update.lock().await = None;
        return Ok(None);
    }
    let variant = updater::detect_variant(host::VARIANT);
    let Some(asset) = updater::pick(&manifest, &variant) else {
        return Ok(None);
    };
    let info = updater::UpdateInfo {
        current,
        version: manifest.version.clone(),
        notes: manifest.notes.clone(),
        size: asset.size,
        file_name: asset.name.clone(),
        variant,
    };
    *state.update.lock().await = Some((info.clone(), asset));
    Ok(Some(info))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    done: u64,
    total: u64,
}

#[tauri::command]
pub fn update_progress(state: State<'_, AppState>) -> UpdateProgress {
    UpdateProgress {
        done: state.update_progress.done.load(Ordering::Relaxed),
        total: state.update_progress.total.load(Ordering::Relaxed),
    }
}

/// Скачивает найденное обновление, проверяет SHA-256 и ставит. Если установщик
/// запущен или программа перезапускается — текущий процесс закрывается.
#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, AppState>) -> Result<updater::InstallResult> {
    let (info, asset) = state
        .update
        .lock()
        .await
        .clone()
        .ok_or_else(|| AppError::Other("Сначала проверьте обновления".into()))?;
    let dir = std::env::temp_dir().join("rrcki-mail-update");
    let file = updater::download(&asset, &dir, &state.update_progress).await?;
    let variant = info.variant.clone();
    let result = tokio::task::spawn_blocking(move || updater::install(&variant, &file))
        .await
        .map_err(|e| AppError::Other(e.to_string()))??;
    if result.restarting {
        let handle = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1200)).await;
            handle.exit(0);
        });
    }
    Ok(result)
}

/// Список команд для `invoke_handler` — один на обе сборки.
macro_rules! app_commands {
    () => {
        tauri::generate_handler![
            crate::commands::get_settings,
            crate::commands::login,
            crate::commands::auto_login,
            crate::commands::logout,
            crate::commands::list_folders,
            crate::commands::list_messages,
            crate::commands::get_message,
            crate::commands::set_flag,
            crate::commands::save_attachment,
            crate::commands::diagnostics,
            crate::commands::get_profile,
            crate::commands::update_profile,
            crate::commands::pick_attachments,
            crate::commands::send_message,
            crate::commands::list_contacts,
            crate::commands::search_contacts,
            crate::commands::contact_details,
            crate::commands::browse_contacts,
            crate::commands::list_events,
            crate::commands::create_event,
            crate::commands::update_event,
            crate::commands::delete_event,
            crate::commands::create_contact,
            crate::commands::update_contact,
            crate::commands::delete_contact,
            crate::commands::nc_status,
            crate::commands::nc_connect_start,
            crate::commands::nc_connect_poll,
            crate::commands::nc_connect_manual,
            crate::commands::nc_disconnect,
            crate::commands::nc_open,
            crate::commands::deck_boards,
            crate::commands::deck_stacks,
            crate::commands::deck_create_card,
            crate::commands::deck_update_card,
            crate::commands::deck_move_card,
            crate::commands::deck_delete_card,
            crate::commands::deck_create_stack,
            crate::commands::deck_board_details,
            crate::commands::deck_assign,
            crate::commands::deck_label,
            crate::commands::deck_comments,
            crate::commands::deck_comment_add,
            crate::commands::deck_comment_delete,
            crate::commands::deck_attachments,
            crate::commands::talk_rooms,
            crate::commands::talk_history,
            crate::commands::talk_poll,
            crate::commands::talk_send,
            crate::commands::talk_send_file,
            crate::commands::talk_react,
            crate::commands::talk_edit,
            crate::commands::talk_delete,
            crate::commands::talk_mentions,
            crate::commands::nc_search_users,
            crate::commands::talk_create,
            crate::commands::open_link,
            crate::commands::nc_files_list,
            crate::commands::nc_files_mkdir,
            crate::commands::nc_files_upload,
            crate::commands::nc_files_download,
            crate::commands::nc_files_delete,
            crate::commands::nc_files_rename,
            crate::commands::nc_shares,
            crate::commands::nc_share_link,
            crate::commands::nc_share_user,
            crate::commands::nc_share_delete,
            crate::commands::nc_link_for,
            crate::commands::nc_upload_links,
            crate::commands::nc_save_mail_attachment,
            crate::commands::app_info,
            crate::commands::move_messages,
            crate::commands::create_folder,
            crate::commands::open_message_window,
            crate::commands::attachment_data,
            crate::commands::message_html_remote,
            crate::commands::nc_avatar,
            crate::commands::poll_new_mail,
            crate::commands::notify,
            crate::commands::notify_poll_secs,
            crate::commands::close_window,
            crate::commands::mail_filters_get,
            crate::commands::mail_filters_save,
            crate::commands::delete_messages,
            crate::commands::check_update,
            crate::commands::update_progress,
            crate::commands::install_update,
            crate::commands::forget_attachment,
        ]
    };
}
pub(crate) use app_commands;
