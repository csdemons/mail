//! Новое письмо: подписи, отправители и подключённые ящики, вложения, черновики, отправка, исходящие и отложенные, шаблоны.

use super::*;

/// Личная подпись. Пока своя не задана — берётся подпись из веб-почты (SOGo).
#[tauri::command]
pub async fn signature_get(state: State<'_, AppState>) -> Result<String> {
    let ctx = state.ctx().await?;
    let profile = ctx.profile().await;
    if let Some(sig) = profile.signature {
        return Ok(sig);
    }
    let Ok((_, base)) = dav_client(&ctx).await else { return Ok(String::new()) };
    match sogo_defaults(&ctx, &base).await {
        Ok((_, d)) => Ok(d.get("SOGoMailSignature").and_then(|v| v.as_str()).map(html_to_text).unwrap_or_default()),
        Err(_) => Ok(String::new()),
    }
}

/// Сохранить подпись в программе и (если доступно) в веб-почте SOGo.
/// Возвращает предупреждение, если в веб-почту записать не удалось.
#[tauri::command]
pub async fn signature_save(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<Option<String>> {
    let ctx = state.ctx().await?;
    let text: String = text.replace("\r\n", "\n").chars().take(4000).collect();
    let profile = {
        let mut p = ctx.profile.lock().await;
        p.signature = Some(text.clone());
        // Список подписей: меняется подпись для новых писем.
        if let Some(id) = p.signature_new.clone() {
            if let Some(sig) = p.signatures.iter_mut().find(|x| x.id == id) {
                sig.text = text.clone();
            }
        }
        p.clone()
    };
    settings::save(&app, &profile)?;
    let synced = async {
        let (_, base) = dav_client(&ctx).await?;
        let (session, mut defaults) = sogo_defaults(&ctx, &base).await?;
        defaults["SOGoMailSignature"] = serde_json::Value::String(text_to_html(&text));
        session.save_defaults(&defaults).await
    }
    .await;
    Ok(synced.err().map(|e| format!("Подпись сохранена в программе, но не в веб-почте: {e}")))
}

/// Подписи учётной записи и выбранные по умолчанию.
#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureSet {
    pub items: Vec<settings::Signature>,
    pub new_id: Option<String>,
    pub reply_id: Option<String>,
}

/// Подписи: свои списком; если списка ещё нет — прежняя единственная подпись «Основная».
#[tauri::command]
pub async fn signatures_get(state: State<'_, AppState>) -> Result<SignatureSet> {
    let ctx = state.ctx().await?;
    let profile = ctx.profile().await;
    if !profile.signatures.is_empty() {
        return Ok(SignatureSet { items: profile.signatures, new_id: profile.signature_new, reply_id: profile.signature_reply });
    }
    let text = signature_get(state).await.unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(SignatureSet { items: Vec::new(), new_id: None, reply_id: None });
    }
    let main = settings::Signature { id: "main".into(), name: "Основная".into(), text };
    Ok(SignatureSet { items: vec![main], new_id: Some("main".into()), reply_id: Some("main".into()) })
}

/// Сохранить подписи. Подпись для новых писем записывается и как прежняя единственная
/// (её же получает веб-почта SOGo). Возвращает предупреждение, если веб-почта не обновилась.
#[tauri::command]
pub async fn signatures_save(app: AppHandle, state: State<'_, AppState>, set: SignatureSet) -> Result<Option<String>> {
    if set.items.len() > 20 {
        return Err(AppError::Other("Не больше 20 подписей".into()));
    }
    let mut items = Vec::new();
    for s in set.items {
        let id: String = s.id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(32).collect();
        let name = s.name.trim().chars().take(60).collect::<String>();
        if id.is_empty() || name.is_empty() || items.iter().any(|x: &settings::Signature| x.id == id) {
            return Err(AppError::Other("У каждой подписи должно быть название".into()));
        }
        items.push(settings::Signature { id, name, text: s.text.replace("\r\n", "\n").chars().take(4000).collect() });
    }
    let known = |id: &Option<String>| id.clone().filter(|i| items.iter().any(|x| &x.id == i));
    let new_id = known(&set.new_id);
    let reply_id = known(&set.reply_id);
    let main_text = new_id.as_ref().and_then(|i| items.iter().find(|x| &x.id == i)).map(|x| x.text.clone()).unwrap_or_default();
    let ctx = state.ctx().await?;
    let changed_main = {
        let mut p = ctx.profile.lock().await;
        let changed = p.signature.as_deref() != Some(main_text.as_str());
        p.signatures = items;
        p.signature_new = new_id;
        p.signature_reply = reply_id;
        p.signature = Some(main_text.clone());
        settings::save(&app, &p)?;
        changed
    };
    if !changed_main {
        return Ok(None);
    }
    let synced = async {
        let (_, base) = dav_client(&ctx).await?;
        let (session, mut defaults) = sogo_defaults(&ctx, &base).await?;
        defaults["SOGoMailSignature"] = serde_json::Value::String(text_to_html(&main_text));
        session.save_defaults(&defaults).await
    }
    .await;
    Ok(synced.err().map(|e| format!("Подписи сохранены в программе, но подпись в веб-почте не обновилась: {e}")))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundMailbox {
    pub(crate) address: String,
    pub(crate) name: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxSet {
    /// Подключённые вручную.
    pub(crate) manual: Vec<String>,
    /// Найденные Autodiscover — подключаются сами, как в Outlook.
    pub(crate) discovered: Vec<FoundMailbox>,
}

/// От чьего имени можно отправить письмо: свой адрес и подключённые ящики.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub(crate) address: String,
    pub(crate) name: String,
    pub(crate) own: bool,
}

pub(crate) async fn identities(ctx: &AccountCtx) -> Vec<Identity> {
    let p = ctx.profile().await;
    let own = p.email().to_lowercase();
    let mut out = vec![Identity { address: own.clone(), name: p.display_name.clone(), own: true }];
    let mut push = |address: String, name: String| {
        if address != own && crate::ews::valid_mailbox(&address) && !out.iter().any(|i| i.address == address) {
            out.push(Identity { address, name, own: false });
        }
    };
    if exchange(ctx) {
        for (a, n) in crate::ews::discovered_mailboxes(&settings::account_key(&p)) {
            push(a, n);
        }
        for a in &p.mailboxes {
            push(a.clone(), a.clone());
        }
    } else if let Ok(folders) = with_ctx!(once ctx, c => imap::list_folders(c)) {
        // Общие ящики IMAP (shared/<адрес>/…), если ящик назван адресом почты.
        for f in folders.iter().filter(|f| f.depth == 1 && f.path.to_lowercase().starts_with("shared/")) {
            let a = f.path[7..].to_lowercase();
            push(a.clone(), a);
        }
    }
    out
}

/// Отправитель письма: свой адрес или подключённый ящик («Отправить как», «Отправить от имени»).
pub(crate) async fn sender(ctx: &AccountCtx, from: Option<&str>) -> Result<(Option<String>, String)> {
    let p = ctx.profile().await;
    let own = p.email();
    let Some(from) = from.map(|f| f.trim().to_lowercase()).filter(|f| !f.is_empty() && !f.eq_ignore_ascii_case(&own)) else {
        return Ok((Some(p.display_name.clone()), own));
    };
    let ids = identities(ctx).await;
    let id = ids.into_iter().find(|i| i.address == from).ok_or_else(|| AppError::Other(format!("Ящик {from} не подключён — отправить от его имени нельзя")))?;
    Ok(((id.name != id.address).then_some(id.name), id.address))
}

#[tauri::command]
pub async fn send_identities(state: State<'_, AppState>) -> Result<Vec<Identity>> {
    let ctx = state.ctx().await?;
    Ok(identities(&ctx).await)
}

/// Подключённые ящики Exchange (общие, делегированные).
#[tauri::command]
pub async fn mailboxes_get(state: State<'_, AppState>) -> Result<MailboxSet> {
    let ctx = state.ctx().await?;
    let p = ctx.profile().await;
    let discovered = crate::ews::discovered_mailboxes(&settings::account_key(&p)).into_iter().map(|(address, name)| FoundMailbox { address, name }).collect();
    Ok(MailboxSet { manual: p.mailboxes.clone(), discovered })
}

/// Сохранить список ящиков, подключённых вручную; новые адреса сначала проверяются
/// (есть ли такой ящик и доступ к нему).
#[tauri::command]
pub async fn mailboxes_save(app: AppHandle, state: State<'_, AppState>, list: Vec<String>) -> Result<()> {
    let ctx = state.ctx().await?;
    if !exchange(&ctx) {
        return Err(AppError::Other("Подключать ящики можно только для учётной записи Exchange".into()));
    }
    let own = ctx.profile().await.email().to_lowercase();
    let mut clean: Vec<String> = Vec::new();
    for a in list {
        let a = a.trim().trim_start_matches("mailto:").to_lowercase();
        if a.is_empty() || clean.contains(&a) {
            continue;
        }
        if !crate::ews::valid_mailbox(&a) {
            return Err(AppError::Other(format!("«{a}» — не адрес почты")));
        }
        if a == own {
            return Err(AppError::Other("Это ваш собственный ящик — он уже подключён".into()));
        }
        clean.push(a);
    }
    if clean.len() > crate::ews::MAX_MAILBOXES {
        return Err(AppError::Other(format!("Не больше {} ящиков", crate::ews::MAX_MAILBOXES)));
    }
    let before = ctx.profile().await.mailboxes;
    for a in clean.iter().filter(|a| !before.contains(a)) {
        let a = a.clone();
        with_ctx!(once ctx, c => async { ews_of(c)?.check_mailbox(&a).await })?;
    }
    let p = {
        let mut p = ctx.profile.lock().await;
        p.mailboxes = clean;
        settings::save(&app, &p)?;
        p.clone()
    };
    crate::ews::set_mailboxes(&settings::account_key(&p), &p.email(), &p.mailboxes);
    Ok(())
}

pub(crate) fn html_to_text(html: &str) -> String {
    if !html.contains('<') {
        return html.trim().to_string();
    }
    let with_breaks = html.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n").replace("</p>", "\n").replace("</div>", "\n");
    let mut out = String::new();
    let mut in_tag = false;
    for c in with_breaks.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&").trim().to_string()
}

pub(crate) fn text_to_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\n', "<br>")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedFile {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) size: u64,
}

pub(crate) const MAX_OUTGOING_MB: u64 = 25;

#[tauri::command]
pub async fn pick_attachments(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<PickedFile>> {
    let ctx = state.ctx().await?;
    // E2E: системный диалог WebDriver не нажать, поэтому в отладочной сборке
    // файлы можно подставить переменной окружения. В релизе этого кода нет.
    #[cfg(debug_assertions)]
    let test_paths: Option<Vec<PathBuf>> = std::env::var("RRCKI_E2E_PICK").ok().map(|v| v.split(';').map(PathBuf::from).collect());
    #[cfg(not(debug_assertions))]
    let test_paths: Option<Vec<PathBuf>> = None;
    let paths = match test_paths {
        Some(p) => Some(p),
        None => host::pick_files(&app).await,
    };
    let Some(paths) = paths else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut picked = ctx.picked.lock().await;
    for path in paths {
        let size = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
        let id = state.next_pick.fetch_add(1, Ordering::Relaxed).to_string();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        picked.insert(id.clone(), path);
        out.push(PickedFile { id, name, size });
    }
    Ok(out)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    /// Папка, куда сохранена копия, или `None`.
    pub(crate) saved_to: Option<String>,
    pub(crate) warning: Option<String>,
    /// Нет связи — письмо в очереди исходящих.
    #[serde(default)]
    pub(crate) queued: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum OutCmd {
    Wait,
    Now,
    Cancel,
}

pub(crate) type Delivery = Option<std::result::Result<SendResult, String>>;

pub(crate) struct OutItem {
    pub(crate) ctl: tokio::sync::watch::Sender<OutCmd>,
    pub(crate) done: tokio::sync::watch::Receiver<Delivery>,
    pub(crate) started: bool,
}

/// Письма, ожидающие окончания «окна отмены».
#[derive(Default)]
pub struct Outbox {
    pub(crate) next: AtomicU64,
    pub(crate) items: std::sync::Mutex<HashMap<u64, OutItem>>,
}

impl Outbox {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, OutItem>> {
        self.items.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Есть письма, которые ещё не ушли.
    pub fn pending(&self) -> bool {
        self.lock().values().any(|i| i.done.borrow().is_none())
    }

    /// Отправить всё ожидающее сразу и дождаться (выход из программы).
    pub async fn flush(&self) {
        let waits: Vec<_> = self
            .lock()
            .values()
            .map(|i| {
                let _ = i.ctl.send(OutCmd::Now);
                i.done.clone()
            })
            .collect();
        for mut w in waits {
            let _ = tokio::time::timeout(Duration::from_secs(90), w.wait_for(|d| d.is_some())).await;
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendTicket {
    /// Письмо в «окне отмены»: ждать `send_wait`, отменить `send_cancel`.
    pub(crate) id: Option<u64>,
    /// Отправлено сразу.
    pub(crate) result: Option<SendResult>,
    /// Отложенная отправка: письмо в «Черновиках» до указанного времени.
    pub(crate) scheduled_at: Option<String>,
}

/// Отправка и копия в «Отправленные» (письмо уже ушло — ошибка копии лишь предупреждение).
/// Всё — от имени и в ящик учётной записи `ctx`, даже если пользователь за это
/// время переключился на другую или вышел.
pub(crate) async fn deliver(ctx: &AccountCtx, raw: &[u8], built: Option<&lettre::Message>) -> Result<SendResult> {
    let _ = built;
    send_as(ctx, raw).await?;
    // Exchange сам сохраняет копию в «Отправленные».
    if ctx.creds.ews.is_some() {
        return Ok(SendResult { saved_to: None, warning: None, queued: false });
    }
    match append_sent_as(ctx, raw).await {
        Ok(folder) => Ok(SendResult { saved_to: Some(folder), warning: None, queued: false }),
        Err(e) => Ok(SendResult { saved_to: None, warning: Some(format!("Письмо отправлено, но копия не сохранена в «Отправленные»: {e}")), queued: false }),
    }
}

/// Отправить письмо от имени учётной записи `ctx`: Exchange — через EWS (её
/// соединение, а после закрытия сеанса — отдельное), остальные — через SMTP с
/// проверкой связи перед передачей: если сервер недоступен, письмо точно не ушло
/// (его можно поставить в очередь), обрыв во время передачи — «результат неизвестен».
pub(crate) async fn send_as(ctx: &AccountCtx, raw: &[u8]) -> Result<()> {
    if ctx.creds.ews.is_some() {
        if !ctx.is_closed() {
            return with_ctx!(once ctx, c => imap::send_via_server(c, raw));
        }
        let mut c = imap::connect(&ctx.creds).await?;
        let r = imap::send_via_server(&mut c, raw).await;
        c.logout().await;
        return r;
    }
    let profile = ctx.profile().await;
    let r = smtp::send_raw(&profile.smtp_host, profile.smtp_port, &ctx.creds.username, &ctx.creds.password, raw).await;
    match (r, imap::from_address(raw)) {
        (Err(AppError::Other(m)), Some(from)) if !from.eq_ignore_ascii_case(&profile.email()) && (m.contains("553") || m.contains("550") || m.contains("554")) => {
            Err(AppError::Other(format!("Сервер не разрешил отправку от имени {from} — попросите администратора дать вам право отправлять письма от этого ящика ({m})")))
        }
        (r, _) => r,
    }
}

/// Копия в «Отправленные» — в ящик учётной записи `ctx`: через её соединение,
/// а если её сеанс уже закрыт (переключение, выход) — отдельным соединением.
pub(crate) async fn append_sent_as(ctx: &AccountCtx, raw: &[u8]) -> Result<String> {
    if !ctx.is_closed() {
        return with_ctx!(once ctx, c => imap::append_sent(c, raw));
    }
    let mut c = tokio::time::timeout(Duration::from_secs(20), imap::connect(&ctx.creds)).await.map_err(|_| AppError::Timeout)??;
    let result = tokio::time::timeout(OP_TIMEOUT, imap::append_sent(&mut c, raw)).await.unwrap_or(Err(AppError::Timeout));
    c.logout().await;
    result
}

/// Вложения письма: файлы, выбранные в диалоге (по их id), — не больше предела сервера.
pub(crate) async fn read_picked(ctx: &AccountCtx, attachments: &[String]) -> Result<Vec<smtp::OutgoingAttachment>> {
    let mut files = Vec::new();
    let mut total = 0u64;
    let picked = ctx.picked.lock().await;
    for id in attachments {
        let path = picked
            .get(id)
            .ok_or_else(|| AppError::Other("Вложение не найдено, выберите файл заново".into()))?;
        // Размер — по метаданным, до чтения: огромный файл не попадает в память.
        total += tokio::fs::metadata(path).await?.len();
        if total > MAX_OUTGOING_MB * 1024 * 1024 {
            return Err(AppError::Other(format!("Вложения больше {MAX_OUTGOING_MB} МБ — почтовый сервер их не примет")));
        }
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        let content_type = mime_guess::from_path(path).first_or_octet_stream().to_string();
        let data = tokio::fs::read(path).await?;
        files.push(smtp::OutgoingAttachment { name, content_type, data });
    }
    Ok(files)
}

/// Черновик из окна письма (автосохранение): новая версия в «Черновиках», прежние — удаляются.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn draft_save(
    state: State<'_, AppState>,
    key: String,
    to: String,
    cc: String,
    subject: String,
    body: String,
    html: Option<String>,
    in_reply_to: Option<String>,
    references: Option<String>,
    attachments: Vec<String>,
    read_receipt: Option<bool>,
    replace_folder: Option<String>,
    replace_uid: Option<u32>,
    from: Option<String>,
) -> Result<String> {
    let ctx = state.ctx().await?;
    if !imap::valid_draft_key(&key) {
        return Err(AppError::Other("Некорректный ключ черновика".into()));
    }
    let (from_name, from_address) = sender(&ctx, from.as_deref()).await?;
    let files = read_picked(&ctx, &attachments).await?;
    let message = smtp::build(smtp::Outgoing {
        from_name,
        from_address,
        to: smtp::split_addresses(&to),
        cc: smtp::split_addresses(&cc),
        subject,
        body,
        html,
        in_reply_to,
        references,
        attachments: files,
        read_receipt: read_receipt.unwrap_or(false),
        extra_headers: vec![(imap::DRAFT_KEY_HEADER.to_string(), key.clone())],
    })?;
    let raw = message.formatted();
    let replace = replace_folder.as_deref().zip(replace_uid);
    with_ctx!(once ctx, c => imap::save_draft(c, &raw, &key, replace))
}

/// Удалить черновик (письмо отправлено или человек отказался от него).
#[tauri::command]
pub async fn draft_discard(state: State<'_, AppState>, key: String, replace_folder: Option<String>, replace_uid: Option<u32>) -> Result<()> {
    let ctx = state.ctx().await?;
    let replace = replace_folder.as_deref().zip(replace_uid);
    with_ctx!(once ctx, c => imap::discard_draft(c, &key, replace))
}

/// Несохранённое письмо на случай сбоя: копия в зашифрованном хранилище на компьютере
/// (без хранилища — не сохраняется). `None` — забыть.
#[tauri::command]
pub async fn draft_recovery_set(state: State<'_, AppState>, data: Option<String>) -> Result<()> {
    let ctx = state.ctx().await?;
    let Some(st) = local(&ctx).await else { return Ok(()) };
    let bytes = data.unwrap_or_default();
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(AppError::Other("Письмо слишком большое для копии на компьютере".into()));
    }
    tokio::task::spawn_blocking(move || st.save_blob("draft-recovery", bytes.as_bytes())).await.map_err(|e| AppError::Other(e.to_string()))?
}

#[tauri::command]
pub async fn draft_recovery_get(state: State<'_, AppState>) -> Result<Option<String>> {
    let ctx = state.ctx().await?;
    let Some(st) = local(&ctx).await else { return Ok(None) };
    let data = tokio::task::spawn_blocking(move || st.load_blob("draft-recovery")).await.map_err(|e| AppError::Other(e.to_string()))?;
    Ok(data.and_then(|d| String::from_utf8(d).ok()).filter(|s| !s.trim().is_empty()))
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    to: String,
    cc: String,
    subject: String,
    body: String,
    html: Option<String>,
    in_reply_to: Option<String>,
    references: Option<String>,
    attachments: Vec<String>,
    read_receipt: Option<bool>,
    delay_secs: Option<u32>,
    send_at: Option<String>,
    from: Option<String>,
) -> Result<SendTicket> {
    let ctx = state.ctx().await?;
    let (from_name, from_address) = sender(&ctx, from.as_deref()).await?;

    let send_at = match send_at.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(t) => {
            let at = chrono::DateTime::parse_from_rfc3339(t).map_err(|_| AppError::Other("Некорректное время отправки".into()))?;
            if at.timestamp() < chrono::Utc::now().timestamp() + 30 {
                return Err(AppError::Other("Время отправки уже прошло — выберите время позже".into()));
            }
            Some(at.to_rfc3339())
        }
        None => None,
    };

    let files = read_picked(&ctx, &attachments).await?;

    let message = smtp::build(smtp::Outgoing {
        from_name,
        from_address,
        to: smtp::split_addresses(&to),
        cc: smtp::split_addresses(&cc),
        subject,
        body,
        html,
        in_reply_to,
        references,
        attachments: files,
        read_receipt: read_receipt.unwrap_or(false),
        extra_headers: send_at.iter().map(|t| (imap::SEND_AT_HEADER.to_string(), t.clone())).collect(),
    })?;
    let raw = message.formatted();

    // Отложенная отправка: письмо ждёт в «Черновиках» (видно и в веб-почте) и
    // уходит в срок, пока программа запущена, — или при следующем запуске.
    if let Some(at) = send_at {
        let queued = if queue_first(&ctx).await && local(&ctx).await.is_some() {
            true
        } else {
            match with_ctx!(once ctx, c => imap::append_draft(c, &raw)) {
                Ok(_) => false,
                Err(e) if queue_on(&ctx, &e).await => true,
                Err(e) => return Err(e),
            }
        };
        if queued {
            let (subject, _) = outgoing_meta(&raw);
            enqueue(&ctx, Op::Schedule { raw: offline::encode(&raw), subject, send_at: at.clone() }).await?;
        }
        forget_picked(&ctx, &attachments).await;
        return Ok(SendTicket { id: None, result: None, scheduled_at: Some(at) });
    }

    let delay = delay_secs.unwrap_or(0).min(60);
    if delay == 0 {
        let result = deliver_or_queue(&ctx, &raw).await?;
        forget_picked(&ctx, &attachments).await;
        return Ok(SendTicket { id: None, result: Some(result), scheduled_at: None });
    }

    // «Окно отмены»: письмо уходит через `delay` секунд, если его не отменили.
    let id = state.outbox.next.fetch_add(1, Ordering::SeqCst) + 1;
    let (ctl_tx, mut ctl_rx) = tokio::sync::watch::channel(OutCmd::Wait);
    let (done_tx, done_rx) = tokio::sync::watch::channel::<Delivery>(None);
    state.outbox.lock().insert(id, OutItem { ctl: ctl_tx, done: done_rx, started: false });
    tauri::async_runtime::spawn(async move {
        use tauri::Manager;
        let state = app.state::<AppState>();
        let sleep = tokio::time::sleep(Duration::from_secs(u64::from(delay)));
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                changed = ctl_rx.changed() => {
                    let cmd = if changed.is_ok() { *ctl_rx.borrow() } else { OutCmd::Now };
                    match cmd {
                        OutCmd::Now => break,
                        OutCmd::Cancel => {
                            let _ = done_tx.send(Some(Err("Отправка отменена".into())));
                            return;
                        }
                        OutCmd::Wait => {}
                    }
                }
            }
        }
        {
            let mut items = state.outbox.lock();
            match items.get_mut(&id) {
                Some(item) if *item.ctl.borrow() == OutCmd::Cancel => {
                    let _ = done_tx.send(Some(Err("Отправка отменена".into())));
                    return;
                }
                Some(item) => item.started = true,
                None => return,
            }
        }
        let result = deliver_or_queue(&ctx, &raw).await;
        if result.is_ok() {
            forget_picked(&ctx, &attachments).await;
        }
        let _ = done_tx.send(Some(result.map_err(|e| e.to_string())));
    });
    Ok(SendTicket { id: Some(id), result: None, scheduled_at: None })
}

pub(crate) async fn forget_picked(ctx: &AccountCtx, ids: &[String]) {
    let mut picked = ctx.picked.lock().await;
    for id in ids {
        // Перетащенный файл лежит в своём временном каталоге — убираем и его.
        if let Some(path) = picked.remove(id) {
            if let Some(dir) = path.parent().filter(|d| d.file_name().is_some_and(|n| n.to_string_lossy().starts_with("drop-"))) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// Дождаться отправки письма из «окна отмены».
#[tauri::command]
pub async fn send_wait(state: State<'_, AppState>, id: u64) -> Result<SendResult> {
    let mut done = state.outbox.lock().get(&id).map(|i| i.done.clone()).ok_or(AppError::NotFound)?;
    let result = done.wait_for(|d| d.is_some()).await.map_err(|_| AppError::Other("Отправка прервана".into()))?.clone();
    state.outbox.lock().remove(&id);
    result.expect("checked above").map_err(AppError::Other)
}

/// Отменить отправку, пока письмо не ушло. `false` — уже отправляется.
#[tauri::command]
pub async fn send_cancel(state: State<'_, AppState>, id: u64) -> Result<bool> {
    let items = state.outbox.lock();
    let Some(item) = items.get(&id) else { return Ok(false) };
    if item.started || item.done.borrow().is_some() {
        return Ok(false);
    }
    let _ = item.ctl.send(OutCmd::Cancel);
    Ok(true)
}

/// Отправить сразу, не дожидаясь конца «окна отмены».
#[tauri::command]
pub async fn send_now(state: State<'_, AppState>, id: u64) -> Result<()> {
    if let Some(item) = state.outbox.lock().get(&id) {
        let _ = item.ctl.send(OutCmd::Now);
    }
    Ok(())
}

/// Запланированные письма (ждут в «Черновиках»).
#[tauri::command]
pub async fn scheduled_list(state: State<'_, AppState>) -> Result<Vec<imap::Scheduled>> {
    with_conn!(state, c => imap::scheduled(c))
}

/// Отправить запланированное письмо сейчас.
#[tauri::command]
pub async fn scheduled_send_now(state: State<'_, AppState>, folder: String, uid: u32, resend: Option<bool>) -> Result<SendResult> {
    let ctx = state.ctx().await?;
    if resend == Some(true) {
        // Пользователь проверил, что письмо не дошло, и отправляет его ещё раз.
        with_ctx!(ctx, c => imap::clear_send_marks(c, &folder, uid))?;
    }
    send_scheduled(&ctx, &folder, uid, "manual")
        .await?
        .ok_or_else(|| AppError::Other("Письмо уже отправлено, отправляется с другого компьютера или его результат неизвестен — обновите список".into()))
}

/// Отменить отложенную отправку: письмо остаётся обычным черновиком.
#[tauri::command]
pub async fn scheduled_cancel(state: State<'_, AppState>, folder: String, uid: u32) -> Result<()> {
    with_conn!(once state, c => async {
        let raw = imap::raw_of(c, &folder, uid).await?;
        let plain = smtp::strip_header(&raw, imap::SEND_AT_HEADER);
        imap::append_draft(c, &plain).await?;
        imap::remove_message(c, &folder, uid).await
    })
}

/// Отправка одного запланированного письма (с захватом, чтобы не отправить дважды).
/// `None` — письмо отправляет другая копия программы, оно уже отправлено или его
/// результат неизвестен. Письмо отправляется от той учётной записи, сеанс которой
/// был открыт при запуске (`ctx`): после переключения или выхода — не отправляется.
///
/// Состояния (метки IMAP): захват `atomclaim…` → SMTP принял → `atomsent` → удаление
/// из «Черновиков». Обрыв связи во время передачи → `atomunknown`: такое письмо
/// автоматически не повторяется — получатель мог его уже получить.
pub(crate) async fn send_scheduled(ctx: &AccountCtx, folder: &str, uid: u32, tag: &str) -> Result<Option<SendResult>> {
    if ctx.is_closed() {
        return Ok(None);
    }
    let creds = ctx.creds.clone();
    let token = format!("{}{}{}", chrono::Utc::now().timestamp(), tag.chars().take(4).collect::<String>(), &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let mut conn = imap::connect(&creds).await?;
    let result = async {
        if !imap::claim(&mut conn, folder, uid, &token).await? {
            return Ok(None);
        }
        let raw = imap::raw_of(&mut conn, folder, uid).await?;
        let final_raw = smtp::finalize_scheduled(&raw, imap::SEND_AT_HEADER);
        match deliver(ctx, &final_raw, None).await {
            Ok(mut r) => {
                // Сначала отметка «отправлено»: даже если удалить черновик не выйдет,
                // письмо больше не уйдёт повторно.
                let marked = imap::set_keyword(&mut conn, folder, &[uid], imap::SENT_MARK, true).await;
                if let Err(e) = imap::remove_message(&mut conn, folder, uid).await {
                    log::warn!("scheduled mail sent but draft not removed: {e}");
                    if marked.is_err() {
                        log::warn!("sent mark not set either; claim keeps it from being resent");
                    }
                    let note = "Письмо отправлено, но осталось в «Черновиках» — повторно оно не уйдёт, черновик уберётся позже.".to_string();
                    r.warning = Some(r.warning.map_or(note.clone(), |w| format!("{w}\n{note}")));
                }
                Ok(Some(r))
            }
            Err(e @ AppError::DeliveryUnknown(_)) => {
                let _ = imap::set_keyword(&mut conn, folder, &[uid], imap::UNKNOWN_MARK, true).await;
                Err(e)
            }
            Err(e) => {
                // Сервер отказал или недоступен — письмо точно не ушло: снимаем
                // свою метку захвата, попробуем в следующий раз.
                let _ = imap::set_keyword(&mut conn, folder, &[uid], &format!("atomclaim{token}"), false).await;
                Err(e)
            }
        }
    }
    .await;
    conn.logout().await;
    result
}

/// Убрать из «Черновиков» уже отправленное письмо (если в прошлый раз не вышло).
pub(crate) async fn cleanup_sent(ctx: &AccountCtx, folder: &str, uid: u32) -> Result<()> {
    with_ctx!(ctx, c => imap::remove_message(c, folder, uid))
}

pub(crate) async fn scheduled_now(ctx: &AccountCtx) -> Result<Vec<imap::Scheduled>> {
    with_ctx!(ctx, c => imap::scheduled(c))
}

/// Фоновая отправка запланированных писем и возврат отложенных — пока сеанс
/// учётной записи `ctx` открыт (только её «Черновики» и её ящик).
pub(crate) async fn run_scheduler(app: AppHandle, ctx: Arc<AccountCtx>) {
    let e2e = cfg!(debug_assertions) && std::env::var_os("RRCKI_E2E_POLL_SECS").is_some();
    let every = if e2e { 5 } else { 60 };
    // Без связи — проверять чаще, чтобы очередь ушла вскоре после подключения.
    let every_offline = if e2e { 3 } else { 15 };
    let mut tracker = RuleTracker::default();
    loop {
        tokio::time::sleep(Duration::from_secs(if ctx.online() { every } else { every_offline })).await;
        if ctx.is_closed() {
            return;
        }
        if !ctx.online() && !try_reconnect(&ctx).await {
            continue;
        }
        if queue_busy(&ctx).await {
            replay(ctx.clone()).await;
        }
        if let Err(e) = process_rules(&app, &ctx, &mut tracker).await {
            log::warn!("mail rules: {e}");
        }
        match return_snoozed(&ctx).await {
            Ok(n) if n > 0 => {
                let _ = host::notify(&app, "Отложенные письма вернулись", &format!("Во «Входящих»: {n}"));
            }
            Err(e) => log::warn!("snoozed mail check failed: {e}"),
            _ => {}
        }
        let list = match scheduled_now(&ctx).await {
            Ok(l) => l,
            Err(e) => {
                log::warn!("scheduled mail check failed: {e}");
                continue;
            }
        };
        let now = chrono::Utc::now();
        for s in list {
            if ctx.is_closed() {
                break;
            }
            if s.status == "sent" {
                if let Err(e) = cleanup_sent(&ctx, &s.folder, s.uid).await {
                    log::warn!("sent scheduled mail not removed from drafts: {e}");
                }
                continue;
            }
            let due = chrono::DateTime::parse_from_rfc3339(&s.send_at).is_ok_and(|t| t <= now);
            // «Неизвестно» и «отправляется» — не трогаем: решает пользователь или другая копия.
            if !due || s.status != "pending" {
                continue;
            }
            match send_scheduled(&ctx, &s.folder, s.uid, "auto").await {
                Ok(Some(_)) => {
                    let _ = host::notify(&app, "Отложенное письмо отправлено", if s.subject.is_empty() { "(без темы)" } else { &s.subject });
                }
                Ok(None) => {}
                Err(e @ AppError::DeliveryUnknown(_)) => {
                    log::warn!("scheduled send outcome unknown: {e}");
                    let _ = host::notify(&app, "Отложенное письмо: результат неизвестен", &format!("{}: проверьте в «Запланированных» — повторно автоматически не отправляется", s.subject));
                }
                Err(e) => {
                    log::warn!("scheduled send failed: {e}");
                    let _ = host::notify(&app, "Отложенное письмо не отправлено", &format!("{}: {e}", s.subject));
                }
            }
        }
    }
}

#[tauri::command]
pub async fn templates_list(state: State<'_, AppState>) -> Result<Vec<imap::Template>> {
    with_conn!(state, c => imap::templates(c))
}

/// Сохранить шаблон письма (папка «Templates» на сервере — как в Thunderbird).
#[tauri::command]
pub async fn template_save(state: State<'_, AppState>, name: String, text: String, html: Option<String>) -> Result<()> {
    let ctx = state.ctx().await?;
    let profile = ctx.profile().await;
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::Other("Укажите название шаблона".into()));
    }
    let msg = smtp::build(smtp::Outgoing {
        from_name: Some(profile.display_name.clone()),
        from_address: profile.email(),
        to: vec![profile.email()],
        cc: vec![],
        subject: name,
        body: text,
        html,
        in_reply_to: None,
        references: None,
        attachments: vec![],
        read_receipt: false,
        extra_headers: vec![],
    })?;
    let raw = msg.formatted();
    with_conn!(once state, c => imap::append_template(c, &raw))
}

#[tauri::command]
pub async fn template_delete(state: State<'_, AppState>, folder: String, uid: u32) -> Result<()> {
    with_conn!(state, c => imap::remove_message(c, &folder, uid))
}

/// Файл, перетащенный в письмо: содержимое приходит из окна (base64), сохраняется
/// в личный временный каталог и дальше вкладывается как выбранный в диалоге.
#[tauri::command]
pub async fn attach_data(state: State<'_, AppState>, name: String, data: String) -> Result<PickedFile> {
    let ctx = state.ctx().await?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|_| AppError::Other("Файл не прочитан".into()))?;
    if bytes.len() as u64 > MAX_OUTGOING_MB * 1024 * 1024 {
        return Err(AppError::Other(format!("Файл больше {MAX_OUTGOING_MB} МБ — отправьте его ссылкой Nextcloud")));
    }
    let safe = imap::sanitize_filename(&name);
    let dir = crate::updater::private_dir("drop")?;
    let path = dir.join(if safe.is_empty() { "file".to_string() } else { safe.clone() });
    {
        use std::io::Write as _;
        let mut f = crate::updater::create_new_file(&path)?;
        f.write_all(&bytes)?;
    }
    let id = state.next_pick.fetch_add(1, Ordering::Relaxed).to_string();
    ctx.picked.lock().await.insert(id.clone(), path);
    Ok(PickedFile { id, name: safe, size: bytes.len() as u64 })
}

/// Ctrl+V: файлы, скопированные в проводнике, или картинка из буфера обмена — вложениями
/// (когда WebView сам их не отдаёт). Пусто — в буфере только текст.
#[tauri::command]
pub async fn clipboard_paste_files(state: State<'_, AppState>) -> Result<Vec<PickedFile>> {
    let ctx = state.ctx().await?;
    let clip = tauri::async_runtime::spawn_blocking(crate::clipboard::read).await.map_err(|e| AppError::Other(e.to_string()))??;
    let limit = MAX_OUTGOING_MB * 1024 * 1024;
    let mut out = Vec::new();
    match clip {
        crate::clipboard::Clip::Nothing => {}
        crate::clipboard::Clip::Files(paths) => {
            let mut picked = ctx.picked.lock().await;
            for path in paths {
                let size = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
                if size > limit {
                    return Err(AppError::Other(format!("«{name}» больше {MAX_OUTGOING_MB} МБ — отправьте его ссылкой Nextcloud")));
                }
                let id = state.next_pick.fetch_add(1, Ordering::Relaxed).to_string();
                picked.insert(id.clone(), path);
                out.push(PickedFile { id, name, size });
            }
        }
        crate::clipboard::Clip::Png(bytes) => {
            if bytes.len() as u64 > limit {
                return Err(AppError::Other(format!("Картинка больше {MAX_OUTGOING_MB} МБ")));
            }
            let name = format!("Снимок {}.png", chrono::Local::now().format("%d-%m-%Y %H-%M-%S"));
            let dir = crate::updater::private_dir("drop")?;
            let path = dir.join(&name);
            {
                use std::io::Write as _;
                let mut f = crate::updater::create_new_file(&path)?;
                f.write_all(&bytes)?;
            }
            let id = state.next_pick.fetch_add(1, Ordering::Relaxed).to_string();
            ctx.picked.lock().await.insert(id.clone(), path);
            out.push(PickedFile { id, name, size: bytes.len() as u64 });
        }
    }
    Ok(out)
}

/// Превью вставленной картинки (data: URL) — только файлов, выбранных в этом сеансе.
#[tauri::command]
pub async fn picked_preview(state: State<'_, AppState>, id: String) -> Result<Option<String>> {
    let ctx = state.ctx().await?;
    let Some(path) = ctx.picked.lock().await.get(&id).cloned() else { return Ok(None) };
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => return Ok(None),
    };
    let meta = tokio::fs::metadata(&path).await?;
    if meta.len() > 10 * 1024 * 1024 {
        return Ok(None);
    }
    use base64::Engine as _;
    let data = tokio::fs::read(&path).await?;
    Ok(Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(data))))
}

/// Сбрасывает выбранный, но не отправленный файл.
#[tauri::command]
pub async fn forget_attachment(state: State<'_, AppState>, attachment: String) -> Result<()> {
    let ctx = state.ctx().await?;
    ctx.picked.lock().await.remove(&attachment);
    Ok(())
}
