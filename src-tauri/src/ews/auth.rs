//! Соединение и вход в Exchange: TLS, NTLMv2 (с привязкой к каналу), Basic,
//! Autodiscover, пошаговая диагностика. Свое соединение HTTP/1.1 поверх TLS —
//! вход NTLM привязан к сокету, пул не подходит.

use base64::Engine as _;
use http::StatusCode;
use url::Url;

use crate::error::{AppError, Result};

use super::*;

fn tls_error(e: &dyn std::error::Error, host: &str) -> AppError {
    let chain = error_chain(e);
    let low = chain.to_lowercase();
    if low.contains("certificate") || low.contains("сертификат") || low.contains("unknownissuer") || low.contains("cert_") || low.contains("self signed") || low.contains("self-signed") || low.contains("untrusted") {
        AppError::Tls(format!("сертификат сервера {host} не принят системой ({chain}). Если сервер использует сертификат внутреннего центра сертификации организации, его нужно установить в доверенные корневые сертификаты компьютера"))
    } else {
        AppError::Tls(format!("{host}: {chain}"))
    }
}

async fn open_pinned(url: &Url) -> Result<Pinned> {
    let host = url.host_str().unwrap_or_default().to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let tcp = tokio::time::timeout(Duration::from_secs(15), tokio::net::TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| AppError::Timeout)?
        .map_err(|e| AppError::Unreachable(host.clone(), e.to_string()))?;
    let _ = tcp.set_nodelay(true);
    let connector = crate::tls::connector().map_err(|e| tls_error(&e, &host))?;
    let tls = tokio::time::timeout(Duration::from_secs(15), tokio_native_tls::TlsConnector::from(connector).connect(&host, tcp))
        .await
        .map_err(|_| AppError::Timeout)?
        .map_err(|e| tls_error(&e, &host))?;
    let cert = tls.get_ref().peer_certificate().ok().flatten().and_then(|c| c.to_der().ok());
    let (sender, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(tls))
        .await
        .map_err(|e| AppError::Other(format!("Exchange: {}", error_chain(&e))))?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    Ok(Pinned { sender, cert, used: std::time::Instant::now() })
}

/// POST по соединению; тело ответа — не больше `limit` байт.
async fn send_pinned(p: &mut Pinned, url: &Url, body: &str, auth: Option<&str>, limit: usize) -> Result<Resp> {
    use http_body_util::BodyExt as _;
    let host = match (url.host_str(), url.port()) {
        (Some(h), Some(port)) => format!("{h}:{port}"),
        (Some(h), None) => h.to_string(),
        _ => String::new(),
    };
    let path = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    };
    let mut req = http::Request::post(path)
        .header(http::header::HOST, host)
        .header(http::header::CONTENT_TYPE, "text/xml; charset=utf-8")
        .header(http::header::CONTENT_LENGTH, body.len())
        // SOAP-ответы — XML, сжимается в 5–10 раз (сжатие динамического содержимого в IIS).
        .header(http::header::ACCEPT_ENCODING, "gzip, deflate")
        .header(http::header::USER_AGENT, concat!("AtomMail/", env!("CARGO_PKG_VERSION")));
    if let Some(a) = auth {
        req = req.header(http::header::AUTHORIZATION, a);
    }
    let req = req.body(http_body_util::Full::new(bytes::Bytes::from(body.to_owned()))).map_err(|e| AppError::Other(e.to_string()))?;
    let work = async {
        p.sender.ready().await.map_err(|_| AppError::ConnectionLost)?;
        let resp = p.sender.send_request(req).await.map_err(|_| AppError::ConnectionLost)?;
        let (parts, b) = resp.into_parts();
        if parts.headers.get(http::header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).is_some_and(|n| n > limit as u64) {
            return Err(too_large(limit));
        }
        let collected = http_body_util::Limited::new(b, limit).collect().await.map_err(|e| {
            if e.downcast_ref::<http_body_util::LengthLimitError>().is_some() { too_large(limit) } else { AppError::ConnectionLost }
        })?;
        let mut headers = parts.headers;
        let enc = headers.remove(http::header::CONTENT_ENCODING).and_then(|v| v.to_str().ok().map(str::to_ascii_lowercase));
        let body = decode_body(enc.as_deref(), collected.to_bytes().to_vec(), limit)?;
        Ok(Resp { status: parts.status, headers, body })
    };
    let out = tokio::time::timeout(Duration::from_secs(120), work).await.map_err(|_| AppError::Timeout)?;
    p.used = std::time::Instant::now();
    out
}

/// Распаковать сжатый ответ (gzip/deflate), не больше `limit` байт после распаковки.
fn decode_body(encoding: Option<&str>, body: Vec<u8>, limit: usize) -> Result<Vec<u8>> {
    use std::io::Read as _;
    let read_all = |mut r: Box<dyn std::io::Read + '_>| -> Result<Vec<u8>> {
        let mut out = Vec::new();
        (&mut r).take(limit as u64 + 1).read_to_end(&mut out).map_err(|e| AppError::Other(format!("Exchange: не удалось распаковать ответ: {e}")))?;
        if out.len() > limit { return Err(too_large(limit)); }
        Ok(out)
    };
    match encoding.map(str::trim) {
        None | Some("") | Some("identity") => Ok(body),
        Some("gzip") | Some("x-gzip") => read_all(Box::new(flate2::read::GzDecoder::new(&body[..]))),
        // «deflate» по HTTP — поток zlib, но некоторые серверы шлют «голый» deflate.
        Some("deflate") => read_all(Box::new(flate2::read::ZlibDecoder::new(&body[..])))
            .or_else(|_| read_all(Box::new(flate2::read::DeflateDecoder::new(&body[..])))),
        Some(other) => Err(AppError::Other(format!("Exchange: неизвестное сжатие ответа «{other}»"))),
    }
}

fn too_large(max: usize) -> AppError {
    AppError::Other(format!("Ответ сервера слишком большой (больше {} МБ) — загрузка прервана", max / (1024 * 1024)))
}

/// Адрес EWS из того, что ввёл пользователь: `mail.corp.ru` → `https://mail.corp.ru/EWS/Exchange.asmx`.
pub fn normalize_url(input: &str) -> Result<Url> {
    let s = input.trim();
    if s.to_ascii_lowercase().starts_with("http://") {
        return Err(AppError::Other("Адрес Exchange: только https:// — по http пароль не отправляется".into()));
    }
    let full = if s.contains("://") { s.to_string() } else { format!("https://{s}") };
    let mut u = Url::parse(&full).map_err(|_| AppError::Other(format!("Некорректный адрес сервера Exchange: {s}")))?;
    if u.scheme() != "https" || u.host_str().is_none() {
        return Err(AppError::Other("Адрес Exchange должен начинаться с https://".into()));
    }
    if u.path() == "/" || u.path().is_empty() {
        u.set_path("/EWS/Exchange.asmx");
    }
    Ok(u)
}
fn www_auth(resp: &Resp) -> Vec<String> {
    resp.headers.get_all("www-authenticate").iter().filter_map(|v| v.to_str().ok().map(str::to_string)).collect()
}

fn ntlm_token(headers: &[String], scheme: &str) -> Option<Vec<u8>> {
    headers.iter().find_map(|h| {
        let (s, tok) = h.split_once(' ')?;
        if !s.eq_ignore_ascii_case(scheme) {
            return None;
        }
        base64::engine::general_purpose::STANDARD.decode(tok.trim()).ok()
    })
}
impl EwsSession {
    /// Соединение сеанса: открыть, если его нет, сервер его закрыл или оно долго
    /// простаивало. Новое соединение — без входа NTLM.
    async fn ensure_conn(&mut self) -> Result<()> {
        let stale = self.conn.as_ref().is_none_or(|p| p.sender.is_closed() || p.used.elapsed() > IDLE_REOPEN);
        if stale {
            self.conn = None;
            self.authed = false;
            self.conn = Some(open_pinned(&self.url).await?);
        }
        Ok(())
    }

    async fn http_send(&mut self, body: &str, auth: Option<String>, limit: usize) -> Result<Resp> {
        self.ensure_conn().await?;
        let p = self.conn.as_mut().ok_or(AppError::ConnectionLost)?;
        let r = send_pinned(p, &self.url, body, auth.as_deref(), limit).await;
        if r.is_err() {
            // Соединение в неизвестном состоянии — следующий запрос откроет новое.
            self.conn = None;
            self.authed = false;
        }
        r
    }

    /// Выполнить запрос SOAP, при необходимости войти (NTLM или Basic).
    pub(super) async fn post_raw(&mut self, body: &str, limit: usize) -> Result<String> {
        let mut tried_basic = false;
        for _ in 0..4 {
            self.ensure_conn().await?;
            let resp = match self.auth {
                Auth::Basic => {
                    let token = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", self.login, self.password));
                    self.http_send(body, Some(format!("Basic {token}")), limit).await?
                }
                Auth::Ntlm(scheme) if !self.authed => self.ntlm(body, scheme, limit).await?,
                _ => self.http_send(body, None, limit).await?,
            };
            match resp.status {
                StatusCode::UNAUTHORIZED => {
                    let offered = www_auth(&resp);
                    let has = |s: &str| offered.iter().any(|h| h.split_whitespace().next().is_some_and(|x| x.eq_ignore_ascii_case(s)));
                    match self.auth {
                        Auth::Unknown => {
                            self.auth = if has("NTLM") {
                                Auth::Ntlm("NTLM")
                            } else if has("Negotiate") {
                                Auth::Ntlm("Negotiate")
                            } else if has("Basic") {
                                Auth::Basic
                            } else {
                                return Err(AppError::Other("Сервер Exchange не предлагает поддерживаемый способ входа (нужен NTLM или Basic)".into()));
                            };
                        }
                        Auth::Ntlm(_) if self.authed => self.authed = false,
                        // NTLM не прошёл, а сервер принимает и простой вход: так бывает за
                        // обратным прокси, который не держит соединение для NTLM.
                        Auth::Ntlm(_) if has("Basic") && !tried_basic => {
                            tried_basic = true;
                            self.auth = Auth::Basic;
                        }
                        _ => return Err(AppError::AuthFailed),
                    }
                }
                StatusCode::OK | StatusCode::INTERNAL_SERVER_ERROR => {
                    if matches!(self.auth, Auth::Ntlm(_)) {
                        self.authed = true;
                    }
                    let ok = resp.status == StatusCode::OK;
                    let text = resp.text();
                    if !ok && !text.contains("Envelope") {
                        return Err(AppError::Other("Сервер Exchange ответил ошибкой (HTTP 500)".into()));
                    }
                    return Ok(text);
                }
                s if s.is_redirection() => {
                    return Err(AppError::Other(format!("Адрес Exchange перенаправляет запросы (HTTP {s}) — укажите точный адрес EWS")));
                }
                s => return Err(AppError::Other(format!("Сервер Exchange ответил HTTP {s}"))),
            }
        }
        Err(AppError::AuthFailed)
    }

    /// Вход NTLM: NEGOTIATE → CHALLENGE → AUTHENTICATE на одном соединении.
    async fn ntlm(&mut self, body: &str, scheme: &'static str, limit: usize) -> Result<Resp> {
        let neg = crate::ntlm::negotiate();
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        // Первый шаг — без тела: большое письмо не передаётся дважды.
        let first = self.http_send("", Some(format!("{scheme} {}", b64(&neg))), crate::netio::SMALL_LIMIT).await?;
        // Не 401 с вызовом NTLM — это не отказ в пароле, а ответ сервера, который надо показать.
        if first.status != StatusCode::UNAUTHORIZED {
            return Err(AppError::Other(format!("Сервер Exchange ответил HTTP {} на начало входа NTLM", first.status)));
        }
        let closes = first.headers.get(http::header::CONNECTION).and_then(|v| v.to_str().ok()).is_some_and(|v| v.eq_ignore_ascii_case("close"));
        let challenge = ntlm_token(&www_auth(&first), scheme).and_then(|t| crate::ntlm::parse_challenge(&t)).ok_or(AppError::AuthFailed)?;
        if closes {
            return Err(AppError::Other("Сервер Exchange закрывает соединение посреди входа NTLM — вход по NTLM через этот адрес невозможен (возможно, мешает прокси)".into()));
        }
        let cert = self.conn.as_ref().and_then(|p| p.cert.clone());
        let (user, domain) = crate::ntlm::split_login(&self.login);
        let mut cc = [0u8; 8];
        getrandom::getrandom(&mut cc).map_err(|e| AppError::Other(e.to_string()))?;
        let spn = format!("HTTP/{}", self.url.host_str().unwrap_or_default());
        let auth = crate::ntlm::authenticate(&challenge, &neg, &user, &domain, &self.password, "", cert.as_deref(), Some(&spn), cc, crate::ntlm::filetime_now());
        // Третий шаг — по тому же соединению (если сервер его закрыл, новое без вызова бесполезно).
        if self.conn.as_ref().is_none_or(|p| p.sender.is_closed()) {
            return Err(AppError::ConnectionLost);
        }
        let p = self.conn.as_mut().ok_or(AppError::ConnectionLost)?;
        let r = send_pinned(p, &self.url, body, Some(&format!("{scheme} {}", b64(&auth))), limit).await;
        if r.is_err() {
            self.conn = None;
        }
        r
    }

    /// Запрос EWS: тело операции без конверта; разобранный ответ.
    pub(super) async fn call(&mut self, op: &str, limit: usize) -> Result<String> {
        let text = self.post_raw(&envelope(self.version, op), limit).await?;
        if self.version == "Exchange2013_SP1" && text.contains("ErrorInvalidServerVersion") {
            self.version = "Exchange2010_SP2";
            return self.post_raw(&envelope(self.version, op), limit).await;
        }
        Ok(text)
    }
}
pub async fn connect(url: &str, login: &str, password: &str, key: &str) -> Result<EwsSession> {
    let url = normalize_url(url)?;
    let mut s = EwsSession {
        conn: None,
        url,
        login: login.to_string(),
        password: password.to_string(),
        key: key.to_string(),
        auth: Auth::Unknown,
        authed: false,
        version: "Exchange2013_SP1",
        folders: Vec::new(),
        root: None,
        map: account_map(key),
    };
    let text = s.call(r#"<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds><t:DistinguishedFolderId Id="msgfolderroot"/></m:FolderIds></m:GetFolder>"#, crate::netio::SMALL_LIMIT).await?;
    let d = doc(&text)?;
    let rs = responses(&d)?;
    if let Some(e) = first_error(&rs) {
        return Err(e);
    }
    s.root = descendants(d.root(), "FolderId").next().and_then(|f| f.attribute("Id")).map(str::to_string);
    if s.root.is_none() {
        return Err(AppError::Other("Сервер не похож на Exchange (нет ответа EWS)".into()));
    }
    Ok(s)
}
fn autodiscover_candidates(domain: &str) -> Vec<String> {
    #[allow(unused_mut)]
    let mut candidates = vec![format!("https://autodiscover.{domain}/autodiscover/autodiscover.xml"), format!("https://{domain}/autodiscover/autodiscover.xml")];
    // E2E (отладочная сборка): тестовый сервер вместо адресов в домене почты.
    #[cfg(debug_assertions)]
    if let Ok(u) = std::env::var("RRCKI_E2E_AUTODISCOVER_EWS") {
        candidates = vec![u];
    }
    candidates
}

pub(super) fn autodiscover_request(email: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006"><Request><EMailAddress>{}</EMailAddress><AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema></Request></Autodiscover>"#,
        xml_escape(email)
    )
}

pub(super) fn probe_session(url: Url, login: &str, password: &str) -> EwsSession {
    EwsSession {
        conn: None,
        url,
        login: login.to_string(),
        password: password.to_string(),
        key: String::new(),
        auth: Auth::Unknown,
        authed: false,
        version: "Exchange2013_SP1",
        folders: Vec::new(),
        root: None,
        map: Arc::new(Mutex::new(AccountMap::default())),
    }
}
// ---------------------------------------------------------------- автообнаружение

/// Адрес EWS по адресу почты (Autodiscover POX). Пробуются только адреса
/// https в домене почты; пароль уходит только туда.
pub async fn autodiscover(email: &str, login: &str, password: &str) -> Result<String> {
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_lowercase()).filter(|d| d.contains('.')).ok_or_else(|| AppError::Other("Укажите адрес почты полностью (имя@домен)".into()))?;
    let body = autodiscover_request(email);
    let candidates = autodiscover_candidates(&domain);
    let mut last = AppError::Other("Сервер Exchange не найден автоматически — укажите его адрес".into());
    for c in candidates {
        let Ok(url) = Url::parse(&c) else { continue };
        let mut s = probe_session(url, login, password);
        match tokio::time::timeout(Duration::from_secs(25), s.post_raw(&body, crate::netio::SMALL_LIMIT)).await {
            Ok(Ok(text)) => {
                if let Some(url) = ews_url_of(&text) {
                    // Пароль уйдёт по этому адресу — только в домене почты или на сервер,
                    // который администратор разрешил в политике (exchangeHosts).
                    let host = Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_lowercase)).unwrap_or_default();
                    #[allow(unused_mut)]
                    let mut allowed = crate::policy::load().exchange_hosts;
                    // E2E (отладочная сборка): тестовый сервер на localhost.
                    #[cfg(debug_assertions)]
                    if let Ok(h) = std::env::var("RRCKI_E2E_EXCHANGE_HOSTS") {
                        allowed.push(h);
                    }
                    if !trusted_ews_host(&host, &domain, &allowed) {
                        return Err(AppError::Other(format!(
                            "Autodiscover предлагает сервер {host} — он не в домене почты {domain}. Если это действительно ваш сервер Exchange, укажите его адрес вручную"
                        )));
                    }
                    return Ok(url);
                }
            }
            Ok(Err(AppError::AuthFailed)) => return Err(AppError::AuthFailed),
            Ok(Err(e)) => last = e,
            Err(_) => last = AppError::Timeout,
        }
    }
    if !matches!(last, AppError::AuthFailed) {
        last = AppError::Other("Сервер Exchange не найден автоматически — укажите его адрес (например, mail.организация.ru)".into());
    }
    Err(last)
}

// ---------------------------------------------------------------- диагностика входа

/// Строки CN из сертификата (сначала издатель, затем владелец) — без разбора ASN.1 целиком.
fn cert_names(der: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 5 < der.len() {
        if der[i..i + 5] == [0x06, 0x03, 0x55, 0x04, 0x03] {
            let len = der.get(i + 6).copied().unwrap_or(0) as usize;
            if let Some(v) = der.get(i + 7..i + 7 + len) {
                if let Ok(t) = std::str::from_utf8(v) {
                    out.push(t.to_string());
                }
            }
            i += 7;
        } else {
            i += 1;
        }
    }
    out
}

/// Сертификат узла без проверки — только для отчёта диагностики: соединение
/// закрывается сразу после рукопожатия TLS, данные по нему не передаются.
async fn peer_cert_unverified(host: &str, port: u16) -> Option<Vec<u8>> {
    let tcp = tokio::time::timeout(Duration::from_secs(10), tokio::net::TcpStream::connect((host, port))).await.ok()?.ok()?;
    let connector = native_tls::TlsConnector::builder().danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true).build().ok()?;
    let tls = tokio::time::timeout(Duration::from_secs(10), tokio_native_tls::TlsConnector::from(connector).connect(host, tcp)).await.ok()?.ok()?;
    let der = tls.get_ref().peer_certificate().ok()??.to_der().ok();
    drop(tls);
    der
}

fn ntlm_info(ch: &crate::ntlm::Challenge) -> String {
    let mut parts = vec![format!("флаги {:#010x}", ch.flags)];
    for (id, v) in crate::ntlm::av_list(&ch.target_info) {
        let text = || String::from_utf16_lossy(&v.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>());
        match id {
            1 => parts.push(format!("сервер {}", text())),
            2 => parts.push(format!("домен NetBIOS {}", text())),
            3 => parts.push(format!("сервер DNS {}", text())),
            4 => parts.push(format!("домен DNS {}", text())),
            5 => parts.push(format!("лес DNS {}", text())),
            7 => parts.push("время сервера есть (подпись MIC)".into()),
            _ => {}
        }
    }
    parts.join(", ")
}

fn header_lines(resp: &Resp) -> String {
    let keep = ["www-authenticate", "server", "x-feserver", "x-beserver", "x-owa-version", "x-aspnet-version", "x-powered-by", "connection", "location", "persistent-auth", "request-id", "x-diaginfo"];
    let mut out = format!("  HTTP {}\n", resp.status);
    for (k, v) in &resp.headers {
        if keep.contains(&k.as_str()) {
            out.push_str(&format!("  {k}: {}\n", v.to_str().unwrap_or("?")));
        }
    }
    out
}

/// Пошаговая проверка входа в Exchange: адрес, сеть, сертификат, способы входа,
/// ответ EWS. Пароль в отчёт не попадает. Делает до трёх попыток входа (их видит
/// контроллер домена — при пороге блокировки учётной записи это важно).
pub async fn diagnose(input: &str, email: &str, login: &str, password: &str) -> String {
    let mut r = String::new();
    let (user, domain) = crate::ntlm::split_login(login);
    r.push_str(&format!("Atom Workspace {} — диагностика Exchange\nПочта: {email}\nЛогин: {login} (пользователь «{user}», домен «{domain}»)\n", env!("CARGO_PKG_VERSION")));
    // 1. Адрес EWS
    let input = input.trim();
    let url = if input.is_empty() || input == "auto" {
        r.push_str("\n## Адрес EWS (Autodiscover)\n");
        match autodiscover(if email.contains('@') { email } else { login }, login, password).await {
            Ok(u) => {
                r.push_str(&format!("найден: {u}\n"));
                u
            }
            Err(e) => {
                r.push_str(&format!("не найден: {e}\n"));
                return r;
            }
        }
    } else {
        match normalize_url(input) {
            Ok(u) => u.to_string(),
            Err(e) => {
                r.push_str(&format!("\nАдрес: {e}\n"));
                return r;
            }
        }
    };
    let Ok(url) = Url::parse(&url) else { return r };
    let host = url.host_str().unwrap_or_default().to_string();
    r.push_str(&format!("\n## Адрес\n{url}\n"));
    // 2. DNS
    r.push_str("\n## DNS\n");
    match tokio::time::timeout(Duration::from_secs(10), tokio::net::lookup_host((host.as_str(), url.port_or_known_default().unwrap_or(443)))).await {
        Ok(Ok(addrs)) => {
            let list: Vec<String> = addrs.map(|a| a.ip().to_string()).collect();
            r.push_str(&format!("{host} → {}\n", list.join(", ")));
        }
        Ok(Err(e)) => {
            r.push_str(&format!("{host}: имя не найдено ({e})\n"));
            return r;
        }
        Err(_) => {
            r.push_str("нет ответа DNS за 10 с\n");
            return r;
        }
    }
    // 3. TLS и первый ответ (без входа)
    r.push_str("\n## TLS и запрос без входа\n");
    let anon = match open_pinned(&url).await {
        Ok(mut p) => {
            if let Some(cert) = &p.cert {
                use sha2::Digest as _;
                let names = cert_names(cert);
                let fp: String = sha2::Sha256::digest(cert).iter().map(|b| format!("{b:02X}")).collect();
                r.push_str(&format!("сертификат принят системой; CN: {}; SHA-256 {}…\n", names.join(" ← "), &fp[..16]));
            }
            send_pinned(&mut p, &url, "", None, crate::netio::SMALL_LIMIT).await
        }
        Err(e) => Err(e),
    };
    let offered = match anon {
        Ok(resp) => {
            r.push_str(&header_lines(&resp));
            let offered = www_auth(&resp);
            let st = resp.status;
            if st.is_redirection() {
                r.push_str("сервер перенаправляет — нужен точный адрес EWS (обычно https://сервер/EWS/Exchange.asmx)\n");
                return r;
            }
            if st != StatusCode::UNAUTHORIZED {
                r.push_str("ожидался ответ 401 с перечнем способов входа — возможно, это не EWS или перед ним стоит прокси с собственной страницей входа\n");
            }
            offered
        }
        Err(e) => {
            r.push_str(&format!("ошибка: {e}\n"));
            // Сертификат не принят: посмотреть, какой он. Только рукопожатие TLS —
            // ни HTTP-запроса, ни логина с паролем этому узлу не отправляется.
            if matches!(e, AppError::Tls(_)) {
                if let Some(cert) = peer_cert_unverified(&host, url.port_or_known_default().unwrap_or(443)).await {
                    r.push_str(&format!("сертификат сервера (не принят системой): CN {}\n", cert_names(&cert).join(" ← ")));
                    r.push_str("— если это сертификат внутреннего центра сертификации, установите корневой сертификат организации в доверенные на этом компьютере\n");
                }
            }
            return r;
        }
    };
    let has = |sch: &str| offered.iter().any(|h| h.split_whitespace().next().is_some_and(|x| x.eq_ignore_ascii_case(sch)));
    let schemes: Vec<&str> = ["NTLM", "Negotiate", "Basic"].into_iter().filter(|x| has(x)).collect();
    r.push_str(&format!("способы входа: {}\n", if schemes.is_empty() { "не предложены".to_string() } else { schemes.join(", ") }));
    // 4. Вход
    let probe = envelope("Exchange2013_SP1", r#"<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds><t:DistinguishedFolderId Id="inbox"/></m:FolderIds></m:GetFolder>"#);
    let mut tries: Vec<(&str, &'static str, bool)> = Vec::new();
    if has("NTLM") || has("Negotiate") {
        let sch = if has("NTLM") { "NTLM" } else { "Negotiate" };
        tries.push(("NTLMv2 с привязкой к TLS (Extended Protection)", sch, true));
        tries.push(("NTLMv2 без привязки к TLS", sch, false));
    }
    if has("Basic") {
        tries.push(("Basic", "Basic", false));
    }
    for (title, sch, cbt) in tries {
        r.push_str(&format!("\n## Вход: {title}\n"));
        // Каждый способ — на своём новом соединении (вход NTLM привязан к соединению).
        let mut c = match open_pinned(&url).await {
            Ok(c) => c,
            Err(e) => {
                r.push_str(&format!("ошибка: {e}\n"));
                continue;
            }
        };
        let resp = if sch == "Basic" {
            let token = base64::engine::general_purpose::STANDARD.encode(format!("{login}:{password}"));
            send_pinned(&mut c, &url, &probe, Some(&format!("Basic {token}")), crate::netio::SMALL_LIMIT).await
        } else {
            let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
            let neg = crate::ntlm::negotiate();
            let first = match send_pinned(&mut c, &url, "", Some(&format!("{sch} {}", b64(&neg))), crate::netio::SMALL_LIMIT).await {
                Ok(f) => f,
                Err(e) => {
                    r.push_str(&format!("ошибка: {e}\n"));
                    continue;
                }
            };
            let hs = www_auth(&first);
            let closes = first.headers.get("connection").and_then(|v| v.to_str().ok()).is_some_and(|v| v.eq_ignore_ascii_case("close"));
            r.push_str(&format!("шаг 1 (NEGOTIATE): HTTP {}{}\n", first.status, if closes { ", сервер закрывает соединение — NTLM так не пройдёт" } else { "" }));
            let Some(ch) = ntlm_token(&hs, sch).and_then(|t| crate::ntlm::parse_challenge(&t)) else {
                r.push_str("сервер не прислал вызов NTLM (CHALLENGE)\n");
                continue;
            };
            r.push_str(&format!("шаг 2 (CHALLENGE): {}\n", ntlm_info(&ch)));
            let mut cc = [0u8; 8];
            let _ = getrandom::getrandom(&mut cc);
            let spn = format!("HTTP/{host}");
            let cert = c.cert.clone();
            let auth = crate::ntlm::authenticate(&ch, &neg, &user, &domain, password, "", if cbt { cert.as_deref() } else { None }, Some(&spn), cc, crate::ntlm::filetime_now());
            send_pinned(&mut c, &url, &probe, Some(&format!("{sch} {}", b64(&auth))), crate::netio::SMALL_LIMIT).await
        };
        match resp {
            Err(e) => r.push_str(&format!("ошибка: {e}\n")),
            Ok(resp) => {
                let st = resp.status;
                r.push_str(&header_lines(&resp));
                let text = resp.text();
                if st == StatusCode::UNAUTHORIZED {
                    r.push_str("вход отклонён\n");
                    continue;
                }
                if let Ok(d) = roxmltree::Document::parse(&text) {
                    if let Some(v) = d.descendants().find(|n| n.tag_name().name() == "ServerVersionInfo") {
                        r.push_str(&format!("версия Exchange: {}.{}.{}.{} ({})\n", v.attribute("MajorVersion").unwrap_or("?"), v.attribute("MinorVersion").unwrap_or("?"), v.attribute("MajorBuildNumber").unwrap_or("?"), v.attribute("MinorBuildNumber").unwrap_or("?"), v.attribute("Version").unwrap_or("")));
                    }
                    if let Some(f) = d.descendants().find(|n| n.tag_name().name() == "faultstring") {
                        r.push_str(&format!("ошибка SOAP: {}\n", f.text().unwrap_or("")));
                    }
                    for m in d.descendants().filter(|n| n.tag_name().name() == "ResponseCode") {
                        r.push_str(&format!("ответ EWS: {}\n", m.text().unwrap_or("")));
                    }
                    for m in d.descendants().filter(|n| n.tag_name().name() == "MessageText") {
                        r.push_str(&format!("текст: {}\n", m.text().unwrap_or("")));
                    }
                } else {
                    let head: String = text.chars().take(300).collect();
                    r.push_str(&format!("ответ не XML: {head}\n"));
                }
                if st == StatusCode::OK {
                    r.push_str("ВХОД УСПЕШЕН — этот способ работает\n");
                    break;
                }
            }
        }
    }
    r
}

/// Сервер EWS из Autodiscover: в домене почты (`mail.corp.ru` для `@corp.ru`) или в
/// списке администратора — точное имя или домен (`corp.ru` разрешает `exchange.corp.ru`).
pub fn trusted_ews_host(host: &str, mail_domain: &str, allowed: &[String]) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    let within = |d: &str| {
        let d = d.trim().trim_start_matches("*.").trim_start_matches('.').trim_end_matches('.').to_ascii_lowercase();
        !d.is_empty() && (host == d || host.ends_with(&format!(".{d}")))
    };
    within(mail_domain) || allowed.iter().any(|d| within(d))
}

pub(super) fn ews_url_of(text: &str) -> Option<String> {
    let d = roxmltree::Document::parse(text).ok()?;
    let protocols: Vec<_> = d.descendants().filter(|n| n.tag_name().name() == "Protocol").collect();
    let pick = |ty: &str| protocols.iter().find(|p| text_of(**p, "Type").as_deref() == Some(ty)).and_then(|p| text_of(*p, "EwsUrl")).filter(|u| u.to_ascii_lowercase().starts_with("https://"));
    pick("EXCH").or_else(|| pick("EXPR"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_responses_are_unpacked_with_a_size_cap() {
        use std::io::Write as _;
        let xml = b"<s:Envelope>".repeat(500);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&xml).unwrap();
        let gz = gz.finish().unwrap();
        assert!(gz.len() < xml.len() / 5);
        assert_eq!(decode_body(Some("gzip"), gz.clone(), 1 << 20).unwrap(), xml);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&xml).unwrap();
        assert_eq!(decode_body(Some("deflate"), z.finish().unwrap(), 1 << 20).unwrap(), xml);
        let mut raw = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        raw.write_all(&xml).unwrap();
        assert_eq!(decode_body(Some("deflate"), raw.finish().unwrap(), 1 << 20).unwrap(), xml);
        assert_eq!(decode_body(None, xml.clone(), 1 << 20).unwrap(), xml);
        // «Бомба»: после распаковки больше лимита — отказ, а не исчерпание памяти.
        assert!(decode_body(Some("gzip"), gz, 1000).is_err());
        assert!(decode_body(Some("br"), xml, 1 << 20).is_err());
    }

    #[test]
    fn autodiscover_server_must_be_in_mail_domain_or_allowed() {
        assert!(trusted_ews_host("mail.corp.ru", "corp.ru", &[]));
        assert!(trusted_ews_host("CORP.RU.", "corp.ru", &[]));
        assert!(!trusted_ews_host("mail.corp.ru.evil.com", "corp.ru", &[]));
        assert!(!trusted_ews_host("evilcorp.ru", "corp.ru", &[]));
        assert!(!trusted_ews_host("exchange.corp.ru", "mail.ru", &[]));
        assert!(trusted_ews_host("exchange.corp.ru", "mail.ru", &["corp.ru".into()]));
        assert!(trusted_ews_host("exchange.corp.ru", "mail.ru", &["*.corp.ru".into()]));
        assert!(!trusted_ews_host("corp.ru.evil.com", "mail.ru", &["corp.ru".into()]));
        assert!(!trusted_ews_host("", "corp.ru", &["".into()]));
    }

    #[test]
    fn urls() {
        assert_eq!(normalize_url("mail.corp.ru").unwrap().as_str(), "https://mail.corp.ru/EWS/Exchange.asmx");
        assert_eq!(normalize_url("https://mail.corp.ru/ews/exchange.asmx").unwrap().as_str(), "https://mail.corp.ru/ews/exchange.asmx");
        assert!(normalize_url("http://mail.corp.ru").is_err());
        let ad = r#"<?xml version="1.0"?><Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/responseschema/2006"><Response xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a"><Account><Protocol><Type>EXPR</Type><EwsUrl>https://ext.corp.ru/EWS/Exchange.asmx</EwsUrl></Protocol><Protocol><Type>EXCH</Type><EwsUrl>https://mail.corp.ru/EWS/Exchange.asmx</EwsUrl></Protocol></Account></Response></Autodiscover>"#;
        assert_eq!(ews_url_of(ad).as_deref(), Some("https://mail.corp.ru/EWS/Exchange.asmx"));
        assert_eq!(ews_url_of(&ad.replace("https://mail", "http://mail")).as_deref(), Some("https://ext.corp.ru/EWS/Exchange.asmx"));
    }
}
