mod error;
#[doc(hidden)]
pub mod imap;
mod settings;

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

async fn start_session(state: &AppState, creds: Credentials) -> Result<()> {
    let conn = imap::connect(&creds).await?;
    if let Some(old) = state.conn.lock().await.replace(conn) {
        old.logout().await;
    }
    *state.creds.lock().await = Some(creds);
    Ok(())
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

#[tauri::command]
async fn login(
    app: AppHandle,
    state: State<'_, AppState>,
    host: String,
    port: u16,
    username: String,
    password: String,
    remember: bool,
) -> Result<LoginResult> {
    let host = host.trim().to_string();
    let username = username.trim().to_string();
    if host.is_empty() || username.is_empty() || password.is_empty() {
        return Err(AppError::Other("Укажите сервер, логин и пароль".into()));
    }

    let creds = Credentials {
        host: host.clone(),
        port,
        username: username.clone(),
        password: password.clone(),
    };
    start_session(&state, creds).await?;

    let s = Settings { host, port, username };
    settings::save(&app, &s)?;

    let warning = if remember {
        settings::store_password(&s, &password).err().map(|e| {
            log::warn!("keyring unavailable: {e}");
            "Не удалось сохранить пароль в системном хранилище — при следующем запуске его нужно будет ввести снова.".to_string()
        })
    } else {
        settings::forget_password(&s);
        None
    };

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
    match start_session(&state, creds).await {
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
    offset: u32,
    limit: u32,
) -> Result<MessagePage> {
    with_conn!(state, c => imap::list_messages(c, &folder, offset, limit))
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running RRCKI Mail");
}
