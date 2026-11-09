//! Файлы и картинки из буфера обмена — для Ctrl+V в чате, когда WebView их не отдаёт
//! (файлы, скопированные в проводнике; снимки экрана в WebKitGTK).

use crate::error::{AppError, Result};
use std::path::PathBuf;

/// Что лежит в буфере обмена.
pub enum Clip {
    /// Скопированные файлы (в проводнике, Nautilus, Dolphin…).
    Files(Vec<PathBuf>),
    /// Картинка — уже в PNG.
    Png(Vec<u8>),
    Nothing,
}

/// Больше файлов за одну вставку не берём.
const MAX_FILES: usize = 20;

/// Прочитать буфер обмена (блокирующий вызов — из `spawn_blocking`).
pub fn read() -> Result<Clip> {
    let mut cb = arboard::Clipboard::new().map_err(|e| AppError::Other(format!("Буфер обмена недоступен: {e}")))?;
    if let Ok(list) = cb.get().file_list() {
        let files: Vec<PathBuf> = list.into_iter().filter(|p| p.is_file()).take(MAX_FILES).collect();
        if !files.is_empty() {
            return Ok(Clip::Files(files));
        }
    }
    match cb.get_image() {
        Ok(img) => Ok(Clip::Png(png_of(img.width, img.height, &img.bytes)?)),
        Err(_) => Ok(Clip::Nothing),
    }
}

/// RGBA → PNG.
fn png_of(width: usize, height: usize, rgba: &[u8]) -> Result<Vec<u8>> {
    let (w, h) = (u32::try_from(width).map_err(|_| AppError::Other("Картинка слишком большая".into()))?, u32::try_from(height).map_err(|_| AppError::Other("Картинка слишком большая".into()))?);
    if w == 0 || h == 0 || rgba.len() != width.saturating_mul(height).saturating_mul(4) {
        return Err(AppError::Other("Картинка в буфере обмена повреждена".into()));
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| AppError::Other(format!("PNG: {e}")))?;
        writer.write_image_data(rgba).map_err(|e| AppError::Other(format!("PNG: {e}")))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_becomes_png() {
        let png = png_of(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert!(png_of(2, 2, &[0; 4]).is_err());
        assert!(png_of(0, 0, &[]).is_err());
    }
}
