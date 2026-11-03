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

/// Убрать временные копии прошлых перетаскиваний (старше часа).
pub fn cleanup_old(base: &Path) {
    let Ok(rd) = std::fs::read_dir(base) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("dragout-") {
            continue;
        }
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > KEEP_SECS);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Записать вложение во временный каталог: путь к файлу с исходным именем.
pub fn stage(name: &str, data: &[u8]) -> crate::error::Result<PathBuf> {
    let dir = crate::updater::private_dir("dragout")?;
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
