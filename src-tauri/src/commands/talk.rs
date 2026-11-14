//! Чат Nextcloud Talk.

use super::*;

/// Уменьшенная копия файла из чата (data: URL) или None, если сервер её не дал.
#[tauri::command]
pub async fn talk_file_preview(state: State<'_, AppState>, file_id: String) -> Result<Option<String>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.file_preview(&file_id, 480).await
}

/// Содержимое файла из чата (base64) для встроенного просмотра: фото, PDF, текст.
#[tauri::command]
pub async fn talk_file_data(state: State<'_, AppState>, path: String) -> Result<String> {
    let ctx = state.ctx().await?;
    use base64::Engine as _;
    let bytes = nc(&ctx).await?.files_read(&path, 50 * 1024 * 1024).await?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[tauri::command]
pub async fn talk_pin(state: State<'_, AppState>, token: String, message_id: i64, until: i64) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_pin(&token, message_id, until).await
}

#[tauri::command]
pub async fn talk_unpin(state: State<'_, AppState>, token: String, message_id: i64, only_me: bool) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_unpin(&token, message_id, only_me).await
}

#[tauri::command]
pub async fn talk_reminder(state: State<'_, AppState>, token: String, message_id: i64) -> Result<Option<i64>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_reminder(&token, message_id).await
}

#[tauri::command]
pub async fn talk_set_reminder(state: State<'_, AppState>, token: String, message_id: i64, at: Option<i64>) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_set_reminder(&token, message_id, at).await
}

#[tauri::command]
pub async fn talk_rooms(state: State<'_, AppState>) -> Result<Vec<nextcloud::TalkRoom>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_rooms().await
}

#[tauri::command]
pub async fn talk_history(state: State<'_, AppState>, token: String, before: Option<i64>) -> Result<nextcloud::TalkChunk> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_history(&token, before).await
}

#[tauri::command]
pub async fn talk_poll(state: State<'_, AppState>, token: String, last_known: i64) -> Result<nextcloud::TalkChunk> {
    let ctx = state.ctx().await?;
    let (creds, uid) = ctx.nc.lock().await.clone().ok_or_else(|| AppError::Other("Nextcloud не подключён".into()))?;
    nextcloud::Nc::long_poll(creds, &uid)?.talk_poll(&token, last_known).await
}

#[tauri::command]
pub async fn talk_send(state: State<'_, AppState>, token: String, message: String, reply_to: Option<i64>, silent: Option<bool>) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_send(&token, &message, reply_to, silent.unwrap_or(false)).await
}

#[tauri::command]
pub async fn talk_poll_create(state: State<'_, AppState>, token: String, question: String, options: Vec<String>, max_votes: i64, hidden: bool) -> Result<crate::nextcloud::TalkPoll> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_poll_create(&token, &question, &options, max_votes, hidden).await
}

#[tauri::command]
pub async fn talk_poll_get(state: State<'_, AppState>, token: String, id: i64) -> Result<crate::nextcloud::TalkPoll> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_poll_get(&token, id).await
}

#[tauri::command]
pub async fn talk_poll_vote(state: State<'_, AppState>, token: String, id: i64, options: Vec<i64>) -> Result<crate::nextcloud::TalkPoll> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_poll_vote(&token, id, &options).await
}

#[tauri::command]
pub async fn talk_poll_close(state: State<'_, AppState>, token: String, id: i64) -> Result<crate::nextcloud::TalkPoll> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_poll_close(&token, id).await
}

/// Медиа, файлы, голосовые и т. п. беседы — для вкладки «Медиа и файлы».
#[tauri::command]
pub async fn talk_shared(state: State<'_, AppState>, token: String, kind: String, before: Option<i64>) -> Result<Vec<crate::nextcloud::TalkMessage>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_shared(&token, &kind, before).await
}

/// Звонок Talk в отдельном окне программы (страница звонка Nextcloud). Окно — обычная
/// веб-страница сервера: доступа к командам программы у неё нет.
#[tauri::command]
pub async fn talk_call_window(app: AppHandle, state: State<'_, AppState>, token: String) -> Result<()> {
    if token.is_empty() || !token.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(AppError::Other("Неверная беседа".into()));
    }
    let ctx = state.ctx().await?;
    let (creds, _) = ctx.nc.lock().await.clone().ok_or_else(|| AppError::Other("Nextcloud не подключён".into()))?;
    let url = creds.base.join(&format!("index.php/call/{token}")).map_err(|e| AppError::Other(e.to_string()))?;
    // E2E (отладочная сборка): окно не открываем — проверяется адрес.
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("RRCKI_E2E_SAVE_DIR") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(std::path::Path::new(&dir).join("call-window.txt"), url.as_str());
        return Ok(());
    }
    host::open_external_window(&app, &format!("call-{token}"), "Звонок — Atom Workspace", url, &web_profile(&app, &ctx.key)?)
}

#[tauri::command]
pub async fn talk_search(state: State<'_, AppState>, token: String, term: String) -> Result<Vec<crate::nextcloud::TalkSearchHit>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_search(&token, &term).await
}

#[tauri::command]
pub async fn talk_participants(state: State<'_, AppState>, token: String) -> Result<Vec<crate::nextcloud::TalkParticipant>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_participants(&token).await
}

/// Действия с беседой: участники, роли, название, описание, режимы, уведомления, выход, удаление.
#[tauri::command]
pub async fn talk_manage(state: State<'_, AppState>, token: String, action: String, value: serde_json::Value) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_manage(&token, &action, &value).await
}

#[tauri::command]
pub async fn talk_favorite(state: State<'_, AppState>, token: String, on: bool) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_favorite(&token, on).await
}

#[tauri::command]
pub async fn talk_react(state: State<'_, AppState>, token: String, message_id: i64, emoji: String, add: bool) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_react(&token, message_id, &emoji, add).await
}

#[tauri::command]
pub async fn talk_edit(state: State<'_, AppState>, token: String, message_id: i64, message: String) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_edit(&token, message_id, &message).await
}

#[tauri::command]
pub async fn talk_delete(state: State<'_, AppState>, token: String, message_id: i64) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_delete(&token, message_id).await
}

#[tauri::command]
pub async fn talk_mentions(state: State<'_, AppState>, token: String, search: String) -> Result<Vec<nextcloud::TalkMention>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_mentions(&token, &search).await
}

#[tauri::command]
pub async fn talk_create(state: State<'_, AppState>, users: Vec<String>, name: Option<String>) -> Result<String> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.talk_create(&users, name.as_deref()).await
}

pub(crate) const MAX_TALK_FILE_MB: u64 = 512;

/// Отправляет в беседу файл, выбранный через `pick_attachments`.
#[tauri::command]
pub async fn talk_send_file(state: State<'_, AppState>, token: String, attachment: String, caption: String, reply_to: Option<i64>, voice: Option<bool>, silent: Option<bool>) -> Result<()> {
    let ctx = state.ctx().await?;
    let (creds, uid) = ctx.nc.lock().await.clone().ok_or_else(|| AppError::Other("Nextcloud не подключён".into()))?;
    let path = ctx
        .picked
        .lock()
        .await
        .get(&attachment)
        .cloned()
        .ok_or_else(|| AppError::Other("Файл не найден, выберите его заново".into()))?;
    let size = tokio::fs::metadata(&path).await?.len();
    if size > MAX_TALK_FILE_MB * 1024 * 1024 {
        return Err(AppError::Other(format!("Файл больше {MAX_TALK_FILE_MB} МБ — отправьте ссылку из Nextcloud")));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
    nextcloud::Nc::upload(creds, &uid)?.talk_send_file(&token, &name, nextcloud::UploadSource::File(path), &caption, nextcloud::TalkFileOpts { reply_to, voice: voice.unwrap_or(false), silent: silent.unwrap_or(false) }).await?;
    ctx.picked.lock().await.remove(&attachment);
    Ok(())
}
