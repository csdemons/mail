//! Папки и письма: список, чтение, флажки, поиск, сохранённые виды, вложения, перемещение и удаление, спам, отписка, «отложить», общая папка «Входящие».

use super::*;

/// Мгновенный поиск по индексу: все свои папки, отправитель/получатели/тема,
/// имена вложений и текст писем, хранимых на компьютере. Запрос — с операторами
/// (`от:`, `кому:`, `тема:`, `вложение:`, `важные`, `после:`… — см. модуль `search`).
#[tauri::command]
pub async fn search_mail(state: State<'_, AppState>, query: String) -> Result<Vec<imap::MessageSummary>> {
    let ctx = state.ctx().await?;
    let q = crate::search::Query::parse(&query);
    if !q.account_ok(&ctx.key) {
        return Ok(Vec::new());
    }
    let Some(st) = local(&ctx).await else { return Ok(Vec::new()) };
    tokio::task::spawn_blocking(move || st.search(&q, 300)).await.map_err(|e| AppError::Other(e.to_string()))
}

/// Письмо по Message-ID среди хранящихся на компьютере (ссылка из карточки Deck и т. п.).
#[tauri::command]
pub async fn mail_by_msgid(state: State<'_, AppState>, msg_id: String) -> Result<Option<imap::MessageSummary>> {
    let ctx = state.ctx().await?;
    let id = format!("<{}>", msg_id.trim().trim_matches(['<', '>']));
    if id.len() < 4 {
        return Ok(None);
    }
    let Some(st) = local(&ctx).await else { return Ok(None) };
    tokio::task::spawn_blocking(move || st.select(&std::collections::HashSet::new(), |m| m.message_id.as_deref() == Some(id.as_str()), 1).into_iter().next())
        .await
        .map_err(|e| AppError::Other(e.to_string()))
}

/// Поиск на сервере по всем своим папкам (без общих ящиков) — отдельным
/// соединением, чтобы не задерживать остальную работу.
pub(crate) async fn server_search(creds: &Credentials, q: &crate::search::Query) -> Result<Vec<imap::MessageSummary>> {
    let mut conn = tokio::time::timeout(Duration::from_secs(20), imap::connect(creds)).await.map_err(|_| AppError::Timeout)??;
    let folders = imap::list_folders(&mut conn).await?;
    let mut out = Vec::new();
    for f in folders.iter().filter(|f| f.selectable && !imap::is_shared(f) && q.folder_ok(&f.path)) {
        let found = tokio::time::timeout(Duration::from_secs(60), async {
            let uids = imap::search_server(&mut conn, &f.id, q).await?;
            let uids: Vec<u32> = uids.into_iter().take(if q.has_attachment { 300 } else { 100 }).collect();
            imap::summaries_of(&mut conn, &f.id, &uids).await
        })
        .await;
        match found {
            Ok(Ok(items)) => out.extend(items.into_iter().filter(|m| q.matches_structure(m)).take(100)),
            Ok(Err(e)) if e.is_connection() => return Err(e),
            Ok(Err(e)) => log::warn!("server search in a folder failed: {e}"),
            Err(_) => return Err(AppError::Timeout),
        }
        if out.len() >= 500 {
            break;
        }
    }
    conn.logout().await;
    out.sort_by_key(|m| std::cmp::Reverse(m.date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map_or(0, |d| d.timestamp())));
    Ok(out)
}

/// Поиск по тексту всех писем на сервере (своих папок).
#[tauri::command]
pub async fn search_server(state: State<'_, AppState>, query: String) -> Result<Vec<imap::MessageSummary>> {
    let ctx = state.ctx().await?;
    let q = crate::search::Query::parse(&query);
    if !q.is_meaningful() || !q.account_ok(&ctx.key) {
        return Ok(Vec::new());
    }
    server_search(&ctx.creds, &q).await
}

/// Сохранённый поиск: запрос с операторами и где искать (`current` | `all`).
#[derive(Serialize, serde::Deserialize, Clone)]
pub struct SavedView {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) query: String,
    pub(crate) scope: String,
}

pub(crate) fn views_path(app: &AppHandle) -> Result<PathBuf> {
    Ok(host::config_dir(app)?.join("views.json"))
}

/// Сохранённые поиски пользователя (общие для всех его учётных записей).
#[tauri::command]
pub fn views_get(app: AppHandle) -> Result<Vec<SavedView>> {
    let Ok(raw) = std::fs::read(views_path(&app)?) else { return Ok(Vec::new()) };
    Ok(serde_json::from_slice(&raw).unwrap_or_default())
}

#[tauri::command]
pub fn views_save(app: AppHandle, views: Vec<SavedView>) -> Result<()> {
    if views.len() > 50 {
        return Err(AppError::Other("Не больше 50 сохранённых поисков".into()));
    }
    for v in &views {
        if v.name.trim().is_empty() || v.name.chars().count() > 60 || v.query.chars().count() > 500 || v.id.len() > 40 || !matches!(v.scope.as_str(), "current" | "all") {
            return Err(AppError::Other(format!("Некорректный сохранённый поиск «{}»", v.name.chars().take(60).collect::<String>())));
        }
    }
    let raw = serde_json::to_vec_pretty(&views).map_err(|e| AppError::Other(e.to_string()))?;
    settings::write_private(&views_path(&app)?, &raw)
}

/// Поиск в других подключённых учётных записях (на их серверах). Каждая — своим
/// соединением, с её паролем; учётные записи, которые не ответили, перечислены в `errors`.
#[tauri::command]
pub async fn search_others(app: AppHandle, state: State<'_, AppState>, query: String) -> Result<UnifiedInbox> {
    state.ctx().await?;
    let q = crate::search::Query::parse(&query);
    let mut items = Vec::new();
    let mut errors = Vec::new();
    if !q.is_meaningful() {
        return Ok(UnifiedInbox { items, errors });
    }
    for s in other_accounts(&app, &state).await {
        let key = settings::account_key(&s);
        if !q.account_ok(&key) {
            continue;
        }
        let Some(password) = password_of(&state, &s).await else {
            errors.push(format!("{}: пароль не сохранён", s.email()));
            continue;
        };
        match server_search(&creds_of(&s, password), &q).await {
            Ok(found) => items.extend(found.into_iter().map(|m| UnifiedItem { account: key.clone(), label: account_label(&s), active: false, message: m })),
            Err(e) => errors.push(format!("{}: {e}", s.email())),
        }
    }
    let ts = |i: &UnifiedItem| i.message.date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map(|d| d.timestamp()).unwrap_or(0);
    items.sort_by_key(|i| std::cmp::Reverse(ts(i)));
    items.truncate(500);
    Ok(UnifiedInbox { items, errors })
}

#[tauri::command]
pub async fn list_folders(state: State<'_, AppState>) -> Result<Vec<Folder>> {
    let ctx = state.ctx().await?;
    let folders = match with_ctx!(ctx, c => imap::list_folders(c)) {
        Ok(f) => f,
        // Без связи — список папок с компьютера.
        Err(e) if e.is_offline() => match local(&ctx).await.and_then(|st| st.folders()) {
            Some(f) => return Ok(f),
            None => return Err(e),
        },
        Err(e) => return Err(e),
    };
    if let Some(st) = local(&ctx).await {
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
    let ctx = state.ctx().await?;
    let page = match with_ctx!(ctx, c => imap::list_messages(c, &folder, before_uid, limit)) {
        Ok(p) => p,
        // Без связи — письма из индекса на компьютере.
        Err(e) if e.is_offline() => match local(&ctx).await.and_then(|st| st.page(&folder, before_uid, limit.clamp(1, 200) as usize)) {
            Some(p) => return Ok(p),
            None => return Err(e),
        },
        Err(e) => return Err(e),
    };
    let mut page = page;
    if let Some(st) = local(&ctx).await {
        st.merge_page(&folder, before_uid, &page);
        st.fill_snippets(&folder, &mut page.items);
    }
    Ok(page)
}

/// Пересылка: вложения исходного письма становятся вложениями нового — как
/// файлы, выбранные в диалоге (в личном временном каталоге, убираются после отправки).
#[tauri::command]
pub async fn forward_attachments(state: State<'_, AppState>, folder: String, uid: u32) -> Result<Vec<PickedFile>> {
    let ctx = state.ctx().await?;
    let raw = match local_raw(&ctx, &folder, uid).await {
        Some(r) => r,
        None => with_ctx!(ctx, c => imap::message_raw(c, &folder, uid))?,
    };
    let parts = tokio::task::spawn_blocking(move || imap::attachments_in(&raw)).await.map_err(|e| AppError::Other(e.to_string()))??;
    let total: u64 = parts.iter().map(|(_, d)| d.len() as u64).sum();
    if total > MAX_OUTGOING_MB * 1024 * 1024 {
        return Err(AppError::Other(format!("Вложения письма больше {MAX_OUTGOING_MB} МБ — почтовый сервер их не примет; отправьте их ссылкой Nextcloud")));
    }
    let mut out = Vec::new();
    for (name, data) in parts {
        let safe = imap::sanitize_filename(&name);
        let safe = if safe.is_empty() { "file".to_string() } else { safe };
        // Свой каталог на каждый файл: одинаковые имена не мешают, после отправки
        // каталог убирается вместе с файлом (`forget_picked`).
        let dir = crate::updater::private_dir("drop")?;
        let path = dir.join(&safe);
        {
            use std::io::Write as _;
            let mut f = crate::updater::create_new_file(&path)?;
            f.write_all(&data)?;
        }
        let id = state.next_pick.fetch_add(1, Ordering::Relaxed).to_string();
        ctx.picked.lock().await.insert(id.clone(), path);
        out.push(PickedFile { id, name: safe, size: data.len() as u64 });
    }
    Ok(out)
}

/// Вложение — в программе компьютера (двойной щелчок): копия в личном временном
/// каталоге, затем открытие программой по умолчанию. Только документы, таблицы,
/// картинки и т. п. — исполняемые файлы так не открываются.
#[tauri::command]
pub async fn attachment_open(app: AppHandle, state: State<'_, AppState>, folder: String, uid: u32, index: usize, allow_macros: Option<bool>) -> Result<String> {
    let ctx = state.ctx().await?;
    let (name, data) = attachment(&ctx, &folder, uid, index).await?;
    if !crate::dragout::openable(&name) {
        return Err(AppError::Other(format!("«{name}» нельзя открыть прямо из письма — сохраните файл и проверьте его перед открытием")));
    }
    if crate::dragout::has_macros(&name) && allow_macros != Some(true) {
        return Err(AppError::Other(format!("«{name}» содержит макросы — откройте его, только если доверяете отправителю (нужно подтверждение)")));
    }
    let path = tokio::task::spawn_blocking(move || crate::dragout::stage_for_open(&name, &data)).await.map_err(|e| AppError::Other(e.to_string()))??;
    // E2E (отладочная сборка): программу не запускаем — проверяется подготовленный файл.
    #[cfg(debug_assertions)]
    if std::env::var_os("RRCKI_E2E_SAVE_DIR").is_some() {
        return Ok(path.to_string_lossy().into_owned());
    }
    host::open_file(&app, &path)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Перетаскивание вложения из письма в папку компьютера: копия во временном
/// каталоге, затем системное перетаскивание из окна, где начали тянуть.
#[tauri::command]
pub async fn attachment_drag_out(app: AppHandle, window: tauri::Window, state: State<'_, AppState>, folder: String, uid: u32, index: usize) -> Result<()> {
    let ctx = state.ctx().await?;
    let (name, data) = attachment(&ctx, &folder, uid, index).await?;
    let path = tokio::task::spawn_blocking(move || crate::dragout::stage(&name, &data)).await.map_err(|e| AppError::Other(e.to_string()))??;
    // E2E (отладочная сборка): в Xvfb нет кнопки мыши, которую отпустят, — системное
    // перетаскивание не начинаем, проверяется подготовленный файл.
    #[cfg(debug_assertions)]
    if std::env::var_os("RRCKI_E2E_SAVE_DIR").is_some() {
        return Ok(());
    }
    let label = window.label().to_string();
    tokio::task::spawn_blocking(move || host::start_drag_out(&app, &label, vec![path])).await.map_err(|e| AppError::Other(e.to_string()))?
}

/// «Это спам» — в «Спам» с меткой $Junk; «Не спам» — во «Входящие» с меткой $NotJunk
/// (метки понимают фильтры спама сервера и Thunderbird).
#[tauri::command]
pub async fn mark_spam(state: State<'_, AppState>, folder: String, uids: Vec<u32>, spam: bool) -> Result<String> {
    let ctx = state.ctx().await?;
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
    if let Some(st) = local(&ctx).await {
        st.remove(&folder, &uids);
    }
    Ok(target)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsubscribeResult {
    /// Готово (одним запросом или письмом).
    pub(crate) done: bool,
    /// Отписка только на сайте рассылки — адрес для браузера.
    pub(crate) open_url: Option<String>,
    pub(crate) how: String,
}

/// Отписаться от рассылки: RFC 8058 (один POST, без браузера), иначе письмом
/// на адрес отписки, иначе — ссылка для браузера.
#[tauri::command]
pub async fn unsubscribe(state: State<'_, AppState>, folder: String, uid: u32) -> Result<UnsubscribeResult> {
    let ctx = state.ctx().await?;
    let raw = match local_raw(&ctx, &folder, uid).await {
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
        let creds = ctx.creds.clone();
        let profile = ctx.profile().await;
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
        if ctx.creds.ews.is_some() {
            send_as(&ctx, &message.formatted()).await?;
        } else {
            smtp::send(&profile.smtp_host, profile.smtp_port, &creds.username, &creds.password, &message).await?;
        }
        return Ok(UnsubscribeResult { done: true, open_url: None, how: "письмом на адрес отписки".into() });
    }
    Ok(UnsubscribeResult { done: false, open_url: u.https, how: "на сайте рассылки".into() })
}

/// «Отложить»: письма уходят в папку «Отложенные» и в срок возвращаются во «Входящие»
/// непрочитанными (срок — ключевое слово на сервере, видно на всех компьютерах).
#[tauri::command]
pub async fn snooze(state: State<'_, AppState>, folder: String, uids: Vec<u32>, until: String) -> Result<String> {
    let ctx = state.ctx().await?;
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
    if let Some(st) = local(&ctx).await {
        st.remove(&folder, &uids);
    }
    Ok(target)
}

/// Вернуть отложенные письма во «Входящие» сейчас.
#[tauri::command]
pub async fn unsnooze(state: State<'_, AppState>, folder: String, uids: Vec<u32>) -> Result<()> {
    let ctx = state.ctx().await?;
    with_conn!(once state, c => async {
        imap::clear_snooze(c, &folder, &uids).await?;
        imap::move_messages(c, &folder, &uids, "INBOX").await
    })?;
    if let Some(st) = local(&ctx).await {
        st.remove(&folder, &uids);
    }
    Ok(())
}

/// Срок наступил — письма возвращаются во «Входящие» непрочитанными.
pub(crate) async fn return_snoozed(ctx: &AccountCtx) -> Result<usize> {
    with_ctx!(once ctx, c => async {
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

/// Ответ на запрос уведомления о прочтении: `send` — отправить, иначе только
/// отметить, что вопрос решён (больше не спрашивать). Метка $MDNSent — как у Thunderbird.
#[tauri::command]
pub async fn mdn_respond(state: State<'_, AppState>, folder: String, uid: u32, send: bool) -> Result<()> {
    let ctx = state.ctx().await?;
    if send {
        let profile = ctx.profile().await;
        let raw = match local_raw(&ctx, &folder, uid).await {
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
        send_as(&ctx, &mdn).await?;
    }
    with_conn!(state, c => imap::set_keyword(c, &folder, &[uid], "$MDNSent", true))?;
    if let Some(st) = local(&ctx).await {
        st.patch(&folder, uid, |m| m.keywords.push("$MDNSent".into()));
    }
    Ok(())
}

#[tauri::command]
pub async fn get_message(state: State<'_, AppState>, folder: String, uid: u32) -> Result<MessageDetail> {
    let ctx = state.ctx().await?;
    let st = local(&ctx).await;
    // Письмо есть на компьютере — открывается сразу, без сервера.
    if let Some(st) = &st {
        if let (Some(raw), Some(summary)) = (st.raw(&folder, uid), st.summary(&folder, uid)) {
            let detail = imap::detail_from_raw(summary, &raw);
            if let Ok(mut guard) = ctx.conn.try_lock() {
                if let Some(c) = guard.as_mut() {
                    c.prime_raw(&folder, uid, raw);
                }
            }
            return Ok(detail);
        }
    }
    let result = with_ctx!(ctx, c => async {
        let detail = imap::get_message(c, &folder, uid).await?;
        if let (Some(st), Some(raw)) = (&st, c.last_raw(&folder, uid)) {
            st.put_raw(&folder, uid, raw);
        }
        Ok(detail)
    });
    match result {
        Err(e) if e.is_offline() && st.is_some() => Err(AppError::Other("Нет связи с сервером, а текст этого письма не сохранён на компьютере — оно откроется, когда связь появится.".into())),
        other => other,
    }
}

#[tauri::command]
pub async fn set_flag(
    state: State<'_, AppState>,
    folder: String,
    uid: u32,
    flag: FlagKind,
    value: bool,
) -> Result<()> {
    let ctx = state.ctx().await?;
    let queued = queue_first(&ctx).await || match with_ctx!(ctx, c => imap::set_flag(c, &folder, uid, flag, value)) {
        Ok(()) => false,
        Err(e) if queue_on(&ctx, &e).await => true,
        Err(e) => return Err(e),
    };
    if queued {
        let targets = targets(&ctx, &folder, &[uid]).await;
        enqueue(&ctx, Op::Flag { folder: folder.clone(), targets, flag: flag_name(flag).into(), value }).await?;
    }
    if let Some(st) = local(&ctx).await {
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
    let ctx = state.ctx().await?;
    let (name, data) = attachment(&ctx, &folder, uid, index).await?;

    let Some(path) = host::save_file(&app, &name).await else {
        return Ok(None);
    };
    tokio::fs::write(&path, data).await?;
    Ok(Some(path.display().to_string()))
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
    pub(crate) name: String,
    /// Содержимое в base64.
    pub(crate) data: String,
}

/// Предпросмотр больше этого размера не загружается в интерфейс — только «Сохранить».
pub(crate) const MAX_PREVIEW_BYTES: usize = 30 * 1024 * 1024;

#[tauri::command]
pub async fn attachment_data(state: State<'_, AppState>, folder: String, uid: u32, index: usize) -> Result<AttachmentData> {
    let ctx = state.ctx().await?;
    use base64::Engine as _;
    let (name, data) = attachment(&ctx, &folder, uid, index).await?;
    if data.len() > MAX_PREVIEW_BYTES {
        return Err(AppError::Other(format!("Файл слишком большой для просмотра ({} МБ) — сохраните его", data.len() / 1024 / 1024)));
    }
    Ok(AttachmentData { name, data: base64::engine::general_purpose::STANDARD.encode(data) })
}

#[tauri::command]
pub async fn create_folder(state: State<'_, AppState>, name: String) -> Result<()> {
    if name.trim().is_empty() || name.len() > 400 {
        return Err(AppError::Other("Некорректное имя папки".into()));
    }
    with_conn!(state, c => imap::create_folder(c, &name))
}

#[tauri::command]
pub async fn move_messages(state: State<'_, AppState>, folder: String, uids: Vec<u32>, target: String) -> Result<()> {
    let ctx = state.ctx().await?;
    let queued = queue_first(&ctx).await || match with_ctx!(ctx, c => imap::move_messages(c, &folder, &uids, &target)) {
        Ok(()) => false,
        Err(e) if queue_on(&ctx, &e).await => true,
        Err(e) => return Err(e),
    };
    if queued {
        let targets = targets(&ctx, &folder, &uids).await;
        enqueue(&ctx, Op::Move { folder: folder.clone(), targets, target: target.clone() }).await?;
    }
    if let Some(st) = local(&ctx).await {
        st.remove(&folder, &uids);
    }
    Ok(())
}

/// Удалить письма: в корзину `trash` или (из корзины) насовсем. `true` — насовсем.
#[tauri::command]
pub async fn delete_messages(state: State<'_, AppState>, folder: String, uids: Vec<u32>, trash: Option<String>) -> Result<bool> {
    let ctx = state.ctx().await?;
    let direct = if queue_first(&ctx).await {
        None
    } else {
        match with_ctx!(ctx, c => imap::delete_messages(c, &folder, &uids, trash.as_deref())) {
            Ok(p) => Some(p),
            Err(e) if queue_on(&ctx, &e).await => None,
            Err(e) => return Err(e),
        }
    };
    let permanent = match direct {
        Some(p) => p,
        None => {
            let targets = targets(&ctx, &folder, &uids).await;
            enqueue(&ctx, Op::Delete { folder: folder.clone(), targets, trash: trash.clone() }).await?;
            trash.as_deref().is_none_or(|t| t == folder)
        }
    };
    if let Some(st) = local(&ctx).await {
        st.remove(&folder, &uids);
    }
    Ok(permanent)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedItem {
    pub(crate) account: String,
    pub(crate) label: String,
    pub(crate) active: bool,
    #[serde(flatten)]
    pub(crate) message: imap::MessageSummary,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedInbox {
    pub(crate) items: Vec<UnifiedItem>,
    /// Учётные записи, которые сейчас не ответили.
    pub(crate) errors: Vec<String>,
}

/// Общие «Входящие»: последние письма всех учётных записей, новые сверху.
#[tauri::command]
pub async fn unified_inbox(app: AppHandle, state: State<'_, AppState>, limit: u32) -> Result<UnifiedInbox> {
    unified(&app, &state, imap::FolderRole::Inbox, limit).await
}

/// Общая папка нескольких учётных записей по роли: inbox, sent, drafts, archive,
/// junk, trash. Последние письма каждой учётной записи, новые сверху; у кого
/// такой папки нет — просто без писем.
#[tauri::command]
pub async fn unified_folder(app: AppHandle, state: State<'_, AppState>, role: String, limit: u32) -> Result<UnifiedInbox> {
    let role = imap::parse_role(&role).ok_or_else(|| AppError::Other("Неизвестная папка".into()))?;
    unified(&app, &state, role, limit).await
}

pub(crate) async fn unified(app: &AppHandle, state: &AppState, role: imap::FolderRole, limit: u32) -> Result<UnifiedInbox> {
    let limit = limit.clamp(1, 100);
    let ctx = state.ctx().await?;
    let profile = ctx.profile().await;
    let active = ctx.key.clone();
    let mut items = Vec::new();
    let mut errors = Vec::new();
    if let Some(page) = with_ctx!(ctx, c => imap::role_messages(c, role, limit))? {
        items.extend(page.items.into_iter().map(|m| UnifiedItem { account: active.clone(), label: account_label(&profile), active: true, message: m }));
    }
    for s in other_accounts(app, state).await {
        let key = settings::account_key(&s);
        match on_other(state, &s, move |c| Box::pin(imap::role_messages(c, role, limit))).await {
            Ok(Some(page)) => items.extend(page.items.into_iter().map(|m| UnifiedItem { account: key.clone(), label: account_label(&s), active: false, message: m })),
            Ok(None) => {}
            Err(e) => errors.push(format!("{}: {e}", s.email())),
        }
    }
    let ts = |i: &UnifiedItem| i.message.date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map(|d| d.timestamp()).unwrap_or(0);
    items.sort_by_key(|i| std::cmp::Reverse(ts(i)));
    items.truncate(limit as usize * 2);
    Ok(UnifiedInbox { items, errors })
}
