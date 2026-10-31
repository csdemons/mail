//! Чтение файлов данных Outlook (.pst, .ost) и сборка писем RFC 822 для импорта.
//! Библиотека outlook-pst (Microsoft, MIT) читает свойства MAPI; письмо собирается
//! заново: заголовки — из PR_TRANSPORT_MESSAGE_HEADERS (у полученных писем) или из
//! свойств отправителя и получателей, тело — текст и HTML, вложения — как есть.

use std::path::Path;
use std::rc::Rc;

use outlook_pst::ltp::prop_context::PropertyValue;
use outlook_pst::ltp::table_context::TableContext;
use outlook_pst::messaging::attachment::{AnsiAttachment, Attachment, AttachmentData, UnicodeAttachment};
use outlook_pst::messaging::message::{AnsiMessage, Message, UnicodeMessage};
use outlook_pst::messaging::store::{AnsiStore, EntryId, Store, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use outlook_pst::{AnsiPstFile, UnicodePstFile};

use crate::error::{AppError, Result};
use crate::mbox::{self, Imported};

/// Письмо больше этого размера пропускается (как и при загрузке с сервера).
const MAX_MESSAGE: usize = crate::imap::MAX_MESSAGE_MB as usize * 1024 * 1024;
/// Глубина вложенных писем (письмо во вложении письма …).
const MAX_DEPTH: usize = 4;

const PR_SUBJECT: u16 = 0x0037;
const PR_MESSAGE_CLASS: u16 = 0x001A;
const PR_CLIENT_SUBMIT_TIME: u16 = 0x0039;
const PR_DELIVERY_TIME: u16 = 0x0E06;
const PR_MESSAGE_FLAGS: u16 = 0x0E07;
const PR_FLAG_STATUS: u16 = 0x1090;
const PR_TRANSPORT_HEADERS: u16 = 0x007D;
const PR_BODY: u16 = 0x1000;
const PR_HTML: u16 = 0x1013;
const PR_INTERNET_CPID: u16 = 0x3FDE;
const PR_MESSAGE_CODEPAGE: u16 = 0x3FFD;
const PR_SENDER_NAME: u16 = 0x0C1A;
const PR_SENDER_EMAIL: u16 = 0x0C1F;
const PR_SENDER_SMTP: u16 = 0x5D01;
const PR_SENT_REPR_NAME: u16 = 0x0042;
const PR_SENT_REPR_EMAIL: u16 = 0x0065;
const PR_SENT_REPR_SMTP: u16 = 0x5D02;
const PR_MESSAGE_ID: u16 = 0x1035;
const PR_IN_REPLY_TO: u16 = 0x1042;
const PR_REFERENCES: u16 = 0x1039;
const PR_RECIPIENT_TYPE: u16 = 0x0C15;
const PR_DISPLAY_NAME: u16 = 0x3001;
const PR_EMAIL_ADDRESS: u16 = 0x3003;
const PR_SMTP_ADDRESS: u16 = 0x39FE;
const PR_ATTACH_DATA: u16 = 0x3701;
const PR_ATTACH_FILENAME: u16 = 0x3704;
const PR_ATTACH_METHOD: u16 = 0x3705;
const PR_ATTACH_LONG_FILENAME: u16 = 0x3707;
const PR_ATTACH_MIME_TAG: u16 = 0x370E;
const PR_ATTACH_CONTENT_ID: u16 = 0x3712;
const PR_DISPLAY_NAME_FOLDER: u16 = 0x3001;
const PR_CONTAINER_CLASS: u16 = 0x3613;

/// Свойства письма или вложения в удобном виде.
struct Props<'a> {
    get: Box<dyn Fn(u16) -> Option<&'a PropertyValue> + 'a>,
    codepage: u16,
}

impl<'a> Props<'a> {
    fn new(get: impl Fn(u16) -> Option<&'a PropertyValue> + 'a) -> Self {
        let codepage = match get(PR_MESSAGE_CODEPAGE).or_else(|| get(PR_INTERNET_CPID)) {
            Some(PropertyValue::Integer32(cp)) => u16::try_from(*cp).unwrap_or(1251),
            _ => 1251,
        };
        Props { get: Box::new(get), codepage }
    }

    fn string(&self, id: u16) -> Option<String> {
        let s = match (self.get)(id)? {
            PropertyValue::Unicode(v) => String::from_utf16_lossy(v.buffer()),
            PropertyValue::String8(v) => decode_codepage(v.buffer(), self.codepage),
            PropertyValue::Binary(v) => decode_codepage(v.buffer(), self.codepage),
            _ => return None,
        };
        let s = s.trim_end_matches('\0').to_string();
        (!s.is_empty()).then_some(s)
    }

    fn int(&self, id: u16) -> Option<i32> {
        match (self.get)(id)? {
            PropertyValue::Integer32(v) => Some(*v),
            _ => None,
        }
    }

    fn time(&self, id: u16) -> Option<i64> {
        match (self.get)(id)? {
            PropertyValue::Time(v) => filetime_to_unix(*v),
            _ => None,
        }
    }

    fn binary(&self, id: u16) -> Option<&'a [u8]> {
        match (self.get)(id)? {
            PropertyValue::Binary(v) => Some(v.buffer()),
            _ => None,
        }
    }
}

/// FILETIME (100 нс с 1601 г.) → секунды Unix.
fn filetime_to_unix(t: i64) -> Option<i64> {
    let secs = t / 10_000_000 - 11_644_473_600;
    (secs > 0 && secs < 32_503_680_000).then_some(secs)
}

/// Тема письма: в PST перед ней бывает служебный префикс «\x01<длина>».
fn clean_subject(s: String) -> String {
    let mut chars = s.chars();
    if chars.next() == Some('\u{1}') {
        chars.next();
        return chars.as_str().to_string();
    }
    s
}

pub fn charset_of(codepage: u16) -> &'static encoding_rs::Encoding {
    let label: &[u8] = match codepage {
        65001 => b"utf-8",
        1250 => b"windows-1250",
        1251 => b"windows-1251",
        1252 | 20127 | 28591 => b"windows-1252",
        1253 => b"windows-1253",
        1254 => b"windows-1254",
        1257 => b"windows-1257",
        866 => b"ibm866",
        20866 => b"koi8-r",
        21866 => b"koi8-u",
        28595 => b"iso-8859-5",
        28592 => b"iso-8859-2",
        936 => b"gbk",
        950 => b"big5",
        932 => b"shift_jis",
        949 => b"euc-kr",
        50220 => b"iso-2022-jp",
        _ => b"windows-1251",
    };
    encoding_rs::Encoding::for_label(label).unwrap_or(encoding_rs::WINDOWS_1251)
}

fn decode_codepage(bytes: &[u8], codepage: u16) -> String {
    charset_of(codepage).decode_without_bom_handling(bytes).0.into_owned()
}

/// Одна запись таблицы MAPI (получатель, вложение) как набор свойств.
fn row_values(table: &dyn TableContext, row: &outlook_pst::ltp::table_context::TableRowData) -> Vec<(u16, PropertyValue)> {
    let context = table.context();
    let Ok(columns) = row.columns(context) else { return Vec::new() };
    context
        .columns()
        .iter()
        .zip(columns)
        .filter_map(|(col, value)| Some((col.prop_id(), table.read_column(value.as_ref()?, col.prop_type()).ok()?)))
        .collect()
}

struct Recipient {
    kind: i32,
    name: String,
    address: String,
}

fn recipients(msg: &dyn Message, codepage: u16) -> Vec<Recipient> {
    let Some(table) = msg.recipient_table() else { return Vec::new() };
    table
        .rows_matrix()
        .filter_map(|row| {
            let values = row_values(table.as_ref(), row);
            let p = Props { get: Box::new(|id| values.iter().find(|(k, _)| *k == id).map(|(_, v)| v)), codepage };
            let kind = p.int(PR_RECIPIENT_TYPE).unwrap_or(1);
            let name = p.string(PR_DISPLAY_NAME).unwrap_or_default();
            let address = p.string(PR_SMTP_ADDRESS).or_else(|| p.string(PR_EMAIL_ADDRESS).filter(|a| a.contains('@'))).unwrap_or_default();
            (!name.is_empty() || !address.is_empty()).then_some(Recipient { kind, name, address })
        })
        .collect()
}

/// Вложение, готовое к записи в письмо.
pub struct Part {
    pub name: String,
    pub mime: String,
    pub content_id: Option<String>,
    pub data: Vec<u8>,
    /// Вложенное письмо (message/rfc822).
    pub message: bool,
}

/// Чтение вложений: у разных форматов PST разные типы, поэтому через замыкание.
type OpenAttachment<'s> = dyn Fn(NodeId) -> Option<Rc<dyn Attachment>> + 's;

fn attachments(msg: &dyn Message, codepage: u16, open: &OpenAttachment<'_>, depth: usize) -> Vec<Part> {
    let Some(table) = msg.attachment_table() else { return Vec::new() };
    let mut out = Vec::new();
    for row in table.rows_matrix() {
        let Some(att) = open(NodeId::from(u32::from(row.id()))) else { continue };
        let props = att.properties();
        let p = Props { get: Box::new(|id| props.get(id)), codepage };
        let name = p.string(PR_ATTACH_LONG_FILENAME).or_else(|| p.string(PR_ATTACH_FILENAME)).or_else(|| p.string(PR_DISPLAY_NAME)).unwrap_or_else(|| "attachment".into());
        let method = p.int(PR_ATTACH_METHOD).unwrap_or(1);
        match att.data() {
            Some(AttachmentData::Message(inner)) if depth < MAX_DEPTH => {
                if let Some(raw) = build(inner.as_ref(), open, depth + 1) {
                    let name = if name.to_lowercase().ends_with(".eml") { name } else { format!("{name}.eml") };
                    out.push(Part { name, mime: "message/rfc822".into(), content_id: None, data: raw, message: true });
                }
            }
            Some(AttachmentData::Binary(bin)) if method == 1 || method == 6 => {
                let mime = p.string(PR_ATTACH_MIME_TAG).filter(|m| m.contains('/')).unwrap_or_else(|| mime_guess::from_path(&name).first_or_octet_stream().essence_str().to_string());
                out.push(Part { name, mime, content_id: p.string(PR_ATTACH_CONTENT_ID), data: bin.buffer().to_vec(), message: false });
            }
            _ => {
                if let Some(data) = p.binary(PR_ATTACH_DATA) {
                    out.push(Part { name: name.clone(), mime: mime_guess::from_path(&name).first_or_octet_stream().essence_str().to_string(), content_id: None, data: data.to_vec(), message: false });
                }
            }
        }
    }
    out
}

/// Заголовки из PR_TRANSPORT_MESSAGE_HEADERS без MIME-части (тело собираем заново).
fn transport_headers(raw: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in raw.split('\n').map(|l| l.trim_end_matches('\r')) {
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some(last) = out.last_mut() {
                last.1.push_str("\r\n");
                last.1.push_str(line);
            }
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            out.push((k.trim().to_string(), v.trim_start().to_string()));
        }
    }
    out.retain(|(k, _)| {
        let k = k.to_ascii_lowercase();
        !(k.starts_with("content-") || k == "mime-version" || k.starts_with("x-ms-exchange-") || k == "x-ms-has-attach" || k == "x-ms-tnef-correlator")
    });
    out
}

fn address(name: &str, addr: &str) -> String {
    match (name.trim(), addr.trim()) {
        ("", a) => a.to_string(),
        (n, "") => mbox::encode_word(n),
        (n, a) if n.eq_ignore_ascii_case(a) => a.to_string(),
        (n, a) => format!("{} <{a}>", mbox::encode_phrase(n)),
    }
}

/// Собирает письмо из свойств MAPI.
fn build(msg: &dyn Message, open: &OpenAttachment<'_>, depth: usize) -> Option<Vec<u8>> {
    let props = msg.properties();
    let p = Props::new(|id| props.get(id));
    let codepage = p.codepage;
    let subject = p.string(PR_SUBJECT).map(clean_subject).unwrap_or_default();
    let date = p.time(PR_CLIENT_SUBMIT_TIME).or_else(|| p.time(PR_DELIVERY_TIME));

    let mut headers: Vec<(String, String)> = match p.string(PR_TRANSPORT_HEADERS) {
        Some(h) => transport_headers(&h),
        None => Vec::new(),
    };
    let has = |headers: &[(String, String)], name: &str| headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
    if !has(&headers, "From") {
        let name = p.string(PR_SENT_REPR_NAME).or_else(|| p.string(PR_SENDER_NAME)).unwrap_or_default();
        let addr = p
            .string(PR_SENT_REPR_SMTP)
            .or_else(|| p.string(PR_SENDER_SMTP))
            .or_else(|| p.string(PR_SENT_REPR_EMAIL).filter(|a| a.contains('@')))
            .or_else(|| p.string(PR_SENDER_EMAIL).filter(|a| a.contains('@')))
            .unwrap_or_default();
        if !name.is_empty() || !addr.is_empty() {
            headers.push(("From".into(), address(&name, &addr)));
        }
    }
    if !has(&headers, "To") && !has(&headers, "Cc") {
        let list = recipients(msg, codepage);
        for (kind, header) in [(1, "To"), (2, "Cc")] {
            let v: Vec<String> = list.iter().filter(|r| r.kind == kind).map(|r| address(&r.name, &r.address)).collect();
            if !v.is_empty() {
                headers.push((header.into(), v.join(",\r\n ")));
            }
        }
    }
    if !has(&headers, "Subject") {
        headers.push(("Subject".into(), mbox::encode_word(&subject)));
    }
    if !has(&headers, "Date") {
        if let Some(t) = date.and_then(|t| chrono::DateTime::from_timestamp(t, 0)) {
            headers.push(("Date".into(), t.to_rfc2822()));
        }
    }
    for (id, name) in [(PR_MESSAGE_ID, "Message-ID"), (PR_IN_REPLY_TO, "In-Reply-To"), (PR_REFERENCES, "References")] {
        if !has(&headers, name) {
            if let Some(v) = p.string(id) {
                headers.push((name.into(), v));
            }
        }
    }

    let text = p.string(PR_BODY);
    let html = match props.get(PR_HTML) {
        Some(PropertyValue::Binary(b)) => {
            let cp = match props.get(PR_INTERNET_CPID) {
                Some(PropertyValue::Integer32(c)) => u16::try_from(*c).unwrap_or(65001),
                _ => codepage,
            };
            Some(decode_codepage(b.buffer(), cp))
        }
        _ => p.string(PR_HTML),
    };
    let parts = attachments(msg, codepage, open, depth);
    let raw = mbox::compose(&headers, text.as_deref(), html.as_deref(), &parts);
    (raw.len() <= MAX_MESSAGE).then_some(raw)
}

/// Папки, в которых нет писем (контакты, календарь, задачи, заметки, журнал).
fn mail_folder(class: Option<&str>) -> bool {
    match class {
        None => true,
        Some(c) => {
            let c = c.to_ascii_lowercase();
            c.is_empty() || c.starts_with("ipf.note") || c == "ipf.imap" || c == "ipf"
        }
    }
}

/// Письмо (класс IPM.Note и производные: отчёты, приглашения тоже письма).
fn mail_message(class: Option<&str>) -> bool {
    match class {
        None => true,
        Some(c) => {
            let c = c.to_ascii_lowercase();
            c.starts_with("ipm.note") || c.starts_with("ipm.schedule.meeting") || c.starts_with("report.") || c == "ipm" || c.starts_with("ipm.post")
        }
    }
}

/// Ход чтения: `on_folder(путь, писем)` перед папкой, `on_message(путь, письмо)` — для
/// каждого письма; `false` из `on_message` прерывает чтение.
pub trait Sink {
    fn on_folder(&mut self, path: &[String], count: usize);
    fn on_message(&mut self, path: &[String], mail: Imported) -> bool;
    fn on_skipped(&mut self, path: &[String], reason: &str);
}

macro_rules! walk_impl {
    ($fn:ident, $store:ty, $file:ty, $message:ty, $attachment:ty) => {
        fn $fn(path: &Path, sink: &mut dyn Sink) -> std::io::Result<bool> {
            let file = <$file>::open(path)?;
            let store: Rc<$store> = <$store>::read(Rc::new(file))?;
            let root = store.properties().ipm_sub_tree_entry_id()?;
            let mut stack: Vec<(EntryId, Vec<String>)> = vec![(root, Vec::new())];
            while let Some((id, folder_path)) = stack.pop() {
                let Ok(folder) = store.open_folder(&id) else {
                    sink.on_skipped(&folder_path, "папка не читается");
                    continue;
                };
                let fprops = folder.properties();
                let fp = Props::new(|id| fprops.get(id));
                let class = fp.string(PR_CONTAINER_CLASS);
                if let Some(h) = folder.hierarchy_table() {
                    let mut subs = Vec::new();
                    for row in h.rows_matrix() {
                        let node = NodeId::from(u32::from(row.id()));
                        let Ok(entry) = store.properties().make_entry_id(node) else { continue };
                        let values = row_values(h.as_ref(), row);
                        let rp = Props { get: Box::new(|id| values.iter().find(|(k, _)| *k == id).map(|(_, v)| v)), codepage: fp.codepage };
                        let name = rp.string(PR_DISPLAY_NAME_FOLDER).unwrap_or_else(|| format!("Папка {}", u32::from(node)));
                        let mut p = folder_path.clone();
                        p.push(name);
                        subs.push((entry, p));
                    }
                    // Обход по алфавиту (стек — в обратном порядке).
                    subs.sort_by(|a, b| b.1.cmp(&a.1));
                    stack.extend(subs);
                }
                if folder_path.is_empty() || !mail_folder(class.as_deref()) {
                    continue;
                }
                let Some(contents) = folder.contents_table() else { continue };
                let rows: Vec<u32> = contents.rows_matrix().map(|r| u32::from(r.id())).collect();
                sink.on_folder(&folder_path, rows.len());
                for row in rows {
                    let Ok(entry) = store.properties().make_entry_id(NodeId::from(row)) else { continue };
                    let msg = match <$message>::read(store.clone(), &entry, None) {
                        Ok(m) => m,
                        Err(e) => {
                            sink.on_skipped(&folder_path, &format!("письмо не читается: {e}"));
                            continue;
                        }
                    };
                    let mprops = msg.properties();
                    let mp = Props::new(|id| mprops.get(id));
                    if !mail_message(mp.string(PR_MESSAGE_CLASS).as_deref()) {
                        sink.on_skipped(&folder_path, "не письмо (контакт, встреча, задача)");
                        continue;
                    }
                    let flags = mp.int(PR_MESSAGE_FLAGS).unwrap_or(0);
                    let flagged = mp.int(PR_FLAG_STATUS) == Some(2);
                    let date = mp.time(PR_DELIVERY_TIME).or_else(|| mp.time(PR_CLIENT_SUBMIT_TIME));
                    let owner = msg.clone();
                    let open = move |node: NodeId| -> Option<Rc<dyn Attachment>> {
                        let a: Rc<dyn Attachment> = <$attachment>::read(owner.clone(), node, None).ok()?;
                        Some(a)
                    };
                    let Some(raw) = build(msg.as_ref(), &open, 0) else {
                        sink.on_skipped(&folder_path, "письмо слишком большое");
                        continue;
                    };
                    let mail = Imported { raw, seen: flags & 0x1 != 0, flagged, answered: false, date };
                    if !sink.on_message(&folder_path, mail) {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        }
    };
}

walk_impl!(walk_unicode, UnicodeStore, UnicodePstFile, UnicodeMessage, UnicodeAttachment);
walk_impl!(walk_ansi, AnsiStore, AnsiPstFile, AnsiMessage, AnsiAttachment);

/// Читает файл .pst/.ost целиком. `Ok(false)` — чтение прервано.
pub fn read(path: &Path, sink: &mut dyn Sink) -> Result<bool> {
    let unicode = UnicodePstFile::open(path).is_ok();
    let result = if unicode { walk_unicode(path, sink) } else { walk_ansi(path, sink) };
    result.map_err(|e| AppError::Other(format!("Файл Outlook не читается ({e}). Закройте Outlook и повторите; файлы, защищённые паролем, не поддерживаются.")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_and_subject() {
        assert_eq!(filetime_to_unix(132_000_000_000_000_000), Some(1_555_526_400));
        assert_eq!(filetime_to_unix(0), None);
        assert_eq!(clean_subject("\u{1}\u{5}RE: тест".into()), "RE: тест");
        assert_eq!(clean_subject("Привет".into()), "Привет");
        assert_eq!(decode_codepage(&[0xcf, 0xf0, 0xe8], 1251), "При");
    }

    #[test]
    fn transport_headers_drop_mime() {
        let h = transport_headers("Received: from a\r\n\tby b\r\nFrom: A <a@b.ru>\r\nContent-Type: multipart/mixed;\r\n boundary=x\r\nSubject: hi\r\n\r\nignored: yes\r\n");
        let names: Vec<_> = h.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["Received", "From", "Subject"]);
        assert!(h[0].1.contains("by b"));
    }

    #[test]
    fn classes() {
        assert!(mail_folder(None) && mail_folder(Some("IPF.Note")) && !mail_folder(Some("IPF.Contact")) && !mail_folder(Some("IPF.Appointment")));
        assert!(mail_message(Some("IPM.Note.SMIME")) && !mail_message(Some("IPM.Contact")) && mail_message(Some("REPORT.IPM.Note.NDR")));
    }

    /// Пример PST из java-libpst (если положен в RRCKI_TEST_PST).
    #[test]
    fn reads_sample_pst() {
        let Some(path) = std::env::var_os("RRCKI_TEST_PST") else { return };
        struct Count(usize, Vec<String>);
        impl Sink for Count {
            fn on_folder(&mut self, path: &[String], count: usize) {
                self.1.push(format!("{} ({count})", path.join("/")));
            }
            fn on_message(&mut self, _: &[String], mail: Imported) -> bool {
                assert!(mail_parser::MessageParser::default().parse(&mail.raw).is_some());
                if self.0 == 0 {
                    eprintln!("{}", String::from_utf8_lossy(&mail.raw[..mail.raw.len().min(1500)]));
                }
                self.0 += 1;
                true
            }
            fn on_skipped(&mut self, path: &[String], reason: &str) {
                eprintln!("skip {}: {reason}", path.join("/"));
            }
        }
        let mut c = Count(0, Vec::new());
        assert!(read(Path::new(&path), &mut c).unwrap());
        eprintln!("folders: {:?}; messages: {}", c.1, c.0);
        assert!(c.0 > 0);
    }
}
