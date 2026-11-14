//! Локальное хранилище писем и работа без сети (очередь действий, повтор при восстановлении связи).

use super::*;

/// Срок хранения писем на компьютере: настройка пользователя в пределах политики.
pub(crate) fn offline_days(profile: &Settings) -> u32 {
    profile.offline_days.unwrap_or(store::DEFAULT_DAYS).min(crate::policy::load().max_offline_days())
}

pub(crate) async fn open_store(app: &AppHandle, ctx: &AccountCtx) {
    close_store(ctx).await;
    let Ok(root) = host::data_dir(app) else { return };
    let account = ctx.key.clone();
    if !crate::policy::load().local_store() {
        // Администратор запретил хранить почту на компьютере — убрать то, что было.
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || store::wipe(&root, &account)).await {
            log::warn!("local store not removed: {e}");
        }
        return;
    }
    let days = offline_days(&ctx.profile().await);
    *ctx.store_error.lock().await = None;
    match tokio::task::spawn_blocking(move || store::Store::open(&root, &account, days)).await {
        Ok(Ok(st)) => {
            let st = Arc::new(st);
            *ctx.store.lock().await = Some(st.clone());
            tauri::async_runtime::spawn(store::run_sync(st, ctx.creds.clone()));
        }
        Ok(Err(e)) => {
            log::warn!("local store unavailable: {e}");
            *ctx.store_error.lock().await = Some(e.to_string());
        }
        Err(e) => log::warn!("local store: {e}"),
    }
}

pub(crate) async fn close_store(ctx: &AccountCtx) {
    if let Some(st) = ctx.store.lock().await.take() {
        let _ = tokio::task::spawn_blocking(move || st.close()).await;
    }
}

pub(crate) async fn local(ctx: &AccountCtx) -> Option<Arc<store::Store>> {
    ctx.store.lock().await.clone()
}

pub(crate) async fn local_raw(ctx: &AccountCtx, folder: &str, uid: u32) -> Option<Vec<u8>> {
    local(ctx).await?.raw(folder, uid)
}

/// Вложение: из локального хранилища, иначе с сервера.
pub(crate) async fn attachment(ctx: &AccountCtx, folder: &str, uid: u32, index: usize) -> Result<(String, Vec<u8>)> {
    if let Some(raw) = local_raw(ctx, folder, uid).await {
        return imap::attachment_in(&raw, index);
    }
    with_ctx!(ctx, c => imap::get_attachment(c, folder, uid, index))
}

/// Папки из локального хранилища — показываются сразу, до ответа сервера.
#[tauri::command]
pub async fn cached_folders(state: State<'_, AppState>) -> Result<Option<Vec<Folder>>> {
    let ctx = state.ctx().await?;
    Ok(local(&ctx).await.and_then(|st| st.folders()))
}

/// Письма папки из локального хранилища (затем интерфейс запрашивает сервер).
#[tauri::command]
pub async fn cached_messages(state: State<'_, AppState>, folder: String, before_uid: Option<u32>, limit: u32) -> Result<Option<MessagePage>> {
    let ctx = state.ctx().await?;
    Ok(local(&ctx).await.and_then(|st| st.page(&folder, before_uid, limit.clamp(1, 200) as usize)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineStatus {
    /// Хранилище работает в этой сессии.
    pub(crate) enabled: bool,
    /// Разрешено администратором.
    pub(crate) allowed: bool,
    /// Срок хранения, выбранный пользователем, и наибольший по политике.
    pub(crate) days: u32,
    pub(crate) max_days: u32,
    pub(crate) messages: usize,
    pub(crate) cached: usize,
    pub(crate) syncing: bool,
    pub(crate) bytes: u64,
    pub(crate) error: Option<String>,
}

#[tauri::command]
pub async fn offline_status(state: State<'_, AppState>) -> Result<OfflineStatus> {
    let ctx = state.ctx().await?;
    let policy = crate::policy::load();
    let profile = ctx.profile().await;
    let error = ctx.store_error.lock().await.clone();
    let st = match local(&ctx).await {
        Some(st) => Some(tokio::task::spawn_blocking(move || st.status()).await.map_err(|e| AppError::Other(e.to_string()))?),
        None => None,
    };
    Ok(OfflineStatus {
        enabled: st.is_some(),
        allowed: policy.local_store(),
        days: offline_days(&profile),
        max_days: policy.max_offline_days(),
        messages: st.map_or(0, |s| s.messages),
        cached: st.map_or(0, |s| s.cached),
        syncing: st.is_some_and(|s| s.syncing),
        bytes: st.map_or(0, |s| s.bytes),
        error,
    })
}

/// Срок хранения писем на компьютере; хранилище открывается заново с новым сроком.
#[tauri::command]
pub async fn set_offline_days(app: AppHandle, state: State<'_, AppState>, days: u32) -> Result<()> {
    let ctx = state.ctx().await?;
    let profile = {
        let mut p = ctx.profile.lock().await;
        p.offline_days = Some(days.min(3650));
        p.clone()
    };
    settings::save(&app, &profile)?;
    open_store(&app, &ctx).await;
    Ok(())
}

/// Удалить письма, хранимые на компьютере (индекс соберётся заново).
#[tauri::command]
pub async fn clear_offline(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let ctx = state.ctx().await?;
    if local(&ctx).await.is_some() {
        let n = with_queue(&ctx, |q| q.items.len()).await.unwrap_or(0);
        if n > 0 {
            return Err(AppError::Other(format!("В очереди без сети {n} действий или писем — дождитесь подключения или уберите их из очереди, затем очистите хранилище")));
        }
    }
    close_store(&ctx).await;
    let root = host::data_dir(&app)?;
    let account = ctx.key.clone();
    tokio::task::spawn_blocking(move || store::wipe(&root, &account)).await.map_err(|e| AppError::Other(e.to_string()))??;
    open_store(&app, &ctx).await;
    Ok(())
}

/// Текст, который видит пользователь, когда письмо ушло в очередь исходящих.
pub(crate) const QUEUED_SEND: &str = "Нет связи с почтовым сервером — письмо в очереди исходящих и уйдёт, как только связь появится.";

/// Очередь учётной записи: из хранилища на компьютере при первом обращении.
/// Без хранилища (выключено администратором) очереди нет.
pub(crate) async fn with_queue<R>(ctx: &AccountCtx, f: impl FnOnce(&mut offline::Queue) -> R) -> Result<R> {
    let st = local(ctx).await.ok_or_else(|| AppError::Other("Нет связи с почтовым сервером, а хранение писем на компьютере выключено — действие не выполнено".into()))?;
    let mut guard = ctx.queue.lock().await;
    if guard.is_none() {
        let st2 = st.clone();
        let raw = tokio::task::spawn_blocking(move || st2.load_blob(offline::BLOB)).await.map_err(|e| AppError::Other(e.to_string()))?;
        *guard = Some(offline::Queue::from_bytes(raw));
    }
    let q = guard.as_mut().expect("loaded above");
    let before = q.clone();
    let out = f(q);
    if *q != before {
        let bytes = q.to_bytes();
        tokio::task::spawn_blocking(move || st.save_blob(offline::BLOB, &bytes)).await.map_err(|e| AppError::Other(e.to_string()))??;
    }
    Ok(out)
}

/// В очереди есть невыполненные действия (новые тогда тоже идут в очередь — по порядку).
pub(crate) async fn queue_busy(ctx: &AccountCtx) -> bool {
    if local(ctx).await.is_none() {
        return false;
    }
    with_queue(ctx, |q| !q.items.is_empty()).await.unwrap_or(false)
}

/// Действие сразу в очередь (без связи или пока очередь не выполнена).
pub(crate) async fn queue_first(ctx: &AccountCtx) -> bool {
    !ctx.online() || queue_busy(ctx).await
}

/// Ошибка, после которой действие ставится в очередь, а не показывается.
pub(crate) async fn queue_on(ctx: &AccountCtx, e: &AppError) -> bool {
    if e.is_offline() {
        if !matches!(e, AppError::Timeout) {
            ctx.went_offline(e);
        }
        return local(ctx).await.is_some();
    }
    false
}

pub(crate) async fn enqueue(ctx: &Arc<AccountCtx>, op: Op) -> Result<()> {
    with_queue(ctx, |q| q.push(op)).await??;
    if ctx.online() {
        tauri::async_runtime::spawn(replay(ctx.clone()));
    }
    Ok(())
}

/// Письма для очереди: UID и Message-ID из индекса на компьютере.
pub(crate) async fn targets(ctx: &AccountCtx, folder: &str, uids: &[u32]) -> Vec<Target> {
    let st = local(ctx).await;
    // Exchange: номера писем привязаны к ItemId (своя таблица), UIDVALIDITY нет.
    let validity = if ctx.creds.ews.is_some() { None } else { st.as_ref().and_then(|s| s.uid_validity(folder)) };
    uids.iter()
        .map(|u| Target { uid: *u, msg_id: st.as_ref().and_then(|s| s.summary(folder, *u)).and_then(|m| m.message_id), validity })
        .collect()
}

pub(crate) fn flag_name(flag: FlagKind) -> &'static str {
    match flag {
        FlagKind::Seen => "seen",
        FlagKind::Flagged => "flagged",
        FlagKind::Important => "important",
    }
}

pub(crate) fn flag_kind(name: &str) -> Option<FlagKind> {
    Some(match name {
        "seen" => FlagKind::Seen,
        "flagged" => FlagKind::Flagged,
        "important" => FlagKind::Important,
        _ => return None,
    })
}

/// Тема и адресаты исходящего письма — для списка очереди.
pub(crate) fn outgoing_meta(raw: &[u8]) -> (String, Vec<String>) {
    let msg = mail_parser::MessageParser::default().parse_headers(raw);
    let subject = msg.as_ref().and_then(|m| m.subject()).unwrap_or_default().to_string();
    let to = msg
        .as_ref()
        .map(|m| [m.to(), m.cc()].into_iter().flatten().flat_map(|l| l.iter().filter_map(|a| a.address().map(str::to_string))).collect())
        .unwrap_or_default();
    (subject, to)
}

/// Отправить письмо, а без связи — поставить в очередь исходящих.
pub(crate) async fn deliver_or_queue(ctx: &Arc<AccountCtx>, raw: &[u8]) -> Result<SendResult> {
    let queue = |ctx: Arc<AccountCtx>, raw: Vec<u8>| async move {
        let (subject, to) = outgoing_meta(&raw);
        enqueue(&ctx, Op::Send { raw: offline::encode(&raw), subject, to }).await?;
        Ok(SendResult { saved_to: None, warning: Some(QUEUED_SEND.to_string()), queued: true })
    };
    if queue_first(ctx).await && local(ctx).await.is_some() {
        return queue(ctx.clone(), raw.to_vec()).await;
    }
    match deliver(ctx, raw, None).await {
        Err(e) if queue_on(ctx, &e).await => queue(ctx.clone(), raw.to_vec()).await,
        other => other,
    }
}

/// Выполнить одно действие из очереди. `Ok(Some(причина))` — конфликт: действие
/// не выполнено (письма уже нет и т. п.), но и повторять его незачем.
pub(crate) async fn run_op(ctx: &Arc<AccountCtx>, op: &Op) -> Result<Option<String>> {
    let gone = |n: usize, folder: &str| {
        let f = utf7_imap::decode_utf7_imap(folder.to_string());
        if n == 1 {
            format!("письма уже нет в «{f}» на сервере (его переместили или удалили в другом месте)")
        } else {
            format!("{n} писем уже нет в «{f}» на сервере (их переместили или удалили в другом месте)")
        }
    };
    match op {
        Op::Flag { folder, targets, flag, value } => {
            let flag = flag_kind(flag).ok_or_else(|| AppError::Other("неизвестная отметка".into()))?;
            let (folder, value) = (folder.clone(), *value);
            let found = with_ctx!(ctx, c => imap::locate(c, &folder, targets))?;
            let uids: Vec<u32> = found.iter().flatten().copied().collect();
            if !uids.is_empty() {
                with_ctx!(ctx, c => imap::set_flag_many(c, &folder, &uids, flag, value))?;
            }
            Ok((uids.len() < targets.len()).then(|| gone(targets.len() - uids.len(), &folder)))
        }
        Op::Keyword { folder, targets, keyword, value } => {
            let found = with_ctx!(ctx, c => imap::locate(c, folder, targets))?;
            let uids: Vec<u32> = found.iter().flatten().copied().collect();
            if !uids.is_empty() {
                with_ctx!(ctx, c => imap::set_keyword(c, folder, &uids, keyword, *value))?;
            }
            Ok((uids.len() < targets.len()).then(|| gone(targets.len() - uids.len(), folder)))
        }
        Op::Due { folder, target, due } => {
            let due = due.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map(|d| d.with_timezone(&chrono::Utc));
            let found = with_ctx!(ctx, c => imap::locate(c, folder, std::slice::from_ref(target)))?;
            match found.first().copied().flatten() {
                Some(uid) => {
                    with_ctx!(ctx, c => imap::set_due(c, folder, uid, due))?;
                    Ok(None)
                }
                None => Ok(Some(gone(1, folder))),
            }
        }
        Op::Move { folder, targets, target } => {
            let found = with_ctx!(ctx, c => imap::locate(c, folder, targets))?;
            let uids: Vec<u32> = found.iter().flatten().copied().collect();
            if !uids.is_empty() {
                with_ctx!(ctx, c => imap::move_messages(c, folder, &uids, target))?;
            }
            // Чего нет на месте — уже в нужной папке? Тогда перемещать нечего.
            let missing: Vec<Target> = targets.iter().zip(&found).filter(|(_, f)| f.is_none()).map(|(t, _)| Target { uid: 0, msg_id: t.msg_id.clone(), validity: None }).collect();
            if missing.is_empty() {
                return Ok(None);
            }
            let there = with_ctx!(ctx, c => imap::locate(c, target, &missing))?;
            let lost = there.iter().filter(|f| f.is_none()).count();
            Ok((lost > 0).then(|| gone(lost, folder)))
        }
        Op::Delete { folder, targets, trash } => {
            let found = with_ctx!(ctx, c => imap::locate(c, folder, targets))?;
            let uids: Vec<u32> = found.iter().flatten().copied().collect();
            if !uids.is_empty() {
                with_ctx!(ctx, c => imap::delete_messages(c, folder, &uids, trash.as_deref()))?;
            }
            // Письма уже нет — удалять нечего.
            Ok(None)
        }
        Op::Send { raw, .. } => {
            let raw = offline::decode(raw)?;
            match deliver(ctx, &raw, None).await {
                Ok(r) => {
                    if let Some(w) = r.warning {
                        log::warn!("queued mail: {w}");
                    }
                    Ok(None)
                }
                Err(e) => Err(e),
            }
        }
        Op::Schedule { raw, .. } => {
            let raw = offline::decode(raw)?;
            with_ctx!(once ctx, c => imap::append_draft(c, &raw))?;
            Ok(None)
        }
    }
}

/// Выполнить очередь на сервере по порядку. Без связи останавливается — продолжит
/// планировщик после подключения.
pub(crate) async fn replay(ctx: Arc<AccountCtx>) {
    let Ok(_one) = ctx.replaying.try_lock() else { return };
    ctx.replay_active.store(true, Ordering::SeqCst);
    loop {
        if ctx.is_closed() || !ctx.online() {
            break;
        }
        let Ok(Some(item)) = with_queue(&ctx, |q| q.items.first().cloned()).await else { break };
        let what = item.op.describe();
        let result = run_op(&ctx, &item.op).await;
        let stop = match result {
            Ok(conflict) => {
                let _ = with_queue(&ctx, |q| {
                    q.items.retain(|i| i.id != item.id);
                    if let Some(reason) = conflict {
                        q.conflict(what.clone(), reason, None);
                    }
                })
                .await;
                false
            }
            Err(e) if e.is_offline() && !matches!(e, AppError::Timeout) => {
                ctx.went_offline(&e);
                true
            }
            // Письмо могло уйти: само не повторяем — решает пользователь.
            Err(AppError::DeliveryUnknown(m)) => {
                let _ = with_queue(&ctx, |q| {
                    q.items.retain(|i| i.id != item.id);
                    q.conflict(what.clone(), format!("связь оборвалась во время отправки — неизвестно, ушло ли письмо ({m})"), Some(item.op.clone()));
                })
                .await;
                false
            }
            Err(e) => {
                // Сервер ответил ошибкой: несколько попыток, затем — в конфликты.
                let text = e.to_string();
                let gave_up = with_queue(&ctx, |q| {
                    let Some(i) = q.items.iter_mut().find(|i| i.id == item.id) else { return false };
                    i.attempts += 1;
                    i.error = Some(text.clone());
                    if i.attempts < offline::MAX_ATTEMPTS {
                        return false;
                    }
                    let op = i.op.clone();
                    q.items.retain(|x| x.id != item.id);
                    let retry = op.is_send().then_some(op);
                    q.conflict(what.clone(), format!("сервер не выполнил: {text}"), retry);
                    true
                })
                .await
                .unwrap_or(false);
                !gave_up
            }
        };
        if stop {
            break;
        }
    }
    ctx.replay_active.store(false, Ordering::SeqCst);
}

/// Подключиться снова (планировщик без связи, кнопка «Проверить связь»).
pub(crate) async fn try_reconnect(ctx: &Arc<AccountCtx>) -> bool {
    match tokio::time::timeout(Duration::from_secs(20), imap::connect(&ctx.creds)).await {
        Ok(Ok(conn)) => {
            let old = ctx.conn.lock().await.replace(conn);
            if let Some(old) = old {
                old.logout().await;
            }
            ctx.came_online();
            if let Some(st) = local(ctx).await {
                st.wake();
            }
            tauri::async_runtime::spawn(replay(ctx.clone()));
            true
        }
        Ok(Err(e)) => {
            ctx.went_offline(&e);
            false
        }
        Err(_) => {
            ctx.went_offline(&AppError::Timeout);
            false
        }
    }
}

pub(crate) async fn offline_status_of(ctx: &AccountCtx) -> offline::Status {
    let available = local(ctx).await.is_some();
    let (items, conflicts) = if available { with_queue(ctx, |q| q.views()).await.unwrap_or_default() } else { Default::default() };
    let info = ctx.offline_info.lock().unwrap_or_else(|p| p.into_inner()).clone();
    offline::Status {
        online: ctx.online(),
        syncing: ctx.replay_active.load(Ordering::SeqCst),
        available,
        offline_since: info.as_ref().map(|(s, _)| s.clone()),
        last_error: info.map(|(_, e)| e),
        items,
        conflicts,
    }
}

/// Связь с сервером и очередь действий без сети.
#[tauri::command]
pub async fn offline_queue(state: State<'_, AppState>) -> Result<offline::Status> {
    let ctx = state.ctx().await?;
    Ok(offline_status_of(&ctx).await)
}

/// Проверить связь сейчас (не дожидаясь планировщика) и выполнить очередь.
#[tauri::command]
pub async fn offline_check(state: State<'_, AppState>) -> Result<offline::Status> {
    let ctx = state.ctx().await?;
    if ctx.online() {
        tauri::async_runtime::spawn(replay(ctx.clone()));
    } else {
        try_reconnect(&ctx).await;
    }
    Ok(offline_status_of(&ctx).await)
}

/// Убрать действие из очереди (`item`) или конфликт из списка; `retry` — письмо
/// с неизвестным результатом отправки снова поставить в очередь.
#[tauri::command]
pub async fn offline_resolve(state: State<'_, AppState>, id: u64, retry: Option<bool>) -> Result<offline::Status> {
    let ctx = state.ctx().await?;
    let again = with_queue(&ctx, |q| {
        if let Some(pos) = q.conflicts.iter().position(|c| c.id == id) {
            let c = q.conflicts.remove(pos);
            return if retry == Some(true) { c.retry } else { None };
        }
        q.items.retain(|i| i.id != id);
        None
    })
    .await?;
    if let Some(op) = again {
        enqueue(&ctx, op).await?;
    }
    Ok(offline_status_of(&ctx).await)
}
