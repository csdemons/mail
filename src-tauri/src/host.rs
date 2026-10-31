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

/// Каталог данных программы (локальное хранилище писем) — не перемещаемый профиль.
pub fn data_dir(app: &AppHandle) -> Result<PathBuf> {
    app.path()
        .app_local_data_dir()
        .map_err(|e| AppError::Other(format!("Каталог данных недоступен: {e}")))
}

/// Проверка орфографии в полях ввода (WebKitGTK: словари hunspell через enchant;
/// в Windows WebView2 проверка включена по умолчанию).
pub fn enable_spellcheck(app: &AppHandle) {
    #[cfg(target_os = "linux")]
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.with_webview(|wv| {
            use webkit2gtk::{WebContextExt, WebViewExt};
            if let Some(ctx) = wv.inner().context() {
                ctx.set_spell_checking_enabled(true);
                ctx.set_spell_checking_languages(&["ru_RU", "en_US"]);
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = app;
}

// ---------------------------------------------------------------- трей и значок

const TRAY_ID: &str = "main";

/// В Linux значок в трее работает через libayatana-appindicator (или libappindicator),
/// которая подгружается при создании значка; без неё создание завершило бы программу.
#[cfg(target_os = "linux")]
fn tray_library_present() -> bool {
    ["libayatana-appindicator3.so.1", "libappindicator3.so.1"].iter().any(|name| {
        let c = std::ffi::CString::new(*name).expect("no NUL");
        // SAFETY: dlopen с корректной строкой; дескриптор сразу закрывается.
        unsafe {
            let h = libc::dlopen(c.as_ptr(), libc::RTLD_LAZY);
            if h.is_null() {
                false
            } else {
                libc::dlclose(h);
                true
            }
        }
    })
}

fn icon_with_badge(count: u32) -> Option<tauri::image::Image<'static>> {
    let base = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png")).ok()?;
    let (w, h) = (base.width(), base.height());
    let mut rgba = base.rgba().to_vec();
    if count > 0 && w == h {
        crate::desktop::draw_badge(&mut rgba, w, count, false);
    }
    Some(tauri::image::Image::new_owned(rgba, w, h))
}

pub fn hide_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Значок в трее: щелчок — открыть окно, меню — «Открыть» и «Выход».
/// Возвращает false, если трей недоступен (тогда окно закрывается как обычно).
pub fn setup_tray(app: &AppHandle) -> bool {
    #[cfg(target_os = "linux")]
    if !tray_library_present() {
        log::info!("tray: appindicator library not found, tray disabled");
        return false;
    }
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    let build = || -> tauri::Result<()> {
        let open = MenuItem::with_id(app, "open", "Открыть Atom Mail", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open, &PredefinedMenuItem::separator(app)?, &quit])?;
        let mut b = TrayIconBuilder::with_id(TRAY_ID)
            .tooltip("Atom Mail")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, e| match e.id().as_ref() {
                "open" => show_main(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, e| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    show_main(tray.app_handle());
                }
            });
        if let Some(icon) = icon_with_badge(0) {
            b = b.icon(icon);
        }
        b.build(app)?;
        Ok(())
    };
    match build() {
        Ok(()) => true,
        Err(e) => {
            log::warn!("tray unavailable: {e}");
            false
        }
    }
}

/// Число непрочитанных: значок в трее, подсказка, значок на панели задач.
pub fn set_unread(app: &AppHandle, count: u32) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let tip = if count > 0 { format!("Atom Mail — непрочитанных: {count}") } else { "Atom Mail".to_string() };
        let _ = tray.set_tooltip(Some(tip));
        if let Some(icon) = icon_with_badge(count) {
            let _ = tray.set_icon(Some(icon));
        }
    }
    if let Some(w) = app.get_webview_window("main") {
        #[cfg(windows)]
        {
            let overlay = (count > 0).then(|| {
                let mut rgba = vec![0u8; 16 * 16 * 4];
                crate::desktop::draw_badge(&mut rgba, 16, count, true);
                tauri::image::Image::new_owned(rgba, 16, 16)
            });
            let _ = w.set_overlay_icon(overlay);
        }
        #[cfg(not(windows))]
        let _ = w.set_badge_count((count > 0).then_some(i64::from(count)));
    }
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

/// Выбор папки. `None` — пользователь отменил.
pub async fn pick_folder(app: &AppHandle) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = tx.send(path);
    });
    rx.await.ok().flatten()?.into_path().ok()
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
        .disable_drag_drop_handler()
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
    // Политика администратора: что из письма можно показывать в уведомлении ОС.
    let (title, body) = crate::policy::load().redact_notification(title, body);
    let (title, body) = (title.as_str(), body.as_str());
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
/// эта страница недоступна: окна `doc-*` не входят в capabilities. Переходы на
/// чужие сайты не выполняются в этом окне (у него нет адресной строки — удобная
/// сцена для поддельной формы входа): такие ссылки открываются в браузере.
pub fn open_external_window(app: &AppHandle, label: &str, title: &str, url: url::Url) -> Result<()> {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    let base = url.clone();
    let (nav_app, new_app) = (app.clone(), app.clone());
    let nav_base = base.clone();
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::External(url))
        .title(title)
        .inner_size(1280.0, 860.0)
        .min_inner_size(640.0, 480.0)
        .on_navigation(move |u| {
            if crate::nextcloud::editor_url_allowed(&nav_base, u) {
                return true;
            }
            if matches!(u.scheme(), "http" | "https" | "mailto") {
                let _ = open_url(&nav_app, u.as_str());
            }
            false
        })
        .on_new_window(move |u, _| {
            if matches!(u.scheme(), "http" | "https" | "mailto") {
                let _ = open_url(&new_app, u.as_str());
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .build()
        .map_err(|e| AppError::Other(format!("Не удалось открыть окно редактора: {e}")))?;
    Ok(())
}

