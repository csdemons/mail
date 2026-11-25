//! Доверие к сертификатам серверов: системное хранилище плюс корни, вшитые в программу
//! (например, корневой и выпускающий сертификаты УЦ Минцифры).
//!
//! Корни добавляются только клиентам, которые ходят на серверы из настроек: IMAP, SMTP,
//! CalDAV/CardDAV, EWS, SOGo, Nextcloud и автообнаружение. Проверка обновлений и загрузка
//! картинок или ссылок из писем остаются на системном хранилище: вшитый УЦ не должен
//! давать возможности подменить обновление программы или запрос к произвольному адресу.

use base64::Engine as _;

use crate::error::{AppError, Result};

/// Вшитые корни. Файл может быть пустым или содержать только комментарии — тогда
/// программа доверяет только системному хранилищу.
const EXTRA_CA: &str = include_str!("../certs/extra-ca.pem");

/// Разобрать PEM: в одном файле может быть несколько сертификатов. Комментарии, пустые
/// строки и блоки, которые не декодируются, пропускаются — файл не должен ломать запуск.
pub fn parse_pem(pem: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut body = String::new();
    let mut inside = false;
    for line in pem.lines() {
        let line = line.trim();
        if line.starts_with("-----BEGIN") {
            inside = true;
            body.clear();
        } else if line.starts_with("-----END") {
            if inside {
                let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
                match base64::engine::general_purpose::STANDARD.decode(compact) {
                    Ok(der) if !der.is_empty() => out.push(der),
                    _ => log::warn!("certs/extra-ca.pem: блок сертификата пропущен — не читается"),
                }
            }
            inside = false;
            body.clear();
        } else if inside && !line.is_empty() && !line.starts_with('#') {
            body.push_str(line);
        }
    }
    out
}

/// DER вшитых корней: разбираются один раз на процесс.
fn extra_der() -> &'static [Vec<u8>] {
    static DER: std::sync::OnceLock<Vec<Vec<u8>>> = std::sync::OnceLock::new();
    DER.get_or_init(|| parse_pem(EXTRA_CA))
}

/// Есть ли вшитые корни (для диагностики и подсказок в интерфейсе).
pub fn has_extra_roots() -> bool {
    !extra_der().is_empty()
}

fn native_roots() -> Result<Vec<native_tls::Certificate>> {
    extra_der()
        .iter()
        .map(|der| {
            native_tls::Certificate::from_der(der)
                .map_err(|e| AppError::Tls(format!("Вшитый корневой сертификат не читается: {e}")))
        })
        .collect()
}

/// TLS-соединение для IMAP, EWS и проверки серверов: системные корни плюс вшитые, TLS 1.2+.
pub fn connector() -> Result<native_tls::TlsConnector> {
    let mut builder = native_tls::TlsConnector::builder();
    builder.min_protocol_version(Some(native_tls::Protocol::Tlsv12));
    for cert in native_roots()? {
        builder.add_root_certificate(cert);
    }
    builder.build().map_err(|e| AppError::Tls(e.to_string()))
}

/// HTTP-клиент для серверов из настроек (DAV, Nextcloud, SOGo, автообнаружение).
pub fn client_builder() -> Result<reqwest::ClientBuilder> {
    let mut builder = reqwest::Client::builder();
    for der in extra_der() {
        let cert = reqwest::Certificate::from_der(der)
            .map_err(|e| AppError::Other(format!("Вшитый корневой сертификат не читается: {e}")))?;
        builder = builder.add_root_certificate(cert);
    }
    Ok(builder)
}

/// Параметры TLS для SMTP: тот же набор корней, что и у остальных серверных клиентов.
pub fn smtp_tls(host: &str) -> Result<lettre::transport::smtp::client::TlsParameters> {
    let mut builder = lettre::transport::smtp::client::TlsParameters::builder(host.to_string());
    for der in extra_der() {
        let cert = lettre::transport::smtp::client::Certificate::from_der(der.clone())
            .map_err(|e| AppError::Other(format!("Вшитый корневой сертификат не читается: {e}")))?;
        builder = builder.add_root_certificate(cert);
    }
    builder.build().map_err(|e| AppError::Tls(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "-----BEGIN CERTIFICATE-----\nQUJD\n-----END CERTIFICATE-----\n";
    const TWO: &str = "# комментарий\n\
        -----BEGIN CERTIFICATE-----\nQUJD\n-----END CERTIFICATE-----\n\
        \n\
        -----BEGIN CERTIFICATE-----\nREVG\n-----END CERTIFICATE-----\n";

    #[test]
    fn empty_file_means_system_roots_only() {
        assert!(parse_pem("").is_empty());
        assert!(parse_pem("# только комментарии\n\n").is_empty());
    }

    #[test]
    fn several_certificates_in_one_file() {
        assert_eq!(parse_pem(ONE), vec![b"ABC".to_vec()]);
        assert_eq!(parse_pem(TWO), vec![b"ABC".to_vec(), b"DEF".to_vec()]);
    }

    #[test]
    fn broken_block_is_skipped_without_failing() {
        let broken = "-----BEGIN CERTIFICATE-----\n***\n-----END CERTIFICATE-----\n";
        assert!(parse_pem(broken).is_empty());
        // Хороший блок рядом с битым остаётся.
        assert_eq!(parse_pem(&format!("{broken}{ONE}")), vec![b"ABC".to_vec()]);
    }

    #[test]
    fn connectors_and_clients_are_built() {
        assert!(connector().is_ok());
        assert!(client_builder().is_ok());
        assert!(smtp_tls("mail.example.test").is_ok());
    }

    #[test]
    fn bundled_roots_are_valid_certificates() {
        // Вшитые сертификаты должны читаться как X.509 на всех платформах: испорченный
        // PEM ловится здесь, а не у пользователя при подключении к серверу.
        let roots = extra_der();
        assert!(!roots.is_empty(), "certs/extra-ca.pem пуст — сертификаты УЦ не вшиты");
        for der in roots {
            native_tls::Certificate::from_der(der)
                .expect("сертификат в certs/extra-ca.pem не читается как X.509");
        }
    }

    #[test]
    #[ignore = "нужен доступ в сеть к серверу с сертификатом УЦ Минцифры"]
    fn extra_roots_alone_trust_russian_ca_site() {
        // Доверяем ТОЛЬКО вшитым корням. zakupki.gov.ru выбран потому, что системное
        // хранилище его не доверяет (проверено: SELF_SIGNED_CERT_IN_CHAIN), а сертификат
        // выпущен УЦ Минцифры — значит успех соединения доказывает работу вшитых корней.
        let mut builder = native_tls::TlsConnector::builder();
        builder.disable_built_in_roots(true);
        for der in extra_der() {
            builder.add_root_certificate(native_tls::Certificate::from_der(der).expect("X.509"));
        }
        let connector = builder.build().expect("connector");
        let host = "zakupki.gov.ru";
        let tcp = std::net::TcpStream::connect((host, 443)).expect("TCP");
        tcp.set_read_timeout(Some(std::time::Duration::from_secs(20))).ok();
        connector.connect(host, tcp).expect("TLS по вшитым корням");
    }
}
