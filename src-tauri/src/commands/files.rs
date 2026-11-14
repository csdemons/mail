//! Файлы Nextcloud: список, загрузка, общий доступ, ссылки на вложения.

use super::*;

#[tauri::command]
pub async fn nc_files_list(state: State<'_, AppState>, path: String) -> Result<Vec<nextcloud::NcFile>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.files_list(&path).await
}

#[tauri::command]
pub async fn nc_files_mkdir(state: State<'_, AppState>, parent: String, name: String) -> Result<String> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.files_mkdir(&parent, &name).await
}

/// Выбранные пользователем файлы (по id из `pick_attachments`): имя, путь и размер.
/// Содержимое не читается — оно уйдёт на сервер потоком.
pub(crate) async fn picked_files(ctx: &AccountCtx, ids: &[String]) -> Result<Vec<(String, String, PathBuf)>> {
    let mut out = Vec::new();
    for id in ids {
        let path = ctx.picked.lock().await.get(id).cloned().ok_or_else(|| AppError::Other("Файл не найден, выберите его заново".into()))?;
        let size = tokio::fs::metadata(&path).await?.len();
        if size > MAX_TALK_FILE_MB * 1024 * 1024 {
            return Err(AppError::Other(format!("Файл больше {MAX_TALK_FILE_MB} МБ")));
        }
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        out.push((id.clone(), name, path));
    }
    Ok(out)
}

pub(crate) async fn nc_upload(ctx: &AccountCtx) -> Result<nextcloud::Nc> {
    let (creds, uid) = ctx.nc.lock().await.clone().ok_or_else(|| AppError::Other("Nextcloud не подключён — подключите его в Настройках".into()))?;
    nextcloud::Nc::upload(creds, &uid)
}

/// Загружает выбранные файлы в папку Nextcloud. Возвращает пути.
#[tauri::command]
pub async fn nc_files_upload(state: State<'_, AppState>, dir: String, attachments: Vec<String>) -> Result<Vec<String>> {
    let ctx = state.ctx().await?;
    let files = picked_files(&ctx, &attachments).await?;
    let client = nc_upload(&ctx).await?;
    let mut paths = Vec::new();
    for (id, name, path) in files {
        paths.push(client.upload_unique_src(&dir, &name, nextcloud::UploadSource::File(path)).await?);
        ctx.picked.lock().await.remove(&id);
    }
    Ok(paths)
}

/// Скачивает файл: куда сохранить, выбирает пользователь в системном диалоге.
#[tauri::command]
pub async fn nc_files_download(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<Option<String>> {
    let ctx = state.ctx().await?;
    let name = path.rsplit('/').next().unwrap_or("file").to_string();
    let Some(target) = host::save_file(&app, &name).await else {
        return Ok(None);
    };
    nc_upload(&ctx).await?.files_download_to(&path, &target).await?;
    Ok(Some(target.display().to_string()))
}

#[tauri::command]
pub async fn nc_files_delete(state: State<'_, AppState>, path: String) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.files_delete(&path).await
}

#[tauri::command]
pub async fn nc_files_rename(state: State<'_, AppState>, path: String, name: String) -> Result<String> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.files_rename(&path, &name).await
}

#[tauri::command]
pub async fn nc_shares(state: State<'_, AppState>, path: String) -> Result<Vec<nextcloud::NcShare>> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.shares_list(&path).await
}

#[tauri::command]
pub async fn nc_share_link(state: State<'_, AppState>, path: String, password: Option<String>, expire: Option<String>) -> Result<nextcloud::NcShare> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.share_link(&path, password.as_deref(), expire.as_deref()).await
}

#[tauri::command]
pub async fn nc_share_user(state: State<'_, AppState>, path: String, uid: String, can_edit: bool) -> Result<nextcloud::NcShare> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.share_user(&path, &uid, can_edit).await
}

#[tauri::command]
pub async fn nc_share_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.share_delete(&id).await
}

/// Публичная ссылка на файл для письма (существующая без пароля или новая).
#[tauri::command]
pub async fn nc_link_for(state: State<'_, AppState>, path: String) -> Result<String> {
    let ctx = state.ctx().await?;
    nc(&ctx).await?.link_for(&path, crate::policy::load().share_link_days()).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedLink {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) url: String,
}

/// Файлы с компьютера → папка в Nextcloud → публичные ссылки (для больших вложений).
#[tauri::command]
pub async fn nc_upload_links(state: State<'_, AppState>, dir: String, attachments: Vec<String>) -> Result<Vec<UploadedLink>> {
    let ctx = state.ctx().await?;
    let days = crate::policy::load().share_link_days();
    let files = picked_files(&ctx, &attachments).await?;
    let client = nc_upload(&ctx).await?;
    let mut out = Vec::new();
    for (id, name, file) in files {
        let path = client.upload_unique_src(&dir, &name, nextcloud::UploadSource::File(file)).await?;
        let url = client.link_for(&path, days).await?;
        ctx.picked.lock().await.remove(&id);
        out.push(UploadedLink { name: path.rsplit('/').next().unwrap_or(&name).to_string(), path, url });
    }
    Ok(out)
}

/// Вложение письма → папка в Nextcloud. Возвращает путь.
#[tauri::command]
pub async fn nc_save_mail_attachment(state: State<'_, AppState>, folder: String, uid: u32, index: usize, dir: String) -> Result<String> {
    let ctx = state.ctx().await?;
    let (name, data) = attachment(&ctx, &folder, uid, index).await?;
    nc_upload(&ctx).await?.upload_unique(&dir, &name, data).await
}
