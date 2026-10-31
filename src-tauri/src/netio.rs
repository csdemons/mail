//! Чтение HTTP-ответов с ограничением размера: тело читается кусками, и загрузка
//! прерывается, как только превышен предел (а не после того, как весь ответ уже
//! в памяти). Защищает от исчерпания памяти неисправным или подменённым сервером.

use crate::error::{AppError, Result};

/// JSON и XML веб-API (SOGo, Nextcloud OCS, PROPFIND): списки контактов, файлов, бесед.
pub const API_LIMIT: usize = 16 * 1024 * 1024;
/// Мелкие ответы: настройки, вход, карточка контакта, autoconfig.
pub const SMALL_LIMIT: usize = 2 * 1024 * 1024;

/// Тело ответа целиком, но не больше `max` байт.
pub async fn read_limited(mut resp: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    if resp.content_length().is_some_and(|n| n > max as u64) {
        return Err(too_large(max));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| AppError::Other(format!("Обрыв ответа сервера: {e}")))? {
        if body.len() + chunk.len() > max {
            return Err(too_large(max));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Текст ответа (UTF-8; недопустимые байты заменяются) не больше `max` байт.
pub async fn text_limited(resp: reqwest::Response, max: usize) -> Result<String> {
    let body = read_limited(resp, max).await?;
    Ok(match String::from_utf8(body) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    })
}

/// JSON ответа не больше `max` байт.
pub async fn json_limited(resp: reqwest::Response, max: usize) -> Result<serde_json::Value> {
    let body = read_limited(resp, max).await?;
    serde_json::from_slice(&body).map_err(|_| AppError::Other("Сервер вернул не JSON".into()))
}

fn too_large(max: usize) -> AppError {
    AppError::Other(format!("Ответ сервера слишком большой (больше {} МБ) — загрузка прервана", max / (1024 * 1024)))
}
