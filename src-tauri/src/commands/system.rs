//! О программе, обновления, импорт и резервная копия.

use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub(crate) version: String,
    /// Сборка: win, win8, deb, astra17, rpm, appimage, unknown.
    pub(crate) variant: String,
    pub(crate) os: String,
}

#[tauri::command]
pub fn app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        variant: updater::detect_variant(host::VARIANT),
        os: std::env::consts::OS.to_string(),
    }
}

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
    pub(crate) done: u64,
    pub(crate) total: u64,
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

/// Тестовая подмена системного диалога (WebDriver его не нажмёт). В релизе этого нет.
pub(crate) fn e2e_paths(var: &str) -> Option<Vec<PathBuf>> {
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

pub(crate) async fn remember_source(state: &AppState, source: crate::transfer::Source) -> crate::transfer::SourceInfo {
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
    let ctx = state.ctx().await?;
    let creds = ctx.creds.clone();
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
    let ctx = state.ctx().await?;
    let creds = ctx.creds.clone();
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
    let ctx = state.ctx().await?;
    let cols = known(&ctx, dav::Kind::Contacts).await?;
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
    if exchange(&ctx) {
        let job = crate::transfer::Job::new("contacts");
        job.update(|s| s.total = list.len() as u64);
        let id = state.jobs.add(job.clone());
        let ctx = ctx.clone();
        tauri::async_runtime::spawn(async move {
            let result = import_contacts_ews(&ctx, list, &job).await;
            job.finish(result);
        });
        return Ok(Some(id));
    }
    let (client, _) = dav_client(&ctx).await?;
    let job = crate::transfer::Job::new("contacts");
    job.update(|s| s.total = list.len() as u64);
    let id = state.jobs.add(job.clone());
    tauri::async_runtime::spawn(async move {
        let result = import_contacts(client, book, list, &job).await;
        job.finish(result);
    });
    Ok(Some(id))
}

/// Загрузка контактов из файла в «Контакты» Exchange (без повторов по адресу и имени).
pub(crate) async fn import_contacts_ews(ctx: &AccountCtx, list: Vec<dav::ContactInput>, job: &crate::transfer::Job) -> Result<String> {
    job.update(|s| s.phase = "Проверка адресной книги".into());
    let existing = with_ctx!(ctx, c => async { ews_of(c)?.contacts().await })?;
    let mut emails: std::collections::HashSet<String> = existing.iter().flat_map(|c| c.emails.iter().map(|e| e.to_lowercase())).collect();
    let mut names: std::collections::HashSet<String> = existing.iter().map(|c| c.full_name.to_lowercase()).collect();
    job.update(|s| s.phase = "Загрузка в «Контакты»".into());
    for c in list {
        if job.cancelled() || ctx.is_closed() {
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
        match with_ctx!(ctx, x => async { ews_of(x)?.create_contact(&c).await }) {
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

pub(crate) async fn import_contacts(mut client: dav::DavClient, book: dav::Collection, list: Vec<dav::ContactInput>, job: &crate::transfer::Job) -> Result<String> {
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
