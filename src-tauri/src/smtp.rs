//! Отправка писем через SMTP (SMTPS 465 или STARTTLS 587) тем же логином и паролем.

use std::time::Duration;

use lettre::message::header::ContentType;
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
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub attachments: Vec<OutgoingAttachment>,
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
        .user_agent(format!("RRCKI Mail {}", env!("CARGO_PKG_VERSION")));
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

    let text = SinglePart::plain(msg.body.replace("\r\n", "\n"));
    let message = if msg.attachments.is_empty() {
        builder.singlepart(text)
    } else {
        let mut mixed = MultiPart::mixed().singlepart(text);
        for a in msg.attachments {
            let ct = ContentType::parse(&a.content_type)
                .unwrap_or_else(|_| ContentType::parse("application/octet-stream").expect("valid"));
            mixed = mixed.singlepart(Attachment::new(a.name).body(a.data, ct));
        }
        builder.multipart(mixed)
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
    fn builds_reply_with_attachment() {
        let msg = build(Outgoing {
            from_name: Some("Юшков Дмитрий".into()),
            from_address: "yushkov_do@nrcki.ru".into(),
            to: vec!["Коллега <colleague@nrcki.ru>".into()],
            cc: vec![],
            subject: "Re: Отчёт".into(),
            body: "Спасибо!\nДмитрий".into(),
            in_reply_to: Some("<abc@nrcki.ru>".into()),
            references: Some("<root@nrcki.ru> <abc@nrcki.ru>".into()),
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
        assert!(raw.contains("multipart/mixed"));
        assert!(raw.contains("=?utf-8?b?"), "non-ASCII headers must be encoded");
        assert!(build(Outgoing {
            from_name: None,
            from_address: "a@b.ru".into(),
            to: vec!["не адрес".into()],
            cc: vec![],
            subject: String::new(),
            body: String::new(),
            in_reply_to: None,
            references: None,
            attachments: vec![],
        })
        .is_err());
    }
}
