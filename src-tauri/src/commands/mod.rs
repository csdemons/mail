//! Команды приложения (вызываются из интерфейса через `invoke`). Код общий для
//! обычной сборки (Tauri 2) и сборки для старых систем (Tauri 1, Astra 1.7):
//! всё, что отличается между версиями Tauri, — в модуле `host`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
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
use crate::offline::{self, Op, Target};

const OP_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Default)]
pub struct AppState {
    /// Активная учётная запись: всё, что принадлежит её сеансу (соединения,
    /// хранилище, Nextcloud, SOGo…). Команда берёт ссылку на контекст в момент
    /// запуска и работает только с ним: переключение на другую учётную запись
    /// посреди операции не переносит её в чужой ящик.
    active: Mutex<Option<Arc<AccountCtx>>>,
    next_pick: AtomicU64,
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
    /// Письма в «окне отмены» (каждое — со своим контекстом учётной записи).
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

/// Всё, что принадлежит сеансу одной учётной записи. Создаётся при входе
/// (переключении), закрывается при выходе, блокировке и переключении; операции,
/// начатые в этом сеансе, держат на него ссылку и не видят другую учётную запись.
pub struct AccountCtx {
    /// `логин@сервер` в нижнем регистре.
    pub key: String,
    creds: Credentials,
    /// Настройки (SMTP, DAV, имя отправителя…) — меняются из окна настроек.
    profile: Mutex<Settings>,
    /// Открытое IMAP-соединение; переиспользуется между командами.
    conn: Mutex<Option<Connection>>,
    /// Файлы, выбранные пользователем для вложения: id → путь. Пути не
    /// передаются во frontend и не принимаются от него.
    picked: Mutex<HashMap<String, PathBuf>>,
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
    /// Локальное хранилище писем и индекс поиска.
    store: Mutex<Option<Arc<store::Store>>>,
    /// Почему хранилище не открылось (показывается в настройках).
    store_error: Mutex<Option<String>>,
    closed: std::sync::atomic::AtomicBool,
    /// Есть связь с почтовым сервером. Без неё команды не пытаются подключиться
    /// (переподключается планировщик) — сразу работают с письмами на компьютере.
    online: std::sync::atomic::AtomicBool,
    /// С какого момента нет связи и почему.
    offline_info: std::sync::Mutex<Option<(String, String)>>,
    /// Очередь действий без сети (из хранилища — при первом обращении).
    queue: Mutex<Option<crate::offline::Queue>>,
    /// Очередь выполняется на сервере (одна попытка за раз).
    replaying: Mutex<()>,
    replay_active: std::sync::atomic::AtomicBool,
}

impl AccountCtx {
    fn new(creds: Credentials, profile: Settings) -> Self {
        AccountCtx {
            key: account_key(&creds.username, &creds.host),
            nc: Mutex::new(load_nc(&profile)),
            creds,
            profile: Mutex::new(profile),
            conn: Mutex::new(None),
            picked: Mutex::default(),
            calendars: Mutex::new(None),
            books: Mutex::new(None),
            nc_flow: Mutex::new(None),
            sogo: Mutex::new(None),
            store: Mutex::new(None),
            store_error: Mutex::new(None),
            closed: std::sync::atomic::AtomicBool::new(false),
            online: std::sync::atomic::AtomicBool::new(true),
            offline_info: std::sync::Mutex::new(None),
            queue: Mutex::new(None),
            replaying: Mutex::new(()),
            replay_active: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn online(&self) -> bool {
        self.online.load(Ordering::SeqCst)
    }

    /// Связи нет: дальше команды работают без сервера, пока планировщик не подключится.
    fn went_offline(&self, e: &AppError) {
        if self.online.swap(false, Ordering::SeqCst) {
            log::warn!("mail server unreachable, working offline: {e}");
        }
        let mut info = self.offline_info.lock().unwrap_or_else(|p| p.into_inner());
        let since = info.as_ref().map(|(s, _)| s.clone()).unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        *info = Some((since, e.to_string()));
    }

    fn came_online(&self) {
        if !self.online.swap(true, Ordering::SeqCst) {
            log::info!("mail server reachable again");
        }
        *self.offline_info.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    async fn profile(&self) -> Settings {
        self.profile.lock().await.clone()
    }

    /// Сеанс закрыт (выход, блокировка, переключение).
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Закрыть сеанс: хранилище, соединения IMAP и SOGo, Nextcloud, календари,
    /// выбранные файлы. Операции, ещё держащие контекст, могут завершиться
    /// (например, копия отправленного письма) — только в ящике этой учётной записи.
    async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Some(st) = self.store.lock().await.take() {
            let _ = tokio::task::spawn_blocking(move || st.close()).await;
        }
        *self.nc.lock().await = None;
        *self.nc_flow.lock().await = None;
        *self.calendars.lock().await = None;
        *self.books.lock().await = None;
        self.picked.lock().await.clear();
        if let Some(Ok(session)) = self.sogo.lock().await.take() {
            session.logout().await;
        }
        if let Some(conn) = self.conn.lock().await.take() {
            conn.logout().await;
        }
    }
}

impl AppState {
    /// Контекст активной учётной записи.
    async fn ctx(&self) -> Result<Arc<AccountCtx>> {
        self.active.lock().await.clone().ok_or(AppError::NotLoggedIn)
    }

    async fn ctx_opt(&self) -> Option<Arc<AccountCtx>> {
        self.active.lock().await.clone()
    }

    /// Закрытие главного окна не завершает программу (она в трее).
    pub fn keeps_running(&self) -> bool {
        self.tray.load(Ordering::SeqCst) && self.close_to_tray.load(Ordering::SeqCst)
    }
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

/// Выполняет операцию на соединении активной учётной записи (контекст берётся в
/// момент вызова). Если соединение оборвалось (сервер закрыл его по таймауту
/// простоя и т.п.) — переподключается и повторяет операцию один раз.
macro_rules! with_conn {
    ($state:expr, $c:ident => $body:expr) => {{
        let __ctx = $state.ctx().await?;
        with_ctx!(__ctx, $c => $body)
    }};
    // Неидемпотентная операция (APPEND в «Отправленные»): после таймаута не повторять —
    // сервер мог уже сохранить письмо, повтор дал бы копию.
    (once $state:expr, $c:ident => $body:expr) => {{
        let __ctx = $state.ctx().await?;
        with_ctx!(once __ctx, $c => $body)
    }};
}

/// То же на соединении заданного контекста учётной записи.
macro_rules! with_ctx {
    ($ctx:expr, $c:ident => $body:expr) => {
        with_ctx!(@run $ctx, $c, $body, false)
    };
    (once $ctx:expr, $c:ident => $body:expr) => {
        with_ctx!(@run $ctx, $c, $body, true)
    };
    (@run $ctx:expr, $c:ident, $body:expr, $no_timeout_retry:expr) => {{
        let mut guard = $ctx.conn.lock().await;
        let mut attempt = 0;
        loop {
            if guard.is_none() {
                // Без связи не ждём таймаута на каждом действии — переподключается планировщик.
                if !$ctx.online() {
                    break Err(AppError::ConnectionLost);
                }
                match imap::connect(&$ctx.creds).await {
                    Ok(c) => *guard = Some(c),
                    Err(e) => {
                        if e.is_offline() {
                            $ctx.went_offline(&e);
                        }
                        break Err(e);
                    }
                }
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

/// Начать сеанс учётной записи: прежний контекст закрывается, создаётся новый.
/// `allow_offline`: пароль уже проверен раньше (сохранён, блокировка) — если
/// сервер недоступен, а письма есть на компьютере, сеанс открывается без сети.
/// Возвращает `true`, если есть связь.
async fn start_session(app: &AppHandle, state: &AppState, creds: Credentials, profile: Settings, allow_offline: bool) -> Result<bool> {
    let conn = match imap::connect(&creds).await {
        Ok(c) => Some(c),
        Err(e) if allow_offline && e.is_offline() && offline_possible(app, &account_key(&creds.username, &creds.host)) => {
            log::warn!("mail server unreachable at start, opening offline: {e}");
            None
        }
        Err(e) => return Err(e),
    };
    state.session.fetch_add(1, Ordering::SeqCst);
    let ctx = Arc::new(AccountCtx::new(creds, profile));
    let online = conn.is_some();
    match conn {
        Some(c) => *ctx.conn.lock().await = Some(c),
        None => ctx.went_offline(&AppError::Unreachable(ctx.creds.host.clone(), "нет связи".into())),
    }
    open_store(app, &ctx).await;
    if let Some(old) = state.active.lock().await.replace(ctx.clone()) {
        old.close().await;
    }
    tauri::async_runtime::spawn(run_scheduler(app.clone(), ctx));
    Ok(online)
}

/// Можно ли открыть сеанс без сети: хранить почту на компьютере разрешено, и
/// хранилище учётной записи уже есть (список папок сохранён).
fn offline_possible(app: &AppHandle, key: &str) -> bool {
    crate::policy::load().local_store() && host::data_dir(app).is_ok_and(|root| store::account_dir(&root, key).join("folders.bin").exists())
}

/// Закрыть сеанс активной учётной записи (контекст убирается из состояния).
async fn end_session(state: &AppState) {
    state.session.fetch_add(1, Ordering::SeqCst);
    state.jobs.cancel_all();
    state.import_sources.lock().await.clear();
    let old = state.active.lock().await.take();
    if let Some(old) = old {
        old.close().await;
    }
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
        offline: false,
        email: s.email(),
        exchange: s.is_exchange(),
    }
}

/// Учётная запись Exchange: календарь и контакты — через EWS на её соединении.
fn exchange(ctx: &AccountCtx) -> bool {
    ctx.creds.ews.is_some()
}

fn ews_of(c: &mut Connection) -> Result<&mut crate::ews::EwsSession> {
    c.ews().ok_or_else(|| AppError::Other("Не учётная запись Exchange".into()))
}

async fn current_account(state: &AppState) -> Result<String> {
    if let Some(c) = state.ctx_opt().await {
        return Ok(c.key.clone());
    }
    if let Some(p) = state.pending.lock().await.as_ref() {
        return Ok(account_key(&p.creds.username, &p.creds.host));
    }
    Err(AppError::NotLoggedIn)
}

async fn nc_status_of(ctx: &AccountCtx) -> NcStatus {
    let profile = ctx.profile().await;
    let nc = ctx.nc.lock().await.clone();
    NcStatus {
        url: if profile.nextcloud_url.is_empty() { settings::DEFAULT_NEXTCLOUD_URL.into() } else { profile.nextcloud_url.clone() },
        connected: nc.is_some(),
        login: nc.as_ref().map(|(c, _)| c.login.clone()).unwrap_or_default(),
        uid: nc.map(|(_, u)| u).unwrap_or_default(),
    }
}

async fn nc(ctx: &AccountCtx) -> Result<nextcloud::Nc> {
    let (creds, uid) = ctx
        .nc
        .lock()
        .await
        .clone()
        .ok_or_else(|| AppError::Other("Nextcloud не подключён — подключите его в Настройках".into()))?;
    nextcloud::Nc::new(creds, &uid)
}

// Команды по областям; здесь — общее: состояние, контекст учётной записи, сеанс, макросы.
pub(crate) mod ui;
pub(crate) mod session;
pub(crate) mod offline_mode;
pub(crate) mod mail;
pub(crate) mod compose;
pub(crate) mod mail_rules;
pub(crate) mod counterparties;
pub(crate) mod calendar;
pub(crate) mod cloud;
pub(crate) mod talk;
pub(crate) mod deck;
pub(crate) mod files;
pub(crate) mod system;

// Всё из модулей видно как crate::commands::… (общие помощники, типы, команды).
#[allow(unused_imports)]
pub(crate) use self::{ui::*, session::*, offline_mode::*, mail::*, compose::*, mail_rules::*, counterparties::*, calendar::*, cloud::*, talk::*, deck::*, files::*, system::*};

/// Обработчик `invoke` — один на обе сборки; список команд — `src-tauri/commands.list`
/// (build.rs собирает из него `$OUT_DIR/app_commands.rs` и проверяет, что он совпадает с
/// функциями `#[tauri::command]`).
macro_rules! app_commands {
    () => {
        include!(concat!(env!("OUT_DIR"), "/app_commands.rs"))
    };
}

pub(crate) use app_commands;
