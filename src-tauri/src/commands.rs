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
use crate::{dav, imap, nextcloud, settings, smtp, sogo, store, totp, updater};
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
    /// Вход, ожидающий кода TOTP (программа заблокирована): почта не подключена.
    pending: Mutex<Option<Pending>>,
    /// Новый секрет TOTP, пока пользователь не подтвердил его кодом.
    totp_setup: Mutex<Option<(String, String)>>,
    /// Перебор при настройке (до сохранения секрета).
    totp_throttle: Mutex<totp::Throttle>,
    /// Проверки кода выполняются по одной.
    totp_gate: Mutex<()>,
    /// Локальное хранилище писем и индекс поиска текущей сессии.
    store: Mutex<Option<std::sync::Arc<store::Store>>>,
    /// Почему хранилище не открылось (показывается в настройках).
    store_error: Mutex<Option<String>>,
    /// Письма в «окне отмены».
    pub outbox: Outbox,
    /// Номер сессии: фоновые задачи прошлой сессии (до выхода/блокировки) завершаются.
    session: AtomicU64,
    /// Значок в трее создан.
    tray: std::sync::atomic::AtomicBool,
    /// Закрытие окна сворачивает в трей.
    close_to_tray: std::sync::atomic::AtomicBool,
    /// Импорт, резервные копии: фоновые задачи с ходом выполнения.
    jobs: crate::transfer::Jobs,
    /// Выбранные источники импорта: id → файлы (пути не уходят во frontend).
    import_sources: Mutex<HashMap<String, crate::transfer::Source>>,
    /// Пароли учётных записей, добавленных без «Запомнить» (только в памяти процесса).
    session_passwords: Mutex<HashMap<String, String>>,
    /// Соединения с другими (неактивными) учётными записями: непрочитанные, общие «Входящие».
    others: Mutex<HashMap<String, Connection>>,
    /// Последний UID «Входящих» других учётных записей (для уведомлений).
    others_last: Mutex<HashMap<String, u32>>,
    /// Переключение, отключение учётной записи и выход выполняются по одному.
    switching: Mutex<()>,
}

impl AppState {
    /// Закрытие главного окна не завершает программу (она в трее).
    pub fn keeps_running(&self) -> bool {
        self.tray.load(Ordering::SeqCst) && self.close_to_tray.load(Ordering::SeqCst)
    }
}

/// Трей, настройки окна и запуск свёрнутым (автозапуск передаёт `--minimized`).
pub fn setup_desktop(app: &AppHandle) {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let prefs = host::config_dir(app).map(|d| crate::desktop::load_prefs(&d)).unwrap_or_default();
    state.close_to_tray.store(prefs.close_to_tray, Ordering::SeqCst);
    let tray = host::setup_tray(app);
    state.tray.store(tray, Ordering::SeqCst);
    if tray && std::env::args().any(|a| a == crate::desktop::MINIMIZED_ARG) {
        host::hide_main(app);
    }
}

/// Выход из программы: письма из «окна отмены» сначала уходят.
pub fn on_exit_requested(app: &AppHandle, prevent: impl FnOnce()) {
    use tauri::Manager;
    if app.state::<AppState>().outbox.pending() {
        prevent();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            app.state::<AppState>().outbox.flush().await;
            app.exit(0);
        });
    }
}

#[derive(Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopPrefs {
    close_to_tray: bool,
    autostart: bool,
    /// Только для чтения: есть ли значок в трее.
    #[serde(default)]
    tray: bool,
}

#[tauri::command]
pub fn desktop_prefs_get(app: AppHandle, state: State<'_, AppState>) -> DesktopPrefs {
    let _ = app;
    DesktopPrefs { close_to_tray: state.close_to_tray.load(Ordering::SeqCst), autostart: crate::desktop::autostart_enabled(), tray: state.tray.load(Ordering::SeqCst) }
}

#[tauri::command]
pub fn desktop_prefs_set(app: AppHandle, state: State<'_, AppState>, prefs: DesktopPrefs) -> Result<()> {
    if prefs.autostart != crate::desktop::autostart_enabled() {
        crate::desktop::set_autostart(prefs.autostart)?;
    }
    state.close_to_tray.store(prefs.close_to_tray, Ordering::SeqCst);
    crate::desktop::save_prefs(&host::config_dir(&app)?, &crate::desktop::UiPrefs { close_to_tray: prefs.close_to_tray })
}

/// Непрочитанные во «Входящих» — для значка в трее и на панели задач.
#[tauri::command]
pub fn set_unread(app: AppHandle, count: u32) {
    host::set_unread(&app, count);
}

/// Показать главное окно (щелчок по уведомлению и т. п.).
#[tauri::command]
pub fn show_main_window(app: AppHandle) {
    host::show_main(&app);
}

/// Сессия, которая начнётся после верного кода TOTP.
struct Pending {
    creds: Credentials,
    profile: Settings,
    /// Ручной вход: сохранить пароль (Some(true)), забыть (Some(false)); блокировка — None.
    remember: Option<bool>,
}

fn account_key(username: &str, host: &str) -> String {
    format!("{}@{}", username.to_lowercase(), host.to_lowercase())
}

/// Выполняет операцию на открытом соединении. Если соединение оборвалось
/// (сервер закрыл его по таймауту простоя и т.п.) — переподключается и
/// повторяет операцию один раз.
macro_rules! with_conn {
    ($state:expr, $c:ident => $body:expr) => {
        with_conn!(@run $state, $c, $body, false)
    };
    // Неидемпотентная операция (APPEND в «Отправленные»): после таймаута не повторять —
    // сервер мог уже сохранить письмо, повтор дал бы копию.
    (once $state:expr, $c:ident => $body:expr) => {
        with_conn!(@run $state, $c, $body, true)
    };
    (@run $state:expr, $c:ident, $body:expr, $no_timeout_retry:expr) => {{
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
                    if attempt == 0 && !($no_timeout_retry && matches!(e, AppError::Timeout)) {
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
    /// Программа заблокирована: нужен код TOTP (почта ещё не подключена).
    locked: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    account: Account,
    /// Предупреждение, если пароль не удалось сохранить в системное хранилище.
    warning: Option<String>,
}

async fn start_session(app: &AppHandle, state: &AppState, creds: Credentials, profile: Settings) -> Result<()> {
    let conn = imap::connect(&creds).await?;
    open_store(app, state, &creds, &profile).await;
    let generation = state.session.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(run_scheduler(app.clone(), generation));
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

// ---------------------------------------------------------------- локальное хранилище

/// Срок хранения писем на компьютере: настройка пользователя в пределах политики.
fn offline_days(profile: &Settings) -> u32 {
    profile.offline_days.unwrap_or(store::DEFAULT_DAYS).min(crate::policy::load().max_offline_days())
}

async fn open_store(app: &AppHandle, state: &AppState, creds: &Credentials, profile: &Settings) {
    close_store(state).await;
    let Ok(root) = host::data_dir(app) else { return };
    let account = account_key(&creds.username, &creds.host);
    if !crate::policy::load().local_store() {
        // Администратор запретил хранить почту на компьютере — убрать то, что было.
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || store::wipe(&root, &account)).await {
            log::warn!("local store not removed: {e}");
        }
        return;
    }
    let days = offline_days(profile);
    *state.store_error.lock().await = None;
    match tokio::task::spawn_blocking(move || store::Store::open(&root, &account, days)).await {
        Ok(Ok(st)) => {
            let st = std::sync::Arc::new(st);
            *state.store.lock().await = Some(st.clone());
            tauri::async_runtime::spawn(store::run_sync(st, creds.clone()));
        }
        Ok(Err(e)) => {
            log::warn!("local store unavailable: {e}");
            *state.store_error.lock().await = Some(e.to_string());
        }
        Err(e) => log::warn!("local store: {e}"),
    }
}

async fn close_store(state: &AppState) {
    if let Some(st) = state.store.lock().await.take() {
        let _ = tokio::task::spawn_blocking(move || st.close()).await;
    }
}

async fn local(state: &AppState) -> Option<std::sync::Arc<store::Store>> {
    state.store.lock().await.clone()
}

async fn local_raw(state: &AppState, folder: &str, uid: u32) -> Option<Vec<u8>> {
    local(state).await?.raw(folder, uid)
}

/// Вложение: из локального хранилища, иначе с сервера.
async fn attachment(state: &AppState, folder: &str, uid: u32, index: usize) -> Result<(String, Vec<u8>)> {
    if let Some(raw) = local_raw(state, folder, uid).await {
        return imap::attachment_in(&raw, index);
    }
    with_conn!(state, c => imap::get_attachment(c, folder, uid, index))
}

/// Серверы почты по адресу (вход «адрес + пароль»).
#[tauri::command]
pub async fn discover_servers(email: String) -> Result<crate::discover::Discovered> {
    crate::discover::discover(&email).await
}

/// Папки из локального хранилища — показываются сразу, до ответа сервера.
#[tauri::command]
pub async fn cached_folders(state: State<'_, AppState>) -> Result<Option<Vec<Folder>>> {
    Ok(local(&state).await.and_then(|st| st.folders()))
}

/// Письма папки из локального хранилища (затем интерфейс запрашивает сервер).
#[tauri::command]
pub async fn cached_messages(state: State<'_, AppState>, folder: String, before_uid: Option<u32>, limit: u32) -> Result<Option<MessagePage>> {
    Ok(local(&state).await.and_then(|st| st.page(&folder, before_uid, limit.clamp(1, 200) as usize)))
}

/// Мгновенный поиск по индексу: все свои папки, отправитель/получатели/тема и
/// текст писем, хранимых на компьютере.
#[tauri::command]
pub async fn search_mail(state: State<'_, AppState>, query: String) -> Result<Vec<imap::MessageSummary>> {
    let Some(st) = local(&state).await else { return Ok(Vec::new()) };
    let q = query.trim().to_string();
    if q.chars().count() < 2 {
        return Ok(Vec::new());
    }
    tokio::task::spawn_blocking(move || st.search(&q, 300)).await.map_err(|e| AppError::Other(e.to_string()))
}

/// Поиск по тексту всех писем на сервере (своих папок) — отдельным соединением,
/// чтобы не задерживать остальную работу.
#[tauri::command]
pub async fn search_server(state: State<'_, AppState>, query: String) -> Result<Vec<imap::MessageSummary>> {
    let q = query.trim().to_string();
    if q.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let mut conn = imap::connect(&creds).await?;
    let folders = imap::list_folders(&mut conn).await?;
    let mut out = Vec::new();
    for f in folders.iter().filter(|f| f.selectable && !imap::is_shared(f)) {
        let found = tokio::time::timeout(Duration::from_secs(60), async {
            let uids = imap::search_server(&mut conn, &f.id, &q).await?;
            let uids: Vec<u32> = uids.into_iter().take(100).collect();
            imap::summaries_of(&mut conn, &f.id, &uids).await
        })
        .await;
        match found {
            Ok(Ok(items)) => out.extend(items),
            Ok(Err(e)) if e.is_connection() => return Err(e),
            Ok(Err(e)) => log::warn!("server search in a folder failed: {e}"),
            Err(_) => return Err(AppError::Timeout),
        }
        if out.len() >= 500 {
            break;
        }
    }
    conn.logout().await;
    out.sort_by(|a, b| {
        let ts = |m: &imap::MessageSummary| m.date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map_or(0, |d| d.timestamp());
        ts(b).cmp(&ts(a))
    });
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineStatus {
    /// Хранилище работает в этой сессии.
    enabled: bool,
    /// Разрешено администратором.
    allowed: bool,
    /// Срок хранения, выбранный пользователем, и наибольший по политике.
    days: u32,
    max_days: u32,
    messages: usize,
    cached: usize,
    syncing: bool,
    bytes: u64,
    error: Option<String>,
}

#[tauri::command]
pub async fn offline_status(state: State<'_, AppState>) -> Result<OfflineStatus> {
    let policy = crate::policy::load();
    let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let st = match local(&state).await {
        Some(st) => Some(tokio::task::spawn_blocking(move || st.status()).await.map_err(|e| AppError::Other(e.to_string()))?),
        None => None,
    };
    Ok(OfflineStatus {
        enabled: st.is_some(),
        allowed: policy.local_store(),
        days: offline_days(&profile),
        max_days: policy.max_offline_days(),
        messages: st.map_or(0, |s| s.messages),
        cached: st.map_or(0, |s| s.cached),
        syncing: st.is_some_and(|s| s.syncing),
        bytes: st.map_or(0, |s| s.bytes),
        error: state.store_error.lock().await.clone(),
    })
}

/// Срок хранения писем на компьютере; хранилище открывается заново с новым сроком.
#[tauri::command]
pub async fn set_offline_days(app: AppHandle, state: State<'_, AppState>, days: u32) -> Result<()> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let profile = {
        let mut guard = state.profile.lock().await;
        let p = guard.as_mut().ok_or(AppError::NotLoggedIn)?;
        p.offline_days = Some(days.min(3650));
        p.clone()
    };
    settings::save(&app, &profile)?;
    open_store(&app, &state, &creds, &profile).await;
    Ok(())
}

/// Удалить письма, хранимые на компьютере (индекс соберётся заново).
#[tauri::command]
pub async fn clear_offline(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    close_store(&state).await;
    let root = host::data_dir(&app)?;
    let account = account_key(&creds.username, &creds.host);
    tokio::task::spawn_blocking(move || store::wipe(&root, &account)).await.map_err(|e| AppError::Other(e.to_string()))??;
    open_store(&app, &state, &creds, &profile).await;
    Ok(())
}

// ---------------------------------------------------------------- подпись

/// Личная подпись. Пока своя не задана — берётся подпись из веб-почты (SOGo).
#[tauri::command]
pub async fn signature_get(state: State<'_, AppState>) -> Result<String> {
    let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    if let Some(sig) = profile.signature {
        return Ok(sig);
    }
    let Ok((_, base)) = dav_client(&state).await else { return Ok(String::new()) };
    match sogo_defaults(&state, &base).await {
        Ok((_, d)) => Ok(d.get("SOGoMailSignature").and_then(|v| v.as_str()).map(html_to_text).unwrap_or_default()),
        Err(_) => Ok(String::new()),
    }
}

/// Сохранить подпись в программе и (если доступно) в веб-почте SOGo.
/// Возвращает предупреждение, если в веб-почту записать не удалось.
#[tauri::command]
pub async fn signature_save(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<Option<String>> {
    let text: String = text.replace("\r\n", "\n").chars().take(4000).collect();
    let profile = {
        let mut guard = state.profile.lock().await;
        let p = guard.as_mut().ok_or(AppError::NotLoggedIn)?;
        p.signature = Some(text.clone());
        p.clone()
    };
    settings::save(&app, &profile)?;
    let synced = async {
        let (_, base) = dav_client(&state).await?;
        let (session, mut defaults) = sogo_defaults(&state, &base).await?;
        defaults["SOGoMailSignature"] = serde_json::Value::String(text_to_html(&text));
        session.save_defaults(&defaults).await
    }
    .await;
    Ok(synced.err().map(|e| format!("Подпись сохранена в программе, но не в веб-почте: {e}")))
}

fn html_to_text(html: &str) -> String {
    if !html.contains('<') {
        return html.trim().to_string();
    }
    let with_breaks = html.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n").replace("</p>", "\n").replace("</div>", "\n");
    let mut out = String::new();
    let mut in_tag = false;
    for c in with_breaks.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&").trim().to_string()
}

fn text_to_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\n', "<br>")
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
        locked: false,
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
    if let Some(u) = dav_url.as_deref() {
        settings::validate_dav_url(u)?;
    }
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
        signature: mine.and_then(|p| p.signature.clone()),
        offline_days: mine.and_then(|p| p.offline_days),
        categories: mine.and_then(|p| p.categories.clone()),
    };

    let creds = Credentials {
        host,
        port,
        username,
        password: password.clone(),
    };
    if totp::load(&account_key(&s.username, &s.host)).is_some() {
        // Пароль проверяем сразу, а почту подключаем только после кода TOTP.
        imap::connect(&creds).await?.logout().await;
        if let Err(e) = settings::save(&app, &s) {
            log::warn!("settings not saved: {e}");
        }
        *state.pending.lock().await = Some(Pending { creds, profile: s.clone(), remember: Some(remember) });
        return Ok(LoginResult { account: Account { locked: true, ..account_of(&s) }, warning: None });
    }
    start_session(&app, &state, creds, s.clone()).await?;

    // Вход уже выполнен: ошибки сохранения настроек/пароля не отменяют его,
    // а показываются пользователю предупреждением.
    let mut warnings = Vec::new();
    if let Err(e) = settings::save(&app, &s) {
        log::warn!("settings not saved: {e}");
        warnings.push(format!("Не удалось сохранить настройки подключения: {e}."));
    }
    // Политика администратора может запретить запоминать пароль.
    let remember = remember && crate::policy::load().ui().allow_remember_password;
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
    if totp::load(&account_key(&s.username, &s.host)).is_some() {
        // Сохранённый пароль используется только после кода из приложения.
        *state.pending.lock().await = Some(Pending { creds, profile: s.clone(), remember: None });
        return Ok(Some(Account { locked: true, ..account_of(&s) }));
    }
    match start_session(&app, &state, creds, s.clone()).await {
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
pub async fn logout(app: AppHandle, state: State<'_, AppState>, reset_totp: Option<bool>) -> Result<()> {
    let _one_at_a_time = state.switching.lock().await;
    // Окна писем показывают данные этой учётной записи.
    host::close_message_windows(&app);
    // Телефон потерян: с экрана блокировки можно выйти со сбросом защиты кодом.
    // Сохранённый пароль при этом тоже забывается — снова войти можно только зная
    // пароль почты (с ним переписку и так можно прочитать в веб-почте).
    if reset_totp == Some(true) {
        if let Some(p) = state.pending.lock().await.as_ref() {
            totp::remove(&account_key(&p.creds.username, &p.creds.host));
        }
    }
    // Письма из «окна отмены» уходят, фоновые задачи сессии останавливаются.
    state.outbox.flush().await;
    state.session.fetch_add(1, Ordering::SeqCst);
    state.jobs.cancel_all();
    state.import_sources.lock().await.clear();
    // Письма на компьютере удаляются вместе с паролем.
    close_store(&state).await;
    let account = match (state.creds.lock().await.as_ref(), state.pending.lock().await.as_ref()) {
        (Some(c), _) => Some(account_key(&c.username, &c.host)),
        (None, Some(p)) => Some(account_key(&p.creds.username, &p.creds.host)),
        _ => settings::load(&app).map(|s| account_key(&s.username, &s.host)),
    };
    if let (Some(account), Ok(root)) = (account.clone(), host::data_dir(&app)) {
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || store::wipe(&root, &account)).await {
            log::warn!("local store not removed on logout: {e}");
        }
    }
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
    *state.pending.lock().await = None;
    *state.totp_setup.lock().await = None;
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
    // Учётная запись уходит из списка; остальные остаются подключёнными.
    if let Some(key) = account {
        state.session_passwords.lock().await.remove(&key);
        if let Some(c) = state.others.lock().await.remove(&key) {
            c.logout().await;
        }
        if let Err(e) = settings::remove_account(&app, &key) {
            log::warn!("account list not updated: {e}");
        }
    }
    // Последняя учётная запись — в памяти не остаётся ни паролей, ни соединений.
    if settings::load_accounts(&app).is_empty() {
        state.session_passwords.lock().await.clear();
        state.others_last.lock().await.clear();
        drop_others(&state).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn list_folders(state: State<'_, AppState>) -> Result<Vec<Folder>> {
    let folders = with_conn!(state, c => imap::list_folders(c))?;
    if let Some(st) = local(&state).await {
        st.set_folders(&folders);
    }
    Ok(folders)
}

#[tauri::command]
pub async fn list_messages(
    state: State<'_, AppState>,
    folder: String,
    before_uid: Option<u32>,
    limit: u32,
) -> Result<MessagePage> {
    let page = with_conn!(state, c => imap::list_messages(c, &folder, before_uid, limit))?;
    if let Some(st) = local(&state).await {
        st.merge_page(&folder, before_uid, &page);
    }
    Ok(page)
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
    password: Option<String>,
) -> Result<Settings> {
    settings::validate_dav_url(&dav_url)?;
    let mut guard = state.profile.lock().await;
    let mut p = guard.clone().ok_or(AppError::NotLoggedIn)?;
    // Смена серверов (куда уходит пароль почты) — только с подтверждением паролем:
    // даже если в интерфейс попадёт чужой код, увести пароль на свой сервер он не сможет.
    let servers_changed = p.smtp_host != smtp_host.trim() || p.smtp_port != smtp_port || p.dav_url != dav_url.trim();
    if servers_changed {
        let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
        if password.as_deref() != Some(creds.password.as_str()) {
            return Err(AppError::Other("Чтобы изменить адреса серверов, введите пароль почты".into()));
        }
    }
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

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    /// Папка, куда сохранена копия, или `None`.
    saved_to: Option<String>,
    warning: Option<String>,
}

// ---------------------------------------------------------------- исходящие: отмена и отложенная отправка

#[derive(Clone, Copy, PartialEq)]
enum OutCmd {
    Wait,
    Now,
    Cancel,
}

type Delivery = Option<std::result::Result<SendResult, String>>;

struct OutItem {
    ctl: tokio::sync::watch::Sender<OutCmd>,
    done: tokio::sync::watch::Receiver<Delivery>,
    started: bool,
}

/// Письма, ожидающие окончания «окна отмены».
#[derive(Default)]
pub struct Outbox {
    next: AtomicU64,
    items: std::sync::Mutex<HashMap<u64, OutItem>>,
}

impl Outbox {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, OutItem>> {
        self.items.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Есть письма, которые ещё не ушли.
    pub fn pending(&self) -> bool {
        self.lock().values().any(|i| i.done.borrow().is_none())
    }

    /// Отправить всё ожидающее сразу и дождаться (выход из программы).
    pub async fn flush(&self) {
        let waits: Vec<_> = self
            .lock()
            .values()
            .map(|i| {
                let _ = i.ctl.send(OutCmd::Now);
                i.done.clone()
            })
            .collect();
        for mut w in waits {
            let _ = tokio::time::timeout(Duration::from_secs(90), w.wait_for(|d| d.is_some())).await;
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendTicket {
    /// Письмо в «окне отмены»: ждать `send_wait`, отменить `send_cancel`.
    id: Option<u64>,
    /// Отправлено сразу.
    result: Option<SendResult>,
    /// Отложенная отправка: письмо в «Черновиках» до указанного времени.
    scheduled_at: Option<String>,
}

/// Отправка и копия в «Отправленные» (письмо уже ушло — ошибка копии лишь предупреждение).
async fn deliver(state: &AppState, profile: &Settings, creds: &Credentials, raw: &[u8], built: Option<&lettre::Message>) -> Result<SendResult> {
    match built {
        Some(m) => smtp::send(&profile.smtp_host, profile.smtp_port, &creds.username, &creds.password, m).await?,
        None => smtp::send_raw(&profile.smtp_host, profile.smtp_port, &creds.username, &creds.password, raw).await?,
    }
    match with_conn!(once state, c => imap::append_sent(c, raw)) {
        Ok(folder) => Ok(SendResult { saved_to: Some(folder), warning: None }),
        Err(e) => Ok(SendResult { saved_to: None, warning: Some(format!("Письмо отправлено, но копия не сохранена в «Отправленные»: {e}")) }),
    }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    to: String,
    cc: String,
    subject: String,
    body: String,
    html: Option<String>,
    in_reply_to: Option<String>,
    references: Option<String>,
    attachments: Vec<String>,
    read_receipt: Option<bool>,
    delay_secs: Option<u32>,
    send_at: Option<String>,
) -> Result<SendTicket> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;

    let send_at = match send_at.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(t) => {
            let at = chrono::DateTime::parse_from_rfc3339(t).map_err(|_| AppError::Other("Некорректное время отправки".into()))?;
            if at.timestamp() < chrono::Utc::now().timestamp() + 30 {
                return Err(AppError::Other("Время отправки уже прошло — выберите время позже".into()));
            }
            Some(at.to_rfc3339())
        }
        None => None,
    };

    let mut files = Vec::new();
    let mut total = 0u64;
    {
        let picked = state.picked.lock().await;
        for id in &attachments {
            let path = picked
                .get(id)
                .ok_or_else(|| AppError::Other("Вложение не найдено, выберите файл заново".into()))?;
            // Размер — по метаданным, до чтения: огромный файл не попадает в память.
            total += tokio::fs::metadata(path).await?.len();
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
            let data = tokio::fs::read(path).await?;
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
        read_receipt: read_receipt.unwrap_or(false),
        extra_headers: send_at.iter().map(|t| (imap::SEND_AT_HEADER.to_string(), t.clone())).collect(),
    })?;
    let raw = message.formatted();

    // Отложенная отправка: письмо ждёт в «Черновиках» (видно и в веб-почте) и
    // уходит в срок, пока программа запущена, — или при следующем запуске.
    if let Some(at) = send_at {
        with_conn!(once state, c => imap::append_draft(c, &raw))?;
        forget_picked(&state, &attachments).await;
        return Ok(SendTicket { id: None, result: None, scheduled_at: Some(at) });
    }

    let delay = delay_secs.unwrap_or(0).min(60);
    if delay == 0 {
        let result = deliver(&state, &profile, &creds, &raw, Some(&message)).await?;
        forget_picked(&state, &attachments).await;
        return Ok(SendTicket { id: None, result: Some(result), scheduled_at: None });
    }

    // «Окно отмены»: письмо уходит через `delay` секунд, если его не отменили.
    let id = state.outbox.next.fetch_add(1, Ordering::SeqCst) + 1;
    let (ctl_tx, mut ctl_rx) = tokio::sync::watch::channel(OutCmd::Wait);
    let (done_tx, done_rx) = tokio::sync::watch::channel::<Delivery>(None);
    state.outbox.lock().insert(id, OutItem { ctl: ctl_tx, done: done_rx, started: false });
    tauri::async_runtime::spawn(async move {
        use tauri::Manager;
        let state = app.state::<AppState>();
        let sleep = tokio::time::sleep(Duration::from_secs(u64::from(delay)));
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                changed = ctl_rx.changed() => {
                    let cmd = if changed.is_ok() { *ctl_rx.borrow() } else { OutCmd::Now };
                    match cmd {
                        OutCmd::Now => break,
                        OutCmd::Cancel => {
                            let _ = done_tx.send(Some(Err("Отправка отменена".into())));
                            return;
                        }
                        OutCmd::Wait => {}
                    }
                }
            }
        }
        {
            let mut items = state.outbox.lock();
            match items.get_mut(&id) {
                Some(item) if *item.ctl.borrow() == OutCmd::Cancel => {
                    let _ = done_tx.send(Some(Err("Отправка отменена".into())));
                    return;
                }
                Some(item) => item.started = true,
                None => return,
            }
        }
        let result = deliver(&state, &profile, &creds, &raw, Some(&message)).await;
        if result.is_ok() {
            forget_picked(&state, &attachments).await;
        }
        let _ = done_tx.send(Some(result.map_err(|e| e.to_string())));
    });
    Ok(SendTicket { id: Some(id), result: None, scheduled_at: None })
}

async fn forget_picked(state: &AppState, ids: &[String]) {
    let mut picked = state.picked.lock().await;
    for id in ids {
        // Перетащенный файл лежит в своём временном каталоге — убираем и его.
        if let Some(path) = picked.remove(id) {
            if let Some(dir) = path.parent().filter(|d| d.file_name().is_some_and(|n| n.to_string_lossy().starts_with("drop-"))) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// Дождаться отправки письма из «окна отмены».
#[tauri::command]
pub async fn send_wait(state: State<'_, AppState>, id: u64) -> Result<SendResult> {
    let mut done = state.outbox.lock().get(&id).map(|i| i.done.clone()).ok_or(AppError::NotFound)?;
    let result = done.wait_for(|d| d.is_some()).await.map_err(|_| AppError::Other("Отправка прервана".into()))?.clone();
    state.outbox.lock().remove(&id);
    result.expect("checked above").map_err(AppError::Other)
}

/// Отменить отправку, пока письмо не ушло. `false` — уже отправляется.
#[tauri::command]
pub async fn send_cancel(state: State<'_, AppState>, id: u64) -> Result<bool> {
    let items = state.outbox.lock();
    let Some(item) = items.get(&id) else { return Ok(false) };
    if item.started || item.done.borrow().is_some() {
        return Ok(false);
    }
    let _ = item.ctl.send(OutCmd::Cancel);
    Ok(true)
}

/// Отправить сразу, не дожидаясь конца «окна отмены».
#[tauri::command]
pub async fn send_now(state: State<'_, AppState>, id: u64) -> Result<()> {
    if let Some(item) = state.outbox.lock().get(&id) {
        let _ = item.ctl.send(OutCmd::Now);
    }
    Ok(())
}

/// Запланированные письма (ждут в «Черновиках»).
#[tauri::command]
pub async fn scheduled_list(state: State<'_, AppState>) -> Result<Vec<imap::Scheduled>> {
    with_conn!(state, c => imap::scheduled(c))
}

/// Отправить запланированное письмо сейчас.
#[tauri::command]
pub async fn scheduled_send_now(state: State<'_, AppState>, folder: String, uid: u32) -> Result<SendResult> {
    send_scheduled(&state, &folder, uid, "manual").await?.ok_or_else(|| AppError::Other("Письмо уже отправляется с другого компьютера".into()))
}

/// Отменить отложенную отправку: письмо остаётся обычным черновиком.
#[tauri::command]
pub async fn scheduled_cancel(state: State<'_, AppState>, folder: String, uid: u32) -> Result<()> {
    with_conn!(once state, c => async {
        let raw = imap::raw_of(c, &folder, uid).await?;
        let plain = smtp::strip_header(&raw, imap::SEND_AT_HEADER);
        imap::append_draft(c, &plain).await?;
        imap::remove_message(c, &folder, uid).await
    })
}

/// Отправка одного запланированного письма (с захватом, чтобы не отправить дважды).
/// `None` — письмо отправляет другая копия программы.
async fn send_scheduled(state: &AppState, folder: &str, uid: u32, tag: &str) -> Result<Option<SendResult>> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let token = format!("{}{}{}", chrono::Utc::now().timestamp(), tag.chars().take(4).collect::<String>(), &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let mut conn = imap::connect(&creds).await?;
    let result = async {
        if !imap::claim(&mut conn, folder, uid, &token).await? {
            return Ok(None);
        }
        let raw = imap::raw_of(&mut conn, folder, uid).await?;
        let final_raw = smtp::finalize_scheduled(&raw, imap::SEND_AT_HEADER);
        match deliver(state, &profile, &creds, &final_raw, None).await {
            Ok(r) => {
                imap::remove_message(&mut conn, folder, uid).await?;
                Ok(Some(r))
            }
            Err(e) => {
                // Не ушло — снимаем свою метку захвата, попробуем в следующий раз.
                let _ = imap::set_keyword(&mut conn, folder, &[uid], &format!("atomclaim{token}"), false).await;
                Err(e)
            }
        }
    }
    .await;
    conn.logout().await;
    result
}

async fn scheduled_now(state: &AppState) -> Result<Vec<imap::Scheduled>> {
    with_conn!(state, c => imap::scheduled(c))
}

/// Фоновая отправка запланированных писем, пока сессия та же.
async fn run_scheduler(app: AppHandle, generation: u64) {
    use tauri::Manager;
    let every = if cfg!(debug_assertions) && std::env::var_os("RRCKI_E2E_POLL_SECS").is_some() { 5 } else { 60 };
    loop {
        tokio::time::sleep(Duration::from_secs(every)).await;
        let state = app.state::<AppState>();
        if state.session.load(Ordering::SeqCst) != generation {
            return;
        }
        match return_snoozed(&state).await {
            Ok(n) if n > 0 => {
                let _ = host::notify(&app, "Отложенные письма вернулись", &format!("Во «Входящих»: {n}"));
            }
            Err(e) => log::warn!("snoozed mail check failed: {e}"),
            _ => {}
        }
        let list = match scheduled_now(&state).await {
            Ok(l) => l,
            Err(e) => {
                log::warn!("scheduled mail check failed: {e}");
                continue;
            }
        };
        let now = chrono::Utc::now();
        for s in list {
            let due = chrono::DateTime::parse_from_rfc3339(&s.send_at).is_ok_and(|t| t <= now);
            if !due || state.session.load(Ordering::SeqCst) != generation {
                continue;
            }
            match send_scheduled(&state, &s.folder, s.uid, "auto").await {
                Ok(Some(_)) => {
                    let _ = host::notify(&app, "Отложенное письмо отправлено", if s.subject.is_empty() { "(без темы)" } else { &s.subject });
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!("scheduled send failed: {e}");
                    let _ = host::notify(&app, "Отложенное письмо не отправлено", &format!("{}: {e}", s.subject));
                }
            }
        }
    }
}

// ---------------------------------------------------------------- категории, сроки, шаблоны

#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    /// Ключевое слово IMAP ($label2… или своё).
    keyword: String,
    name: String,
    color: String,
}

/// Метки веб-почты SOGo по умолчанию — с русскими названиями.
fn default_categories() -> Vec<Category> {
    [("$label1", "Важное", "#f00"), ("$label2", "Работа", "#ff9a00"), ("$label3", "Личное", "#009900"), ("$label4", "К исполнению", "#3333ff"), ("$label5", "Позже", "#993399")]
        .iter()
        .map(|(k, n, c)| Category { keyword: (*k).into(), name: (*n).into(), color: (*c).into() })
        .collect()
}

/// Английские названия меток SOGo по умолчанию показываем по-русски.
fn localize_label(name: &str) -> String {
    match name {
        "Important" => "Важное",
        "Work" => "Работа",
        "Personal" => "Личное",
        "To Do" => "К исполнению",
        "Later" => "Позже",
        other => other,
    }
    .to_string()
}

fn valid_color(c: &str) -> bool {
    let h = c.strip_prefix('#').unwrap_or("");
    (h.len() == 3 || h.len() == 6) && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Категории писем — общие с веб-почтой (SOGo: SOGoMailLabelsColors).
#[tauri::command]
pub async fn categories_get(state: State<'_, AppState>) -> Result<Vec<Category>> {
    if let Ok((_, base)) = dav_client(&state).await {
        if let Ok((_, d)) = sogo_defaults(&state, &base).await {
            if let Some(map) = d.get("SOGoMailLabelsColors").and_then(|v| v.as_object()) {
                let mut list: Vec<Category> = map
                    .iter()
                    .filter_map(|(k, v)| {
                        let arr = v.as_array()?;
                        Some(Category { keyword: k.clone(), name: localize_label(arr.first()?.as_str()?), color: arr.get(1)?.as_str()?.to_string() })
                    })
                    .filter(|c| imap::valid_keyword(&c.keyword))
                    .collect();
                list.sort_by(|a, b| a.keyword.cmp(&b.keyword));
                if !list.is_empty() {
                    return Ok(list);
                }
            }
            return Ok(default_categories());
        }
    }
    let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    Ok(profile
        .categories
        .map(|l| l.into_iter().map(|(keyword, name, color)| Category { keyword, name, color }).collect())
        .unwrap_or_else(default_categories))
}

/// Сохранить категории (в веб-почте SOGo; если она недоступна — в программе).
#[tauri::command]
pub async fn categories_save(app: AppHandle, state: State<'_, AppState>, categories: Vec<Category>) -> Result<Option<String>> {
    for c in &categories {
        if !imap::valid_keyword(&c.keyword) || c.name.trim().is_empty() || c.name.len() > 60 || !valid_color(&c.color) {
            return Err(AppError::Other(format!("Некорректная категория «{}»", c.name)));
        }
    }
    let profile = {
        let mut guard = state.profile.lock().await;
        let p = guard.as_mut().ok_or(AppError::NotLoggedIn)?;
        p.categories = Some(categories.iter().map(|c| (c.keyword.clone(), c.name.trim().to_string(), c.color.clone())).collect());
        p.clone()
    };
    settings::save(&app, &profile)?;
    let synced = async {
        let (_, base) = dav_client(&state).await?;
        let (session, mut defaults) = sogo_defaults(&state, &base).await?;
        let map: serde_json::Map<String, serde_json::Value> =
            categories.iter().map(|c| (c.keyword.clone(), serde_json::json!([c.name.trim(), c.color]))).collect();
        defaults["SOGoMailLabelsColors"] = serde_json::Value::Object(map);
        session.save_defaults(&defaults).await
    }
    .await;
    Ok(synced.err().map(|e| format!("Категории сохранены в программе, но не в веб-почте: {e}")))
}

/// Поставить или снять категорию (ключевое слово) у писем.
#[tauri::command]
pub async fn set_keywords(state: State<'_, AppState>, folder: String, uids: Vec<u32>, keyword: String, value: bool) -> Result<()> {
    with_conn!(state, c => imap::set_keyword(c, &folder, &uids, &keyword, value))?;
    if let Some(st) = local(&state).await {
        for uid in &uids {
            st.patch(&folder, *uid, |m| {
                m.keywords.retain(|k| !k.eq_ignore_ascii_case(&keyword));
                if value {
                    m.keywords.push(keyword.clone());
                }
            });
        }
    }
    Ok(())
}

/// Флажок со сроком (RFC 3339) или без срока (`None`). Срок хранится на сервере
/// ключевым словом — виден на всех компьютерах пользователя.
#[tauri::command]
pub async fn set_due(state: State<'_, AppState>, folder: String, uid: u32, due: Option<String>) -> Result<()> {
    let due = match due {
        Some(d) => Some(chrono::DateTime::parse_from_rfc3339(&d).map_err(|_| AppError::Other("Некорректная дата".into()))?.with_timezone(&chrono::Utc)),
        None => None,
    };
    with_conn!(state, c => imap::set_due(c, &folder, uid, due))?;
    if let Some(st) = local(&state).await {
        st.patch(&folder, uid, |m| {
            m.flagged = true;
            m.keywords.retain(|k| !k.starts_with(imap::DUE_PREFIX));
            if let Some(at) = due {
                m.keywords.push(imap::due_keyword(&at));
            }
        });
    }
    Ok(())
}

/// Письма с флажком и сроком во всех своих папках (для напоминаний).
#[tauri::command]
pub async fn due_list(state: State<'_, AppState>) -> Result<Vec<imap::MessageSummary>> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let mut conn = imap::connect(&creds).await?;
    let folders = imap::list_folders(&mut conn).await?;
    let mut out = Vec::new();
    for f in folders.iter().filter(|f| f.selectable && !imap::is_shared(f) && !matches!(f.role, imap::FolderRole::Trash | imap::FolderRole::Junk)) {
        match imap::due_in(&mut conn, &f.id).await {
            Ok(items) => out.extend(items),
            Err(e) if e.is_connection() => return Err(e),
            Err(e) => log::warn!("due scan failed in a folder: {e}"),
        }
    }
    conn.logout().await;
    out.sort_by_key(|m| imap::due_of(&m.keywords));
    Ok(out)
}

#[tauri::command]
pub async fn templates_list(state: State<'_, AppState>) -> Result<Vec<imap::Template>> {
    with_conn!(state, c => imap::templates(c))
}

/// Сохранить шаблон письма (папка «Templates» на сервере — как в Thunderbird).
#[tauri::command]
pub async fn template_save(state: State<'_, AppState>, name: String, text: String, html: Option<String>) -> Result<()> {
    let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::Other("Укажите название шаблона".into()));
    }
    let msg = smtp::build(smtp::Outgoing {
        from_name: Some(profile.display_name.clone()),
        from_address: profile.email(),
        to: vec![profile.email()],
        cc: vec![],
        subject: name,
        body: text,
        html,
        in_reply_to: None,
        references: None,
        attachments: vec![],
        read_receipt: false,
        extra_headers: vec![],
    })?;
    let raw = msg.formatted();
    with_conn!(once state, c => imap::append_template(c, &raw))
}

#[tauri::command]
pub async fn template_delete(state: State<'_, AppState>, folder: String, uid: u32) -> Result<()> {
    with_conn!(state, c => imap::remove_message(c, &folder, uid))
}

// ---------------------------------------------------------------- перетаскивание, спам, отписка, «отложить»

/// Файл, перетащенный в письмо: содержимое приходит из окна (base64), сохраняется
/// в личный временный каталог и дальше вкладывается как выбранный в диалоге.
#[tauri::command]
pub async fn attach_data(state: State<'_, AppState>, name: String, data: String) -> Result<PickedFile> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|_| AppError::Other("Файл не прочитан".into()))?;
    if bytes.len() as u64 > MAX_OUTGOING_MB * 1024 * 1024 {
        return Err(AppError::Other(format!("Файл больше {MAX_OUTGOING_MB} МБ — отправьте его ссылкой Nextcloud")));
    }
    let safe = imap::sanitize_filename(&name);
    let dir = crate::updater::private_dir("drop")?;
    let path = dir.join(if safe.is_empty() { "file".to_string() } else { safe.clone() });
    {
        use std::io::Write as _;
        let mut f = crate::updater::create_new_file(&path)?;
        f.write_all(&bytes)?;
    }
    let id = state.next_pick.fetch_add(1, Ordering::Relaxed).to_string();
    state.picked.lock().await.insert(id.clone(), path);
    Ok(PickedFile { id, name: safe, size: bytes.len() as u64 })
}

/// «Это спам» — в «Спам» с меткой $Junk; «Не спам» — во «Входящие» с меткой $NotJunk
/// (метки понимают фильтры спама сервера и Thunderbird).
#[tauri::command]
pub async fn mark_spam(state: State<'_, AppState>, folder: String, uids: Vec<u32>, spam: bool) -> Result<String> {
    let target = with_conn!(once state, c => async {
        let (add, remove) = if spam { ("$Junk", "$NotJunk") } else { ("$NotJunk", "$Junk") };
        imap::set_keyword(c, &folder, &uids, remove, false).await?;
        imap::set_keyword(c, &folder, &uids, add, true).await?;
        let target = if spam { imap::junk_folder(c).await? } else { "INBOX".to_string() };
        if target != folder {
            imap::move_messages(c, &folder, &uids, &target).await?;
        }
        Ok(target)
    })?;
    if let Some(st) = local(&state).await {
        st.remove(&folder, &uids);
    }
    Ok(target)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsubscribeResult {
    /// Готово (одним запросом или письмом).
    done: bool,
    /// Отписка только на сайте рассылки — адрес для браузера.
    open_url: Option<String>,
    how: String,
}

/// Отписаться от рассылки: RFC 8058 (один POST, без браузера), иначе письмом
/// на адрес отписки, иначе — ссылка для браузера.
#[tauri::command]
pub async fn unsubscribe(state: State<'_, AppState>, folder: String, uid: u32) -> Result<UnsubscribeResult> {
    let raw = match local_raw(&state, &folder, uid).await {
        Some(r) => r,
        None => with_conn!(state, c => imap::raw_of(c, &folder, uid))?,
    };
    let msg = mail_parser::MessageParser::default().parse_headers(&raw).ok_or(AppError::NotFound)?;
    let u = imap::unsubscribe_of(&msg).ok_or_else(|| AppError::Other("В письме нет способа отписаться".into()))?;
    if let (true, Some(link)) = (u.one_click, u.https.as_deref()) {
        let url = url::Url::parse(link).map_err(|_| AppError::Other("Некорректная ссылка отписки".into()))?;
        // Тот же контроль, что и для картинок: только публичный интернет, без перенаправлений.
        let policy = crate::policy::load();
        let host = url.host_str().unwrap_or_default().trim_start_matches('[').trim_end_matches(']').to_string();
        // E2E: сервер рассылки — мок на localhost (в релизе только публичные адреса).
        let allow_local = cfg!(debug_assertions) && std::env::var_os("RRCKI_E2E_ALLOW_LOCAL_IMAGES").is_some();
        if let Some(addr) = crate::html::resolve_public(&url, allow_local, &policy).await {
            let mut b = reqwest::Client::builder().timeout(Duration::from_secs(20)).redirect(reqwest::redirect::Policy::none());
            if policy.remote_images != crate::policy::RemoteImages::Proxy {
                b = b.no_proxy().resolve(&host, addr);
            }
            let client = b.build().map_err(|e| AppError::Other(e.to_string()))?;
            let resp = client
                .post(url.clone())
                .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body("List-Unsubscribe=One-Click")
                .send()
                .await;
            if resp.as_ref().is_ok_and(|r| r.status().is_success() || r.status().is_redirection()) {
                return Ok(UnsubscribeResult { done: true, open_url: None, how: "запросом на сервер рассылки".into() });
            }
        }
    }
    if let Some(mailto) = u.mailto.as_deref() {
        let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
        let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
        let rest = &mailto["mailto:".len()..];
        let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
        let addr = percent_encoding::percent_decode_str(addr).decode_utf8_lossy().to_string();
        let param = |k: &str| query.split('&').find_map(|kv| kv.split_once('=').filter(|(a, _)| a.eq_ignore_ascii_case(k)).map(|(_, v)| percent_encoding::percent_decode_str(&v.replace('+', " ")).decode_utf8_lossy().to_string()));
        let message = smtp::build(smtp::Outgoing {
            from_name: Some(profile.display_name.clone()),
            from_address: profile.email(),
            to: vec![addr],
            cc: vec![],
            subject: param("subject").unwrap_or_else(|| "unsubscribe".into()),
            body: param("body").unwrap_or_else(|| "unsubscribe".into()),
            html: None,
            in_reply_to: None,
            references: None,
            attachments: vec![],
            read_receipt: false,
            extra_headers: vec![],
        })?;
        smtp::send(&profile.smtp_host, profile.smtp_port, &creds.username, &creds.password, &message).await?;
        return Ok(UnsubscribeResult { done: true, open_url: None, how: "письмом на адрес отписки".into() });
    }
    Ok(UnsubscribeResult { done: false, open_url: u.https, how: "на сайте рассылки".into() })
}

/// «Отложить»: письма уходят в папку «Отложенные» и в срок возвращаются во «Входящие»
/// непрочитанными (срок — ключевое слово на сервере, видно на всех компьютерах).
#[tauri::command]
pub async fn snooze(state: State<'_, AppState>, folder: String, uids: Vec<u32>, until: String) -> Result<String> {
    let at = chrono::DateTime::parse_from_rfc3339(&until).map_err(|_| AppError::Other("Некорректное время".into()))?.with_timezone(&chrono::Utc);
    if at <= chrono::Utc::now() {
        return Err(AppError::Other("Выберите время в будущем".into()));
    }
    // Срок — с точностью до минуты, округляется вверх (не раньше выбранного времени).
    let at = if at.timestamp() % 60 == 0 { at } else { at + chrono::Duration::seconds(60 - at.timestamp() % 60) };
    let kw = format!("{}{}", imap::SNOOZE_PREFIX, at.format("%Y%m%d%H%M"));
    let target = with_conn!(once state, c => async {
        let target = imap::snoozed_folder(c, true).await?.expect("created");
        imap::clear_snooze(c, &folder, &uids).await?;
        imap::set_keyword(c, &folder, &uids, &kw, true).await?;
        if target != folder {
            imap::move_messages(c, &folder, &uids, &target).await?;
        }
        Ok(target)
    })?;
    if let Some(st) = local(&state).await {
        st.remove(&folder, &uids);
    }
    Ok(target)
}

/// Вернуть отложенные письма во «Входящие» сейчас.
#[tauri::command]
pub async fn unsnooze(state: State<'_, AppState>, folder: String, uids: Vec<u32>) -> Result<()> {
    with_conn!(once state, c => async {
        imap::clear_snooze(c, &folder, &uids).await?;
        imap::move_messages(c, &folder, &uids, "INBOX").await
    })?;
    if let Some(st) = local(&state).await {
        st.remove(&folder, &uids);
    }
    Ok(())
}

/// Срок наступил — письма возвращаются во «Входящие» непрочитанными.
async fn return_snoozed(state: &AppState) -> Result<usize> {
    with_conn!(once state, c => async {
        let Some(folder) = imap::snoozed_folder(c, false).await? else { return Ok(0) };
        let due = imap::snoozed_due(c, &folder, chrono::Utc::now()).await?;
        if due.is_empty() {
            return Ok(0);
        }
        imap::clear_snooze(c, &folder, &due).await?;
        let _: () = imap::set_flag_many(c, &folder, &due, FlagKind::Seen, false).await?;
        imap::move_messages(c, &folder, &due, "INBOX").await?;
        Ok(due.len())
    })
}

// ---------------------------------------------------------------- уведомления о прочтении

/// Ответ на запрос уведомления о прочтении: `send` — отправить, иначе только
/// отметить, что вопрос решён (больше не спрашивать). Метка $MDNSent — как у Thunderbird.
#[tauri::command]
pub async fn mdn_respond(state: State<'_, AppState>, folder: String, uid: u32, send: bool) -> Result<()> {
    if send {
        let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
        let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
        let raw = match local_raw(&state, &folder, uid).await {
            Some(r) => r,
            None => with_conn!(state, c => imap::raw_of(c, &folder, uid))?,
        };
        let msg = mail_parser::MessageParser::default().parse_headers(&raw).ok_or(AppError::NotFound)?;
        let to = msg
            .header_raw("Disposition-Notification-To")
            .and_then(|v| smtp::split_addresses(v).into_iter().next())
            .map(|a| a.rsplit('<').next().unwrap_or(&a).trim_end_matches('>').trim().to_string())
            .filter(|a| a.contains('@'))
            .ok_or_else(|| AppError::Other("Отправитель не запрашивал уведомление".into()))?;
        let id = msg.message_id().map(|i| format!("<{}>", i.trim_matches(['<', '>'])));
        let mdn = smtp::build_mdn(&profile.display_name, &profile.email(), &to, msg.subject().unwrap_or_default(), id.as_deref());
        smtp::send_raw(&profile.smtp_host, profile.smtp_port, &creds.username, &creds.password, &mdn).await?;
    }
    with_conn!(state, c => imap::set_keyword(c, &folder, &[uid], "$MDNSent", true))?;
    if let Some(st) = local(&state).await {
        st.patch(&folder, uid, |m| m.keywords.push("$MDNSent".into()));
    }
    Ok(())
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
        log::debug!("browse {source}: {l}");
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
    let st = local(&state).await;
    // Письмо есть на компьютере — открывается сразу, без сервера.
    if let Some(st) = &st {
        if let (Some(raw), Some(summary)) = (st.raw(&folder, uid), st.summary(&folder, uid)) {
            let detail = imap::detail_from_raw(summary, &raw);
            if let Ok(mut guard) = state.conn.try_lock() {
                if let Some(c) = guard.as_mut() {
                    c.prime_raw(&folder, uid, raw);
                }
            }
            return Ok(detail);
        }
    }
    with_conn!(state, c => async {
        let detail = imap::get_message(c, &folder, uid).await?;
        if let (Some(st), Some(raw)) = (&st, c.last_raw(&folder, uid)) {
            st.put_raw(&folder, uid, raw);
        }
        Ok(detail)
    })
}

#[tauri::command]
pub async fn set_flag(
    state: State<'_, AppState>,
    folder: String,
    uid: u32,
    flag: FlagKind,
    value: bool,
) -> Result<()> {
    with_conn!(state, c => imap::set_flag(c, &folder, uid, flag, value))?;
    if let Some(st) = local(&state).await {
        st.patch(&folder, uid, |m| match flag {
            FlagKind::Seen => m.unread = !value,
            FlagKind::Flagged => m.flagged = value,
            FlagKind::Important => m.important = value,
        });
    }
    Ok(())
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
    let (name, data) = attachment(&state, &folder, uid, index).await?;

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

/// Как открылся документ: в окне программы (Direct Editing) или в браузере.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditResult {
    in_app: bool,
    path: String,
}

fn editor_label(path: &str) -> String {
    format!("doc-{:x}", path.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)))
}

/// Открывает файл Nextcloud на редактирование: через Direct Editing — в окне
/// программы (вход не нужен), иначе — страница файла в браузере.
async fn edit_in_nextcloud(app: &AppHandle, nc: &nextcloud::Nc, path: &str, mime: &str) -> Result<EditResult> {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    // Тип от сервера бывает общим (application/zip, octet-stream) — пробуем и тип по расширению.
    let editors = nc.editors().await;
    let editor = editors.for_mime(mime).or_else(|| nextcloud::office_mime(&name).and_then(|m| editors.for_mime(m)));
    if let Some(editor) = editor {
        let url = nc.direct_open(path, &editor.id).await?;
        let url = url::Url::parse(&url).map_err(|e| AppError::Other(e.to_string()))?;
        host::open_external_window(app, &editor_label(path), &format!("{name} — {}", editor.name), url)?;
        return Ok(EditResult { in_app: true, path: path.to_string() });
    }
    let id = nc.file_id(path).await?;
    let url = nc.creds.base.join(&format!("index.php/f/{id}")).map_err(|e| AppError::Other(e.to_string()))?;
    host::open_url(app, url.as_str())?;
    Ok(EditResult { in_app: false, path: path.to_string() })
}

/// Редакторы документов на сервере Nextcloud (для меню «Создать» и диагностики).
#[tauri::command]
pub async fn nc_editors(state: State<'_, AppState>) -> Result<nextcloud::NcEditors> {
    Ok(nc(&state).await?.editors().await)
}

#[tauri::command]
pub async fn nc_edit(app: AppHandle, state: State<'_, AppState>, path: String, mime: String) -> Result<EditResult> {
    let nc = nc(&state).await?;
    edit_in_nextcloud(&app, &nc, &path, &mime).await
}

/// Новый документ/таблица/презентация в папке `dir` и сразу на редактирование.
#[tauri::command]
pub async fn nc_create_document(app: AppHandle, state: State<'_, AppState>, dir: String, name: String, kind: String) -> Result<EditResult> {
    let (ext, blank): (&str, &[u8]) = match kind.as_str() {
        "docx" => ("docx", include_bytes!("../templates/blank.docx")),
        "xlsx" => ("xlsx", include_bytes!("../templates/blank.xlsx")),
        "pptx" => ("pptx", include_bytes!("../templates/blank.pptx")),
        _ => return Err(AppError::Other("Неизвестный тип документа".into())),
    };
    let base = name.trim().trim_end_matches(&format!(".{ext}")).trim().to_string();
    if base.is_empty() {
        return Err(AppError::Other("Укажите название".into()));
    }
    let file = format!("{base}.{ext}");
    let nc = nc(&state).await?;
    let editors = nc.editors().await;
    if let Some(c) = editors.creators.iter().find(|c| c.extension == ext) {
        let path = format!("{}/{file}", dir.trim_end_matches('/'));
        let url = nc.direct_create(&path, &c.editor, &c.id).await?;
        let url = url::Url::parse(&url).map_err(|e| AppError::Other(e.to_string()))?;
        let editor = editors.editors.iter().find(|e| e.id == c.editor).map(|e| e.name.clone()).unwrap_or_default();
        host::open_external_window(&app, &editor_label(&path), &format!("{file} — {editor}"), url)?;
        return Ok(EditResult { in_app: true, path });
    }
    // Нет Direct Editing — пустой файл из шаблона программы, затем открыть.
    let path = nc_upload(&state).await?.upload_unique(&dir, &file, blank.to_vec()).await?;
    edit_in_nextcloud(&app, &nc, &path, nextcloud::office_mime(&file).unwrap_or("")).await
}

/// Вложение письма → Nextcloud («Почта») → на редактирование.
#[tauri::command]
pub async fn nc_edit_attachment(app: AppHandle, state: State<'_, AppState>, folder: String, uid: u32, index: usize, dir: String) -> Result<EditResult> {
    let (name, data) = attachment(&state, &folder, uid, index).await?;
    let path = nc_upload(&state).await?.upload_unique(&dir, &name, data).await?;
    let nc = nc(&state).await?;
    edit_in_nextcloud(&app, &nc, &path, nextcloud::office_mime(&name).unwrap_or("")).await
}

/// Политика администратора для интерфейса (галочка пароля, внешние картинки, срок ссылок).
#[tauri::command]
pub fn get_policy() -> crate::policy::UiPolicy {
    crate::policy::load().ui()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpStatus {
    enabled: bool,
    keyring_available: bool,
    recovery_left: usize,
}

async fn current_account(state: &AppState) -> Result<String> {
    if let Some(c) = state.creds.lock().await.as_ref() {
        return Ok(account_key(&c.username, &c.host));
    }
    if let Some(p) = state.pending.lock().await.as_ref() {
        return Ok(account_key(&p.creds.username, &p.creds.host));
    }
    Err(AppError::NotLoggedIn)
}

/// Проверка кода TOTP или резервного (резервный удаляется). Состояние — последний
/// принятый шаг и счётчик ошибок — хранится вместе с секретом и переживает перезапуск.
async fn check_totp(state: &AppState, account: &str, code: &str) -> Result<()> {
    // Проверки по очереди: состояние читается и записывается целиком.
    let _gate = state.totp_gate.lock().await;
    let (account, code) = (account.to_string(), code.to_string());
    tokio::task::spawn_blocking(move || totp::check_and_record(&account, &code)).await.map_err(|e| AppError::Other(e.to_string()))?
}

#[tauri::command]
pub async fn totp_status(state: State<'_, AppState>) -> Result<TotpStatus> {
    let account = current_account(&state).await?;
    let stored = totp::load(&account);
    Ok(TotpStatus {
        enabled: stored.is_some(),
        keyring_available: stored.is_some() || totp::keyring_available(&account),
        recovery_left: stored.map_or(0, |s| s.recovery.len()),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpSetup {
    secret: String,
    uri: String,
    qr_svg: String,
}

/// Начать настройку: новый секрет и QR-код для приложения-аутентификатора.
#[tauri::command]
pub async fn totp_setup_begin(state: State<'_, AppState>) -> Result<TotpSetup> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let account = account_key(&creds.username, &creds.host);
    let secret = totp::new_secret()?;
    let uri = totp::otpauth_uri(&secret, &creds.username);
    let qr_svg = totp::qr_svg(&uri)?;
    *state.totp_setup.lock().await = Some((account, secret.clone()));
    Ok(TotpSetup { secret, uri, qr_svg })
}

/// Подтвердить настройку кодом из приложения; возвращает резервные коды (показываются один раз).
#[tauri::command]
pub async fn totp_setup_confirm(state: State<'_, AppState>, code: String) -> Result<Vec<String>> {
    state.totp_throttle.lock().await.check(totp::unix_now())?;
    let (account, secret) = state.totp_setup.lock().await.clone().ok_or_else(|| AppError::Other("Сначала начните настройку".into()))?;
    let Some(step) = totp::verify(&secret, &code, totp::unix_now() / totp::STEP, None) else {
        state.totp_throttle.lock().await.fail(totp::unix_now());
        return Err(AppError::Other("Неверный код — проверьте время на телефоне и введите новый код".into()));
    };
    let (codes, hashes) = totp::new_recovery_codes()?;
    totp::store(&account, &totp::Stored { secret, recovery: hashes, last_step: Some(step), throttle: totp::Throttle::default() })?;
    state.totp_throttle.lock().await.ok();
    *state.totp_setup.lock().await = None;
    Ok(codes)
}

/// Отключить защиту — нужен текущий код (или резервный).
#[tauri::command]
pub async fn totp_disable(state: State<'_, AppState>, code: String) -> Result<()> {
    let account = current_account(&state).await?;
    check_totp(&state, &account, &code).await?;
    totp::remove(&account);
    Ok(())
}

/// Заблокировать программу: почта отключается, для продолжения нужен код.
#[tauri::command]
pub async fn lock_app(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    if totp::load(&account_key(&creds.username, &creds.host)).is_none() {
        return Err(AppError::Other("Сначала включите защиту кодом в настройках".into()));
    }
    let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    host::close_message_windows(&app);
    close_store(&state).await;
    state.session.fetch_add(1, Ordering::SeqCst);
    state.jobs.cancel_all();
    drop_others(&state).await;
    *state.pending.lock().await = Some(Pending { creds, profile, remember: None });
    *state.creds.lock().await = None;
    *state.nc.lock().await = None;
    *state.calendars.lock().await = None;
    *state.books.lock().await = None;
    state.picked.lock().await.clear();
    if let Some(Ok(session)) = state.sogo.lock().await.take() {
        session.logout().await;
    }
    if let Some(conn) = state.conn.lock().await.take() {
        conn.logout().await;
    }
    Ok(())
}

/// Разблокировать кодом TOTP (или резервным) и подключить почту.
#[tauri::command]
pub async fn totp_unlock(app: AppHandle, state: State<'_, AppState>, code: String) -> Result<LoginResult> {
    let account = {
        let p = state.pending.lock().await;
        let p = p.as_ref().ok_or_else(|| AppError::Other("Программа не заблокирована".into()))?;
        account_key(&p.creds.username, &p.creds.host)
    };
    check_totp(&state, &account, &code).await?;
    let p = state.pending.lock().await.take().ok_or(AppError::NotLoggedIn)?;
    let mut warning = None;
    if let Err(e) = start_session(&app, &state, p.creds.clone(), p.profile.clone()).await {
        if matches!(e, AppError::AuthFailed) && p.remember.is_none() {
            settings::forget_password(&p.profile);
        }
        return Err(e);
    }
    match p.remember {
        Some(true) => {
            if settings::store_password(&p.profile, &p.creds.password).is_err() {
                warning = Some("Не удалось сохранить пароль в системном хранилище — при следующем запуске его нужно будет ввести снова.".to_string());
            }
        }
        Some(false) => settings::forget_password(&p.profile),
        None => {}
    }
    Ok(LoginResult { account: account_of(&p.profile), warning })
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
    let policy = crate::policy::load();
    if policy.remote_images == crate::policy::RemoteImages::Off {
        return Err(AppError::Other("Загрузка картинок из интернета отключена администратором".into()));
    }
    let images = crate::html::fetch_remote(&urls, allow_local, &policy).await;
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
    let (name, data) = attachment(&state, &folder, uid, index).await?;
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
    // Адрес входа приходит от сервера: открываем только https на том же сервере
    // (не file:, smb: и не чужие обработчики протоколов).
    let parsed = url::Url::parse(&login_url).map_err(|_| AppError::Other("Nextcloud вернул некорректный адрес входа".into()))?;
    if parsed.scheme() != "https" || parsed.host_str() != base.host_str() || parsed.port_or_known_default() != base.port_or_known_default() {
        return Err(AppError::Other("Nextcloud вернул адрес входа на другом сервере — вход отменён".into()));
    }
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
    let q = search.trim().to_string();
    if q.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let nc = nc(&state).await?;
    let mut users = nc.search_users(&q).await?;
    // Nextcloud часто ограничивает подсказки «теми, с кем уже общались» или своей
    // группой, но находит сотрудника по точному e-mail. Поэтому дополняем поиск
    // корпоративной адресной книгой: имя → e-mail → пользователь Nextcloud.
    if users.len() < 5 && !q.contains('@') {
        if let Ok(contacts) = search_contacts(state.clone(), q.clone()).await {
            let emails: Vec<String> = contacts.iter().filter_map(|c| c.emails.first().cloned()).take(12).collect();
            let found = futures::future::join_all(emails.iter().map(|e| nc.search_users(e))).await;
            for (email, list) in emails.iter().zip(found) {
                let Ok(list) = list else { continue };
                // Только однозначное совпадение по адресу.
                let exact: Vec<_> = list.into_iter().filter(|u| u.sub.eq_ignore_ascii_case(email) || u.id.eq_ignore_ascii_case(email)).collect();
                if exact.len() == 1 && !users.iter().any(|u| u.id == exact[0].id) {
                    users.push(exact.into_iter().next().expect("one"));
                }
            }
        }
    }
    Ok(users)
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

/// Выбранные пользователем файлы (по id из `pick_attachments`): имя, путь и размер.
/// Содержимое не читается — оно уйдёт на сервер потоком.
async fn picked_files(state: &AppState, ids: &[String]) -> Result<Vec<(String, String, PathBuf)>> {
    let mut out = Vec::new();
    for id in ids {
        let path = state.picked.lock().await.get(id).cloned().ok_or_else(|| AppError::Other("Файл не найден, выберите его заново".into()))?;
        let size = tokio::fs::metadata(&path).await?.len();
        if size > MAX_TALK_FILE_MB * 1024 * 1024 {
            return Err(AppError::Other(format!("Файл больше {MAX_TALK_FILE_MB} МБ")));
        }
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        out.push((id.clone(), name, path));
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
    let files = picked_files(&state, &attachments).await?;
    let client = nc_upload(&state).await?;
    let mut paths = Vec::new();
    for (id, name, path) in files {
        paths.push(client.upload_unique_src(&dir, &name, nextcloud::UploadSource::File(path)).await?);
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
    nc_upload(&state).await?.files_download_to(&path, &target).await?;
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
    nc(&state).await?.link_for(&path, crate::policy::load().share_link_days()).await
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
    let days = crate::policy::load().share_link_days();
    let files = picked_files(&state, &attachments).await?;
    let client = nc_upload(&state).await?;
    let mut out = Vec::new();
    for (id, name, file) in files {
        let path = client.upload_unique_src(&dir, &name, nextcloud::UploadSource::File(file)).await?;
        let url = client.link_for(&path, days).await?;
        state.picked.lock().await.remove(&id);
        out.push(UploadedLink { name: path.rsplit('/').next().unwrap_or(&name).to_string(), path, url });
    }
    Ok(out)
}

/// Вложение письма → папка в Nextcloud. Возвращает путь.
#[tauri::command]
pub async fn nc_save_mail_attachment(state: State<'_, AppState>, folder: String, uid: u32, index: usize, dir: String) -> Result<String> {
    let (name, data) = attachment(&state, &folder, uid, index).await?;
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
    nextcloud::Nc::upload(creds, &uid)?.talk_send_file(&token, &name, nextcloud::UploadSource::File(path), &caption, reply_to).await?;
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
    with_conn!(state, c => imap::move_messages(c, &folder, &uids, &target))?;
    if let Some(st) = local(&state).await {
        st.remove(&folder, &uids);
    }
    Ok(())
}

/// Удалить письма: в корзину `trash` или (из корзины) насовсем. `true` — насовсем.
#[tauri::command]
pub async fn delete_messages(state: State<'_, AppState>, folder: String, uids: Vec<u32>, trash: Option<String>) -> Result<bool> {
    let permanent = with_conn!(state, c => imap::delete_messages(c, &folder, &uids, trash.as_deref()))?;
    if let Some(st) = local(&state).await {
        st.remove(&folder, &uids);
    }
    Ok(permanent)
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
    let dir = updater::private_download_dir()?;
    let file = updater::download(&asset, &dir, &state.update_progress).await?;
    let variant = info.variant.clone();
    let sha = asset.sha256.clone();
    let result = tokio::task::spawn_blocking(move || {
        // Ещё раз по файлу — прямо перед установкой.
        updater::verify_file(&file, &sha)?;
        updater::install(&variant, &file)
    })
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

// ---------------------------------------------------------------- импорт и резервная копия

/// Тестовая подмена системного диалога (WebDriver его не нажмёт). В релизе этого нет.
fn e2e_paths(var: &str) -> Option<Vec<PathBuf>> {
    #[cfg(debug_assertions)]
    {
        std::env::var(var).ok().map(|v| v.split(';').map(PathBuf::from).collect())
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = var;
        None
    }
}

async fn remember_source(state: &AppState, source: crate::transfer::Source) -> crate::transfer::SourceInfo {
    let id = format!("src{}", state.next_pick.fetch_add(1, Ordering::Relaxed));
    let info = source.info(id.clone());
    let mut map = state.import_sources.lock().await;
    if map.len() > 30 {
        map.clear();
    }
    map.insert(id, source);
    info
}

/// Профили Thunderbird на этом компьютере (учётные записи с локальными папками).
#[tauri::command]
pub async fn import_thunderbird(state: State<'_, AppState>) -> Result<Vec<crate::transfer::SourceInfo>> {
    let found = tokio::task::spawn_blocking(|| {
        crate::mbox::thunderbird_accounts()
            .into_iter()
            .filter_map(|a| crate::transfer::source_from_dir(std::path::Path::new(&a.path), Some(a.name), Some(a.account)).ok())
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))?;
    let mut out = Vec::new();
    for s in found {
        out.push(remember_source(&state, s).await);
    }
    Ok(out)
}

/// Выбрать файлы для импорта (.pst, .eml, mbox). `None` — отменено.
#[tauri::command]
pub async fn import_pick_files(app: AppHandle, state: State<'_, AppState>) -> Result<Option<crate::transfer::SourceInfo>> {
    let paths = match e2e_paths("RRCKI_E2E_PICK_IMPORT") {
        Some(p) => Some(p),
        None => host::pick_files(&app).await,
    };
    let Some(paths) = paths.filter(|p| !p.is_empty()) else { return Ok(None) };
    let source = tokio::task::spawn_blocking(move || crate::transfer::source_from_files(paths)).await.map_err(|e| AppError::Other(e.to_string()))??;
    Ok(Some(remember_source(&state, source).await))
}

/// Выбрать папку для импорта (профиль или учётная запись Thunderbird, папка с .eml).
#[tauri::command]
pub async fn import_pick_folder(app: AppHandle, state: State<'_, AppState>) -> Result<Option<crate::transfer::SourceInfo>> {
    let dir = match e2e_paths("RRCKI_E2E_PICK_IMPORT_DIR") {
        Some(p) => p.into_iter().next(),
        None => host::pick_folder(&app).await,
    };
    let Some(dir) = dir else { return Ok(None) };
    let source = tokio::task::spawn_blocking(move || crate::transfer::source_from_dir(&dir, None, None)).await.map_err(|e| AppError::Other(e.to_string()))??;
    Ok(Some(remember_source(&state, source).await))
}

/// Начать импорт выбранного источника в папку `root` (по умолчанию «Импорт»).
#[tauri::command]
pub async fn import_start(state: State<'_, AppState>, source: String, root: Option<String>) -> Result<u64> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let src = state.import_sources.lock().await.get(&source).cloned().ok_or_else(|| AppError::Other("Источник не найден — выберите файлы ещё раз".into()))?;
    if state.jobs.running("import") {
        return Err(AppError::Other("Импорт уже идёт — дождитесь окончания".into()));
    }
    let root = root.map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).unwrap_or_else(|| "Импорт".into());
    if root.chars().count() > 80 || root.chars().any(|c| c.is_control() || c == '*' || c == '%') {
        return Err(AppError::Other("Недопустимое имя папки для импорта".into()));
    }
    let job = crate::transfer::Job::new("import");
    let id = state.jobs.add(job.clone());
    tauri::async_runtime::spawn(crate::transfer::import_mail(creds, src, root, job));
    Ok(id)
}

#[tauri::command]
pub fn job_status(state: State<'_, AppState>, id: u64) -> Result<crate::transfer::JobStatus> {
    state.jobs.get(id).map(|j| j.status()).ok_or(AppError::NotFound)
}

#[tauri::command]
pub fn job_cancel(state: State<'_, AppState>, id: u64) {
    if let Some(j) = state.jobs.get(id) {
        j.cancel();
    }
}

/// Резервная копия папок (пустой список — все) в файлы mbox в выбранном каталоге.
#[tauri::command]
pub async fn backup_start(app: AppHandle, state: State<'_, AppState>, folders: Vec<String>) -> Result<Option<u64>> {
    let creds = state.creds.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    if state.jobs.running("backup") {
        return Err(AppError::Other("Резервная копия уже создаётся".into()));
    }
    let all = with_conn!(state, c => imap::list_folders(c))?;
    let chosen: Vec<crate::transfer::BackupFolder> = all
        .iter()
        .filter(|f| f.selectable && (folders.is_empty() || folders.contains(&f.id)))
        .map(|f| {
            // Путь из имён уровней: служебные папки — под привычными названиями.
            let mut display = Vec::new();
            let mut cur = Some(f);
            while let Some(x) = cur {
                let name = match (x.system, x.role) {
                    (true, imap::FolderRole::Inbox) => "Входящие".to_string(),
                    (true, imap::FolderRole::Sent) => "Отправленные".to_string(),
                    (true, imap::FolderRole::Drafts) => "Черновики".to_string(),
                    (true, imap::FolderRole::Trash) => "Корзина".to_string(),
                    (true, imap::FolderRole::Junk) => "Спам".to_string(),
                    (true, imap::FolderRole::Archive) => "Архив".to_string(),
                    _ => x.name.clone(),
                };
                display.insert(0, name);
                cur = x.parent.as_ref().and_then(|p| all.iter().find(|y| &y.id == p));
            }
            crate::transfer::BackupFolder { id: f.id.clone(), display }
        })
        .collect();
    if chosen.is_empty() {
        return Err(AppError::Other("Не выбраны папки для копии".into()));
    }
    let dir = match e2e_paths("RRCKI_E2E_PICK_DIR") {
        Some(p) => p.into_iter().next(),
        None => host::pick_folder(&app).await,
    };
    let Some(dir) = dir else { return Ok(None) };
    let job = crate::transfer::Job::new("backup");
    let id = state.jobs.add(job.clone());
    tauri::async_runtime::spawn(crate::transfer::backup(creds, chosen, dir, job));
    Ok(Some(id))
}

/// Импорт контактов из .vcf или .csv в адресную книгу `book_url`.
#[tauri::command]
pub async fn contacts_import(app: AppHandle, state: State<'_, AppState>, book_url: String) -> Result<Option<u64>> {
    let cols = known(&state, dav::Kind::Contacts).await?;
    let book = cols.iter().find(|c| c.url == book_url).cloned().ok_or_else(|| AppError::Other("Адресная книга не найдена".into()))?;
    require_writable(&book)?;
    if state.jobs.running("contacts") {
        return Err(AppError::Other("Контакты уже загружаются".into()));
    }
    let path = match e2e_paths("RRCKI_E2E_PICK_CONTACTS") {
        Some(p) => p.into_iter().next(),
        None => host::pick_files(&app).await.and_then(|p| p.into_iter().next()),
    };
    let Some(path) = path else { return Ok(None) };
    let list = tokio::task::spawn_blocking(move || crate::transfer::read_contacts(&path)).await.map_err(|e| AppError::Other(e.to_string()))??;
    if list.is_empty() {
        return Err(AppError::Other("В файле не найдено контактов с адресом почты или телефоном".into()));
    }
    let (client, _) = dav_client(&state).await?;
    let job = crate::transfer::Job::new("contacts");
    job.update(|s| s.total = list.len() as u64);
    let id = state.jobs.add(job.clone());
    tauri::async_runtime::spawn(async move {
        let result = import_contacts(client, book, list, &job).await;
        job.finish(result);
    });
    Ok(Some(id))
}

async fn import_contacts(mut client: dav::DavClient, book: dav::Collection, list: Vec<dav::ContactInput>, job: &crate::transfer::Job) -> Result<String> {
    job.update(|s| s.phase = "Проверка адресной книги".into());
    let existing = client.contacts(&book).await?;
    let mut emails: std::collections::HashSet<String> = existing.iter().flat_map(|c| c.emails.iter().map(|e| e.to_lowercase())).collect();
    let mut names: std::collections::HashSet<String> = existing.iter().map(|c| c.full_name.to_lowercase()).collect();
    job.update(|s| s.phase = format!("Загрузка в «{}»", book.name));
    for c in list {
        if job.cancelled() {
            break;
        }
        let dup = if c.emails.is_empty() { names.contains(&c.full_name.to_lowercase()) } else { c.emails.iter().any(|e| emails.contains(&e.to_lowercase())) };
        if dup {
            job.update(|s| {
                s.duplicates += 1;
                s.done += 1;
            });
            continue;
        }
        let uid = dav::new_uid();
        let result = match dav::build_vcard(&uid, &c) {
            Ok(card) => client.put(&collection_child(&book.url, &uid, "vcf")?, "text/vcard; charset=utf-8", &card, None).await,
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => {
                emails.extend(c.emails.iter().map(|e| e.to_lowercase()));
                names.insert(c.full_name.to_lowercase());
                job.update(|s| s.imported += 1);
            }
            Err(e) => {
                job.update(|s| s.skipped += 1);
                job.note(format!("{}: {e}", c.full_name));
            }
        }
        job.update(|s| s.done += 1);
    }
    let s = job.status();
    let mut text = format!("Добавлено контактов: {}", s.imported);
    if s.duplicates > 0 {
        text.push_str(&format!(", уже были в книге: {}", s.duplicates));
    }
    if s.skipped > 0 {
        text.push_str(&format!(", не добавлены: {}", s.skipped));
    }
    Ok(text)
}

// ---------------------------------------------------------------- несколько учётных записей

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountEntry {
    key: String,
    username: String,
    host: String,
    port: u16,
    display_name: String,
    active: bool,
    /// Пароль известен (сохранён или введён в этой сессии) — можно переключиться.
    can_switch: bool,
}

async fn active_key(state: &AppState) -> Option<String> {
    if let Some(c) = state.creds.lock().await.as_ref() {
        return Some(account_key(&c.username, &c.host));
    }
    state.pending.lock().await.as_ref().map(|p| account_key(&p.creds.username, &p.creds.host))
}

async fn password_of(state: &AppState, s: &Settings) -> Option<String> {
    let key = settings::account_key(s);
    if let Some(p) = state.session_passwords.lock().await.get(&key) {
        return Some(p.clone());
    }
    let s = s.clone();
    tokio::task::spawn_blocking(move || settings::load_password(&s)).await.ok().flatten()
}

fn creds_of(s: &Settings, password: String) -> Credentials {
    Credentials { host: s.host.clone(), port: s.port, username: s.username.clone(), password }
}

/// Подключённые учётные записи (активная — первой).
#[tauri::command]
pub async fn accounts_list(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<AccountEntry>> {
    let active = active_key(&state).await;
    let mut out = Vec::new();
    for s in settings::load_accounts(&app) {
        let key = settings::account_key(&s);
        let is_active = active.as_deref() == Some(key.as_str());
        let can_switch = is_active || password_of(&state, &s).await.is_some();
        out.push(AccountEntry { key, username: s.username.clone(), host: s.host.clone(), port: s.port, display_name: s.display_name.clone(), active: is_active, can_switch });
    }
    out.sort_by_key(|a| !a.active);
    Ok(out)
}

/// Добавить учётную запись (без переключения на неё): пароль проверяется входом на сервер.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn account_add(
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
) -> Result<AccountEntry> {
    if let Some(u) = dav_url.as_deref() {
        settings::validate_dav_url(u)?;
    }
    let host = host.trim().to_string();
    let username = username.trim().to_string();
    if host.is_empty() || username.is_empty() || password.is_empty() {
        return Err(AppError::Other("Укажите сервер, логин и пароль".into()));
    }
    let key = account_key(&username, &host);
    let mut list = settings::load_accounts(&app);
    if list.iter().any(|a| settings::account_key(a) == key) {
        return Err(AppError::Other("Эта учётная запись уже подключена".into()));
    }
    if list.len() >= 10 {
        return Err(AppError::Other("Можно подключить до 10 учётных записей".into()));
    }
    let s = Settings {
        host: host.clone(),
        port,
        username: username.clone(),
        smtp_host: smtp_host.map(|h| h.trim().to_string()).filter(|h| !h.is_empty()).unwrap_or_else(|| host.clone()),
        smtp_port: smtp_port.unwrap_or(settings::DEFAULT_SMTP_PORT),
        dav_url: dav_url.map(|u| u.trim().to_string()).unwrap_or_default(),
        display_name: String::new(),
        nextcloud_url: settings::DEFAULT_NEXTCLOUD_URL.to_string(),
        nextcloud_login: String::new(),
        nextcloud_uid: String::new(),
        signature: None,
        offline_days: None,
        categories: None,
    };
    let creds = creds_of(&s, password.clone());
    let conn = imap::connect(&creds).await?;
    let remember = remember && crate::policy::load().ui().allow_remember_password;
    if !(remember && settings::store_password(&s, &password).is_ok()) {
        state.session_passwords.lock().await.insert(key.clone(), password);
    }
    state.others.lock().await.insert(key.clone(), conn);
    list.push(s.clone());
    settings::save_accounts(&app, &list)?;
    Ok(AccountEntry { key, username, host, port, display_name: String::new(), active: false, can_switch: true })
}

/// Переключиться на другую учётную запись: сессия текущей закрывается, её пароль
/// остаётся в памяти, чтобы вернуться без ввода.
#[tauri::command]
pub async fn account_switch(app: AppHandle, state: State<'_, AppState>, key: String) -> Result<LoginResult> {
    let _one_at_a_time = state.switching.lock().await;
    let s = settings::load_accounts(&app)
        .into_iter()
        .find(|a| settings::account_key(a) == key)
        .ok_or_else(|| AppError::Other("Учётная запись не найдена".into()))?;
    if active_key(&state).await.as_deref() == Some(key.as_str()) && state.creds.lock().await.is_some() {
        return Ok(LoginResult { account: account_of(&s), warning: None });
    }
    let password = password_of(&state, &s).await.ok_or_else(|| AppError::Other("Пароль этой учётной записи не сохранён — удалите её и добавьте снова".into()))?;
    let creds = creds_of(&s, password);
    // Сначала проверяем вход: при ошибке остаёмся в текущей учётной записи.
    let check = match state.others.lock().await.remove(&key) {
        Some(c) => c,
        None => imap::connect(&creds).await?,
    };
    check.logout().await;

    host::close_message_windows(&app);
    state.outbox.flush().await;
    state.jobs.cancel_all();
    state.import_sources.lock().await.clear();
    if let Some(c) = state.creds.lock().await.clone() {
        state.session_passwords.lock().await.insert(account_key(&c.username, &c.host), c.password);
    }
    if let Some(p) = state.pending.lock().await.take() {
        state.session_passwords.lock().await.insert(account_key(&p.creds.username, &p.creds.host), p.creds.password);
    }
    *state.nc_flow.lock().await = None;
    *state.totp_setup.lock().await = None;
    if totp::load(&key).is_some() {
        host::close_message_windows(&app);
        close_store(&state).await;
        state.session.fetch_add(1, Ordering::SeqCst);
        *state.creds.lock().await = None;
        if let Some(c) = state.conn.lock().await.take() {
            c.logout().await;
        }
        settings::save(&app, &s)?;
        *state.pending.lock().await = Some(Pending { creds, profile: s.clone(), remember: None });
        return Ok(LoginResult { account: Account { locked: true, ..account_of(&s) }, warning: None });
    }
    start_session(&app, &state, creds, s.clone()).await?;
    let warning = settings::save(&app, &s).err().map(|e| format!("Не удалось сохранить настройки: {e}"));
    Ok(LoginResult { account: account_of(&s), warning })
}

/// Отключить неактивную учётную запись: забыть пароль и письма на компьютере.
#[tauri::command]
pub async fn account_remove(app: AppHandle, state: State<'_, AppState>, key: String) -> Result<()> {
    let _one_at_a_time = state.switching.lock().await;
    if active_key(&state).await.as_deref() == Some(key.as_str()) {
        return Err(AppError::Other("Это текущая учётная запись — для неё «Выйти»".into()));
    }
    let list = settings::load_accounts(&app);
    let Some(s) = list.iter().find(|a| settings::account_key(a) == key).cloned() else { return Ok(()) };
    settings::forget_password(&s);
    if !s.nextcloud_login.is_empty() {
        if let Ok(base) = nextcloud::normalize_base(&s.nextcloud_url) {
            settings::nc_forget_password(base.as_str(), &s.nextcloud_login);
        }
    }
    state.session_passwords.lock().await.remove(&key);
    state.others_last.lock().await.remove(&key);
    if let Some(c) = state.others.lock().await.remove(&key) {
        c.logout().await;
    }
    if let Ok(root) = host::data_dir(&app) {
        let k = key.clone();
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || store::wipe(&root, &k)).await {
            log::warn!("local store of removed account not wiped: {e}");
        }
    }
    settings::remove_account(&app, &key)
}

async fn drop_others(state: &AppState) {
    let conns: Vec<Connection> = state.others.lock().await.drain().map(|(_, c)| c).collect();
    for c in conns {
        c.logout().await;
    }
}

/// Операция на соединении с неактивной учётной записью (с одним переподключением).
async fn on_other<T, F>(state: &AppState, s: &Settings, op: F) -> Result<T>
where
    F: for<'c> Fn(&'c mut Connection) -> futures::future::BoxFuture<'c, Result<T>>,
{
    let key = settings::account_key(s);
    let password = password_of(state, s).await.ok_or_else(|| AppError::Other("пароль не сохранён".into()))?;
    let creds = creds_of(s, password);
    let mut pool = state.others.lock().await;
    for attempt in 0..2 {
        if !pool.contains_key(&key) {
            let c = tokio::time::timeout(Duration::from_secs(20), imap::connect(&creds)).await.map_err(|_| AppError::Timeout)??;
            pool.insert(key.clone(), c);
        }
        let conn = pool.get_mut(&key).expect("соединение есть");
        let result = match tokio::time::timeout(OP_TIMEOUT, op(conn)).await {
            Ok(r) => r,
            Err(_) => Err(AppError::Timeout),
        };
        match result {
            Err(e) if e.is_connection() && attempt == 0 => {
                pool.remove(&key);
            }
            other => return other,
        }
    }
    Err(AppError::ConnectionLost)
}

async fn other_accounts(app: &AppHandle, state: &AppState) -> Vec<Settings> {
    let active = active_key(state).await;
    settings::load_accounts(app).into_iter().filter(|a| Some(settings::account_key(a)) != active).collect()
}

fn account_label(s: &Settings) -> String {
    if s.display_name.trim().is_empty() { s.email() } else { format!("{} ({})", s.display_name.trim(), s.email()) }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUnread {
    key: String,
    unread: Option<u32>,
    error: Option<String>,
}

/// Непрочитанные во «Входящих» других учётных записей.
#[tauri::command]
pub async fn accounts_unread(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<AccountUnread>> {
    if state.creds.lock().await.is_none() {
        return Err(AppError::NotLoggedIn);
    }
    let mut out = Vec::new();
    for s in other_accounts(&app, &state).await {
        let key = settings::account_key(&s);
        match on_other(&state, &s, |c| Box::pin(imap::inbox_unseen(c))).await {
            Ok(n) => out.push(AccountUnread { key, unread: Some(n), error: None }),
            Err(e) => out.push(AccountUnread { key, unread: None, error: Some(e.to_string()) }),
        }
    }
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherNewMail {
    key: String,
    label: String,
    items: Vec<imap::MessageSummary>,
}

/// Новые непрочитанные письма других учётных записей (для уведомлений).
/// Первая проверка только запоминает последний UID.
#[tauri::command]
pub async fn accounts_poll(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<OtherNewMail>> {
    if state.creds.lock().await.is_none() {
        return Err(AppError::NotLoggedIn);
    }
    let mut out = Vec::new();
    for s in other_accounts(&app, &state).await {
        let key = settings::account_key(&s);
        let known = state.others_last.lock().await.get(&key).copied();
        let after = known.unwrap_or(0);
        let limit = if known.is_some() { 5 } else { 0 };
        match on_other(&state, &s, move |c| Box::pin(imap::new_unseen(c, "INBOX", after, limit))).await {
            Ok((items, last)) => {
                state.others_last.lock().await.insert(key.clone(), last.max(after));
                if !items.is_empty() {
                    out.push(OtherNewMail { key, label: account_label(&s), items });
                }
            }
            Err(e) => log::warn!("new mail check for {key}: {e}"),
        }
    }
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedItem {
    account: String,
    label: String,
    active: bool,
    #[serde(flatten)]
    message: imap::MessageSummary,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedInbox {
    items: Vec<UnifiedItem>,
    /// Учётные записи, которые сейчас не ответили.
    errors: Vec<String>,
}

/// Общие «Входящие»: последние письма всех учётных записей, новые сверху.
#[tauri::command]
pub async fn unified_inbox(app: AppHandle, state: State<'_, AppState>, limit: u32) -> Result<UnifiedInbox> {
    let limit = limit.clamp(1, 100);
    let profile = state.profile.lock().await.clone().ok_or(AppError::NotLoggedIn)?;
    let active = settings::account_key(&profile);
    let mut items = Vec::new();
    let mut errors = Vec::new();
    let page = with_conn!(state, c => imap::list_messages(c, "INBOX", None, limit))?;
    items.extend(page.items.into_iter().map(|m| UnifiedItem { account: active.clone(), label: account_label(&profile), active: true, message: m }));
    for s in other_accounts(&app, &state).await {
        let key = settings::account_key(&s);
        match on_other(&state, &s, move |c| Box::pin(imap::list_messages(c, "INBOX", None, limit))).await {
            Ok(page) => items.extend(page.items.into_iter().map(|m| UnifiedItem { account: key.clone(), label: account_label(&s), active: false, message: m })),
            Err(e) => errors.push(format!("{}: {e}", s.email())),
        }
    }
    let ts = |i: &UnifiedItem| i.message.date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map(|d| d.timestamp()).unwrap_or(0);
    items.sort_by_key(|i| std::cmp::Reverse(ts(i)));
    items.truncate(limit as usize * 2);
    Ok(UnifiedInbox { items, errors })
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
            crate::commands::totp_status,
            crate::commands::totp_setup_begin,
            crate::commands::totp_setup_confirm,
            crate::commands::totp_disable,
            crate::commands::lock_app,
            crate::commands::totp_unlock,
            crate::commands::cached_folders,
            crate::commands::cached_messages,
            crate::commands::search_mail,
            crate::commands::search_server,
            crate::commands::offline_status,
            crate::commands::set_offline_days,
            crate::commands::clear_offline,
            crate::commands::signature_get,
            crate::commands::signature_save,
            crate::commands::send_wait,
            crate::commands::send_cancel,
            crate::commands::send_now,
            crate::commands::scheduled_list,
            crate::commands::scheduled_send_now,
            crate::commands::scheduled_cancel,
            crate::commands::mdn_respond,
            crate::commands::desktop_prefs_get,
            crate::commands::desktop_prefs_set,
            crate::commands::set_unread,
            crate::commands::show_main_window,
            crate::commands::categories_get,
            crate::commands::discover_servers,
            crate::commands::attach_data,
            crate::commands::accounts_list,
            crate::commands::account_add,
            crate::commands::account_switch,
            crate::commands::account_remove,
            crate::commands::accounts_unread,
            crate::commands::accounts_poll,
            crate::commands::unified_inbox,
            crate::commands::import_thunderbird,
            crate::commands::import_pick_files,
            crate::commands::import_pick_folder,
            crate::commands::import_start,
            crate::commands::job_status,
            crate::commands::job_cancel,
            crate::commands::backup_start,
            crate::commands::contacts_import,
            crate::commands::mark_spam,
            crate::commands::unsubscribe,
            crate::commands::snooze,
            crate::commands::unsnooze,
            crate::commands::categories_save,
            crate::commands::set_keywords,
            crate::commands::set_due,
            crate::commands::due_list,
            crate::commands::templates_list,
            crate::commands::template_save,
            crate::commands::template_delete,
            crate::commands::get_policy,
            crate::commands::nc_editors,
            crate::commands::nc_edit,
            crate::commands::nc_create_document,
            crate::commands::nc_edit_attachment,
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
