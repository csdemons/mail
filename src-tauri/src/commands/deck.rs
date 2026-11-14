//! Задачи Nextcloud Deck.

use super::*;

#[tauri::command]
pub async fn deck_boards(state: State<'_, AppState>) -> Result<Vec<nextcloud::DeckBoard>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_boards().await
}

#[tauri::command]
pub async fn deck_stacks(state: State<'_, AppState>, board_id: i64) -> Result<Vec<nextcloud::DeckStack>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_stacks(board_id).await
}

#[tauri::command]
pub async fn deck_create_card(
    state: State<'_, AppState>,
    board_id: i64,
    stack_id: i64,
    title: String,
    description: String,
    duedate: Option<String>,
) -> Result<i64> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_create_card(board_id, stack_id, &title, &description, duedate.as_deref()).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn deck_update_card(
    state: State<'_, AppState>,
    board_id: i64,
    stack_id: i64,
    card_id: i64,
    title: String,
    description: String,
    duedate: Option<String>,
    done: bool,
) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_update_card(board_id, stack_id, card_id, &title, &description, duedate.as_deref(), done).await
}

#[tauri::command]
pub async fn deck_move_card(
    state: State<'_, AppState>,
    board_id: i64,
    stack_id: i64,
    card_id: i64,
    target_stack_id: i64,
    order: i64,
) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_move_card(board_id, stack_id, card_id, target_stack_id, order).await
}

#[tauri::command]
pub async fn deck_delete_card(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_delete_card(board_id, stack_id, card_id).await
}

#[tauri::command]
pub async fn deck_board_details(state: State<'_, AppState>, board_id: i64) -> Result<nextcloud::DeckBoardDetails> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_board_details(board_id).await
}

#[tauri::command]
pub async fn deck_assign(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64, uid: String, assign: bool) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_assign(board_id, stack_id, card_id, &uid, assign).await
}

#[tauri::command]
pub async fn deck_label(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64, label_id: i64, add: bool) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_label(board_id, stack_id, card_id, label_id, add).await
}

#[tauri::command]
pub async fn deck_comments(state: State<'_, AppState>, card_id: i64) -> Result<Vec<nextcloud::DeckComment>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_comments(card_id).await
}

#[tauri::command]
pub async fn deck_comment_add(state: State<'_, AppState>, card_id: i64, message: String) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_comment_add(card_id, &message).await
}

#[tauri::command]
pub async fn deck_comment_delete(state: State<'_, AppState>, card_id: i64, comment_id: i64) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_comment_delete(card_id, comment_id).await
}

/// Добавить к карточке файл, выбранный через `pick_attachments` (не больше 50 МБ).
#[tauri::command]
pub async fn deck_attachment_add(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64, attachment: String) -> Result<()> {
    let ctx = state.ctx().await?;
    let path = ctx.picked.lock().await.get(&attachment).cloned().ok_or_else(|| AppError::Other("Файл не найден, выберите его заново".into()))?;
    if tokio::fs::metadata(&path).await?.len() > 50 * 1024 * 1024 {
        return Err(AppError::Other("Файл больше 50 МБ — положите его в Nextcloud и добавьте ссылку".into()));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
    let data = tokio::fs::read(&path).await?;
    nc(&ctx).await?.deck_attachment_upload(board_id, stack_id, card_id, &name, data).await?;
    ctx.picked.lock().await.remove(&attachment);
    Ok(())
}

#[tauri::command]
pub async fn deck_attachment_delete(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64, attachment_id: i64, kind: String) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_attachment_delete(board_id, stack_id, card_id, attachment_id, &kind).await
}

#[tauri::command]
pub async fn deck_archive(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64, archive: bool) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_archive(board_id, stack_id, card_id, archive).await
}

#[tauri::command]
pub async fn deck_activity(state: State<'_, AppState>, card_id: i64) -> Result<Vec<nextcloud::DeckActivity>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_activity(card_id).await
}

#[tauri::command]
pub async fn deck_attachments(state: State<'_, AppState>, board_id: i64, stack_id: i64, card_id: i64) -> Result<Vec<nextcloud::DeckAttachment>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_attachments(board_id, stack_id, card_id).await
}

#[tauri::command]
pub async fn deck_create_board(state: State<'_, AppState>, title: String, color: String, stacks: Vec<String>) -> Result<nextcloud::DeckBoard> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_create_board(&title, &color, &stacks).await
}

#[tauri::command]
pub async fn deck_create_stack(state: State<'_, AppState>, board_id: i64, title: String) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.deck_create_stack(board_id, &title).await
}
