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

pub fn data_dir(app: &AppHandle) -> Result<PathBuf> {
    app.path_resolver()
        .app_local_data_dir()
        .ok_or_else(|| AppError::Other("Каталог данных недоступен".into()))
}

/// Проверка орфографии (WebKitGTK 4.0: словари hunspell через enchant).
pub fn enable_spellcheck(app: &AppHandle) {
    #[cfg(not(target_os = "linux"))]
    let _ = app;
    #[cfg(target_os = "linux")]
    if let Some(w) = app.get_window("main") {
        let _ = w.with_webview(|wv| {
            use webkit2gtk::{WebContextExt, WebViewExt};
            if let Some(ctx) = wv.inner().context() {
                ctx.set_spell_checking_enabled(true);
                ctx.set_spell_checking_languages(&["ru_RU", "en_US"]);
            }
        });
    }
}

/// Трея в сборке для Astra 1.7 нет: окно закрывается как обычно.
pub fn setup_tray(_app: &AppHandle) -> bool {
    false
}

pub fn set_unread(_app: &AppHandle, _count: u32) {}

pub fn hide_main(app: &AppHandle) {
    if let Some(w) = app.get_window("main") {
        let _ = w.hide();
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub async fn pick_files(_app: &AppHandle) -> Option<Vec<PathBuf>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    FileDialogBuilder::new().pick_files(move |paths| {
        let _ = tx.send(paths);
    });
    rx.await.ok().flatten()
}

pub async fn pick_folder(_app: &AppHandle) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    FileDialogBuilder::new().pick_folder(move |path| {
        let _ = tx.send(path);
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

/// Файл — в программе по умолчанию.
pub fn open_file(_app: &AppHandle, path: &std::path::Path) -> Result<()> {
    open::that_detached(path).map_err(|e| AppError::Other(format!("Не удалось открыть файл: {e}")))
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
        .disable_file_drop_handler()
        .build()
        .map_err(|e| AppError::Other(format!("Не удалось открыть окно: {e}")))?;
    Ok(())
}

/// Начать перетаскивание файлов из окна `label` в папку компьютера (в главном потоке).
pub fn start_drag_out(app: &AppHandle, label: &str, paths: Vec<PathBuf>) -> Result<()> {
    let w = app.get_window(label).ok_or_else(|| AppError::Other("Окно не найдено".into()))?;
    let (tx, rx) = std::sync::mpsc::channel();
    let win = w.clone();
    app.run_on_main_thread(move || {
        #[cfg(target_os = "linux")]
        let ok = win.gtk_window().map(|g| crate::dragout::start_gtk(&g, paths)).unwrap_or(false);
        #[cfg(not(target_os = "linux"))]
        let ok = drag::start_drag(&win, drag::DragItem::Files(paths), drag::Image::Raw(include_bytes!("../../src-tauri/icons/32x32.png").to_vec()), |_, _| {}, drag::Options::default()).is_ok();
        let _ = tx.send(ok);
    })
    .map_err(|e| AppError::Other(e.to_string()))?;
    match rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(true) => Ok(()),
        _ => Err(AppError::Other("Не удалось начать перетаскивание — сохраните вложение кнопкой «Сохранить»".into())),
    }
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
    // Политика администратора: что из письма можно показывать в уведомлении ОС.
    let (title, body) = crate::policy::load().redact_notification(title, body);
    let (title, body) = (title.as_str(), body.as_str());
    if notify_test_log(title, body) {
        return Ok(());
    }
    tauri::api::notification::Notification::new(&app.config().tauri.bundle.identifier)
        .title(title)
        .body(body)
        .show()
        .map_err(|e| AppError::Other(format!("Уведомление не показано: {e}")))
}

/// Окно с внешней страницей (редактор документов Nextcloud), без доступа к IPC.
/// Переходы на чужие сайты открываются в браузере, а не в этом окне.
pub fn open_external_window(app: &AppHandle, label: &str, title: &str, url: url::Url) -> Result<()> {
    if let Some(w) = app.get_window(label) {
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    let base = url.clone();
    tauri::WindowBuilder::new(app, label, tauri::WindowUrl::External(url))
        .title(title)
        .inner_size(1280.0, 860.0)
        .min_inner_size(640.0, 480.0)
        .on_navigation(move |u| {
            if crate::nextcloud::editor_url_allowed(&base, &u) {
                return true;
            }
            if matches!(u.scheme(), "http" | "https" | "mailto") {
                let _ = open::that_detached(u.as_str());
            }
            false
        })
        .build()
        .map_err(|e| AppError::Other(format!("Не удалось открыть окно редактора: {e}")))?;
    Ok(())
}
