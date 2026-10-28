//! Реализация `host` для Tauri 1 (см. `src-tauri/src/host.rs`).

use std::path::PathBuf;

use tauri::api::dialog::FileDialogBuilder;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, Result};

/// Эта сборка — пакет для Astra Linux SE 1.7.
pub const VARIANT: Option<&str> = Some("astra17");

pub fn config_dir(app: &AppHandle) -> Result<PathBuf> {
    app.path_resolver()
        .app_config_dir()
        .ok_or_else(|| AppError::Other("Каталог настроек недоступен".into()))
}

pub async fn pick_files(_app: &AppHandle) -> Option<Vec<PathBuf>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    FileDialogBuilder::new().pick_files(move |paths| {
        let _ = tx.send(paths);
    });
    rx.await.ok().flatten()
}

pub async fn save_file(_app: &AppHandle, name: &str) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    FileDialogBuilder::new().set_file_name(name).save_file(move |path| {
        let _ = tx.send(path);
    });
    rx.await.ok().flatten()
}

pub fn open_url(_app: &AppHandle, url: &str) -> Result<()> {
    open::that_detached(url).map_err(|e| AppError::Other(format!("Не удалось открыть браузер: {e}")))
}

pub fn open_window(app: &AppHandle, label: &str, title: &str, init: &str) -> Result<()> {
    if let Some(w) = app.get_window(label) {
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    tauri::WindowBuilder::new(app, label, tauri::WindowUrl::App("index.html".into()))
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
    for (label, w) in app.windows() {
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

/// Всплывающее уведомление ОС.
pub fn notify(app: &AppHandle, title: &str, body: &str) -> Result<()> {
    if notify_test_log(title, body) {
        return Ok(());
    }
    tauri::api::notification::Notification::new(&app.config().tauri.bundle.identifier)
        .title(title)
        .body(body)
        .show()
        .map_err(|e| AppError::Other(format!("Уведомление не показано: {e}")))
}
