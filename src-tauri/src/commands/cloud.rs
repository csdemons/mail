//! Nextcloud: подключение, поиск, аватары, профиль и занятость сотрудника, редакторы документов, предпросмотр ссылок.

use super::*;

/// Как открылся документ: в окне программы (Direct Editing) или в браузере.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditResult {
    pub(crate) in_app: bool,
    pub(crate) path: String,
}

pub(crate) fn editor_label(path: &str) -> String {
    format!("doc-{:x}", path.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)))
}

/// Открывает файл Nextcloud на редактирование: через Direct Editing — в окне
/// программы (вход не нужен), иначе — страница файла в браузере.
pub(crate) async fn edit_in_nextcloud(app: &AppHandle, account: &str, nc: &nextcloud::Nc, path: &str, mime: &str) -> Result<EditResult> {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    // Тип от сервера бывает общим (application/zip, octet-stream) — пробуем и тип по расширению.
    let editors = nc.editors().await;
    let editor = editors.for_mime(mime).or_else(|| nextcloud::office_mime(&name).and_then(|m| editors.for_mime(m)));
    if let Some(editor) = editor {
        let url = nc.direct_open(path, &editor.id).await?;
        let url = url::Url::parse(&url).map_err(|e| AppError::Other(e.to_string()))?;
        host::open_external_window(app, &editor_label(path), &format!("{name} — {}", editor.name), url, &web_profile(app, account)?)?;
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
    let ctx = state.ctx().await?;
    Ok(nc(&ctx).await?.editors().await)
}

#[tauri::command]
pub async fn nc_edit(app: AppHandle, state: State<'_, AppState>, path: String, mime: String) -> Result<EditResult> {
    let ctx = state.ctx().await?;
    let nc = nc(&ctx).await?;
    edit_in_nextcloud(&app, &ctx.key, &nc, &path, &mime).await
}

/// Новый документ/таблица/презентация в папке `dir` и сразу на редактирование.
#[tauri::command]
pub async fn nc_create_document(app: AppHandle, state: State<'_, AppState>, dir: String, name: String, kind: String) -> Result<EditResult> {
    let ctx = state.ctx().await?;
    let (ext, blank): (&str, &[u8]) = match kind.as_str() {
        "docx" => ("docx", include_bytes!("../../templates/blank.docx")),
        "xlsx" => ("xlsx", include_bytes!("../../templates/blank.xlsx")),
        "pptx" => ("pptx", include_bytes!("../../templates/blank.pptx")),
        _ => return Err(AppError::Other("Неизвестный тип документа".into())),
    };
    let base = name.trim().trim_end_matches(&format!(".{ext}")).trim().to_string();
    if base.is_empty() {
        return Err(AppError::Other("Укажите название".into()));
    }
    let file = format!("{base}.{ext}");
    let nc = nc(&ctx).await?;
    let editors = nc.editors().await;
    if let Some(c) = editors.creators.iter().find(|c| c.extension == ext) {
        let path = format!("{}/{file}", dir.trim_end_matches('/'));
        let url = nc.direct_create(&path, &c.editor, &c.id).await?;
        let url = url::Url::parse(&url).map_err(|e| AppError::Other(e.to_string()))?;
        let editor = editors.editors.iter().find(|e| e.id == c.editor).map(|e| e.name.clone()).unwrap_or_default();
        host::open_external_window(&app, &editor_label(&path), &format!("{file} — {editor}"), url, &web_profile(&app, &ctx.key)?)?;
        return Ok(EditResult { in_app: true, path });
    }
    // Нет Direct Editing — пустой файл из шаблона программы, затем открыть.
    let path = nc_upload(&ctx).await?.upload_unique(&dir, &file, blank.to_vec()).await?;
    edit_in_nextcloud(&app, &ctx.key, &nc, &path, nextcloud::office_mime(&file).unwrap_or("")).await
}

/// Вложение письма → Nextcloud («Почта») → на редактирование.
#[tauri::command]
pub async fn nc_edit_attachment(app: AppHandle, state: State<'_, AppState>, folder: String, uid: u32, index: usize, dir: String) -> Result<EditResult> {
    let ctx = state.ctx().await?;
    let (name, data) = attachment(&ctx, &folder, uid, index).await?;
    let path = nc_upload(&ctx).await?.upload_unique(&dir, &name, data).await?;
    let nc = nc(&ctx).await?;
    edit_in_nextcloud(&app, &ctx.key, &nc, &path, nextcloud::office_mime(&name).unwrap_or("")).await
}

/// Аватар Nextcloud: `kind` = "user" (ID пользователя) или "room" (токен беседы Talk).
#[tauri::command]
pub async fn nc_avatar(state: State<'_, AppState>, kind: String, id: String) -> Result<Option<String>> {
    let ctx = state.ctx().await?;
    let nc = nc(&ctx).await?;
    match kind.as_str() {
        "user" => nc.user_avatar(&id, 64).await,
        "room" => nc.room_avatar(&id).await,
        _ => Ok(None),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcStatus {
    pub(crate) url: String,
    pub(crate) connected: bool,
    pub(crate) login: String,
    /// ID пользователя Nextcloud («Мои карточки» в Deck, свои сообщения в Talk).
    pub(crate) uid: String,
}

/// Сохраняет подключение: проверяет пароль приложения и узнаёт ID пользователя.
pub(crate) async fn nc_save(app: &AppHandle, ctx: &AccountCtx, creds: nextcloud::NcCreds) -> Result<NcStatus> {
    let probe = nextcloud::Nc::new(creds.clone(), &creds.login)?;
    let (uid, _name) = probe.user_info().await?;
    let mut guard = ctx.profile.lock().await;
    let mut p = guard.clone();
    p.nextcloud_url = creds.base.to_string();
    p.nextcloud_login = creds.login.clone();
    p.nextcloud_uid = uid.clone();
    settings::save(app, &p)?;
    *guard = p;
    drop(guard);
    if let Err(e) = settings::nc_store_password(creds.base.as_str(), &creds.login, &creds.app_password) {
        log::warn!("nextcloud keyring: {e}");
    }
    *ctx.nc.lock().await = Some((creds, uid));
    Ok(nc_status_of(ctx).await)
}

#[tauri::command]
pub async fn nc_status(state: State<'_, AppState>) -> Result<NcStatus> {
    let ctx = state.ctx().await?;
    Ok(nc_status_of(&ctx).await)
}

/// Начинает вход: открывает страницу входа Nextcloud в браузере и возвращает её
/// адрес (на случай, если браузер не открылся).
#[tauri::command]
pub async fn nc_connect_start(app: AppHandle, state: State<'_, AppState>, url: String) -> Result<String> {
    let ctx = state.ctx().await?;
    let base = nextcloud::normalize_base(&url)?;
    let flow = nextcloud::login_flow_start(&base).await?;
    let login_url = flow.login_url.clone();
    // Адрес входа приходит от сервера: открываем только https на том же сервере
    // (не file:, smb: и не чужие обработчики протоколов).
    let parsed = url::Url::parse(&login_url).map_err(|_| AppError::Other("Nextcloud вернул некорректный адрес входа".into()))?;
    if parsed.scheme() != "https" || parsed.host_str() != base.host_str() || parsed.port_or_known_default() != base.port_or_known_default() {
        return Err(AppError::Other("Nextcloud вернул адрес входа на другом сервере — вход отменён".into()));
    }
    *ctx.nc_flow.lock().await = Some((base, flow));
    if let Err(e) = host::open_url(&app, &login_url) {
        log::warn!("browser not opened: {e}");
    }
    Ok(login_url)
}

/// `None` — вход в браузере ещё не подтверждён.
#[tauri::command]
pub async fn nc_connect_poll(app: AppHandle, state: State<'_, AppState>) -> Result<Option<NcStatus>> {
    let ctx = state.ctx().await?;
    let Some((base, flow)) = ctx.nc_flow.lock().await.clone() else {
        return Err(AppError::Other("Вход в Nextcloud не начат".into()));
    };
    match nextcloud::login_flow_poll(&base, &flow).await? {
        None => Ok(None),
        Some(creds) => {
            *ctx.nc_flow.lock().await = None;
            Ok(Some(nc_save(&app, &ctx, creds).await?))
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
    let ctx = state.ctx().await?;
    nc_save(&app, &ctx, creds).await
}

#[tauri::command]
pub async fn nc_disconnect(app: AppHandle, state: State<'_, AppState>) -> Result<NcStatus> {
    let ctx = state.ctx().await?;
    // Окна редактора и звонков работают с этим Nextcloud — закрыть и забыть их вход.
    host::close_account_windows(&app);
    forget_web_profile(&app, &ctx.key);
    *ctx.nc_flow.lock().await = None;
    if let Some((creds, uid)) = ctx.nc.lock().await.take() {
        if let Ok(client) = nextcloud::Nc::new(creds.clone(), &uid) {
            client.revoke().await;
        }
        settings::nc_forget_password(creds.base.as_str(), &creds.login);
    }
    {
        let mut p = ctx.profile.lock().await;
        p.nextcloud_login.clear();
        p.nextcloud_uid.clear();
        let _ = settings::save(&app, &p);
    }
    Ok(nc_status_of(&ctx).await)
}

/// Открывает страницу Nextcloud (звонок, доска) в браузере — только на своём сервере.
#[tauri::command]
pub async fn nc_open(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<()> {
    let ctx = state.ctx().await?;
    let client = nc(&ctx).await?;
    let target = client.creds.base.join(path.trim_start_matches('/')).map_err(|e| AppError::Other(e.to_string()))?;
    if target.origin() != client.creds.base.origin() {
        return Err(AppError::Other("Можно открывать только страницы своего Nextcloud".into()));
    }
    host::open_url(&app, target.as_str())
}

/// Общий поиск по Nextcloud: файлы (по имени), карточки Deck, беседы и сообщения чата.
/// С `domain` — ещё и все файлы из папки организации «/Контрагенты/<домен>».
#[tauri::command]
pub async fn nc_search(state: State<'_, AppState>, term: String, domain: Option<String>, limit: Option<u32>) -> Result<crate::nextcloud::NcSearch> {
    let ctx = state.ctx().await?;
    let nc = nc(&ctx).await?;
    let limit = limit.unwrap_or(20).clamp(1, 100) as usize;
    let term = term.trim().trim_start_matches('@').to_string();
    let folder = domain.as_deref().and_then(crate::counterparty::normalize).map(|d| format!("{COUNTERPARTY_DIR}/{d}"));
    let t = Duration::from_secs(30);
    let (files, cards, chat, tree) = tokio::join!(
        tokio::time::timeout(t, nc.files_search(&term, limit)),
        tokio::time::timeout(t, nc.deck_search(&term, limit)),
        tokio::time::timeout(t, nc.chat_search(&term, limit)),
        async {
            match &folder {
                Some(p) => Some(tokio::time::timeout(t, nc.files_tree(p, 3)).await),
                None => None,
            }
        },
    );
    let mut out = crate::nextcloud::NcSearch::default();
    let mut take = |what: &str, r: std::result::Result<Result<()>, tokio::time::error::Elapsed>| match r {
        Ok(Ok(())) => {}
        Ok(Err(e)) => out.errors.push(format!("{what}: {e}")),
        Err(_) => out.errors.push(format!("{what}: сервер не ответил за 30 секунд")),
    };
    let mut found_files = Vec::new();
    let mut found_cards = Vec::new();
    let mut found_chat = Vec::new();
    take("Файлы", files.map(|r| r.map(|v| found_files = v)));
    take("Задачи", cards.map(|r| r.map(|v| found_cards = v)));
    take("Чат", chat.map(|r| r.map(|v| found_chat = v)));
    if let Some(tree) = tree {
        let mut in_folder = Vec::new();
        take("Папка организации", tree.map(|r| r.map(|v| in_folder = v)));
        for f in in_folder {
            if !found_files.iter().any(|x: &crate::nextcloud::NcFile| x.path == f.path) {
                found_files.push(f);
            }
        }
    }
    out.files = found_files;
    out.cards = found_cards;
    out.chat = found_chat;
    Ok(out)
}

#[tauri::command]
pub async fn nc_user_profile(state: State<'_, AppState>, user: String) -> Result<nextcloud::NcProfile> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.user_profile(&user).await
}

/// Занятость сотрудника (по календарю Nextcloud) на период, не длиннее 31 дня.
#[tauri::command]
pub async fn nc_free_busy(state: State<'_, AppState>, email: String, from: i64, to: i64) -> Result<Vec<nextcloud::BusySlot>> {
    if to <= from || to - from > 31 * 86400 {
        return Err(AppError::Other("Неверный период".into()));
    }
    let ctx = state.ctx().await?;
    nc(&ctx).await?.free_busy(&email, from, to).await
}

/// Предпросмотр ссылки из сообщения (через Nextcloud).
#[tauri::command]
pub async fn nc_link_preview(state: State<'_, AppState>, url: String) -> Result<Option<crate::nextcloud::LinkPreview>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.link_preview(&url).await
}

#[tauri::command]
pub async fn nc_search_users(state: State<'_, AppState>, search: String) -> Result<Vec<nextcloud::NcUser>> {
    let ctx = state.ctx().await?;
    let q = search.trim().to_string();
    if q.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let nc = nc(&ctx).await?;
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
