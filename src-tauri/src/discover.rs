//! Автоопределение серверов по адресу почты — вход «адрес + пароль», как в Thunderbird:
//! 1) известные провайдеры; 2) Mozilla autoconfig (сервер домена и база Thunderbird);
//! 3) пробное подключение к imap./mail./smtp. домена.

use std::time::Duration;

use serde::Serialize;

use crate::error::{AppError, Result};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    /// Логин: обычно весь адрес, у некоторых серверов — часть до @.
    pub username: String,
    /// Откуда взяты настройки (для подсказки пользователю).
    pub source: String,
    /// Предупреждение (например, Gmail — нужен пароль приложения).
    pub note: Option<String>,
}

fn split_email(email: &str) -> Result<(String, String)> {
    let email = email.trim();
    let (local, domain) = email.rsplit_once('@').ok_or_else(|| AppError::Other("Введите адрес почты целиком, например ivanov@yandex.ru".into()))?;
    let domain = domain.trim().trim_end_matches('.').to_lowercase();
    let valid = !local.is_empty()
        && domain.contains('.')
        && domain.len() <= 253
        && domain.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !l.starts_with('-'));
    if !valid {
        return Err(AppError::Other(format!("Некорректный домен почты: «{domain}»")));
    }
    Ok((local.to_string(), domain))
}

/// Известные почтовые службы (без сетевых запросов).
fn known(domain: &str, email: &str) -> Option<Discovered> {
    let d = |imap: &str, smtp: &str, note: Option<&str>| Discovered {
        imap_host: imap.into(),
        imap_port: 993,
        smtp_host: smtp.into(),
        smtp_port: 465,
        username: email.to_string(),
        source: "известный провайдер".into(),
        note: note.map(str::to_string),
    };
    let app_pw = "Для внешних почтовых программ провайдер требует «пароль приложения» — создайте его в настройках безопасности почтового ящика.";
    Some(match domain {
        "yandex.ru" | "ya.ru" | "yandex.com" | "yandex.by" | "yandex.kz" | "narod.ru" => d("imap.yandex.ru", "smtp.yandex.ru", Some(app_pw)),
        "mail.ru" | "inbox.ru" | "list.ru" | "bk.ru" | "internet.ru" | "my.com" => d("imap.mail.ru", "smtp.mail.ru", Some(app_pw)),
        "gmail.com" | "googlemail.com" => d("imap.gmail.com", "smtp.gmail.com", Some(app_pw)),
        "rambler.ru" | "lenta.ru" | "autorambler.ru" | "myrambler.ru" | "ro.ru" => d("imap.rambler.ru", "smtp.rambler.ru", None),
        "icloud.com" | "me.com" | "mac.com" => Discovered { smtp_port: 587, ..d("imap.mail.me.com", "smtp.mail.me.com", Some(app_pw)) },
        "nrcki.ru" => d("rp.nrcki.ru", "rp.nrcki.ru", None),
        _ => return None,
    })
}

fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(5))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|e| AppError::Other(e.to_string()))
}

/// Разбор Mozilla autoconfig (config-v1.1.xml): IMAP только с TLS (SSL, порт 993),
/// SMTP — SSL (465) или STARTTLS (587).
pub fn parse_autoconfig(xml: &str, email: &str) -> Option<Discovered> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let local = email.split('@').next().unwrap_or_default();
    let text = |n: roxmltree::Node<'_, '_>, tag: &str| n.children().find(|c| c.has_tag_name(tag)).and_then(|c| c.text()).map(|t| t.trim().to_string());
    let user = |n: roxmltree::Node<'_, '_>| {
        text(n, "username").map(|u| u.replace("%EMAILADDRESS%", email).replace("%EMAILLOCALPART%", local)).unwrap_or_else(|| email.to_string())
    };
    let imap = doc
        .descendants()
        .filter(|n| n.has_tag_name("incomingServer") && n.attribute("type") == Some("imap"))
        .find(|n| text(*n, "socketType").as_deref() == Some("SSL"))?;
    let smtps: Vec<_> = doc.descendants().filter(|n| n.has_tag_name("outgoingServer") && n.attribute("type") == Some("smtp")).collect();
    let smtp = smtps
        .iter()
        .find(|n| text(**n, "socketType").as_deref() == Some("SSL"))
        .or_else(|| smtps.iter().find(|n| text(**n, "socketType").as_deref() == Some("STARTTLS")))?;
    Some(Discovered {
        imap_host: text(imap, "hostname")?,
        imap_port: text(imap, "port")?.parse().ok()?,
        smtp_host: text(*smtp, "hostname")?,
        smtp_port: text(*smtp, "port")?.parse().ok()?,
        username: user(imap),
        source: String::new(),
        note: None,
    })
}

async fn autoconfig(domain: &str, email: &str) -> Option<Discovered> {
    let client = http().ok()?;
    let enc = percent_encoding::utf8_percent_encode(email, percent_encoding::NON_ALPHANUMERIC).to_string();
    let mut urls = vec![
        (format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={enc}"), "настройки домена"),
        (format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={enc}"), "настройки домена"),
        (format!("https://autoconfig.thunderbird.net/v1.1/{domain}"), "база Thunderbird"),
    ];
    // Автотесты: адрес autoconfig задаётся явно (в релизе этого нет).
    #[cfg(debug_assertions)]
    if let Ok(u) = std::env::var("RRCKI_E2E_AUTOCONFIG_URL") {
        urls.insert(0, (format!("{u}?emailaddress={enc}"), "настройки домена"));
    }
    for (url, source) in urls {
        let Ok(resp) = client.get(&url).send().await else { continue };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(body) = crate::netio::text_limited(resp, 256 * 1024).await else { continue };
        if let Some(mut found) = parse_autoconfig(&body, email) {
            found.source = source.into();
            return Some(found);
        }
    }
    None
}

/// Проверка: на хосте отвечает TLS-сервер с правильным сертификатом.
async fn tls_ok(host: &str, port: u16) -> bool {
    let attempt = async {
        let tcp = tokio::net::TcpStream::connect((host, port)).await.ok()?;
        let connector = native_tls::TlsConnector::builder().min_protocol_version(Some(native_tls::Protocol::Tlsv12)).build().ok()?;
        tokio_native_tls::TlsConnector::from(connector).connect(host, tcp).await.ok()
    };
    matches!(tokio::time::timeout(Duration::from_secs(5), attempt).await, Ok(Some(_)))
}

async fn tcp_ok(host: &str, port: u16) -> bool {
    matches!(tokio::time::timeout(Duration::from_secs(4), tokio::net::TcpStream::connect((host, port))).await, Ok(Ok(_)))
}

/// Пробное подключение к типичным именам серверов домена.
async fn probe(domain: &str, email: &str) -> Option<Discovered> {
    let candidates = [format!("imap.{domain}"), format!("mail.{domain}"), domain.to_string()];
    let checks = futures::future::join_all(candidates.iter().map(|h| tls_ok(h, 993))).await;
    let imap_host = candidates.iter().zip(checks).find(|(_, ok)| *ok)?.0.clone();
    let smtp_candidates = [format!("smtp.{domain}"), format!("mail.{domain}"), imap_host.clone()];
    let mut smtp = None;
    for h in &smtp_candidates {
        if tls_ok(h, 465).await {
            smtp = Some((h.clone(), 465));
            break;
        }
        if tcp_ok(h, 587).await {
            smtp = Some((h.clone(), 587));
            break;
        }
    }
    let (smtp_host, smtp_port) = smtp.unwrap_or((imap_host.clone(), 465));
    Some(Discovered { imap_host, imap_port: 993, smtp_host, smtp_port, username: email.to_string(), source: "найдено подключением к серверу".into(), note: None })
}

pub async fn discover(email: &str) -> Result<Discovered> {
    let (_, domain) = split_email(email)?;
    let email = email.trim();
    if let Some(d) = known(&domain, email) {
        return Ok(d);
    }
    if let Some(d) = autoconfig(&domain, email).await {
        return Ok(d);
    }
    if let Some(d) = probe(&domain, email).await {
        return Ok(d);
    }
    Err(AppError::Other(format!("Не удалось определить серверы для «{domain}». Нажмите «Настроить вручную» и укажите адреса серверов (их сообщает администратор почты).")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_providers_and_validation() {
        let y = known("yandex.ru", "a@yandex.ru").unwrap();
        assert_eq!((y.imap_host.as_str(), y.imap_port, y.smtp_host.as_str(), y.smtp_port), ("imap.yandex.ru", 993, "smtp.yandex.ru", 465));
        assert!(known("bk.ru", "a@bk.ru").unwrap().imap_host == "imap.mail.ru");
        assert!(known("example.org", "a@example.org").is_none());
        assert!(split_email("ivanov").is_err());
        assert!(split_email("a@localhost").is_err());
        assert!(split_email("a@ex ample.ru").is_err());
        assert_eq!(split_email(" Ivanov@Example.RU ").unwrap().1, "example.ru");
    }

    #[test]
    fn parses_autoconfig() {
        let xml = r#"<?xml version="1.0"?><clientConfig version="1.1"><emailProvider id="example.org">
          <incomingServer type="pop3"><hostname>pop.example.org</hostname><port>995</port><socketType>SSL</socketType></incomingServer>
          <incomingServer type="imap"><hostname>imap.example.org</hostname><port>143</port><socketType>STARTTLS</socketType><username>%EMAILADDRESS%</username></incomingServer>
          <incomingServer type="imap"><hostname>mx.example.org</hostname><port>993</port><socketType>SSL</socketType><username>%EMAILLOCALPART%</username></incomingServer>
          <outgoingServer type="smtp"><hostname>smtp.example.org</hostname><port>587</port><socketType>STARTTLS</socketType></outgoingServer>
        </emailProvider></clientConfig>"#;
        let d = parse_autoconfig(xml, "ivan@example.org").unwrap();
        assert_eq!((d.imap_host.as_str(), d.imap_port, d.smtp_host.as_str(), d.smtp_port, d.username.as_str()), ("mx.example.org", 993, "smtp.example.org", 587, "ivan"));
        assert!(parse_autoconfig("<clientConfig/>", "a@b.ru").is_none());
        assert!(parse_autoconfig("not xml", "a@b.ru").is_none());
    }
}
