//! Письмо целиком: разбор MIME, сводка и текст, HTML, вложения, поиск по тексту.
//!
//! Часть модуля `imap` — общий обзор в `mod.rs`.

use super::prelude::*;
use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInfo {
    pub index: usize,
    pub name: String,
    pub content_type: String,
    pub size: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDetail {
    #[serde(flatten)]
    pub summary: MessageSummary,
    pub cc: Vec<String>,
    /// Message-ID и References — для заголовков ответа.
    pub message_id: Option<String>,
    pub references: Option<String>,
    /// Reply-To, если отличается от отправителя.
    pub reply_to: Option<String>,
    pub body_text: String,
    /// Очищенный HTML (если письмо в HTML): встроенные картинки уже внутри.
    pub body_html: Option<String>,
    /// Сколько картинок из интернета не показано (загружаются по кнопке).
    pub remote_images: usize,
    pub attachments: Vec<AttachmentInfo>,
    /// Отправитель просит уведомление о прочтении (адрес из Disposition-Notification-To).
    pub disposition_to: Option<String>,
    /// Рассылка: способы отписаться (List-Unsubscribe).
    pub unsubscribe: Option<Unsubscribe>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Unsubscribe {
    pub https: Option<String>,
    pub mailto: Option<String>,
    /// RFC 8058: отписка одним POST-запросом, без браузера.
    pub one_click: bool,
}

/// List-Unsubscribe: `<mailto:…>, <https://…>` и List-Unsubscribe-Post (RFC 2369, 8058).
pub fn unsubscribe_of(msg: &mail_parser::Message<'_>) -> Option<Unsubscribe> {
    let raw = msg.header_raw("List-Unsubscribe")?;
    let mut https = None;
    let mut mailto = None;
    for part in raw.split(',') {
        let v: String = part.trim().trim_start_matches('<').trim_end_matches('>').chars().filter(|c| !c.is_whitespace()).collect();
        let lower = v.to_ascii_lowercase();
        if lower.starts_with("https://") && https.is_none() && url::Url::parse(&v).is_ok() {
            https = Some(v);
        } else if lower.starts_with("mailto:") && mailto.is_none() {
            mailto = Some(v);
        }
    }
    let one_click = https.is_some()
        && msg.header_raw("List-Unsubscribe-Post").is_some_and(|p| p.to_ascii_lowercase().replace(' ', "").contains("list-unsubscribe=one-click"));
    (https.is_some() || mailto.is_some()).then_some(Unsubscribe { https, mailto, one_click })
}

pub(crate) fn addresses(addr: Option<&mail_parser::Address<'_>>) -> Vec<String> {
    let Some(addr) = addr else { return Vec::new() };
    addr.iter()
        .map(|a| match (a.name(), a.address()) {
            (Some(n), Some(e)) if !n.trim().is_empty() => format!("{} <{}>", n.trim(), e),
            (_, Some(e)) => e.to_string(),
            (Some(n), None) => n.to_string(),
            (None, None) => String::new(),
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Некоторые почтовики кодируют поле From целиком (`=?utf-8?b?...?=` вместе с
/// `<адресом>`), и после декодирования адрес оказывается внутри имени.
pub(crate) fn repair_display_address(name: String, address: String) -> (String, String) {
    if !address.is_empty() {
        return (name, address);
    }
    if let (Some(open), Some(close)) = (name.rfind('<'), name.rfind('>')) {
        if open < close {
            let candidate = name[open + 1..close].trim();
            if candidate.contains('@') && !candidate.contains(char::is_whitespace) {
                let display = name[..open].trim().trim_matches('"').trim().to_string();
                return (display, candidate.to_string());
            }
        }
    }
    if name.contains('@') && !name.contains(char::is_whitespace) {
        return (String::new(), name);
    }
    (name, address)
}

fn has_param(params: &async_imap::imap_proto::types::BodyParams<'_>, names: &[&str]) -> bool {
    params
        .as_ref()
        .is_some_and(|list| list.iter().any(|(k, _)| names.iter().any(|n| k.eq_ignore_ascii_case(n))))
}

/// Имена файлов во вложениях (не больше 20, каждое до 200 символов).
pub(crate) fn attachment_names(bs: &async_imap::imap_proto::types::BodyStructure<'_>) -> Vec<String> {
    use async_imap::imap_proto::types::BodyStructure as B;
    fn param(params: &async_imap::imap_proto::types::BodyParams<'_>, names: &[&str]) -> Option<String> {
        params.as_ref()?.iter().find(|(k, _)| names.iter().any(|n| k.eq_ignore_ascii_case(n))).map(|(k, v)| decode_param(k, v))
    }
    fn walk(bs: &B<'_>, out: &mut Vec<String>) {
        match bs {
            B::Multipart { bodies, .. } => bodies.iter().for_each(|b| walk(b, out)),
            B::Basic { common, .. } | B::Text { common, .. } | B::Message { common, .. } => {
                let name = common
                    .disposition
                    .as_ref()
                    .and_then(|d| param(&d.params, &["filename*", "filename"]))
                    .or_else(|| param(&common.ty.params, &["name*", "name"]));
                if let Some(n) = name.map(|n| n.trim().chars().take(200).collect::<String>()).filter(|n| !n.is_empty()) {
                    if out.len() < 20 && !out.contains(&n) {
                        out.push(n);
                    }
                }
                if let B::Message { body, .. } = bs {
                    walk(body, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(bs, &mut out);
    out
}

/// Значение параметра MIME: RFC 2231 (`utf-8''%D0%9E…`) или RFC 2047 (`=?utf-8?B?…?=`).
pub fn decode_param(key: &str, value: &str) -> String {
    if key.ends_with('*') {
        if let Some((charset, rest)) = value.split_once('\'') {
            let encoded = rest.split_once('\'').map_or(rest, |(_, v)| v);
            let mut bytes = Vec::new();
            let b = encoded.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'%' {
                    if let Some(x) = encoded.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                        bytes.push(x);
                        i += 3;
                        continue;
                    }
                }
                bytes.push(b[i]);
                i += 1;
            }
            let enc = encoding_rs::Encoding::for_label(charset.trim().as_bytes()).unwrap_or(encoding_rs::UTF_8);
            return enc.decode(&bytes).0.into_owned();
        }
    }
    if value.contains("=?") {
        let header = format!("Subject: {value}\r\n\r\n");
        if let Some(m) = MessageParser::default().parse_headers(header.as_bytes()) {
            if let Some(subj) = m.subject() {
                return subj.to_string();
            }
        }
    }
    value.to_string()
}

/// Есть ли во вложенной структуре MIME части-файлы: часть с
/// `Content-Disposition: attachment`, нетекстовая часть с именем файла или
/// вложенное письмо (message/rfc822).
pub(crate) fn has_attachment_part(bs: &async_imap::imap_proto::types::BodyStructure<'_>) -> bool {
    use async_imap::imap_proto::types::BodyStructure as B;
    fn walk(bs: &B<'_>, is_root: bool) -> bool {
        match bs {
            B::Multipart { bodies, .. } => bodies.iter().any(|b| walk(b, false)),
            B::Message { .. } => !is_root,
            B::Basic { common, .. } | B::Text { common, .. } => {
                if is_root {
                    return false;
                }
                let disposition_attachment = common
                    .disposition
                    .as_ref()
                    .is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"));
                let named = has_param(&common.ty.params, &["name"])
                    || common
                        .disposition
                        .as_ref()
                        .is_some_and(|d| has_param(&d.params, &["filename", "filename*"]));
                disposition_attachment || (named && matches!(bs, B::Basic { .. }))
            }
        }
    }
    walk(bs, true)
}

async fn fetch_raw(conn: &mut Connection, folder: &str, uid: u32) -> Result<Fetch> {
    conn.ensure_selected(folder).await?;

    let sizes: Vec<Fetch> = conn
        .imap()?
        .uid_fetch(uid.to_string(), "(UID RFC822.SIZE)")
        .await?
        .try_collect()
        .await?;
    let size = sizes
        .iter()
        .find(|f| f.uid == Some(uid))
        .ok_or(AppError::NotFound)?
        .size
        .unwrap_or(0);
    if size > MAX_MESSAGE_MB * 1024 * 1024 {
        return Err(AppError::TooLarge(size / 1024 / 1024, MAX_MESSAGE_MB));
    }

    let mut fetches: Vec<Fetch> = conn
        .imap()?
        .uid_fetch(
            uid.to_string(),
            "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[])",
        )
        .await?
        .try_collect()
        .await?;
    let pos = fetches
        .iter()
        .position(|f| f.uid == Some(uid) && f.body().is_some())
        .ok_or(AppError::NotFound)?;
    let fetch = fetches.swap_remove(pos);
    if let Some(body) = fetch.body() {
        conn.raw_cache = Some((folder.to_string(), uid, body.to_vec()));
    }
    Ok(fetch)
}

/// Исходник письма: из кэша последнего открытого или с сервера.
async fn raw_message(conn: &mut Connection, folder: &str, uid: u32) -> Result<Vec<u8>> {
    if let Some((f, u, raw)) = &conn.raw_cache {
        if f == folder && *u == uid {
            return Ok(raw.clone());
        }
    }
    if let Some(e) = conn.ews() {
        let raw = e.mime(folder, uid).await?;
        conn.raw_cache = Some((folder.to_string(), uid, raw.clone()));
        return Ok(raw);
    }
    let fetch = fetch_raw(conn, folder, uid).await?;
    Ok(fetch.body().ok_or(AppError::NotFound)?.to_vec())
}

fn attachment_name(part: &mail_parser::MessagePart<'_>, index: usize) -> String {
    let raw = part
        .attachment_name()
        .map(str::to_string)
        .unwrap_or_else(|| format!("attachment-{}", index + 1));
    sanitize_filename(&raw)
}

/// Имя файла без пути и без символов, запрещённых в Windows.
pub fn sanitize_filename(name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = leaf
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let mut cleaned = cleaned
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .to_string();
    if cleaned.is_empty() {
        return "attachment".to_string();
    }
    // Зарезервированные имена Windows (CON, NUL, COM1 … в т.ч. с расширением).
    let stem = cleaned.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        cleaned.insert(0, '_');
    }
    // Слишком длинные имена режем, сохраняя расширение.
    if cleaned.chars().count() > 180 {
        let ext: String = cleaned
            .rfind('.')
            .map(|i| cleaned[i..].chars().take(16).collect())
            .unwrap_or_default();
        let head: String = cleaned.chars().take(180 - ext.chars().count()).collect();
        cleaned = format!("{}{}", head.trim_end_matches(['.', ' ']), ext);
    }
    cleaned
}

pub async fn get_message(conn: &mut Connection, folder: &str, uid: u32) -> Result<MessageDetail> {
    if let Some(e) = conn.ews() {
        let raw = e.mime(folder, uid).await?;
        let summary = e.summaries_of(folder, &[uid]).await?.into_iter().next().ok_or(AppError::NotFound)?;
        let detail = detail_from_raw(summary, &raw);
        conn.raw_cache = Some((folder.to_string(), uid, raw));
        return Ok(detail);
    }
    let fetch = fetch_raw(conn, folder, uid).await?;
    let body = fetch.body().ok_or(AppError::NotFound)?;
    let parsed = MessageParser::default().parse(body);
    let summary = summary_from(folder, &fetch, parsed.as_ref());
    Ok(detail_from(summary, body, parsed.as_ref()))
}

/// Письмо из локального хранилища: исходник и сведения из индекса.
pub fn detail_from_raw(summary: MessageSummary, raw: &[u8]) -> MessageDetail {
    let parsed = MessageParser::default().parse(raw);
    detail_from(summary, raw, parsed.as_ref())
}

/// Текст письма для поиска (без HTML-разметки, не длиннее `max` символов).
pub fn search_text(raw: &[u8], max: usize) -> String {
    let Some(msg) = MessageParser::default().parse(raw) else { return String::new() };
    let mut text: String = msg.body_text(0).map(|b| b.into_owned()).unwrap_or_default();
    // Разделитель: до него — текст письма (строка списка берёт начало отсюда).
    text.push('\u{1}');
    let cc = addresses(msg.cc()).join(" ");
    if !cc.is_empty() {
        text.push(' ');
        text.push_str(&cc);
    }
    for (i, part) in msg.attachments().enumerate() {
        text.push(' ');
        text.push_str(&attachment_name(part, i));
    }
    text.chars().take(max).collect()
}

fn detail_from(summary: MessageSummary, body: &[u8], parsed: Option<&mail_parser::Message<'_>>) -> MessageDetail {

    let (message_id, references, reply_to) = match parsed {
        Some(msg) => {
            let ids = |h: &mail_parser::HeaderValue<'_>| -> Option<String> {
                let list: Vec<String> = match h {
                    mail_parser::HeaderValue::Text(t) => vec![t.to_string()],
                    mail_parser::HeaderValue::TextList(l) => l.iter().map(|t| t.to_string()).collect(),
                    _ => vec![],
                };
                let joined = list.iter().map(|id| format!("<{}>", id.trim_matches(['<', '>']))).collect::<Vec<_>>().join(" ");
                (!joined.is_empty()).then_some(joined)
            };
            (
                msg.message_id().map(|id| format!("<{}>", id.trim_matches(['<', '>']))),
                ids(msg.references()),
                addresses(msg.reply_to()).into_iter().next(),
            )
        }
        None => (None, None, None),
    };

    let (cc, body_text, body_html, remote_images, attachments) = match parsed {
        Some(msg) => {
            let rendered = crate::html::html_part(msg).map(|h| crate::html::render(msg, h, None));
            let inline = rendered.as_ref().map(|r| r.inline.clone()).unwrap_or_default();
            let attachments: Vec<AttachmentInfo> = msg
                .attachments()
                .enumerate()
                .filter(|(index, _)| !inline.contains(index))
                .map(|(index, part)| AttachmentInfo {
                    index,
                    name: attachment_name(part, index),
                    content_type: part
                        .content_type()
                        .map(|ct| match ct.subtype() {
                            Some(sub) => format!("{}/{}", ct.ctype(), sub),
                            None => ct.ctype().to_string(),
                        })
                        .unwrap_or_else(|| "application/octet-stream".into()),
                    size: part.contents().len(),
                })
                .collect();
            (
                addresses(msg.cc()),
                msg.body_text(0).map(|b| b.into_owned()).unwrap_or_default(),
                rendered.as_ref().map(|r| r.html.clone()),
                rendered.as_ref().map_or(0, |r| r.remote.len()),
                attachments,
            )
        }
        None => (Vec::new(), String::from_utf8_lossy(body).into_owned(), None, 0, Vec::new()),
    };

    let mut summary = summary;
    summary.has_attachments = !attachments.is_empty();
    let unsubscribe = parsed.and_then(unsubscribe_of);
    let disposition_to = parsed
        .and_then(|m| m.header_raw("Disposition-Notification-To"))
        .map(|v| v.trim().to_string())
        .filter(|v| v.contains('@'));

    MessageDetail {
        summary,
        cc,
        message_id,
        references,
        reply_to,
        body_text: body_text.replace("\r\n", "\n"),
        body_html,
        remote_images,
        attachments,
        disposition_to,
        unsubscribe,
    }
}

/// HTML письма вместе с картинками из интернета (по кнопке «Показать картинки»).
pub async fn message_html_with_remote(conn: &mut Connection, folder: &str, uid: u32) -> Result<(String, Vec<String>)> {
    let raw = raw_message(conn, folder, uid).await?;
    let msg = MessageParser::default().parse(&raw).ok_or(AppError::NotFound)?;
    let html = crate::html::html_part(&msg).ok_or(AppError::NotFound)?;
    Ok((html.to_string(), crate::html::render(&msg, html, None).remote))
}

/// Повторная отрисовка с загруженными картинками (без обращения к IMAP).
pub async fn rerender_html(conn: &mut Connection, folder: &str, uid: u32, remote: &std::collections::HashMap<String, String>) -> Result<String> {
    let raw = raw_message(conn, folder, uid).await?;
    let msg = MessageParser::default().parse(&raw).ok_or(AppError::NotFound)?;
    let html = crate::html::html_part(&msg).ok_or(AppError::NotFound)?;
    Ok(crate::html::render(&msg, html, Some(remote)).html)
}

/// Возвращает (имя файла, содержимое) вложения.
pub async fn get_attachment(
    conn: &mut Connection,
    folder: &str,
    uid: u32,
    index: usize,
) -> Result<(String, Vec<u8>)> {
    let raw = raw_message(conn, folder, uid).await?;
    attachment_in(&raw, index)
}

/// Все вложения исходника письма: имя и содержимое (для пересылки).
pub fn attachments_in(raw: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let msg = MessageParser::default().parse(raw).ok_or(AppError::NotFound)?;
    Ok(msg.attachments().enumerate().map(|(i, part)| (attachment_name(part, i), part.contents().to_vec())).collect())
}

/// Исходник письма с сервера (папка открывается при необходимости).
pub async fn message_raw(conn: &mut Connection, folder: &str, uid: u32) -> Result<Vec<u8>> {
    raw_message(conn, folder, uid).await
}

/// Вложение из исходника письма (в том числе из локального хранилища).
pub fn attachment_in(raw: &[u8], index: usize) -> Result<(String, Vec<u8>)> {
    let msg = MessageParser::default().parse(raw).ok_or(AppError::NotFound)?;
    let part = msg.attachments().nth(index).ok_or(AppError::NotFound)?;
    Ok((attachment_name(part, index), part.contents().to_vec()))
}

/// Исходник письма целиком (папка, UID) — для отправки отложенного письма.
pub async fn raw_of(conn: &mut Connection, folder: &str, uid: u32) -> Result<Vec<u8>> {
    if let Some(e) = conn.ews() {
        return e.mime(folder, uid).await;
    }
    conn.ensure_selected(folder).await?;
    let fetches: Vec<Fetch> = conn.imap()?.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])").await?.try_collect().await?;
    fetches.iter().find(|f| f.uid == Some(uid)).and_then(|f| f.body()).map(<[u8]>::to_vec).ok_or(AppError::NotFound)
}

