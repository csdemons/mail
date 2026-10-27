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
    if let Some(s) = settings::load(&app) {
        settings::forget_password(&s);
    }
    *state.creds.lock().await = None;
    *state.nc.lock().await = None;
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
    Ok((dav::DavClient::new(&creds.username, &creds.password)?, base))
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
                for c in list {
                    let dup = out.iter().any(|o| o.emails.iter().any(|e| c.emails.iter().any(|x| x.eq_ignore_ascii_case(e))));
                    if !dup {
                        out.push(c);
                    }
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

async fn sogo_search(state: &AppState, base: &url::Url, query: &str) -> Result<Vec<dav::Contact>> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    for attempt in 0..2 {
        let cached = state.sogo.lock().await.clone();
        let session = match cached {
            Some(Ok(s)) if attempt == 0 => s,
            Some(Err(e)) => return Err(AppError::Other(e)),
            _ => {
                let mut log = Vec::new();
                match sogo::login(base, &creds.username, &creds.password, &mut log).await {
                    Ok(s) => {
                        *state.sogo.lock().await = Some(Ok(s.clone()));
                        s
                    }
                    Err(e) => {
                        *state.sogo.lock().await = Some(Err(e.to_string()));
                        return Err(e);
                    }
                }
            }
        };
        match session.search(query).await {
            Ok(list) => return Ok(list),
            // Сессия истекла — войти заново один раз.
            Err(_) if attempt == 0 => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(Vec::new())
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

// ---------------------------------------------------------------- Nextcloud

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcStatus {
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
pub async fn deck_create_stack(state: State<'_, AppState>, board_id: i64, title: String) -> Result<()> {
    nc(&state).await?.deck_create_stack(board_id, &title).await
}

#[tauri::command]
pub async fn talk_rooms(state: State<'_, AppState>) -> Result<Vec<nextcloud::TalkRoom>> {
    nc(&state).await?.talk_rooms().await
}

#[tauri::command]
pub async fn talk_history(state: State<'_, AppState>, token: String, before: Option<i64>) -> Result<Vec<nextcloud::TalkMessage>> {
    nc(&state).await?.talk_history(&token, before).await
}

#[tauri::command]
pub async fn talk_poll(state: State<'_, AppState>, token: String, last_known: i64) -> Result<Vec<nextcloud::TalkMessage>> {
    let (creds, uid) = state.nc.lock().await.clone().ok_or_else(|| AppError::Other("Nextcloud не подключён".into()))?;
    nextcloud::Nc::long_poll(creds, &uid)?.talk_poll(&token, last_known).await
}

#[tauri::command]
pub async fn talk_send(state: State<'_, AppState>, token: String, message: String, reply_to: Option<i64>) -> Result<()> {
    nc(&state).await?.talk_send(&token, &message, reply_to).await
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
            crate::commands::talk_rooms,
            crate::commands::talk_history,
            crate::commands::talk_poll,
            crate::commands::talk_send,
            crate::commands::talk_send_file,
            crate::commands::app_info,
            crate::commands::check_update,
            crate::commands::update_progress,
            crate::commands::install_update,
            crate::commands::forget_attachment,
        ]
    };
}
pub(crate) use app_commands;
