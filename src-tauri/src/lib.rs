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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::Mutex;

use crate::error::{AppError, Result};
use crate::imap::{Connection, Credentials, FlagKind, Folder, MessageDetail, MessagePage};
use crate::settings::Settings;

const OP_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Default)]
struct AppState {
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
struct Account {
    username: String,
    host: String,
    port: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginResult {
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
fn get_settings(app: AppHandle) -> Option<Settings> {
    settings::load(&app)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn login(
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
        display_name: previous.as_ref().map(|p| p.display_name.clone()).unwrap_or_default(),
        nextcloud_url: previous
            .as_ref()
            .map(|p| p.nextcloud_url.clone())
            .unwrap_or_else(|| settings::DEFAULT_NEXTCLOUD_URL.to_string()),
        nextcloud_login: previous.as_ref().map(|p| p.nextcloud_login.clone()).unwrap_or_default(),
        nextcloud_uid: previous.as_ref().map(|p| p.nextcloud_uid.clone()).unwrap_or_default(),
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
async fn auto_login(app: AppHandle, state: State<'_, AppState>) -> Result<Option<Account>> {
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
async fn logout(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    if let Some(s) = settings::load(&app) {
        settings::forget_password(&s);
    }
    *state.creds.lock().await = None;
    *state.nc.lock().await = None;
    *state.profile.lock().await = None;
    *state.calendars.lock().await = None;
    *state.books.lock().await = None;
    state.picked.lock().await.clear();
    if let Some(conn) = state.conn.lock().await.take() {
        conn.logout().await;
    }
    Ok(())
}

#[tauri::command]
async fn list_folders(state: State<'_, AppState>) -> Result<Vec<Folder>> {
    with_conn!(state, c => imap::list_folders(c))
}

#[tauri::command]
async fn list_messages(
    state: State<'_, AppState>,
    folder: String,
    before_uid: Option<u32>,
    limit: u32,
) -> Result<MessagePage> {
    with_conn!(state, c => imap::list_messages(c, &folder, before_uid, limit))
}

/// Текстовый отчёт о том, что отвечает сервер (без пароля) — для разбора проблем.
#[tauri::command]
async fn diagnostics(state: State<'_, AppState>, folder: Option<String>) -> Result<String> {
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
            match dav::DavClient::new(&creds.username, &creds.password) {
                Ok(mut client) => {
                    let base = url::Url::parse(&p.dav_base()).map_err(|e| AppError::Other(e.to_string()))?;
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
    Ok(format!("{header}{report}{extra}"))
}

// ---------------------------------------------------------------- профиль

#[tauri::command]
async fn get_profile(state: State<'_, AppState>) -> Result<Option<Settings>> {
    Ok(state.profile.lock().await.clone())
}

#[tauri::command]
async fn update_profile(
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
    }
    settings::save(&app, &p)?;
    *guard = Some(p.clone());
    Ok(p)
}

// ---------------------------------------------------------------- отправка

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PickedFile {
    id: String,
    name: String,
    size: u64,
}

const MAX_OUTGOING_MB: u64 = 25;

#[tauri::command]
async fn pick_attachments(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<PickedFile>> {
    // E2E: системный диалог WebDriver не нажать, поэтому в отладочной сборке
    // файлы можно подставить переменной окружения. В релизе этого кода нет.
    #[cfg(debug_assertions)]
    let test_paths: Option<Vec<tauri_plugin_dialog::FilePath>> = std::env::var("RRCKI_E2E_PICK")
        .ok()
        .map(|v| v.split(';').map(|p| tauri_plugin_dialog::FilePath::Path(PathBuf::from(p))).collect());
    #[cfg(not(debug_assertions))]
    let test_paths: Option<Vec<tauri_plugin_dialog::FilePath>> = None;
    let paths = if let Some(p) = test_paths { Some(p) } else {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog().file().pick_files(move |paths| {
            let _ = tx.send(paths);
        });
        rx.await.ok().flatten()
    };
    let Some(paths) = paths else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut picked = state.picked.lock().await;
    for p in paths {
        let Ok(path) = p.into_path() else { continue };
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
struct SendResult {
    /// Папка, куда сохранена копия, или `None`.
    saved_to: Option<String>,
    warning: Option<String>,
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn send_message(
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
    Ok((dav::DavClient::new(&creds.username, &creds.password)?, base))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContactsResult {
    books: Vec<dav::Collection>,
    contacts: Vec<dav::Contact>,
    errors: Vec<String>,
}

#[tauri::command]
async fn list_contacts(state: State<'_, AppState>) -> Result<ContactsResult> {
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
#[tauri::command]
async fn search_contacts(state: State<'_, AppState>, query: String) -> Result<Vec<dav::Contact>> {
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
    for book in &books {
        match client.search_contacts(book, &query, 25).await {
            Ok(list) => {
                for c in list {
                    if !out.iter().any(|o| o.href == c.href) {
                        out.push(c);
                    }
                }
            }
            Err(e) => {
                log::warn!("поиск в «{}»: {e}", book.name);
                last_error = Some(e);
            }
        }
    }
    if out.is_empty() {
        if let Some(e) = last_error {
            return Err(e);
        }
    }
    Ok(out)
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
async fn create_event(state: State<'_, AppState>, calendar_url: String, input: dav::EventInput) -> Result<()> {
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
async fn update_event(
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
async fn delete_event(state: State<'_, AppState>, href: String, etag: Option<String>) -> Result<()> {
    let cal = owning_collection(&known(&state, dav::Kind::Calendar).await?, &href)?;
    require_writable(&cal)?;
    let (mut client, _) = dav_client(&state).await?;
    client.delete(&href, etag.as_deref()).await
}

#[tauri::command]
async fn create_contact(state: State<'_, AppState>, book_url: String, input: dav::ContactInput) -> Result<()> {
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
async fn update_contact(state: State<'_, AppState>, href: String, input: dav::ContactInput) -> Result<()> {
    let book = owning_collection(&known(&state, dav::Kind::Contacts).await?, &href)?;
    require_writable(&book)?;
    let (mut client, _) = dav_client(&state).await?;
    let (current, etag) = client.get(&href).await?;
    let card = dav::update_vcard(&current, &input)?;
    client.put(&href, "text/vcard; charset=utf-8", &card, etag.as_deref().or(Some("*"))).await
}

#[tauri::command]
async fn delete_contact(state: State<'_, AppState>, href: String, etag: Option<String>) -> Result<()> {
    let book = owning_collection(&known(&state, dav::Kind::Contacts).await?, &href)?;
    require_writable(&book)?;
    let (mut client, _) = dav_client(&state).await?;
    client.delete(&href, etag.as_deref()).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventsResult {
    calendars: Vec<dav::Collection>,
    events: Vec<dav::CalEvent>,
    errors: Vec<String>,
}

#[tauri::command]
async fn list_events(state: State<'_, AppState>, start: String, end: String) -> Result<EventsResult> {
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
async fn get_message(state: State<'_, AppState>, folder: String, uid: u32) -> Result<MessageDetail> {
    with_conn!(state, c => imap::get_message(c, &folder, uid))
}

#[tauri::command]
async fn set_flag(
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
async fn save_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: String,
    uid: u32,
    index: usize,
) -> Result<Option<String>> {
    let (name, data) = with_conn!(state, c => imap::get_attachment(c, &folder, uid, index))?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|e| AppError::Other(format!("Некорректный путь: {e}")))?;
    tokio::fs::write(&path, data).await?;
    Ok(Some(path.display().to_string()))
}

// ---------------------------------------------------------------- Nextcloud

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NcStatus {
    url: String,
    connected: bool,
    login: String,
}

async fn nc_status_of(state: &AppState) -> NcStatus {
    let profile = state.profile.lock().await.clone();
    let nc = state.nc.lock().await.clone();
    NcStatus {
        url: profile.as_ref().map(|p| p.nextcloud_url.clone()).unwrap_or_else(|| settings::DEFAULT_NEXTCLOUD_URL.into()),
        connected: nc.is_some(),
        login: nc.map(|(c, _)| c.login).unwrap_or_default(),
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
async fn nc_status(state: State<'_, AppState>) -> Result<NcStatus> {
    Ok(nc_status_of(&state).await)
}

/// Начинает вход: открывает страницу входа Nextcloud в браузере и возвращает её
/// адрес (на случай, если браузер не открылся).
#[tauri::command]
async fn nc_connect_start(app: AppHandle, state: State<'_, AppState>, url: String) -> Result<String> {
    use tauri_plugin_opener::OpenerExt;
    let base = nextcloud::normalize_base(&url)?;
    let flow = nextcloud::login_flow_start(&base).await?;
    let login_url = flow.login_url.clone();
    *state.nc_flow.lock().await = Some((base, flow));
    if let Err(e) = app.opener().open_url(&login_url, None::<&str>) {
        log::warn!("browser not opened: {e}");
    }
    Ok(login_url)
}

/// `None` — вход в браузере ещё не подтверждён.
#[tauri::command]
async fn nc_connect_poll(app: AppHandle, state: State<'_, AppState>) -> Result<Option<NcStatus>> {
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
async fn nc_connect_manual(
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
async fn nc_disconnect(app: AppHandle, state: State<'_, AppState>) -> Result<NcStatus> {
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
async fn nc_open(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let client = nc(&state).await?;
    let target = client.creds.base.join(path.trim_start_matches('/')).map_err(|e| AppError::Other(e.to_string()))?;
    if target.origin() != client.creds.base.origin() {
        return Err(AppError::Other("Можно открывать только страницы своего Nextcloud".into()));
    }
    app.opener()
        .open_url(target.as_str(), None::<&str>)
        .map_err(|e| AppError::Other(format!("Не удалось открыть браузер: {e}")))
}

#[tauri::command]
async fn deck_boards(state: State<'_, AppState>) -> Result<Vec<nextcloud::DeckBoard>> {
    nc(&state).await?.deck_boards().await
}

#[tauri::command]
async fn deck_stacks(state: State<'_, AppState>, board_id: i64) -> Result<Vec<nextcloud::DeckStack>> {
    nc(&state).await?.deck_stacks(board_id).await
}

#[tauri::command]
async fn deck_create_card(
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
async fn deck_update_card(
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
async fn deck_move_card(
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
async fn deck_delete_card(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64) -> Result<()> {
    nc(&state).await?.deck_delete_card(board_id, stack_id, card_id).await
}

#[tauri::command]
async fn deck_create_stack(state: State<'_, AppState>, board_id: i64, title: String) -> Result<()> {
    nc(&state).await?.deck_create_stack(board_id, &title).await
}

#[tauri::command]
async fn talk_rooms(state: State<'_, AppState>) -> Result<Vec<nextcloud::TalkRoom>> {
    nc(&state).await?.talk_rooms().await
}

#[tauri::command]
async fn talk_history(state: State<'_, AppState>, token: String, before: Option<i64>) -> Result<Vec<nextcloud::TalkMessage>> {
    nc(&state).await?.talk_history(&token, before).await
}

#[tauri::command]
async fn talk_poll(state: State<'_, AppState>, token: String, last_known: i64) -> Result<Vec<nextcloud::TalkMessage>> {
    let (creds, uid) = state.nc.lock().await.clone().ok_or_else(|| AppError::Other("Nextcloud не подключён".into()))?;
    nextcloud::Nc::long_poll(creds, &uid)?.talk_poll(&token, last_known).await
}

#[tauri::command]
async fn talk_send(state: State<'_, AppState>, token: String, message: String, reply_to: Option<i64>) -> Result<()> {
    nc(&state).await?.talk_send(&token, &message, reply_to).await
}

const MAX_TALK_FILE_MB: u64 = 512;

/// Отправляет в беседу файл, выбранный через `pick_attachments`.
#[tauri::command]
async fn talk_send_file(state: State<'_, AppState>, token: String, attachment: String, caption: String, reply_to: Option<i64>) -> Result<()> {
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
async fn forget_attachment(state: State<'_, AppState>, attachment: String) -> Result<()> {
    state.picked.lock().await.remove(&attachment);
    Ok(())
}

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
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            login,
            auto_login,
            logout,
            list_folders,
            list_messages,
            get_message,
            set_flag,
            save_attachment,
            diagnostics,
            get_profile,
            update_profile,
            pick_attachments,
            send_message,
            list_contacts,
            search_contacts,
            list_events,
            create_event,
            update_event,
            delete_event,
            create_contact,
            update_contact,
            delete_contact,
            nc_status,
            nc_connect_start,
            nc_connect_poll,
            nc_connect_manual,
            nc_disconnect,
            nc_open,
            deck_boards,
            deck_stacks,
            deck_create_card,
            deck_update_card,
            deck_move_card,
            deck_delete_card,
            deck_create_stack,
            talk_rooms,
            talk_history,
            talk_poll,
            talk_send,
            talk_send_file,
            forget_attachment,
        ])
        .run(tauri::generate_context!())
        .expect("error while running RRCKI Mail");
}
