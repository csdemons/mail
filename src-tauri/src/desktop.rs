//! Интеграция с рабочим столом, общая для обеих сборок: автозапуск, настройки
//! окна (сворачивать в трей) и картинки значка непрочитанных.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

/// Аргумент запуска «свернуть в трей» (так программу запускает автозапуск).
pub const MINIMIZED_ARG: &str = "--minimized";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiPrefs {
    /// Закрытие окна сворачивает программу в трей (почта и уведомления продолжают работать).
    pub close_to_tray: bool,
}

impl Default for UiPrefs {
    fn default() -> Self {
        UiPrefs { close_to_tray: true }
    }
}

pub fn load_prefs(config_dir: &std::path::Path) -> UiPrefs {
    std::fs::read(config_dir.join("ui.json")).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
}

pub fn save_prefs(config_dir: &std::path::Path, p: &UiPrefs) -> Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let data = serde_json::to_vec_pretty(p).map_err(|e| AppError::Other(e.to_string()))?;
    std::fs::write(config_dir.join("ui.json"), data)?;
    Ok(())
}

/// Путь для запуска: AppImage — сам файл образа, иначе исполняемый файл.
fn launch_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("APPIMAGE") {
        return Ok(PathBuf::from(p));
    }
    std::env::current_exe().map_err(|e| AppError::Other(format!("Путь к программе: {e}")))
}

#[cfg(target_os = "macos")]
fn autostart_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join("Library/LaunchAgents/ru.rrcki.mail.plist"))
}

#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn autostart_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart").join("ru.rrcki.mail.desktop"))
}

/// Запускать вместе с системой (свёрнутой в трей).
pub fn autostart_enabled() -> bool {
    #[cfg(windows)]
    {
        win::run_value().is_some()
    }
    #[cfg(not(windows))]
    {
        autostart_file().is_some_and(|f| f.exists())
    }
}

pub fn set_autostart(on: bool) -> Result<()> {
    let exe = launch_path()?;
    #[cfg(windows)]
    {
        if on {
            win::set_run_value(&format!("\"{}\" {MINIMIZED_ARG}", exe.display()))
        } else {
            win::delete_run_value()
        }
    }
    #[cfg(not(windows))]
    {
        let file = autostart_file().ok_or_else(|| AppError::Other("Не найден домашний каталог".into()))?;
        if !on {
            return match std::fs::remove_file(&file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            };
        }
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // macOS: агент запуска (LaunchAgent) — его видно в «Объектах входа».
        #[cfg(target_os = "macos")]
        {
            std::fs::write(
                &file,
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key><string>ru.rrcki.mail</string>\n  <key>ProgramArguments</key><array><string>{}</string><string>{MINIMIZED_ARG}</string></array>\n  <key>RunAtLoad</key><true/>\n  <key>ProcessType</key><string>Interactive</string>\n</dict>\n</plist>\n",
                    xml_escape(&exe.display().to_string())
                ),
            )?;
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let quoted = format!("\"{}\"", exe.display().to_string().replace('\\', "\\\\").replace('"', "\\\""));
            std::fs::write(
                &file,
                format!("[Desktop Entry]\nType=Application\nName=Atom Mail\nExec={quoted} {MINIMIZED_ARG}\nIcon=rrcki-mail\nX-GNOME-Autostart-enabled=true\nTerminal=false\n"),
            )?;
            Ok(())
        }
    }
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

    use crate::error::{AppError, Result};

    const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const NAME: &str = "Atom Mail";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn run_value() -> Option<String> {
        let (key, name) = (wide(RUN), wide(NAME));
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: буфер и его размер в байтах переданы корректно.
        let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
        (rc == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    }

    pub fn set_run_value(cmd: &str) -> Result<()> {
        let (key, name, data) = (wide(RUN), wide(NAME), wide(cmd));
        // SAFETY: строки UTF-16 с завершающим нулём, размер данных в байтах.
        let rc = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) };
        if rc == ERROR_SUCCESS { Ok(()) } else { Err(AppError::Other(format!("Реестр (автозапуск): ошибка {rc}"))) }
    }

    pub fn delete_run_value() -> Result<()> {
        let (key, name) = (wide(RUN), wide(NAME));
        // SAFETY: строки UTF-16 с завершающим нулём.
        let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
        if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(AppError::Other(format!("Реестр (автозапуск): ошибка {rc}"))) }
    }
}

// ---------------------------------------------------------------- значок непрочитанных

/// Цифры 3×5 точек для значка.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Красный кружок с числом (до 99, больше — «99») поверх RGBA-картинки `size×size`
/// в правом верхнем углу; `full` — кружок на всю картинку (значок панели задач).
pub fn draw_badge(rgba: &mut [u8], size: u32, count: u32, full: bool) {
    let s = size as i32;
    let d = if full { s } else { s * 3 / 5 };
    let (cx, cy, r) = (s - d / 2, d / 2, d as f32 / 2.0);
    let put = |buf: &mut [u8], x: i32, y: i32, c: [u8; 4]| {
        if x >= 0 && y >= 0 && x < s && y < s {
            let i = ((y * s + x) * 4) as usize;
            buf[i..i + 4].copy_from_slice(&c);
        }
    };
    for y in 0..s {
        for x in 0..s {
            let (dx, dy) = (x as f32 + 0.5 - cx as f32, y as f32 + 0.5 - cy as f32);
            if dx * dx + dy * dy <= r * r {
                put(rgba, x, y, [220, 38, 38, 255]);
            }
        }
    }
    let text: Vec<usize> = count.min(99).to_string().bytes().map(|b| (b - b'0') as usize).collect();
    // Масштаб цифр: чтобы число поместилось в кружок.
    let scale = ((d as f32 * 0.6) / (text.len() as f32 * 4.0 - 1.0)).min(d as f32 * 0.11).floor().max(1.0) as i32;
    let (w, h) = ((text.len() as i32 * 4 - 1) * scale, 5 * scale);
    let (ox, oy) = (cx - w / 2, cy - h / 2);
    for (i, digit) in text.iter().enumerate() {
        for (row, bits) in DIGITS[*digit].iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            put(rgba, ox + (i as i32 * 4 + col) * scale + sx, oy + row as i32 * scale + sy, [255, 255, 255, 255]);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_draws_red_circle_and_white_digits() {
        let mut img = vec![0u8; 32 * 32 * 4];
        draw_badge(&mut img, 32, 7, true);
        let px = |x: usize, y: usize| &img[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert_eq!(px(3, 16), [220, 38, 38, 255]);
        assert_eq!(px(0, 0), [0, 0, 0, 0], "углы прозрачные");
        assert!(img.chunks(4).any(|p| p == [255, 255, 255, 255]), "цифра нарисована");
        let mut small = vec![0u8; 16 * 16 * 4];
        draw_badge(&mut small, 16, 123, false);
    }

    #[test]
    fn prefs_default_close_to_tray() {
        let dir = std::env::temp_dir().join(format!("rrcki-ui-{}", std::process::id()));
        assert!(load_prefs(&dir).close_to_tray);
        save_prefs(&dir, &UiPrefs { close_to_tray: false }).unwrap();
        assert!(!load_prefs(&dir).close_to_tray);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
