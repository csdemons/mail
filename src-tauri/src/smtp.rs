//! Отправка писем через SMTP (SMTPS 465 или STARTTLS 587) тем же логином и паролем.

use std::time::Duration;

use lettre::message::header::{ContentType, HeaderName, HeaderValue};
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::error::{AppError, Result};

pub struct OutgoingAttachment {
    pub name: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

pub struct Outgoing {
    pub from_name: Option<String>,
    pub from_address: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
    /// HTML-версия (оформленное письмо). Текст `body` уходит рядом как
    /// альтернатива для клиентов без HTML.
    pub html: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub attachments: Vec<OutgoingAttachment>,
    /// Запросить уведомление о прочтении (Disposition-Notification-To: отправитель).
    pub read_receipt: bool,
    /// Служебные заголовки (например, время отложенной отправки).
    pub extra_headers: Vec<(String, String)>,
}

fn parse_mailbox(raw: &str) -> Result<Mailbox> {
    raw.trim()
        .parse::<Mailbox>()
        .map_err(|_| AppError::Other(format!("Некорректный адрес: «{}»", raw.trim())))
}

/// Разбивает строку адресов «a@b.ru, Имя <c@d.ru>; e@f.ru».
pub fn split_addresses(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_angle = false;
    let mut in_quotes = false;
    for ch in raw.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(ch);
            }
            '<' if !in_quotes => {
                in_angle = true;
                cur.push(ch);
            }
            '>' if !in_quotes => {
                in_angle = false;
                cur.push(ch);
            }
            ',' | ';' if !in_angle && !in_quotes => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Message-ID и References из заголовков — только безопасные символы.
fn clean_header_value(v: &str) -> Option<String> {
    let v: String = v.chars().filter(|c| !c.is_control()).collect();
    let v = v.trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// Чистит HTML из редактора: только оформление текста, без скриптов,
/// обработчиков, форм, внешних картинок и `data:`-ссылок.
pub fn sanitize_html(html: &str) -> String {
    use std::collections::HashSet;
    let tags: HashSet<&str> = [
        "a", "b", "strong", "i", "em", "u", "s", "strike", "del", "sub", "sup", "br", "p", "div", "span", "font",
        "blockquote", "ul", "ol", "li", "h1", "h2", "h3", "h4", "pre", "code", "hr", "table", "thead", "tbody",
        "tr", "td", "th",
    ]
    .into_iter()
    .collect();
    let generic: HashSet<&str> = ["style", "align", "dir"].into_iter().collect();
    let mut b = ammonia::Builder::default();
    b.tags(tags)
        .generic_attributes(generic)
        .add_tag_attributes("font", &["face", "size", "color"])
        .add_tag_attributes("a", &["href", "title"])
        .add_tag_attributes("td", &["colspan", "rowspan"])
        .add_tag_attributes("th", &["colspan", "rowspan"])
        .url_schemes(["http", "https", "mailto", "tel"].into_iter().collect())
        .link_rel(Some("noopener noreferrer"));
    b.clean(html).to_string()
}

/// Обёртка письма: кодировка и базовый шрифт, как в веб-клиентах.
pub fn html_document(fragment: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"></head>\n<body><div style=\"font-family: Arial, Helvetica, sans-serif; font-size: 14px; line-height: 1.4;\">{}</div></body></html>\n",
        sanitize_html(fragment)
    )
}

pub fn build(msg: Outgoing) -> Result<Message> {
    if msg.to.is_empty() {
        return Err(AppError::Other("Укажите хотя бы одного получателя".into()));
    }
    let from = Mailbox::new(
        msg.from_name.filter(|n| !n.trim().is_empty()),
        msg.from_address
            .parse()
            .map_err(|_| AppError::Other(format!("Некорректный адрес отправителя: {}", msg.from_address)))?,
    );

    let mut builder = Message::builder()
        .from(from)
        .subject(msg.subject)
        .date_now()
        .user_agent(format!("Atom Mail {}", env!("CARGO_PKG_VERSION")));
    for to in &msg.to {
        builder = builder.to(parse_mailbox(to)?);
    }
    for cc in &msg.cc {
        builder = builder.cc(parse_mailbox(cc)?);
    }
    if let Some(id) = msg.in_reply_to.as_deref().and_then(clean_header_value) {
        builder = builder.in_reply_to(id);
    }
    if let Some(refs) = msg.references.as_deref().and_then(clean_header_value) {
        builder = builder.references(refs);
    }
    if msg.read_receipt {
        builder = builder.raw_header(HeaderValue::new(HeaderName::new_from_ascii_str("Disposition-Notification-To"), format!("<{}>", msg.from_address.trim())));
    }
    for (name, value) in msg.extra_headers {
        let name = HeaderName::new_from_ascii(name).map_err(|_| AppError::Other("Некорректный заголовок".into()))?;
        if let Some(v) = clean_header_value(&value) {
            builder = builder.raw_header(HeaderValue::new(name, v));
        }
    }

    let text = SinglePart::plain(msg.body.replace("\r\n", "\n"));
    let html = msg.html.filter(|h| !h.trim().is_empty()).map(|h| SinglePart::html(html_document(&h)));
    let message = match (html, msg.attachments.is_empty()) {
        (None, true) => builder.singlepart(text),
        (Some(html), true) => builder.multipart(MultiPart::alternative().singlepart(text).singlepart(html)),
        (html, false) => {
            let mut mixed = match html {
                Some(html) => MultiPart::mixed().multipart(MultiPart::alternative().singlepart(text).singlepart(html)),
                None => MultiPart::mixed().singlepart(text),
            };
            for a in msg.attachments {
                let ct = ContentType::parse(&a.content_type)
                    .unwrap_or_else(|_| ContentType::parse("application/octet-stream").expect("valid"));
                mixed = mixed.singlepart(Attachment::new(a.name).body(a.data, ct));
            }
            builder.multipart(mixed)
        }
    };
    message.map_err(|e| AppError::Other(format!("Не удалось сформировать письмо: {e}")))
}

fn transport(host: &str, port: u16, user: &str, pass: &str) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let creds = Credentials::new(user.to_string(), pass.to_string());
    // 465 — SMTPS (TLS сразу), иначе STARTTLS (обычно 587). Проверка сертификата
    // через системное хранилище, как и для IMAP.
    let builder = if port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(host)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
    }
    .map_err(|e| AppError::Other(format!("SMTP {host}:{port}: {e}")))?;
    Ok(builder
        .port(port)
        .credentials(creds)
        .timeout(Some(Duration::from_secs(60)))
        .build())
}

fn smtp_error(host: &str, port: u16, e: lettre::transport::smtp::Error) -> AppError {
    let text = e.to_string();
    if e.is_permanent() && (text.contains("535") || text.to_lowercase().contains("auth")) {
        AppError::Other("SMTP: сервер отклонил логин или пароль".into())
    } else if e.is_permanent() {
        AppError::Other(format!("SMTP-сервер отказал в отправке: {text}"))
    } else {
        AppError::Other(format!("SMTP-сервер {host}:{port} недоступен или ответил ошибкой: {text}"))
    }
}

pub async fn send(host: &str, port: u16, user: &str, pass: &str, message: &Message) -> Result<()> {
    transport(host, port, user, pass)?
        .send(message.clone())
        .await
        .map(|_| ())
        .map_err(|e| smtp_error(host, port, e))
}

/// Отправка готового исходника (отложенное письмо, уведомление о прочтении):
/// получатели — из заголовков To и Cc, отправитель — From.
pub async fn send_raw(host: &str, port: u16, user: &str, pass: &str, raw: &[u8]) -> Result<()> {
    let msg = mail_parser::MessageParser::default().parse_headers(raw).ok_or_else(|| AppError::Other("Не удалось разобрать письмо".into()))?;
    let addr = |a: &mail_parser::Addr<'_>| a.address().and_then(|s| s.parse::<lettre::Address>().ok());
    let from = msg.from().and_then(|f| f.first()).and_then(addr);
    let mut to: Vec<lettre::Address> = Vec::new();
    for list in [msg.to(), msg.cc()].into_iter().flatten() {
        to.extend(list.iter().filter_map(addr));
    }
    if to.is_empty() {
        return Err(AppError::Other("В письме нет получателей".into()));
    }
    let envelope = lettre::address::Envelope::new(from, to).map_err(|e| AppError::Other(format!("Конверт письма: {e}")))?;
    transport(host, port, user, pass)?
        .send_raw(&envelope, raw)
        .await
        .map(|_| ())
        .map_err(|e| smtp_error(host, port, e))
}

/// Убрать служебный заголовок и поставить текущую дату (отправка отложенного письма).
pub fn finalize_scheduled(raw: &[u8], header: &str) -> Vec<u8> {
    let mut bytes = format!("Date: {}\r\n", chrono::Utc::now().to_rfc2822()).into_bytes();
    bytes.extend_from_slice(&remove_headers(raw, &[header, "date"]));
    bytes
}

/// Убрать служебный заголовок (письмо снова обычный черновик).
pub fn strip_header(raw: &[u8], header: &str) -> Vec<u8> {
    remove_headers(raw, &[header])
}

fn remove_headers(raw: &[u8], names: &[&str]) -> Vec<u8> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 2).unwrap_or(raw.len());
    let (head, body) = raw.split_at(split);
    let head = String::from_utf8_lossy(head);
    let mut out = String::new();
    let mut skip = false;
    let prefixes: Vec<String> = names.iter().map(|n| format!("{}:", n.to_ascii_lowercase())).collect();
    for line in head.split_inclusive("\r\n") {
        let continuation = line.starts_with(' ') || line.starts_with('\t');
        if !continuation {
            let l = line.to_ascii_lowercase();
            skip = prefixes.iter().any(|p| l.starts_with(p.as_str()));
        }
        if !skip {
            out.push_str(line);
        }
    }
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Уведомление о прочтении (RFC 8098): multipart/report с машиночитаемой частью.
pub fn build_mdn(from_name: &str, from: &str, to: &str, original_subject: &str, original_id: Option<&str>) -> Vec<u8> {
    use base64::Engine as _;
    let enc = |s: &str| format!("=?utf-8?b?{}?=", base64::engine::general_purpose::STANDARD.encode(s.as_bytes()));
    let boundary = format!("mdn-{}", uuid::Uuid::new_v4().simple());
    let date = chrono::Utc::now().to_rfc2822();
    let from_hdr = if from_name.trim().is_empty() { format!("<{from}>") } else { format!("{} <{from}>", enc(from_name.trim())) };
    let text = format!("Ваше письмо «{original_subject}» прочитано {}.", chrono::Local::now().format("%d.%m.%Y в %H:%M"));
    let mut raw = String::new();
    raw.push_str(&format!("Date: {date}\r\nFrom: {from_hdr}\r\nTo: <{to}>\r\nSubject: {}\r\n", enc(&format!("Прочитано: {original_subject}"))));
    raw.push_str(&format!("Message-ID: <{}@atom-mail>\r\nMIME-Version: 1.0\r\n", uuid::Uuid::new_v4()));
    if let Some(id) = original_id.and_then(clean_header_value) {
        raw.push_str(&format!("References: {id}\r\nIn-Reply-To: {id}\r\n"));
    }
    raw.push_str(&format!("Content-Type: multipart/report; report-type=disposition-notification; boundary=\"{boundary}\"\r\n\r\n"));
    raw.push_str(&format!("--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n", base64::engine::general_purpose::STANDARD.encode(text.as_bytes())));
    raw.push_str(&format!("--{boundary}\r\nContent-Type: message/disposition-notification\r\n\r\nReporting-UA: Atom Mail {}\r\nFinal-Recipient: rfc822;{from}\r\n", env!("CARGO_PKG_VERSION")));
    if let Some(id) = original_id.and_then(clean_header_value) {
        raw.push_str(&format!("Original-Message-ID: {id}\r\n"));
    }
    raw.push_str(&format!("Disposition: manual-action/MDN-sent-manually; displayed\r\n\r\n--{boundary}--\r\n"));
    raw.into_bytes()
}

/// Проверка подключения (TLS + EHLO) — для диагностики.
pub async fn check(host: &str, port: u16, user: &str, pass: &str) -> std::result::Result<bool, String> {
    transport(host, port, user, pass)
        .map_err(|e| e.to_string())?
        .test_connection()
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_address_lists() {
        assert_eq!(
            split_addresses("a@b.ru, \"Иванов, Иван\" <ivanov@nrcki.ru>; c@d.ru ,"),
            vec!["a@b.ru", "\"Иванов, Иван\" <ivanov@nrcki.ru>", "c@d.ru"]
        );
    }

    #[test]
    fn sanitizes_outgoing_html() {
        let clean = sanitize_html(r##"<p style="font-family: Georgia" onclick="x()"><b>Жирный</b> <font face="Courier New" size="5" color="#ff0000">шрифт</font></p><script>alert(1)</script><img src="https://t/x.png"><a href="javascript:alert(1)">a</a><a href="https://nrcki.ru">b</a><iframe src="https://x"></iframe>"##);
        assert!(clean.contains(r#"<p style="font-family: Georgia"><b>Жирный</b>"#), "{clean}");
        assert!(clean.contains(r##"<font face="Courier New" size="5" color="#ff0000">"##), "{clean}");
        assert!(!clean.contains("onclick") && !clean.contains("script") && !clean.contains("<img") && !clean.contains("iframe"), "{clean}");
        assert!(!clean.contains("javascript") && clean.contains(r#"href="https://nrcki.ru""#), "{clean}");
        assert!(html_document("<b>x</b>").contains("<meta charset=\"utf-8\">"));
    }

    #[test]
    fn builds_reply_with_attachment() {
        let msg = build(Outgoing {
            from_name: Some("Юшков Дмитрий".into()),
            from_address: "yushkov_do@nrcki.ru".into(),
            to: vec!["Коллега <colleague@nrcki.ru>".into()],
            cc: vec![],
            subject: "Re: Отчёт".into(),
            body: "Спасибо!\nДмитрий".into(),
            html: Some("<p><b>Спасибо!</b></p><p style=\"color: red\" onclick=\"x()\">Дмитрий</p><script>alert(1)</script><img src=\"https://track/x.png\">".into()),
            in_reply_to: Some("<abc@nrcki.ru>".into()),
            references: Some("<root@nrcki.ru> <abc@nrcki.ru>".into()),
            read_receipt: true,
            extra_headers: vec![("X-Atom-Send-At".into(), "2030-01-01T09:00:00+03:00".into())],
            attachments: vec![OutgoingAttachment {
                name: "Отчёт.txt".into(),
                content_type: "text/plain".into(),
                data: "данные".as_bytes().to_vec(),
            }],
        })
        .expect("message");
        let raw = String::from_utf8(msg.formatted()).expect("utf8");
        assert!(raw.contains("In-Reply-To: <abc@nrcki.ru>"));
        assert!(raw.contains("References: <root@nrcki.ru> <abc@nrcki.ru>"));
        assert!(raw.contains("multipart/mixed") && raw.contains("multipart/alternative") && raw.contains("text/html"));
        assert!(raw.contains("=?utf-8?b?"), "non-ASCII headers must be encoded");
        assert!(raw.contains("Disposition-Notification-To: <yushkov_do@nrcki.ru>"), "{raw}");
        assert!(raw.contains("X-Atom-Send-At: 2030-01-01T09:00:00+03:00"));
        let fin = String::from_utf8(finalize_scheduled(raw.as_bytes(), "X-Atom-Send-At")).unwrap();
        assert!(!fin.contains("X-Atom-Send-At") && fin.starts_with("Date: ") && fin.matches("\nDate: ").count() == 0, "{}", &fin[..300]);
        assert!(fin.contains("Disposition-Notification-To") && fin.ends_with(raw.split_once("\r\n\r\n").unwrap().1));
        let mdn = String::from_utf8(build_mdn("Тест", "me@nrcki.ru", "you@nrcki.ru", "Отчёт", Some("<abc@nrcki.ru>"))).unwrap();
        assert!(mdn.contains("report-type=disposition-notification") && mdn.contains("Original-Message-ID: <abc@nrcki.ru>") && mdn.contains("Disposition: manual-action/MDN-sent-manually; displayed"));
        assert!(build(Outgoing {
            from_name: None,
            from_address: "a@b.ru".into(),
            to: vec!["не адрес".into()],
            cc: vec![],
            subject: String::new(),
            body: String::new(),
            html: None,
            in_reply_to: None,
            references: None,
            attachments: vec![],
            read_receipt: false,
            extra_headers: vec![],
        })
        .is_err());
    }
}
