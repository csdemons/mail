//! Перетаскивание вложения из письма в папку компьютера (рабочий стол, проводник,
//! файловый менеджер). Вложение сначала записывается в личный временный каталог,
//! затем окно начинает системное перетаскивание этого файла: в Windows и macOS —
//! через crate `drag`, в Linux — средствами GTK (здесь: crate `drag` не кодирует
//! пути в адресах `file://`, и файлы с пробелами или кириллицей в имени
//! файловые менеджеры не принимают).

use std::path::{Path, PathBuf};

/// Через сколько убирать временные копии перетащенных вложений: файловый
/// менеджер копирует файл уже после того, как его отпустили.
const KEEP_SECS: u64 = 3600;
/// Копии, открытые в программе компьютера (Word, Excel…): с ними могут работать долго.
const OPEN_KEEP_SECS: u64 = 7 * 24 * 3600;

/// Убрать временные копии прошлых перетаскиваний (старше часа) и открытых вложений (старше недели).
pub fn cleanup_old(base: &Path) {
    let Ok(rd) = std::fs::read_dir(base) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let keep = if name.starts_with("dragout-") {
            KEEP_SECS
        } else if name.starts_with("open-") {
            OPEN_KEEP_SECS
        } else {
            continue;
        };
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > keep);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Записать вложение во временный каталог: путь к файлу с исходным именем.
pub fn stage(name: &str, data: &[u8]) -> crate::error::Result<PathBuf> {
    stage_in("dragout", name, data)
}

fn stage_in(prefix: &str, name: &str, data: &[u8]) -> crate::error::Result<PathBuf> {
    let dir = crate::updater::private_dir(prefix)?;
    if let Some(base) = dir.parent() {
        cleanup_old(base);
    }
    let safe = crate::imap::sanitize_filename(name);
    let path = dir.join(if safe.is_empty() { "file".to_string() } else { safe });
    {
        use std::io::Write as _;
        let mut f = crate::updater::create_new_file(&path)?;
        f.write_all(data)?;
    }
    Ok(std::fs::canonicalize(&path).unwrap_or(path))
}

/// Файлы, которые можно открыть программой компьютера прямо из письма: документы,
/// таблицы, презентации, PDF, картинки, текст, архивы. Исполняемые файлы, скрипты,
/// ярлыки и всё неизвестное так не открываются — только «Сохранить».
const OPENABLE: &[&str] = &[
    "doc", "docx", "docm", "dot", "dotx", "rtf", "odt", "ott", "xls", "xlsx", "xlsm", "xlt", "xltx", "csv", "ods", "ots", "ppt", "pptx", "pptm", "pps", "ppsx", "odp", "otp",
    "pdf", "djvu", "txt", "log", "md", "xml", "json", "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "svg", "heic", "vsd", "vsdx", "odg", "mpp", "zip", "7z", "rar",
    "eml", "msg", "ics", "vcf", "mp3", "mp4", "wav", "avi", "mkv", "mov",
];

pub fn openable(name: &str) -> bool {
    let n = name.trim().trim_end_matches(['.', ' ']).to_ascii_lowercase();
    n.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && OPENABLE.contains(&ext))
}

/// Вложение — во временную копию для открытия программой компьютера. В Windows
/// копия помечается как файл из интернета (Mark of the Web, зона «Интернет»):
/// Office открывает её в защищённом просмотре, макросы не запускаются сами.
pub fn stage_for_open(name: &str, data: &[u8]) -> crate::error::Result<PathBuf> {
    if !openable(name) {
        return Err(crate::error::AppError::Other(format!("«{name}» нельзя открыть прямо из письма — сохраните файл и проверьте его перед открытием")));
    }
    let path = stage_in("open", name, data)?;
    #[cfg(windows)]
    {
        let mut ads = path.as_os_str().to_owned();
        ads.push(":Zone.Identifier");
        let _ = std::fs::write(std::path::PathBuf::from(ads), b"[ZoneTransfer]\r\nZoneId=3\r\n");
    }
    Ok(path)
}

/// Linux: начать перетаскивание файлов из окна (вызывать в главном потоке, пока
/// кнопка мыши нажата — по событию `dragstart` страницы).
#[cfg(target_os = "linux")]
pub fn start_gtk(window: &gtk::ApplicationWindow, paths: Vec<PathBuf>) -> bool {
    use gtk::prelude::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    let uris: Vec<String> = paths.iter().filter_map(|p| gtk::glib::filename_to_uri(p, None).ok().map(|u| u.to_string())).collect();
    if uris.is_empty() {
        return false;
    }
    window.drag_source_set(gtk::gdk::ModifierType::BUTTON1_MASK, &[], gtk::gdk::DragAction::COPY);
    window.drag_source_add_uri_targets();
    let handlers: Rc<RefCell<Vec<gtk::glib::SignalHandlerId>>> = Rc::new(RefCell::new(Vec::new()));
    let data_get = window.connect_drag_data_get(move |_, _, data, _, _| {
        let refs: Vec<&str> = uris.iter().map(String::as_str).collect();
        data.set_uris(&refs);
    });
    handlers.borrow_mut().push(data_get);
    let started = window
        .drag_source_get_target_list()
        .and_then(|targets| window.drag_begin_with_coordinates(&targets, gtk::gdk::DragAction::COPY, 1, None, -1, -1));
    let cleanup = {
        let handlers = handlers.clone();
        move |w: &gtk::ApplicationWindow| {
            for h in handlers.borrow_mut().drain(..) {
                w.disconnect(h);
            }
            w.drag_source_unset();
        }
    };
    if started.is_none() {
        cleanup(window);
        return false;
    }
    let end = window.connect_drag_end(move |w, _| cleanup(w));
    handlers.borrow_mut().push(end);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_documents_open_directly() {
        for ok in ["Смета.xlsx", "Отчёт.DOCX", "план.pdf", "фото.jpg", "a.b.odt"] {
            assert!(openable(ok), "{ok}");
        }
        for bad in ["setup.exe", "отчёт.docx.exe", "run.bat", "x.ps1", "x.js", "x.vbs", "x.lnk", "x.desktop", "x.sh", "x.jar", "x.scr", "x.hta", "x.msi", "README", ".docx"] {
            assert!(!openable(bad), "{bad}");
        }
    }
}
