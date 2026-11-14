//! Рабочий стол: окна, трей, уведомления о новой почте, масштаб, навигация окон и веб-профили.

use super::*;

/// Трей, настройки окна и запуск свёрнутым (автозапуск передаёт `--minimized`).
pub fn setup_desktop(app: &AppHandle) {
    use tauri::Manager;
    cleanup_web_profiles(app);
    if let Ok(dir) = host::data_dir(app) {
        crate::ews::set_data_dir(dir);
    }
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
    pub(crate) close_to_tray: bool,
    pub(crate) autostart: bool,
    /// Только для чтения: есть ли значок в трее.
    #[serde(default)]
    pub(crate) tray: bool,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMail {
    pub(crate) items: Vec<imap::MessageSummary>,
    pub(crate) last_uid: u32,
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

/// Адреса, по которым может ходить главное окно и окна писем: только сама программа
/// (`tauri://localhost`, в Windows `http(s)://tauri.localhost`), пустые и встроенные рамки
/// (`about:blank`, `about:srcdoc`) и `blob:` самой программы; в отладочной сборке — ещё
/// сервер разработки. Всё остальное (ссылка, обошедшая обработчики, перенаправление
/// встроенной страницы) не открывается в окне программы.
pub fn app_url_allowed(u: &url::Url) -> bool {
    match u.scheme() {
        "tauri" => u.host_str() == Some("localhost"),
        "http" | "https" => u.host_str() == Some("tauri.localhost") || (cfg!(debug_assertions) && u.host_str() == Some("localhost") && u.port() == Some(5173)),
        "about" => matches!(u.path(), "blank" | "srcdoc"),
        "blob" => url::Url::parse(u.path()).is_ok_and(|inner| inner.scheme() != "blob" && app_url_allowed(&inner)),
        _ => false,
    }
}

#[cfg(test)]
mod nav_tests {
    use super::app_url_allowed;
    fn ok(s: &str) -> bool { app_url_allowed(&url::Url::parse(s).unwrap()) }

    #[test]
    fn only_the_app_itself() {
        assert!(ok("tauri://localhost/index.html"));
        assert!(ok("http://tauri.localhost/"));
        assert!(ok("https://tauri.localhost/index.html#x"));
        assert!(ok("about:blank") && ok("about:srcdoc"));
        assert!(ok("blob:tauri://localhost/2f1c-uuid"));
        assert!(!ok("https://evil.example/login"));
        assert!(!ok("http://tauri.localhost.evil.example/"));
        assert!(!ok("tauri://evil/"));
        assert!(!ok("data:text/html,<script>1</script>"));
        assert!(!ok("javascript:alert(1)"));
        assert!(!ok("file:///etc/passwd"));
        assert!(!ok("blob:https://evil.example/uuid"));
        assert!(!ok("about:config"));
    }
}

/// Профиль веб-движка для страниц Nextcloud учётной записи (окна редактора и звонков):
/// `<данные программы>/web/<хэш учётной записи>`, только для владельца.
pub(crate) fn web_profile(app: &AppHandle, account: &str) -> Result<PathBuf> {
    let dir = host::data_dir(app)?.join("web").join(store::account_hash(account));
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Other(format!("Каталог профиля окна недоступен: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        if let Some(parent) = dir.parent() {
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    crate::winacl::restrict(&dir);
    Ok(dir)
}

/// Удалить профиль веб-движка учётной записи (куки входа в Nextcloud и т. п.). Окна,
/// которые им пользуются, к этому моменту уже закрыты.
pub(crate) fn forget_web_profile(app: &AppHandle, account: &str) {
    let Ok(root) = host::data_dir(app) else { return };
    let web = root.join("web");
    let dir = web.join(store::account_hash(account));
    if dir.exists() && std::fs::remove_dir_all(&dir).is_err() {
        // Процесс веб-движка ещё держит файлы (Windows) — удалим при следующем запуске.
        let _ = std::fs::write(web.join(format!("{}.forget", store::account_hash(account))), b"");
        log::warn!("web profile is busy, will be removed on next start");
    }
}

/// При запуске: удалить профили веб-движка, которые не удалось удалить при выходе.
pub fn cleanup_web_profiles(app: &AppHandle) {
    let Ok(root) = host::data_dir(app) else { return };
    let Ok(list) = std::fs::read_dir(root.join("web")) else { return };
    for e in list.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "forget") {
            let dir = p.with_extension("");
            if !dir.exists() || std::fs::remove_dir_all(&dir).is_ok() {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
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

/// Масштаб интерфейса окна, из которого вызвана команда (настройка «Масштаб», Ctrl +/−/0).
/// `false` — движок не поддерживает, интерфейс масштабируется средствами CSS.
#[tauri::command]
pub async fn set_zoom(app: AppHandle, window: tauri::Window, factor: f64) -> Result<bool> {
    if !(0.5..=3.0).contains(&factor) {
        return Err(AppError::Other("Масштаб вне допустимых пределов".into()));
    }
    Ok(host::set_zoom(&app, window.label(), factor))
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
    let title = if title.trim().is_empty() { "Письмо — Atom Workspace".to_string() } else { format!("{title} — Atom Workspace") };
    crate::host::open_window(&app, &label, &title, &format!("window.__RRCKI_MESSAGE = {params};"))
}
