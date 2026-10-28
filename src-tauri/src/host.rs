//! Всё, что зависит от версии Tauri: системные диалоги, открытие ссылок в
//! браузере, каталог настроек. Это реализация для Tauri 2; у сборки для старых
//! систем (Tauri 1) — своя, в `legacy/src/host.rs`, с теми же функциями.

use std::path::PathBuf;

use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, Result};

/// Сборка определяется во время работы (Windows / deb / rpm / AppImage).
pub const VARIANT: Option<&str> = None;

pub fn config_dir(app: &AppHandle) -> Result<PathBuf> {
    app.path()
        .app_config_dir()
        .map_err(|e| AppError::Other(format!("Каталог настроек недоступен: {e}")))
}

/// Выбор файлов (несколько). `None` — пользователь отменил.
pub async fn pick_files(app: &AppHandle) -> Option<Vec<PathBuf>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_files(move |paths| {
        let _ = tx.send(paths);
    });
    let paths = rx.await.ok().flatten()?;
    Some(paths.into_iter().filter_map(|p| p.into_path().ok()).collect())
}

/// Куда сохранить файл. `None` — пользователь отменил.
pub async fn save_file(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_file_name(name).save_file(move |path| {
        let _ = tx.send(path);
    });
    rx.await.ok().flatten()?.into_path().ok()
}

pub fn open_url(app: &AppHandle, url: &str) -> Result<()> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::Other(format!("Не удалось открыть браузер: {e}")))
}

/// Отдельное окно приложения (тот же интерфейс); `init` выполняется до загрузки страницы.
pub fn open_window(app: &AppHandle, label: &str, title: &str, init: &str) -> Result<()> {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(920.0, 820.0)
        .min_inner_size(480.0, 400.0)
        .initialization_script(init)
        .build()
        .map_err(|e| AppError::Other(format!("Не удалось открыть окно: {e}")))?;
    Ok(())
}

/// Закрывает окна писем (при выходе из учётной записи).
pub fn close_message_windows(app: &AppHandle) {
    for (label, w) in app.webview_windows() {
        if label.starts_with("msg-") {
            let _ = w.close();
        }
    }
}

/// E2E: в отладочной сборке уведомления можно писать в файл (в Xvfb нет службы уведомлений).
fn notify_test_log(title: &str, body: &str) -> bool {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("RRCKI_E2E_NOTIFY_LOG") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{title}\t{body}");
        }
        return true;
    }
    let _ = (title, body);
    false
}

/// Всплывающее уведомление ОС (Windows — центр уведомлений, Linux — служба уведомлений рабочего стола).
pub fn notify(app: &AppHandle, title: &str, body: &str) -> Result<()> {
    if notify_test_log(title, body) {
        return Ok(());
    }
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| AppError::Other(format!("Уведомление не показано: {e}")))
}

/// Окно с внешней страницей (редактор документов Nextcloud). Командам программы
/// эта страница недоступна: окна `doc-*` не входят в capabilities.
pub fn open_external_window(app: &AppHandle, label: &str, title: &str, url: url::Url) -> Result<()> {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::External(url))
        .title(title)
        .inner_size(1280.0, 860.0)
        .min_inner_size(640.0, 480.0)
        .build()
        .map_err(|e| AppError::Other(format!("Не удалось открыть окно редактора: {e}")))?;
    Ok(())
}
