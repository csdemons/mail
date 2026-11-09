//! Exchange Web Services (EWS): почта своего сервера Exchange (2013 и новее).
//!
//! Для остальной программы ящик Exchange выглядит как ящик IMAP: те же папки
//! (идентификатор папки — путь в modified UTF-7, «Входящие» — `INBOX`), те же
//! номера писем (UID) и флаги. Модуль `imap` передаёт сюда вызовы, если
//! соединение — с Exchange.
//!
//! Номер письма (UID) — время получения письма в секундах с 2000 года (при
//! совпадении — следующее свободное число): номера растут вместе с приходом
//! почты, как UID в IMAP, и одинаковы при каждом запуске. Соответствие номера и
//! идентификатора элемента Exchange (ItemId) хранится на компьютере.
//!
//! Вход: NTLMv2 (с привязкой к TLS-каналу) или Basic, только по HTTPS; пароль
//! отправляется только на адрес EWS из настроек.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use chrono::{DateTime, TimeZone, Utc};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{AppError, Result};
use crate::imap::{FlagKind, FlagState, Folder, FolderRole, MessagePage, MessageSummary};

const NS_T: &str = "http://schemas.microsoft.com/exchange/services/2006/types";
const NS_M: &str = "http://schemas.microsoft.com/exchange/services/2006/messages";
/// Большие ответы (исходники писем в base64).
const MIME_LIMIT: usize = 96 * 1024 * 1024;
/// Сколько элементов запрашивать за раз в GetItem.
const BATCH: usize = 50;
/// Начало отсчёта номеров писем.
const EPOCH_2000: i64 = 946_684_800;

// ---------------------------------------------------------------- соответствие UID ↔ ItemId

#[derive(Serialize, Deserialize, Default)]
struct FolderMap {
    /// UID → ItemId (Id без ChangeKey).
    items: HashMap<u32, String>,
    #[serde(skip)]
    rev: HashMap<String, u32>,
}

impl FolderMap {
    fn rebuild(&mut self) {
        self.rev = self.items.iter().map(|(u, id)| (id.clone(), *u)).collect();
    }

    fn uid_for(&mut self, id: &str, received: Option<DateTime<Utc>>) -> u32 {
        if let Some(u) = self.rev.get(id) {
            return *u;
        }
        let secs = received.map(|d| d.timestamp() - EPOCH_2000).unwrap_or(1).clamp(1, i64::from(u32::MAX - 1_000_000)) as u32;
        let mut uid = secs;
        while self.items.contains_key(&uid) {
            uid += 1;
        }
        self.items.insert(uid, id.to_string());
        self.rev.insert(id.to_string(), uid);
        uid
    }

    fn forget(&mut self, uid: u32) {
        if let Some(id) = self.items.remove(&uid) {
            self.rev.remove(&id);
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct AccountMap {
    /// По FolderId Exchange (не меняется при переименовании папки).
    folders: HashMap<String, FolderMap>,
    #[serde(skip)]
    dirty: bool,
    #[serde(skip)]
    saved: Option<std::time::Instant>,
}

type Shared = Arc<Mutex<AccountMap>>;

fn registry() -> &'static Mutex<HashMap<String, Shared>> {
    static R: OnceLock<Mutex<HashMap<String, Shared>>> = OnceLock::new();
    R.get_or_init(Mutex::default)
}

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Каталог данных программы (задаётся при запуске): там хранится соответствие номеров писем.
pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

fn map_file(key: &str) -> Option<PathBuf> {
    use sha2::Digest as _;
    let h: String = sha2::Sha256::digest(key.as_bytes()).iter().take(10).map(|b| format!("{b:02x}")).collect();
    DATA_DIR.get().map(|d| d.join("exchange").join(format!("{h}.json")))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn account_map(key: &str) -> Shared {
    let mut reg = lock(registry());
    reg.entry(key.to_string())
        .or_insert_with(|| {
            let mut m: AccountMap = map_file(key).and_then(|p| std::fs::read(p).ok()).and_then(|r| serde_json::from_slice(&r).ok()).unwrap_or_default();
            for f in m.folders.values_mut() {
                f.rebuild();
            }
            Arc::new(Mutex::new(m))
        })
        .clone()
}

fn save_map(key: &str, map: &Shared, force: bool) {
    let mut m = lock(map);
    if !m.dirty || (!force && m.saved.is_some_and(|t| t.elapsed() < Duration::from_secs(10))) {
        return;
    }
    let Some(path) = map_file(key) else { return };
    if let Ok(raw) = serde_json::to_vec(&*m) {
        if crate::settings::write_private(&path, &raw).is_ok() {
            m.dirty = false;
            m.saved = Some(std::time::Instant::now());
        }
    }
}

/// Учётную запись удалили — соответствие номеров писем больше не нужно.
pub fn forget(key: &str) {
    lock(registry()).remove(key);
    lock(mailbox_registry()).remove(key);
    if let Some(p) = map_file(key) {
        let _ = std::fs::remove_file(p);
    }
}

// ---------------------------------------------------------------- HTTP и вход

#[derive(Clone, Copy, PartialEq, Debug)]
enum Auth {
    Unknown,
    Basic,
    /// Схема заголовка: `NTLM` или `Negotiate` (с токеном NTLM).
    Ntlm(&'static str),
}

/// Папка Exchange: путь (как у IMAP), FolderId, роль.
#[derive(Clone, Debug)]
struct EwsFolder {
    path: String,
    id: String,
    role: FolderRole,
}

pub struct EwsSession {
    /// Своё соединение (не пул): вход NTLM действует на сокете, где он прошёл.
    conn: Option<Pinned>,
    url: Url,
    login: String,
    password: String,
    key: String,
    auth: Auth,
    /// Соединение уже прошло NTLM (IIS помнит вход на соединении).
    authed: bool,
    version: &'static str,
    folders: Vec<EwsFolder>,
    root: Option<String>,
    map: Shared,
}

// ---------------------------------------------------------------- соединение

/// Одно соединение HTTP/1.1 поверх TLS. Вход NTLM (как и в Outlook) привязан к TCP-
/// соединению: IIS помнит его на сокете. Пул соединений общего HTTP-клиента мог
/// отправить третий шаг входа по другому соединению — тогда вход «не проходил»
/// при верном пароле. Здесь все запросы сеанса идут по одному соединению.
struct Pinned {
    sender: hyper::client::conn::http1::SendRequest<http_body_util::Full<bytes::Bytes>>,
    /// Сертификат сервера (DER) — для привязки входа к TLS-каналу.
    cert: Option<Vec<u8>>,
    used: std::time::Instant,
}

/// Ответ сервера целиком (с ограничением размера).
struct Resp {
    status: StatusCode,
    headers: http::HeaderMap,
    body: Vec<u8>,
}

impl Resp {
    fn text(self) -> String {
        match String::from_utf8(self.body) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        }
    }
}

/// Простаивающее соединение IIS закрывает (по умолчанию через 2 минуты) — раньше этого
/// открываем новое сами, а не узнаём об обрыве посреди запроса.
const IDLE_REOPEN: Duration = Duration::from_secs(50);

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
    let connector = native_tls::TlsConnector::builder().build().map_err(|e| tls_error(&e, &host))?;
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

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn envelope(version: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:t="{NS_T}" xmlns:m="{NS_M}"><soap:Header><t:RequestServerVersion Version="{version}"/></soap:Header><soap:Body>{body}</soap:Body></soap:Envelope>"#
    )
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

/// Полный текст ошибки (с причинами): главное часто спрятано в `source()`.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        let t = c.to_string();
        if !out.contains(&t) {
            out.push_str(": ");
            out.push_str(&t);
        }
        cur = c.source();
    }
    out
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
    async fn post_raw(&mut self, body: &str, limit: usize) -> Result<String> {
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
    async fn call(&mut self, op: &str, limit: usize) -> Result<String> {
        let text = self.post_raw(&envelope(self.version, op), limit).await?;
        if self.version == "Exchange2013_SP1" && text.contains("ErrorInvalidServerVersion") {
            self.version = "Exchange2010_SP2";
            return self.post_raw(&envelope(self.version, op), limit).await;
        }
        Ok(text)
    }
}

// ---------------------------------------------------------------- разбор XML

fn doc(text: &str) -> Result<roxmltree::Document<'_>> {
    roxmltree::Document::parse(text).map_err(|e| AppError::Other(format!("Ответ Exchange не разобран: {e}")))
}

fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn text_of(n: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    child(n, name).and_then(|c| c.text()).map(|s| s.trim().to_string())
}

fn descendants<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = roxmltree::Node<'a, 'i>> + 'a {
    n.descendants().filter(move |c| c.is_element() && c.tag_name().name() == name)
}

/// Ответы операции (`*ResponseMessage`) и ошибка SOAP целиком.
fn responses<'a, 'i>(d: &'a roxmltree::Document<'i>) -> Result<Vec<roxmltree::Node<'a, 'i>>> {
    if let Some(f) = d.descendants().find(|n| n.tag_name().name() == "Fault") {
        let msg = descendants(f, "faultstring").next().and_then(|n| n.text()).unwrap_or("ошибка SOAP");
        return Err(AppError::Other(format!("Exchange: {msg}")));
    }
    Ok(d.descendants().filter(|n| n.is_element() && n.tag_name().name().ends_with("ResponseMessage") && n.attribute("ResponseClass").is_some()).collect())
}

fn response_error(r: roxmltree::Node<'_, '_>) -> Option<AppError> {
    if r.attribute("ResponseClass") == Some("Success") || r.attribute("ResponseClass") == Some("Warning") {
        return None;
    }
    let code = text_of(r, "ResponseCode").unwrap_or_default();
    let text = text_of(r, "MessageText").unwrap_or_else(|| code.clone());
    Some(match code.as_str() {
        "ErrorItemNotFound" | "ErrorFolderNotFound" => AppError::NotFound,
        _ => AppError::Other(format!("Exchange: {text}")),
    })
}

/// Первая ошибка среди ответов (или `None`).
fn first_error(rs: &[roxmltree::Node<'_, '_>]) -> Option<AppError> {
    rs.iter().find_map(|r| response_error(*r))
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim()).ok().map(|d| d.with_timezone(&Utc))
}

const ITEM_TAGS: [&str; 7] = ["Message", "MeetingRequest", "MeetingResponse", "MeetingCancellation", "PostItem", "Item", "MeetingMessage"];

fn items_of<'a, 'i>(n: roxmltree::Node<'a, 'i>) -> Vec<roxmltree::Node<'a, 'i>> {
    n.descendants().filter(|c| c.is_element() && ITEM_TAGS.contains(&c.tag_name().name()) && child(*c, "ItemId").is_some()).collect()
}

fn item_id(n: roxmltree::Node<'_, '_>) -> Option<String> {
    child(n, "ItemId").and_then(|i| i.attribute("Id")).map(str::to_string)
}

fn mailbox(n: roxmltree::Node<'_, '_>) -> (String, String) {
    let name = text_of(n, "Name").unwrap_or_default();
    let mut addr = text_of(n, "EmailAddress").unwrap_or_default();
    // Внутренний адрес Exchange (/O=…) — не почтовый: показываем имя.
    if addr.starts_with('/') {
        addr.clear();
    }
    (name, addr)
}

fn recipients(n: roxmltree::Node<'_, '_>, name: &str) -> Vec<String> {
    child(n, name)
        .map(|r| {
            r.children()
                .filter(|m| m.is_element() && m.tag_name().name() == "Mailbox")
                .map(|m| {
                    let (name, addr) = mailbox(m);
                    match (name.is_empty(), addr.is_empty()) {
                        (false, false) if name != addr => format!("{name} <{addr}>"),
                        (_, false) => addr,
                        _ => name,
                    }
                })
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn wrap_id(id: &str) -> String {
    format!("<{}>", id.trim().trim_matches(['<', '>']))
}

/// Свойства письма для списка (GetItem).
const SUMMARY_PROPS: &str = r#"<t:FieldURI FieldURI="item:Subject"/><t:FieldURI FieldURI="item:DateTimeReceived"/><t:FieldURI FieldURI="message:From"/><t:FieldURI FieldURI="message:ToRecipients"/><t:FieldURI FieldURI="message:IsRead"/><t:FieldURI FieldURI="item:Importance"/><t:FieldURI FieldURI="item:Categories"/><t:FieldURI FieldURI="item:Flag"/><t:FieldURI FieldURI="message:InternetMessageId"/><t:FieldURI FieldURI="item:InReplyTo"/><t:FieldURI FieldURI="message:References"/><t:FieldURI FieldURI="item:HasAttachments"/><t:FieldURI FieldURI="item:Attachments"/><t:FieldURI FieldURI="item:Size"/><t:ExtendedFieldURI PropertyTag="0x1081" PropertyType="Integer"/>"#;

/// Свойства для флагов (полная синхронизация папки).
const FLAG_PROPS: &str = r#"<t:FieldURI FieldURI="item:DateTimeReceived"/><t:FieldURI FieldURI="message:IsRead"/><t:FieldURI FieldURI="item:Importance"/><t:FieldURI FieldURI="item:Categories"/><t:FieldURI FieldURI="item:Flag"/><t:ExtendedFieldURI PropertyTag="0x1081" PropertyType="Integer"/>"#;

fn flags_of(n: roxmltree::Node<'_, '_>) -> FlagState {
    let categories: Vec<String> = child(n, "Categories").map(|c| c.children().filter_map(|s| s.text()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    let is_important = |k: &str| k.eq_ignore_ascii_case(crate::imap::IMPORTANT) || k.eq_ignore_ascii_case("$Important");
    let verb = descendants(n, "ExtendedProperty")
        .find(|e| child(*e, "ExtendedFieldURI").and_then(|f| f.attribute("PropertyTag")).is_some_and(|t| t.eq_ignore_ascii_case("0x1081")))
        .and_then(|e| text_of(e, "Value"))
        .and_then(|v| v.parse::<i32>().ok());
    FlagState {
        unread: text_of(n, "IsRead").is_some_and(|v| v == "false"),
        flagged: child(n, "Flag").and_then(|f| text_of(f, "FlagStatus")).is_some_and(|s| s == "Flagged"),
        important: text_of(n, "Importance").is_some_and(|v| v == "High") || categories.iter().any(|k| is_important(k)),
        answered: matches!(verb, Some(102) | Some(103)),
        keywords: categories.into_iter().filter(|k| !is_important(k)).collect(),
    }
}

impl EwsSession {
    fn summary(&self, folder: &str, uid: u32, n: roxmltree::Node<'_, '_>) -> MessageSummary {
        let (from_name, from_address) = child(n, "From").and_then(|f| child(f, "Mailbox")).map(mailbox).unwrap_or_default();
        let attachments: Vec<String> = child(n, "Attachments")
            .map(|a| {
                a.children()
                    .filter(|x| x.is_element() && x.tag_name().name() == "FileAttachment" && text_of(*x, "IsInline").as_deref() != Some("true"))
                    .filter_map(|x| text_of(x, "Name"))
                    .collect()
            })
            .unwrap_or_default();
        let mut thread: Vec<String> = text_of(n, "References").map(|r| r.split_whitespace().map(wrap_id).collect()).unwrap_or_default();
        if let Some(irt) = text_of(n, "InReplyTo") {
            thread.push(wrap_id(&irt));
        }
        let mut seen = std::collections::HashSet::new();
        thread.retain(|id| seen.insert(id.clone()));
        thread.truncate(20);
        let mut s = MessageSummary {
            folder: folder.to_string(),
            uid,
            from_name,
            from_address,
            to: recipients(n, "ToRecipients"),
            subject: text_of(n, "Subject").unwrap_or_default(),
            date: text_of(n, "DateTimeReceived").and_then(|d| parse_time(&d)).map(|d| d.to_rfc3339()),
            unread: false,
            flagged: false,
            important: false,
            answered: false,
            keywords: Vec::new(),
            message_id: text_of(n, "InternetMessageId").map(|i| wrap_id(&i)),
            thread,
            has_attachments: text_of(n, "HasAttachments").is_some_and(|v| v == "true") || !attachments.is_empty(),
            attachments,
            size: text_of(n, "Size").and_then(|v| v.parse().ok()).unwrap_or(0),
            snippet: None,
        };
        s.set_flags(flags_of(n));
        s
    }
}

// ---------------------------------------------------------------- подключение и папки

/// Подключиться к EWS и проверить вход (папка верхнего уровня ящика).
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

const DISTINGUISHED: [(&str, FolderRole); 5] = [
    ("inbox", FolderRole::Inbox),
    ("drafts", FolderRole::Drafts),
    ("sentitems", FolderRole::Sent),
    ("deleteditems", FolderRole::Trash),
    ("junkemail", FolderRole::Junk),
];

/// Имя папки для пути: «/» внутри имени заменяется, чтобы не путать с уровнями.
fn segment(name: &str) -> String {
    name.replace('/', "\u{2215}")
}

// ---------------------------------------------------------------- подключённые ящики

/// Корень подключённых ящиков в дереве папок — как `shared/<ящик>/…` у IMAP.
const SHARED_ROOT: &str = "shared";

/// Сколько ящиков подключаем самое большее (и найденных Autodiscover, и вручную).
pub const MAX_MAILBOXES: usize = 30;

/// Повторить Autodiscover, если в прошлый раз сервер не ответил.
const DISCOVER_RETRY: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
struct Mailboxes {
    /// Свой адрес почты: для Autodiscover и чтобы не подключить свой ящик второй раз.
    email: String,
    /// Добавленные пользователем.
    manual: Vec<String>,
    /// Найденные Autodiscover (ящики с полным доступом — Outlook подключает их сам):
    /// (адрес, имя). `None` — ещё не спрашивали.
    discovered: Option<Vec<(String, String)>>,
    discovered_at: Option<std::time::Instant>,
    /// Ответ Autodiscover был (а не ошибка сети) — тогда не повторяем.
    discovered_ok: bool,
}

fn mailbox_registry() -> &'static Mutex<HashMap<String, Mailboxes>> {
    static R: OnceLock<Mutex<HashMap<String, Mailboxes>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Ящик учётной записи и подключённые вручную ящики (при входе и после изменения в настройках).
pub fn set_mailboxes(key: &str, email: &str, manual: &[String]) {
    let mut reg = lock(mailbox_registry());
    let m = reg.entry(key.to_string()).or_default();
    if !m.email.eq_ignore_ascii_case(email) {
        m.discovered = None;
        m.discovered_at = None;
        m.discovered_ok = false;
    }
    m.email = email.trim().to_lowercase();
    m.manual = manual.iter().map(|a| a.trim().to_lowercase()).filter(|a| valid_mailbox(a)).collect();
}

/// Найденные Autodiscover ящики — для настроек.
pub fn discovered_mailboxes(key: &str) -> Vec<(String, String)> {
    lock(mailbox_registry()).get(key).and_then(|m| m.discovered.clone()).unwrap_or_default()
}

/// Адрес ящика: имя@домен, без пробелов и угловых скобок.
pub fn valid_mailbox(a: &str) -> bool {
    let a = a.trim();
    a.len() <= 254
        && a.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.'))
        && !a.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"' | ',' | ';' | '/' | '\\'))
}

/// Ящики из ответа Autodiscover (`AlternativeMailbox`): делегированные и общие
/// (Delegate, TeamMailbox); архив (Archive) — это часть своего ящика, не подключаем.
fn alternative_mailboxes(text: &str, own: &str) -> Vec<(String, String)> {
    let Ok(d) = roxmltree::Document::parse(text) else { return Vec::new() };
    let mut out: Vec<(String, String)> = Vec::new();
    for n in d.descendants().filter(|n| n.is_element() && n.tag_name().name() == "AlternativeMailbox") {
        let ty = text_of(n, "Type").unwrap_or_default();
        if !matches!(ty.as_str(), "Delegate" | "TeamMailbox") {
            continue;
        }
        let Some(smtp) = text_of(n, "SmtpAddress").or_else(|| text_of(n, "OwnerSmtpAddress")).map(|a| a.trim().to_lowercase()) else { continue };
        if !valid_mailbox(&smtp) || smtp.eq_ignore_ascii_case(own) || out.iter().any(|(a, _)| *a == smtp) {
            continue;
        }
        let name = text_of(n, "DisplayName").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| smtp.clone());
        out.push((smtp, name));
    }
    out.truncate(MAX_MAILBOXES);
    out
}

/// Адреса Autodiscover для домена почты (только https в самом домене).
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

fn autodiscover_request(email: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006"><Request><EMailAddress>{}</EMailAddress><AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema></Request></Autodiscover>"#,
        xml_escape(email)
    )
}

fn probe_session(url: Url, login: &str, password: &str) -> EwsSession {
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

/// Ящики с полным доступом по Autodiscover; `None` — сервер Autodiscover не ответил.
async fn discover_alternative(email: &str, login: &str, password: &str) -> Option<Vec<(String, String)>> {
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_lowercase()).filter(|d| d.contains('.'))?;
    let body = autodiscover_request(email);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    for c in autodiscover_candidates(&domain) {
        let Ok(url) = Url::parse(&c) else { continue };
        let mut s = probe_session(url, login, password);
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        if let Ok(Ok(text)) = tokio::time::timeout(left, s.post_raw(&body, crate::netio::SMALL_LIMIT)).await {
            if ews_url_of(&text).is_some() || text.contains("AlternativeMailbox") {
                return Some(alternative_mailboxes(&text, email));
            }
        }
    }
    None
}

/// Строка FindFolder: (id, родитель, имя, класс, всего, непрочитанных, вложенных).
type RawFolder = (String, String, String, String, u32, u32, u32);

/// Ссылка на служебную папку — своего ящика или подключённого.
fn distinguished(id: &str, mailbox: Option<&str>) -> String {
    match mailbox {
        Some(m) => format!(r#"<t:DistinguishedFolderId Id="{id}"><t:Mailbox><t:EmailAddress>{}</t:EmailAddress></t:Mailbox></t:DistinguishedFolderId>"#, xml_escape(m)),
        None => format!(r#"<t:DistinguishedFolderId Id="{id}"/>"#),
    }
}

const FOLDER_PROPS: &str = r#"<t:FieldURI FieldURI="folder:DisplayName"/><t:FieldURI FieldURI="folder:ParentFolderId"/><t:FieldURI FieldURI="folder:FolderClass"/><t:FieldURI FieldURI="folder:TotalCount"/><t:FieldURI FieldURI="folder:UnreadCount"/><t:FieldURI FieldURI="folder:ChildFolderCount"/><t:ExtendedFieldURI PropertyTag="0x10F4" PropertyType="Boolean"/>"#;

/// Почтовая папка из ответа (скрытые и не почтовые — `None`).
fn raw_folder(f: roxmltree::Node<'_, '_>) -> Option<RawFolder> {
    let hidden = descendants(f, "ExtendedProperty").any(|e| text_of(e, "Value").as_deref() == Some("true"));
    let class = text_of(f, "FolderClass").unwrap_or_default();
    if hidden || !(class.is_empty() || class == "IPF.Note" || class.starts_with("IPF.Note.")) {
        return None;
    }
    let id = child(f, "FolderId").and_then(|x| x.attribute("Id")).unwrap_or_default().to_string();
    let parent = child(f, "ParentFolderId").and_then(|x| x.attribute("Id")).unwrap_or_default().to_string();
    let num = |name: &str| text_of(f, name).and_then(|v| v.parse().ok()).unwrap_or(0);
    Some((id, parent, text_of(f, "DisplayName").unwrap_or_default(), class, num("TotalCount"), num("UnreadCount"), num("ChildFolderCount")))
}

/// Пути папок от корня ящика (`prefix/…`); «Входящие» — INBOX.
fn build_folders(raw: &[RawFolder], roles: &HashMap<String, FolderRole>, root: &str, prefix: Option<&str>) -> Vec<(Folder, EwsFolder)> {
    let by_id: HashMap<&str, usize> = raw.iter().enumerate().map(|(i, r)| (r.0.as_str(), i)).collect();
    let path_of = |i: usize| -> Option<String> {
        let mut parts = Vec::new();
        let mut cur = i;
        for _ in 0..32 {
            let r = &raw[cur];
            parts.push(if roles.get(&r.0) == Some(&FolderRole::Inbox) { "INBOX".to_string() } else { segment(&r.2) });
            if r.1 == root {
                if let Some(p) = prefix {
                    parts.push(p.to_string());
                }
                parts.reverse();
                return Some(parts.join("/"));
            }
            cur = *by_id.get(r.1.as_str())?;
        }
        None
    };
    let base = prefix.map_or(0, |p| p.matches('/').count() + 1);
    let mut out = Vec::new();
    for (i, r) in raw.iter().enumerate() {
        let Some(path) = path_of(i) else { continue };
        let role = roles.get(&r.0).copied().unwrap_or(FolderRole::Other);
        let id = utf7_imap::encode_utf7_imap(path.clone());
        let depth = path.matches('/').count();
        let parent = path.rsplit_once('/').map(|(p, _)| utf7_imap::encode_utf7_imap(p.to_string()));
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        out.push((
            Folder {
                id: id.clone(),
                name: if role == FolderRole::Inbox { "INBOX".into() } else { name },
                path: path.clone(),
                parent,
                depth,
                role,
                system: role != FolderRole::Other && depth == base,
                selectable: true,
                has_children: r.6 > 0,
                total: r.4,
                unseen: r.5,
            },
            EwsFolder { path: id, id: r.0.clone(), role },
        ));
    }
    out
}

fn node_folder(path: &str, name: &str, depth: usize) -> Folder {
    Folder {
        id: utf7_imap::encode_utf7_imap(path.to_string()),
        name: name.to_string(),
        path: path.to_string(),
        parent: path.rsplit_once('/').map(|(p, _)| utf7_imap::encode_utf7_imap(p.to_string())),
        depth,
        role: FolderRole::Other,
        system: false,
        selectable: false,
        has_children: true,
        total: 0,
        unseen: 0,
    }
}

impl EwsSession {
    /// Все почтовые папки под корнем (FindFolder Deep, постранично).
    async fn find_folders(&mut self, root: &str) -> Result<Vec<RawFolder>> {
        let mut raw = Vec::new();
        let mut offset = 0;
        loop {
            let op = format!(
                r#"<m:FindFolder Traversal="Deep"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{FOLDER_PROPS}</t:AdditionalProperties></m:FolderShape><m:IndexedPageFolderView MaxEntriesReturned="500" Offset="{offset}" BasePoint="Beginning"/><m:ParentFolderIds>{root}</m:ParentFolderIds></m:FindFolder>"#
            );
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            let rs = responses(&d)?;
            if let Some(e) = first_error(&rs) {
                return Err(e);
            }
            let folders: Vec<_> = d.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Folder" && child(*n, "FolderId").is_some()).collect();
            raw.extend(folders.iter().filter_map(|f| raw_folder(*f)));
            let last = descendants(d.root(), "RootFolder").next().and_then(|r| r.attribute("IncludesLastItemInRange")).is_none_or(|v| v == "true");
            offset += folders.len();
            if last || folders.is_empty() || offset > 20_000 {
                break;
            }
        }
        Ok(raw)
    }

    /// Корень ящика, служебные папки и (если они есть) сами эти папки — для подключённого
    /// ящика, где FindFolder по всему ящику может быть запрещён.
    async fn mailbox_roots(&mut self, mailbox: Option<&str>) -> Result<(String, HashMap<String, FolderRole>, Vec<RawFolder>)> {
        let ids: String = std::iter::once(distinguished("msgfolderroot", mailbox)).chain(DISTINGUISHED.iter().map(|(id, _)| distinguished(id, mailbox))).collect();
        let props = if mailbox.is_some() { format!("<t:AdditionalProperties>{FOLDER_PROPS}</t:AdditionalProperties>") } else { String::new() };
        let text = self.call(&format!(r#"<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape>{props}</m:FolderShape><m:FolderIds>{ids}</m:FolderIds></m:GetFolder>"#), crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        let rs = responses(&d)?;
        let mut it = rs.into_iter();
        let root_resp = it.next().ok_or_else(|| AppError::Other("Сервер Exchange не ответил на запрос папок".into()))?;
        let mut roles = HashMap::new();
        let mut known: Vec<RawFolder> = Vec::new();
        for (r, (_, role)) in it.zip(DISTINGUISHED.iter()) {
            if response_error(r).is_some() {
                continue;
            }
            if let Some(f) = descendants(r, "Folder").next() {
                if let Some(id) = child(f, "FolderId").and_then(|x| x.attribute("Id")) {
                    roles.insert(id.to_string(), *role);
                }
                if mailbox.is_some() {
                    known.extend(raw_folder(f));
                }
            } else if let Some(id) = descendants(r, "FolderId").next().and_then(|f| f.attribute("Id")) {
                roles.insert(id.to_string(), *role);
            }
        }
        let root = match response_error(root_resp) {
            None => descendants(root_resp, "FolderId").next().and_then(|f| f.attribute("Id")).unwrap_or_default().to_string(),
            // Права только на отдельные папки (например, «Входящие») — корень ящика не виден,
            // но его id — родитель этих папок.
            Some(_) if mailbox.is_some() && known.iter().any(|k| !k.1.is_empty()) => known.iter().find(|k| !k.1.is_empty()).map(|k| k.1.clone()).unwrap_or_default(),
            Some(e) => {
                let code = text_of(root_resp, "ResponseCode").unwrap_or_default();
                return Err(match (mailbox, code.as_str()) {
                    (Some(m), "ErrorNonExistentMailbox" | "ErrorInvalidSmtpAddress" | "ErrorMailRecipientNotFound") => AppError::Other(format!("Ящика {m} нет на сервере Exchange")),
                    (Some(m), "ErrorAccessDenied" | "ErrorFolderNotFound" | "ErrorItemNotFound") => AppError::Other(format!("Нет доступа к ящику {m} — попросите администратора дать вам права на него")),
                    _ => e,
                });
            }
        };
        for k in known.iter_mut().filter(|k| k.1.is_empty()) {
            k.1 = root.clone();
        }
        Ok((root, roles, known))
    }

    /// Есть ли доступ к ящику (перед тем как подключить его вручную).
    pub async fn check_mailbox(&mut self, address: &str) -> Result<()> {
        self.mailbox_roots(Some(address)).await.map(|_| ())
    }

    /// Подключённые ящики: найденные Autodiscover (как у Outlook) и добавленные вручную.
    async fn mailbox_list(&mut self) -> Vec<(String, String)> {
        let (email, manual, need) = {
            let reg = lock(mailbox_registry());
            match reg.get(&self.key) {
                Some(m) => (
                    m.email.clone(),
                    m.manual.clone(),
                    m.discovered.is_none() || (!m.discovered_ok && m.discovered_at.is_none_or(|t| t.elapsed() > DISCOVER_RETRY)),
                ),
                None => return Vec::new(),
            }
        };
        if need && email.contains('@') {
            let found = discover_alternative(&email, &self.login, &self.password).await;
            let mut reg = lock(mailbox_registry());
            if let Some(m) = reg.get_mut(&self.key) {
                m.discovered_ok = found.is_some();
                m.discovered = Some(found.unwrap_or_default());
                m.discovered_at = Some(std::time::Instant::now());
            }
        }
        let discovered = discovered_mailboxes(&self.key);
        let mut out: Vec<(String, String)> = Vec::new();
        for (a, n) in discovered.into_iter().chain(manual.into_iter().map(|a| (a.clone(), a))) {
            if !a.eq_ignore_ascii_case(&email) && !out.iter().any(|(x, _)| x.eq_ignore_ascii_case(&a)) {
                out.push((a, n));
            }
        }
        out.truncate(MAX_MAILBOXES);
        out
    }

    /// Список почтовых папок (дерево, как у IMAP): свой ящик, затем подключённые — `shared/<адрес>/…`.
    pub async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        let (_, roles, _) = self.mailbox_roots(None).await?;
        let raw = self.find_folders(&distinguished("msgfolderroot", None)).await?;
        let root = self.root.clone().unwrap_or_default();
        let (mut out, mut cache): (Vec<Folder>, Vec<EwsFolder>) = build_folders(&raw, &roles, &root, None).into_iter().unzip();
        // Как у IMAP: служебные папки по порядку, вложенные — сразу под своей папкой.
        let rank = |r: FolderRole| match r {
            FolderRole::Inbox => 0,
            FolderRole::Drafts => 1,
            FolderRole::Sent => 2,
            FolderRole::Archive => 3,
            FolderRole::Junk => 4,
            FolderRole::Trash => 5,
            FolderRole::Other => 6,
        };
        let sort = |out: &mut Vec<Folder>, base: usize| {
            let top_role: HashMap<String, FolderRole> = out.iter().filter(|f| f.depth == base).map(|f| (f.path.clone(), f.role)).collect();
            out.sort_by_key(|f: &Folder| {
                let top = f.path.split('/').take(base + 1).collect::<Vec<_>>().join("/");
                (rank(top_role.get(&top).copied().unwrap_or(FolderRole::Other)), f.path.to_lowercase())
            });
        };
        sort(&mut out, 0);

        let boxes = self.mailbox_list().await;
        let mut shared: Vec<Folder> = Vec::new();
        for (address, name) in boxes {
            let prefix = format!("{SHARED_ROOT}/{}", segment(&address));
            let (root, roles, known) = match self.mailbox_roots(Some(&address)).await {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("mailbox {address}: {e}");
                    continue;
                }
            };
            // Весь ящик (полный доступ) или хотя бы его служебные папки (доступ к отдельным папкам).
            let raw = match self.find_folders(&format!(r#"<t:FolderId Id="{}"/>"#, xml_escape(&root))).await {
                Ok(raw) if !raw.is_empty() => raw,
                Ok(_) => known,
                Err(e) => {
                    log::info!("mailbox {address}: folder list unavailable ({e}), showing well-known folders");
                    known
                }
            };
            if raw.is_empty() {
                continue;
            }
            let (mut folders, ews): (Vec<Folder>, Vec<EwsFolder>) = build_folders(&raw, &roles, &root, Some(&prefix)).into_iter().unzip();
            if folders.is_empty() {
                continue;
            }
            sort(&mut folders, 2);
            shared.push(node_folder(&prefix, &name, 1));
            shared.extend(folders);
            cache.extend(ews);
        }
        if !shared.is_empty() {
            out.push(node_folder(SHARED_ROOT, "Подключённые ящики", 0));
            out.extend(shared);
        }
        self.folders = cache;
        Ok(out)
    }

    /// FolderId папки по её пути (папки загружаются при первом обращении).
    async fn folder_id(&mut self, folder: &str) -> Result<String> {
        if !self.folders.iter().any(|f| f.path == folder) {
            self.list_folders().await?;
        }
        self.folders.iter().find(|f| f.path == folder).map(|f| f.id.clone()).ok_or(AppError::NotFound)
    }

    pub async fn role_folder(&mut self, role: FolderRole) -> Result<Option<String>> {
        if self.folders.is_empty() {
            self.list_folders().await?;
        }
        Ok(self.folders.iter().find(|f| f.role == role && !f.path.starts_with("shared/")).map(|f| f.path.clone()))
    }

    fn uid_of(&self, folder_id: &str, item: &str, received: Option<DateTime<Utc>>) -> u32 {
        let mut m = lock(&self.map);
        m.dirty = true;
        m.folders.entry(folder_id.to_string()).or_default().uid_for(item, received)
    }

    fn known(&self, folder_id: &str, uid: u32) -> Option<String> {
        lock(&self.map).folders.get(folder_id).and_then(|f| f.items.get(&uid).cloned())
    }

    fn persist(&self) {
        save_map(&self.key, &self.map, false);
    }

    /// ItemId писем по их номерам. Неизвестный номер (например, после
    /// переустановки) ищется по времени получения.
    async fn ids_of(&mut self, folder: &str, uids: &[u32]) -> Result<(String, Vec<(u32, String)>)> {
        let fid = self.folder_id(folder).await?;
        let mut out = Vec::new();
        let mut unknown = Vec::new();
        for u in uids {
            match self.known(&fid, *u) {
                Some(id) => out.push((*u, id)),
                None => unknown.push(*u),
            }
        }
        if let (Some(min), Some(max)) = (unknown.iter().min(), unknown.iter().max()) {
            let at = |u: u32| Utc.timestamp_opt(i64::from(u) + EPOCH_2000, 0).single().unwrap_or_else(Utc::now).format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let restriction = format!(
                r#"<m:Restriction><t:And><t:IsGreaterThanOrEqualTo><t:FieldURI FieldURI="item:DateTimeReceived"/><t:FieldURIOrConstant><t:Constant Value="{}"/></t:FieldURIOrConstant></t:IsGreaterThanOrEqualTo><t:IsLessThanOrEqualTo><t:FieldURI FieldURI="item:DateTimeReceived"/><t:FieldURIOrConstant><t:Constant Value="{}"/></t:FieldURIOrConstant></t:IsLessThanOrEqualTo></t:And></m:Restriction>"#,
                at(min.saturating_sub(30)),
                at(max + 30)
            );
            self.find(&fid, folder, None, 200, &restriction).await?;
            for u in unknown {
                if let Some(id) = self.known(&fid, u) {
                    out.push((u, id));
                }
            }
        }
        Ok((fid, out))
    }

    /// FindItem: номера писем (новые сверху) и общее число в выборке.
    async fn find(&mut self, fid: &str, _folder: &str, offset: Option<usize>, max: usize, restriction: &str) -> Result<(Vec<u32>, u32, bool)> {
        let op = format!(
            r#"<m:FindItem Traversal="Shallow"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI="item:DateTimeReceived"/></t:AdditionalProperties></m:ItemShape><m:IndexedPageItemView MaxEntriesReturned="{max}" Offset="{}" BasePoint="Beginning"/>{restriction}<m:SortOrder><t:FieldOrder Order="Descending"><t:FieldURI FieldURI="item:DateTimeReceived"/></t:FieldOrder></m:SortOrder><m:ParentFolderIds><t:FolderId Id="{}"/></m:ParentFolderIds></m:FindItem>"#,
            offset.unwrap_or(0),
            xml_escape(fid)
        );
        let text = self.call(&op, crate::netio::API_LIMIT).await?;
        let d = doc(&text)?;
        let rs = responses(&d)?;
        if let Some(e) = first_error(&rs) {
            return Err(e);
        }
        let root = descendants(d.root(), "RootFolder").next();
        let total = root.and_then(|r| r.attribute("TotalItemsInView")).and_then(|v| v.parse().ok()).unwrap_or(0);
        let last = root.and_then(|r| r.attribute("IncludesLastItemInRange")).is_none_or(|v| v == "true");
        let uids = items_of(d.root())
            .into_iter()
            .filter_map(|n| Some(self.uid_of(fid, &item_id(n)?, text_of(n, "DateTimeReceived").and_then(|t| parse_time(&t)))))
            .collect();
        self.persist();
        Ok((uids, total, !last))
    }

    /// GetItem: сводки писем по ItemId.
    async fn get_summaries(&mut self, folder: &str, fid: &str, items: &[(u32, String)]) -> Result<Vec<MessageSummary>> {
        let mut out = Vec::new();
        for chunk in items.chunks(BATCH) {
            let ids: String = chunk.iter().map(|(_, id)| format!(r#"<t:ItemId Id="{}"/>"#, xml_escape(id))).collect();
            let op = format!(r#"<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{SUMMARY_PROPS}</t:AdditionalProperties></m:ItemShape><m:ItemIds>{ids}</m:ItemIds></m:GetItem>"#);
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            for (r, (uid, _)) in responses(&d)?.into_iter().zip(chunk) {
                match response_error(r) {
                    None => {
                        if let Some(n) = items_of(r).into_iter().next() {
                            out.push(self.summary(folder, *uid, n));
                        }
                    }
                    Some(AppError::NotFound) => lock(&self.map).folders.entry(fid.to_string()).or_default().forget(*uid),
                    Some(e) => return Err(e),
                }
            }
        }
        out.sort_by_key(|m| std::cmp::Reverse(m.uid));
        Ok(out)
    }

    // ------------------------------------------------------------ письма

    pub async fn list_messages(&mut self, folder: &str, before_uid: Option<u32>, limit: u32) -> Result<MessagePage> {
        let fid = self.folder_id(folder).await?;
        let limit = limit.clamp(1, 200) as usize;
        let restriction = match before_uid {
            Some(b) => {
                let t = Utc.timestamp_opt(i64::from(b) + EPOCH_2000, 0).single().unwrap_or_else(Utc::now);
                format!(r#"<m:Restriction><t:IsLessThanOrEqualTo><t:FieldURI FieldURI="item:DateTimeReceived"/><t:FieldURIOrConstant><t:Constant Value="{}"/></t:FieldURIOrConstant></t:IsLessThanOrEqualTo></m:Restriction>"#, t.format("%Y-%m-%dT%H:%M:%SZ"))
            }
            None => String::new(),
        };
        // С запасом: письма того же времени, что и `before_uid`, отсекаются ниже.
        let (mut uids, in_view, more) = self.find(&fid, folder, None, limit + 8, &restriction).await?;
        if let Some(b) = before_uid {
            uids.retain(|u| *u < b);
        }
        let has_more = more || uids.len() > limit;
        uids.truncate(limit);
        let total = if before_uid.is_none() { in_view } else { self.folder_total(&fid).await.unwrap_or(in_view) };
        let ids: Vec<(u32, String)> = uids.iter().filter_map(|u| self.known(&fid, *u).map(|id| (*u, id))).collect();
        let items = self.get_summaries(folder, &fid, &ids).await?;
        Ok(MessagePage { items, total, has_more })
    }

    async fn folder_total(&mut self, fid: &str) -> Result<u32> {
        let (total, _) = self.folder_counts(fid).await?;
        Ok(total)
    }

    async fn folder_counts(&mut self, fid: &str) -> Result<(u32, u32)> {
        let op = format!(r#"<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI="folder:TotalCount"/><t:FieldURI FieldURI="folder:UnreadCount"/></t:AdditionalProperties></m:FolderShape><m:FolderIds><t:FolderId Id="{}"/></m:FolderIds></m:GetFolder>"#, xml_escape(fid));
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        let rs = responses(&d)?;
        if let Some(e) = first_error(&rs) {
            return Err(e);
        }
        let n = |name: &str| descendants(d.root(), name).next().and_then(|x| x.text()).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        Ok((n("TotalCount"), n("UnreadCount")))
    }

    pub async fn unread(&mut self, folder: &str) -> Result<u32> {
        let fid = self.folder_id(folder).await?;
        Ok(self.folder_counts(&fid).await?.1)
    }

    pub async fn summaries_of(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<MessageSummary>> {
        let (fid, ids) = self.ids_of(folder, uids).await?;
        self.get_summaries(folder, &fid, &ids).await
    }

    /// Новые непрочитанные после `after_uid` и последний номер в папке.
    pub async fn new_unseen(&mut self, folder: &str, after_uid: u32, limit: u32) -> Result<(Vec<MessageSummary>, u32)> {
        let fid = self.folder_id(folder).await?;
        let (latest, _, _) = self.find(&fid, folder, None, 1, "").await?;
        let last_uid = latest.first().copied().unwrap_or(0);
        if limit == 0 {
            return Ok((Vec::new(), last_uid));
        }
        let unread = r#"<m:Restriction><t:IsEqualTo><t:FieldURI FieldURI="message:IsRead"/><t:FieldURIOrConstant><t:Constant Value="false"/></t:FieldURIOrConstant></t:IsEqualTo></m:Restriction>"#;
        let (mut uids, _, _) = self.find(&fid, folder, None, 20, unread).await?;
        uids.retain(|u| *u > after_uid);
        uids.truncate(limit.min(10) as usize);
        let ids: Vec<(u32, String)> = uids.iter().filter_map(|u| self.known(&fid, *u).map(|id| (*u, id))).collect();
        let items = self.get_summaries(folder, &fid, &ids).await?.into_iter().filter(|m| m.unread).collect();
        Ok((items, last_uid.max(uids.first().copied().unwrap_or(0))))
    }

    /// Исходник письма (MIME).
    pub async fn mime(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        let (_, ids) = self.ids_of(folder, &[uid]).await?;
        let (_, id) = ids.into_iter().next().ok_or(AppError::NotFound)?;
        let mut got = self.mimes_of(&[(uid, id)], u32::MAX).await?;
        got.pop().map(|(_, raw)| raw).ok_or(AppError::NotFound)
    }

    async fn mimes_of(&mut self, items: &[(u32, String)], max_each: u32) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut out = Vec::new();
        for chunk in items.chunks(10) {
            let ids: String = chunk.iter().map(|(_, id)| format!(r#"<t:ItemId Id="{}"/>"#, xml_escape(id))).collect();
            let op = format!(r#"<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:IncludeMimeContent>true</t:IncludeMimeContent></m:ItemShape><m:ItemIds>{ids}</m:ItemIds></m:GetItem>"#);
            let text = self.call(&op, MIME_LIMIT).await?;
            let d = doc(&text)?;
            for (r, (uid, _)) in responses(&d)?.into_iter().zip(chunk) {
                if response_error(r).is_some() {
                    continue;
                }
                let Some(b64) = descendants(r, "MimeContent").next().and_then(|m| m.text()) else { continue };
                let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
                if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(clean) {
                    if raw.len() <= max_each as usize {
                        out.push((*uid, raw));
                    }
                }
            }
        }
        Ok(out)
    }

    /// Исходники писем папки (синхронизация хранилища на компьютере).
    pub async fn mimes(&mut self, folder: &str, uids: &[u32], max_each: u32) -> Result<Vec<(u32, Vec<u8>)>> {
        let (_, ids) = self.ids_of(folder, uids).await?;
        self.mimes_of(&ids, max_each).await
    }

    /// Флаги всех писем папки (синхронизация хранилища). Номера писем, которых
    /// больше нет, забываются.
    pub async fn folder_flags(&mut self, folder: &str) -> Result<(u32, Vec<(u32, FlagState)>)> {
        let fid = self.folder_id(folder).await?;
        let mut out = Vec::new();
        let mut present = std::collections::HashSet::new();
        let mut offset = 0usize;
        loop {
            let op = format!(
                r#"<m:FindItem Traversal="Shallow"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{FLAG_PROPS}</t:AdditionalProperties></m:ItemShape><m:IndexedPageItemView MaxEntriesReturned="1000" Offset="{offset}" BasePoint="Beginning"/><m:ParentFolderIds><t:FolderId Id="{}"/></m:ParentFolderIds></m:FindItem>"#,
                xml_escape(&fid)
            );
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            let rs = responses(&d)?;
            if let Some(e) = first_error(&rs) {
                return Err(e);
            }
            let items = items_of(d.root());
            for n in &items {
                let Some(id) = item_id(*n) else { continue };
                let uid = self.uid_of(&fid, &id, text_of(*n, "DateTimeReceived").and_then(|t| parse_time(&t)));
                present.insert(uid);
                out.push((uid, flags_of(*n)));
            }
            let last = descendants(d.root(), "RootFolder").next().and_then(|r| r.attribute("IncludesLastItemInRange")).is_none_or(|v| v == "true");
            offset += items.len();
            if last || items.is_empty() || offset > 500_000 {
                break;
            }
        }
        {
            let mut m = lock(&self.map);
            if let Some(f) = m.folders.get_mut(&fid) {
                let gone: Vec<u32> = f.items.keys().filter(|u| !present.contains(u)).copied().collect();
                for u in gone {
                    f.forget(u);
                }
            }
            m.dirty = true;
        }
        self.persist();
        Ok((1, out))
    }

    // ------------------------------------------------------------ изменения

    async fn update(&mut self, folder: &str, uids: &[u32], change: impl Fn(&str) -> String) -> Result<()> {
        let (_, ids) = self.ids_of(folder, uids).await?;
        for chunk in ids.chunks(BATCH) {
            let changes: String = chunk.iter().map(|(_, id)| format!(r#"<t:ItemChange><t:ItemId Id="{}"/><t:Updates>{}</t:Updates></t:ItemChange>"#, xml_escape(id), change(id))).collect();
            let op = format!(r#"<m:UpdateItem MessageDisposition="SaveOnly" ConflictResolution="AlwaysOverwrite"><m:ItemChanges>{changes}</m:ItemChanges></m:UpdateItem>"#);
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            if let Some(e) = first_error(&responses(&d)?) {
                return Err(e);
            }
        }
        Ok(())
    }

    pub async fn set_flag(&mut self, folder: &str, uids: &[u32], flag: FlagKind, value: bool) -> Result<()> {
        let set = match flag {
            FlagKind::Seen => format!(r#"<t:SetItemField><t:FieldURI FieldURI="message:IsRead"/><t:Message><t:IsRead>{value}</t:IsRead></t:Message></t:SetItemField>"#),
            FlagKind::Flagged => format!(r#"<t:SetItemField><t:FieldURI FieldURI="item:Flag"/><t:Message><t:Flag><t:FlagStatus>{}</t:FlagStatus></t:Flag></t:Message></t:SetItemField>"#, if value { "Flagged" } else { "NotFlagged" }),
            FlagKind::Important => format!(r#"<t:SetItemField><t:FieldURI FieldURI="item:Importance"/><t:Message><t:Importance>{}</t:Importance></t:Message></t:SetItemField>"#, if value { "High" } else { "Normal" }),
        };
        self.update(folder, uids, |_| set.clone()).await
    }

    /// Категория (ключевое слово IMAP) у писем: категории Exchange с тем же именем.
    pub async fn set_keyword(&mut self, folder: &str, uids: &[u32], keyword: &str, value: bool) -> Result<()> {
        if keyword.eq_ignore_ascii_case(crate::imap::IMPORTANT) {
            return self.set_flag(folder, uids, FlagKind::Important, value).await;
        }
        let current = self.summaries_of(folder, uids).await?;
        for m in current {
            let mut cats: Vec<String> = m.keywords.iter().filter(|k| !k.eq_ignore_ascii_case(keyword)).cloned().collect();
            if value {
                cats.push(keyword.to_string());
            }
            let set = if cats.is_empty() {
                r#"<t:DeleteItemField><t:FieldURI FieldURI="item:Categories"/></t:DeleteItemField>"#.to_string()
            } else {
                let list: String = cats.iter().map(|c| format!("<t:String>{}</t:String>", xml_escape(c))).collect();
                format!(r#"<t:SetItemField><t:FieldURI FieldURI="item:Categories"/><t:Message><t:Categories>{list}</t:Categories></t:Message></t:SetItemField>"#)
            };
            self.update(folder, &[m.uid], |_| set.clone()).await?;
        }
        Ok(())
    }

    pub async fn move_messages(&mut self, folder: &str, uids: &[u32], target: &str) -> Result<()> {
        if uids.is_empty() || folder == target {
            return Ok(());
        }
        let to = self.folder_id(target).await?;
        let (fid, ids) = self.ids_of(folder, uids).await?;
        for chunk in ids.chunks(BATCH) {
            let list: String = chunk.iter().map(|(_, id)| format!(r#"<t:ItemId Id="{}"/>"#, xml_escape(id))).collect();
            let op = format!(r#"<m:MoveItem><m:ToFolderId><t:FolderId Id="{}"/></m:ToFolderId><m:ItemIds>{list}</m:ItemIds></m:MoveItem>"#, xml_escape(&to));
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            if let Some(e) = first_error(&responses(&d)?) {
                return Err(e);
            }
            let mut m = lock(&self.map);
            let f = m.folders.entry(fid.clone()).or_default();
            for (u, _) in chunk {
                f.forget(*u);
            }
            m.dirty = true;
        }
        self.persist();
        Ok(())
    }

    /// Удаление: в «Удалённые» (или в папку `trash`), а из неё — насовсем.
    pub async fn delete_messages(&mut self, folder: &str, uids: &[u32], trash: Option<&str>) -> Result<bool> {
        if uids.is_empty() {
            return Ok(false);
        }
        if let Some(t) = trash.filter(|t| *t != folder) {
            self.move_messages(folder, uids, t).await?;
            return Ok(false);
        }
        let (fid, ids) = self.ids_of(folder, uids).await?;
        for chunk in ids.chunks(BATCH) {
            let list: String = chunk.iter().map(|(_, id)| format!(r#"<t:ItemId Id="{}"/>"#, xml_escape(id))).collect();
            let op = format!(r#"<m:DeleteItem DeleteType="HardDelete"><m:ItemIds>{list}</m:ItemIds></m:DeleteItem>"#);
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            if let Some(e) = first_error(&responses(&d)?).filter(|e| !matches!(e, AppError::NotFound)) {
                return Err(e);
            }
            let mut m = lock(&self.map);
            let f = m.folders.entry(fid.clone()).or_default();
            for (u, _) in chunk {
                f.forget(*u);
            }
            m.dirty = true;
        }
        self.persist();
        Ok(true)
    }

    /// Положить письмо (MIME) в папку. `read` — прочитанное и не черновик
    /// (копия отправленного, импорт); иначе — черновик.
    pub async fn append(&mut self, folder: &str, raw: &[u8], read: bool) -> Result<()> {
        let fid = self.folder_id(folder).await?;
        let flags = if read { r#"<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag="0x0E07" PropertyType="Integer"/><t:Value>1</t:Value></t:ExtendedProperty>"# } else { "" };
        let op = format!(
            r#"<m:CreateItem MessageDisposition="SaveOnly"><m:SavedItemFolderId><t:FolderId Id="{}"/></m:SavedItemFolderId><m:Items><t:Message><t:MimeContent CharacterSet="UTF-8">{}</t:MimeContent>{flags}</t:Message></m:Items></m:CreateItem>"#,
            xml_escape(&fid),
            base64::engine::general_purpose::STANDARD.encode(raw)
        );
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(()), Err)
    }

    /// Отправить письмо (MIME) через Exchange с копией в «Отправленные».
    /// От имени подключённого ящика (From — его адрес) Exchange сам проверяет право «Отправить как»
    /// или «Отправить от имени»; копия — в «Отправленные» того ящика, если она видна, иначе в свои.
    pub async fn send(&mut self, raw: &[u8]) -> Result<()> {
        let own = lock(mailbox_registry()).get(&self.key).map(|m| m.email.clone()).unwrap_or_default();
        let from = crate::imap::from_address(raw).filter(|f| !own.is_empty() && !f.eq_ignore_ascii_case(&own));
        if from.is_some() && self.folders.is_empty() {
            let _ = self.list_folders().await;
        }
        let saved = from
            .as_ref()
            .and_then(|f| {
                let prefix = format!("{SHARED_ROOT}/{}/", segment(f));
                self.folders.iter().find(|x| x.role == FolderRole::Sent && utf7_imap::decode_utf7_imap(x.path.clone()).starts_with(&prefix))
            })
            .map_or_else(|| distinguished("sentitems", None), |x| format!(r#"<t:FolderId Id="{}"/>"#, xml_escape(&x.id)));
        let op = format!(
            r#"<m:CreateItem MessageDisposition="SendAndSaveCopy"><m:SavedItemFolderId>{saved}</m:SavedItemFolderId><m:Items><t:Message><t:MimeContent CharacterSet="UTF-8">{}</t:MimeContent></t:Message></m:Items></m:CreateItem>"#,
            base64::engine::general_purpose::STANDARD.encode(raw)
        );
        // Запрос ушёл, а ответа нет — сервер мог уже отправить письмо.
        let text = match self.call(&op, crate::netio::SMALL_LIMIT).await {
            Ok(t) => t,
            Err(e @ (AppError::Unreachable(..) | AppError::AuthFailed)) => return Err(e),
            Err(e @ AppError::Other(_)) => return Err(e),
            Err(e) => return Err(AppError::DeliveryUnknown(e.to_string())),
        };
        let d = doc(&text)?;
        let rs = responses(&d)?;
        if let (Some(f), Some(r)) = (&from, rs.first()) {
            if matches!(text_of(*r, "ResponseCode").as_deref(), Some("ErrorSendAsDenied" | "ErrorAccessDenied")) {
                return Err(AppError::Other(format!("Нет права отправлять письма от имени {f} — попросите администратора выдать право «Отправить как» или «Отправить от имени» для этого ящика")));
            }
        }
        first_error(&rs).map_or(Ok(()), Err)
    }

    pub async fn create_folder(&mut self, name: &str) -> Result<()> {
        let path = utf7_imap::decode_utf7_imap(name.to_string());
        let (parent, leaf) = match path.rsplit_once('/') {
            Some((p, l)) => (Some(utf7_imap::encode_utf7_imap(p.to_string())), l.to_string()),
            None => (None, path.clone()),
        };
        if self.folder_id(name).await.is_ok() {
            return Ok(());
        }
        let parent_xml = match parent {
            Some(p) => format!(r#"<t:FolderId Id="{}"/>"#, xml_escape(&self.folder_id(&p).await?)),
            None => r#"<t:DistinguishedFolderId Id="msgfolderroot"/>"#.to_string(),
        };
        let op = format!(r#"<m:CreateFolder><m:ParentFolderId>{parent_xml}</m:ParentFolderId><m:Folders><t:Folder><t:FolderClass>IPF.Note</t:FolderClass><t:DisplayName>{}</t:DisplayName></t:Folder></m:Folders></m:CreateFolder>"#, xml_escape(&leaf));
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        let rs = responses(&d)?;
        let exists = rs.iter().any(|r| text_of(*r, "ResponseCode").as_deref() == Some("ErrorFolderExists"));
        if !exists {
            if let Some(e) = first_error(&rs) {
                return Err(e);
            }
        }
        self.folders.clear();
        Ok(())
    }

    /// Поиск на сервере (AQS, Exchange 2013+): номера найденных писем.
    pub async fn search(&mut self, folder: &str, query: &crate::search::Query) -> Result<Vec<u32>> {
        let fid = self.folder_id(folder).await?;
        let aqs = aqs_of(query);
        if aqs.is_empty() {
            return Ok(Vec::new());
        }
        let op = format!(
            r#"<m:FindItem Traversal="Shallow"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI="item:DateTimeReceived"/></t:AdditionalProperties></m:ItemShape><m:IndexedPageItemView MaxEntriesReturned="300" Offset="0" BasePoint="Beginning"/><m:SortOrder><t:FieldOrder Order="Descending"><t:FieldURI FieldURI="item:DateTimeReceived"/></t:FieldOrder></m:SortOrder><m:ParentFolderIds><t:FolderId Id="{}"/></m:ParentFolderIds><m:QueryString>{}</m:QueryString></m:FindItem>"#,
            xml_escape(&fid),
            xml_escape(&aqs)
        );
        let text = self.call(&op, crate::netio::API_LIMIT).await?;
        let d = doc(&text)?;
        if let Some(e) = first_error(&responses(&d)?) {
            return Err(e);
        }
        let uids = items_of(d.root()).into_iter().filter_map(|n| Some(self.uid_of(&fid, &item_id(n)?, text_of(n, "DateTimeReceived").and_then(|t| parse_time(&t))))).collect();
        self.persist();
        Ok(uids)
    }

    /// Номер, с которого начнутся новые письма (как UIDNEXT).
    pub async fn uid_next(&mut self, folder: &str) -> Result<u32> {
        let fid = self.folder_id(folder).await?;
        let (latest, _, _) = self.find(&fid, folder, None, 1, "").await?;
        let now = (Utc::now().timestamp() - EPOCH_2000).max(1) as u32;
        Ok(latest.first().map_or(now, |u| u + 1))
    }

    /// Номера писем начиная с `from` (новые письма — для правил).
    pub async fn uids_from(&mut self, folder: &str, from: u32) -> Result<Vec<u32>> {
        let fid = self.folder_id(folder).await?;
        let (mut uids, _, _) = self.find(&fid, folder, None, 100, "").await?;
        uids.retain(|u| *u >= from);
        uids.sort_unstable();
        Ok(uids)
    }

    /// Где письма из очереди без сети: номер, если письмо ещё в папке.
    pub async fn locate(&mut self, folder: &str, targets: &[crate::offline::Target]) -> Result<Vec<Option<u32>>> {
        let Ok(fid) = self.folder_id(folder).await else { return Ok(vec![None; targets.len()]) };
        let mut out = Vec::new();
        for t in targets {
            let id = if t.uid > 0 { self.known(&fid, t.uid) } else { None };
            let found = match id {
                Some(id) => {
                    let s = self.get_summaries(folder, &fid, &[(t.uid, id)]).await?;
                    s.first().filter(|m| t.msg_id.as_ref().is_none_or(|x| m.message_id.as_ref().is_some_and(|y| crate::offline::norm_id(x) == crate::offline::norm_id(y)))).map(|m| m.uid)
                }
                None => None,
            };
            let found = match (found, &t.msg_id) {
                (Some(u), _) => Some(u),
                (None, Some(mid)) => {
                    let q = format!("<{}>", crate::offline::norm_id(mid));
                    let restriction = format!(r#"<m:Restriction><t:IsEqualTo><t:FieldURI FieldURI="message:InternetMessageId"/><t:FieldURIOrConstant><t:Constant Value="{}"/></t:FieldURIOrConstant></t:IsEqualTo></m:Restriction>"#, xml_escape(&q));
                    self.find(&fid, folder, None, 5, &restriction).await.ok().and_then(|(u, _, _)| u.first().copied())
                }
                _ => None,
            };
            out.push(found);
        }
        Ok(out)
    }

    pub fn close(&self) {
        save_map(&self.key, &self.map, true);
    }
}

// ---------------------------------------------------------------- календарь и контакты

/// Windows-имя часового пояса компьютера (для событий «на весь день»: Exchange
/// хранит их в поясе запроса). По смещению — пояса России, иначе UTC.
fn windows_tz() -> &'static str {
    match chrono::Local::now().offset().local_minus_utc() / 3600 {
        2 => "Kaliningrad Standard Time",
        3 => "Russian Standard Time",
        4 => "Russia Time Zone 3",
        5 => "Ekaterinburg Standard Time",
        6 => "Omsk Standard Time",
        7 => "North Asia Standard Time",
        8 => "North Asia East Standard Time",
        9 => "Yakutsk Standard Time",
        10 => "Vladivostok Standard Time",
        11 => "Magadan Standard Time",
        12 => "Russia Time Zone 11",
        _ => "UTC",
    }
}

/// Адрес коллекции календаря/контактов Exchange в общих типах программы.
pub const CONTACTS_URL: &str = "ews:contacts";
pub const GAL_URL: &str = "ews:gal";

fn ews_href(kind: &str, flags: &str, id: &str) -> String {
    format!("ews:{kind}:{flags}:{id}")
}

/// `ews:cal:<флаги>:<ItemId>` → (флаги, ItemId).
pub fn parse_href<'a>(href: &'a str, kind: &str) -> Option<(&'a str, &'a str)> {
    let rest = href.strip_prefix("ews:")?.strip_prefix(kind)?.strip_prefix(':')?;
    rest.split_once(':')
}

fn utc(s: &str) -> String {
    s.to_string()
}

/// Значение поля пустое (`<t:X></t:X>` или `<t:Body …></t:Body>`).
fn empty_field(xml: &str) -> bool {
    xml.find('>').is_some_and(|i| xml[i + 1..].starts_with("</"))
}

const CAL_PROPS: &str = r#"<t:FieldURI FieldURI="item:Subject"/><t:FieldURI FieldURI="calendar:Start"/><t:FieldURI FieldURI="calendar:End"/><t:FieldURI FieldURI="calendar:IsAllDayEvent"/><t:FieldURI FieldURI="calendar:Location"/><t:FieldURI FieldURI="calendar:Organizer"/><t:FieldURI FieldURI="calendar:IsRecurring"/><t:FieldURI FieldURI="calendar:CalendarItemType"/><t:FieldURI FieldURI="calendar:UID"/><t:FieldURI FieldURI="calendar:MyResponseType"/><t:FieldURI FieldURI="calendar:IsMeeting"/>"#;

const CONTACT_PROPS: &str = r#"<t:FieldURI FieldURI="contacts:DisplayName"/><t:FieldURI FieldURI="contacts:EmailAddresses"/><t:FieldURI FieldURI="contacts:PhoneNumbers"/><t:FieldURI FieldURI="contacts:CompanyName"/><t:FieldURI FieldURI="contacts:JobTitle"/><t:FieldURI FieldURI="contacts:Department"/><t:FieldURI FieldURI="contacts:PhysicalAddresses"/>"#;

const PHONE_KEYS: [&str; 4] = ["BusinessPhone", "MobilePhone", "HomePhone", "OtherTelephone"];

fn local_date(t: &str) -> Option<chrono::NaiveDate> {
    parse_time(t).map(|d| d.with_timezone(&chrono::Local).date_naive())
}

fn entries(n: roxmltree::Node<'_, '_>, name: &str) -> Vec<(String, String)> {
    child(n, name)
        .map(|c| c.children().filter(|e| e.is_element() && e.tag_name().name() == "Entry").filter_map(|e| Some((e.attribute("Key").unwrap_or_default().to_string(), e.text()?.trim().to_string()))).filter(|(_, v)| !v.is_empty()).collect())
        .unwrap_or_default()
}

fn address_of(n: roxmltree::Node<'_, '_>) -> Option<String> {
    let addrs = child(n, "PhysicalAddresses")?;
    let e = addrs.children().find(|e| e.is_element() && e.tag_name().name() == "Entry")?;
    let parts: Vec<String> = ["Street", "City", "State", "PostalCode", "CountryOrRegion"].iter().filter_map(|k| text_of(e, k)).filter(|v| !v.is_empty()).collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn contact_of(n: roxmltree::Node<'_, '_>, id: &str, change_key: Option<String>, body: Option<String>) -> crate::dav::Contact {
    let emails: Vec<String> = entries(n, "EmailAddresses").into_iter().map(|(_, v)| v.trim_start_matches("SMTP:").trim_start_matches("smtp:").to_string()).filter(|v| v.contains('@')).collect();
    let phones: Vec<String> = entries(n, "PhoneNumbers").into_iter().map(|(_, v)| v).collect();
    let name = text_of(n, "DisplayName").filter(|s| !s.is_empty()).or_else(|| emails.first().cloned()).unwrap_or_else(|| "(без имени)".into());
    let href = ews_href("c", "-", id);
    crate::dav::Contact {
        id: href.clone(),
        book: "Контакты".into(),
        href,
        etag: change_key,
        editable: true,
        full_name: name,
        emails,
        phones,
        org: text_of(n, "CompanyName").filter(|s| !s.is_empty()),
        title: text_of(n, "JobTitle").filter(|s| !s.is_empty()),
        note: body.filter(|b| !b.trim().is_empty()),
        address: address_of(n),
    }
}

impl EwsSession {
    /// Календари ящика (основной и вложенные), как коллекции CalDAV.
    pub async fn calendars(&mut self) -> Result<Vec<crate::dav::Collection>> {
        let op = r#"<m:FindFolder Traversal="Deep"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI="folder:DisplayName"/><t:FieldURI FieldURI="folder:FolderClass"/><t:ExtendedFieldURI PropertyTag="0x10F4" PropertyType="Boolean"/></t:AdditionalProperties></m:FolderShape><m:IndexedPageFolderView MaxEntriesReturned="500" Offset="0" BasePoint="Beginning"/><m:ParentFolderIds><t:DistinguishedFolderId Id="msgfolderroot"/></m:ParentFolderIds></m:FindFolder>"#;
        let text = self.call(op, crate::netio::API_LIMIT).await?;
        let d = doc(&text)?;
        if let Some(e) = first_error(&responses(&d)?) {
            return Err(e);
        }
        let mut out: Vec<crate::dav::Collection> = d
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name().ends_with("Folder") && child(*n, "FolderId").is_some())
            .filter(|f| text_of(*f, "FolderClass").is_some_and(|c| c == "IPF.Appointment" || c.starts_with("IPF.Appointment.")))
            .filter(|f| !descendants(*f, "ExtendedProperty").any(|e| text_of(e, "Value").as_deref() == Some("true")))
            .filter_map(|f| {
                let id = child(f, "FolderId")?.attribute("Id")?;
                Some(crate::dav::Collection { url: format!("ews:cal:{id}"), name: text_of(f, "DisplayName").unwrap_or_else(|| "Календарь".into()), color: None, writable: true })
            })
            .collect();
        // Основной календарь — первым.
        let main = r#"<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds><t:DistinguishedFolderId Id="calendar"/></m:FolderIds></m:GetFolder>"#;
        if let Ok(text) = self.call(main, crate::netio::SMALL_LIMIT).await {
            if let Ok(d) = doc(&text) {
                if let Some(id) = descendants(d.root(), "FolderId").next().and_then(|f| f.attribute("Id")) {
                    let url = format!("ews:cal:{id}");
                    out.sort_by_key(|c| c.url != url);
                }
            }
        }
        Ok(out)
    }

    /// События календаря за период (повторения развёрнуты сервером).
    pub async fn events(&mut self, cal: &crate::dav::Collection, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<crate::dav::CalEvent>> {
        let fid = cal.url.strip_prefix("ews:cal:").ok_or(AppError::NotFound)?.to_string();
        let mut out = Vec::new();
        // CalendarView — не больше двух лет за запрос.
        let mut from = start;
        while from < end {
            let to = (from + chrono::Duration::days(366)).min(end);
            let op = format!(
                r#"<m:FindItem Traversal="Shallow"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{CAL_PROPS}</t:AdditionalProperties></m:ItemShape><m:CalendarView MaxEntriesReturned="1000" StartDate="{}" EndDate="{}"/><m:ParentFolderIds><t:FolderId Id="{}"/></m:ParentFolderIds></m:FindItem>"#,
                from.format("%Y-%m-%dT%H:%M:%SZ"),
                to.format("%Y-%m-%dT%H:%M:%SZ"),
                xml_escape(&fid)
            );
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            if let Some(e) = first_error(&responses(&d)?) {
                return Err(e);
            }
            let items: Vec<_> = d.descendants().filter(|n| n.is_element() && n.tag_name().name() == "CalendarItem").collect();
            let mut ids = Vec::new();
            for n in &items {
                let Some(idn) = child(*n, "ItemId") else { continue };
                let id = idn.attribute("Id").unwrap_or_default().to_string();
                let ck = idn.attribute("ChangeKey").map(str::to_string);
                let all_day = text_of(*n, "IsAllDayEvent").as_deref() == Some("true");
                let (s, e) = (text_of(*n, "Start").unwrap_or_default(), text_of(*n, "End").unwrap_or_default());
                let (start_s, end_s) = if all_day {
                    let sd = local_date(&s).unwrap_or_default();
                    let ed = local_date(&e).filter(|x| *x > sd).unwrap_or(sd + chrono::Duration::days(1));
                    (sd.to_string(), ed.to_string())
                } else {
                    (parse_time(&s).map(|t| t.to_rfc3339()).unwrap_or(utc(&s)), parse_time(&e).map(|t| t.to_rfc3339()).unwrap_or(utc(&e)))
                };
                let kind = text_of(*n, "CalendarItemType").unwrap_or_default();
                let recurring = kind == "Occurrence" || kind == "Exception" || text_of(*n, "IsRecurring").as_deref() == Some("true");
                let organizer_meeting = text_of(*n, "MyResponseType").as_deref() == Some("Organizer") && text_of(*n, "IsMeeting").as_deref() == Some("true");
                let flags = format!("{}{}", if recurring && kind != "RecurringMaster" { "r" } else { "" }, if organizer_meeting { "m" } else { "" });
                let organizer = child(*n, "Organizer").and_then(|o| child(o, "Mailbox")).map(mailbox);
                ids.push(id.clone());
                out.push(crate::dav::CalEvent {
                    id: format!("{id}@{start_s}"),
                    uid: text_of(*n, "UID").unwrap_or_else(|| id.clone()),
                    href: ews_href("cal", if flags.is_empty() { "-" } else { &flags }, &id),
                    etag: ck,
                    editable: true,
                    calendar_url: cal.url.clone(),
                    calendar: cal.name.clone(),
                    color: cal.color.clone(),
                    title: text_of(*n, "Subject").filter(|s| !s.is_empty()).unwrap_or_else(|| "(без названия)".into()),
                    location: text_of(*n, "Location").filter(|s| !s.is_empty()),
                    description: None,
                    organizer: organizer.as_ref().map(|(n, a)| if n.is_empty() { a.clone() } else { n.clone() }),
                    participants: organizer.map(|(_, a)| a.to_lowercase()).filter(|a| a.contains('@')).into_iter().collect(),
                    start: start_s,
                    end: end_s,
                    all_day,
                    recurring,
                });
            }
            // Описание и участники — отдельным запросом (CalendarView их не отдаёт).
            let details = self.event_details(&ids).await.unwrap_or_default();
            for ev in out.iter_mut() {
                if let Some((_, id)) = parse_href(&ev.href, "cal") {
                    if let Some((desc, people)) = details.get(id) {
                        if ev.description.is_none() {
                            ev.description = desc.clone();
                        }
                        for p in people {
                            if !ev.participants.contains(p) {
                                ev.participants.push(p.clone());
                            }
                        }
                    }
                }
            }
            from = to;
        }
        Ok(out)
    }

    async fn event_details(&mut self, ids: &[String]) -> Result<HashMap<String, (Option<String>, Vec<String>)>> {
        let mut out = HashMap::new();
        for chunk in ids.chunks(BATCH) {
            let list: String = chunk.iter().map(|id| format!(r#"<t:ItemId Id="{}"/>"#, xml_escape(id))).collect();
            let op = format!(r#"<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:BodyType>Text</t:BodyType><t:AdditionalProperties><t:FieldURI FieldURI="item:Body"/><t:FieldURI FieldURI="calendar:RequiredAttendees"/><t:FieldURI FieldURI="calendar:OptionalAttendees"/></t:AdditionalProperties></m:ItemShape><m:ItemIds>{list}</m:ItemIds></m:GetItem>"#);
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            for (r, id) in responses(&d)?.into_iter().zip(chunk) {
                if response_error(r).is_some() {
                    continue;
                }
                let Some(n) = descendants(r, "CalendarItem").next() else { continue };
                let body = text_of(n, "Body").filter(|b| !b.trim().is_empty());
                let people: Vec<String> = ["RequiredAttendees", "OptionalAttendees"]
                    .iter()
                    .filter_map(|k| child(n, k))
                    .flat_map(|a| descendants(a, "Mailbox").map(mailbox).collect::<Vec<_>>())
                    .map(|(_, addr)| addr.to_lowercase())
                    .filter(|a| a.contains('@'))
                    .collect();
                out.insert(id.clone(), (body, people));
            }
        }
        Ok(out)
    }

    fn event_fields(input: &crate::dav::EventInput, with_time: bool) -> Result<(String, Vec<(&'static str, String)>)> {
        let title = input.title.trim();
        if title.is_empty() {
            return Err(AppError::Other("Укажите название события".into()));
        }
        let mut fields: Vec<(&'static str, String)> = vec![("item:Subject", format!("<t:Subject>{}</t:Subject>", xml_escape(title)))];
        fields.push(("item:Body", format!(r#"<t:Body BodyType="Text">{}</t:Body>"#, xml_escape(input.description.as_deref().unwrap_or_default()))));
        let mut tz = String::new();
        if with_time {
            let (start, end) = if input.all_day {
                let p = |v: &str| chrono::NaiveDate::parse_from_str(&v[..10.min(v.len())], "%Y-%m-%d").map_err(|_| AppError::Other(format!("Некорректная дата: {v}")));
                let s = p(&input.start)?;
                let e = p(&input.end)?.max(s + chrono::Duration::days(1));
                tz = windows_tz().to_string();
                (format!("{s}T00:00:00"), format!("{e}T00:00:00"))
            } else {
                let p = |v: &str| DateTime::parse_from_rfc3339(v).map(|d| d.with_timezone(&Utc)).map_err(|_| AppError::Other(format!("Некорректное время: {v}")));
                let (s, e) = (p(&input.start)?, p(&input.end)?);
                if e < s {
                    return Err(AppError::Other("Окончание раньше начала".into()));
                }
                (s.format("%Y-%m-%dT%H:%M:%SZ").to_string(), e.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            };
            fields.push(("calendar:Start", format!("<t:Start>{start}</t:Start>")));
            fields.push(("calendar:End", format!("<t:End>{end}</t:End>")));
            fields.push(("calendar:IsAllDayEvent", format!("<t:IsAllDayEvent>{}</t:IsAllDayEvent>", input.all_day)));
        }
        fields.push(("calendar:Location", format!("<t:Location>{}</t:Location>", xml_escape(input.location.as_deref().unwrap_or_default().trim()))));
        Ok((tz, fields))
    }

    /// Запрос с поясом компьютера (события «на весь день» задаются местным временем).
    async fn call_tz(&mut self, op: &str, tz: &str) -> Result<String> {
        if tz.is_empty() {
            return self.call(op, crate::netio::API_LIMIT).await;
        }
        let body = envelope(self.version, op).replacen("</soap:Header>", &format!(r#"<t:TimeZoneContext><t:TimeZoneDefinition Id="{tz}"/></t:TimeZoneContext></soap:Header>"#), 1);
        self.post_raw(&body, crate::netio::API_LIMIT).await
    }

    pub async fn create_event(&mut self, cal: &crate::dav::Collection, input: &crate::dav::EventInput) -> Result<()> {
        let fid = cal.url.strip_prefix("ews:cal:").ok_or(AppError::NotFound)?.to_string();
        let (tz, fields) = Self::event_fields(input, true)?;
        let xml: String = fields.iter().map(|(_, x)| x.as_str()).collect();
        let op = format!(r#"<m:CreateItem SendMeetingInvitations="SendToNone"><m:SavedItemFolderId><t:FolderId Id="{}"/></m:SavedItemFolderId><m:Items><t:CalendarItem>{xml}</t:CalendarItem></m:Items></m:CreateItem>"#, xml_escape(&fid));
        let text = self.call_tz(&op, &tz).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(()), Err)
    }

    /// Ссылка на событие в запросе: повторение — через всю серию.
    fn event_ref(flags: &str, id: &str) -> String {
        if flags.contains('r') {
            format!(r#"<t:RecurringMasterItemId OccurrenceId="{}"/>"#, xml_escape(id))
        } else {
            format!(r#"<t:ItemId Id="{}"/>"#, xml_escape(id))
        }
    }

    pub async fn update_event(&mut self, href: &str, input: &crate::dav::EventInput, change_time: bool) -> Result<()> {
        let (flags, id) = parse_href(href, "cal").ok_or(AppError::NotFound)?;
        let (tz, fields) = Self::event_fields(input, change_time)?;
        let updates: String = fields
            .iter()
            .map(|(uri, x)| {
                if (*uri == "calendar:Location" || *uri == "item:Body") && empty_field(x) {
                    format!(r#"<t:DeleteItemField><t:FieldURI FieldURI="{uri}"/></t:DeleteItemField>"#)
                } else {
                    format!(r#"<t:SetItemField><t:FieldURI FieldURI="{uri}"/><t:CalendarItem>{x}</t:CalendarItem></t:SetItemField>"#)
                }
            })
            .collect();
        let op = format!(
            r#"<m:UpdateItem ConflictResolution="AlwaysOverwrite" SendMeetingInvitationsOrCancellations="{}"><m:ItemChanges><t:ItemChange>{}<t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>"#,
            if flags.contains('m') { "SendToChangedAndSaveCopy" } else { "SendToNone" },
            Self::event_ref(flags, id)
        );
        let text = self.call_tz(&op, &tz).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(()), Err)
    }

    pub async fn delete_event(&mut self, href: &str) -> Result<()> {
        let (flags, id) = parse_href(href, "cal").ok_or(AppError::NotFound)?;
        // Своё совещание — участники получают отмену; чужое — просто убирается из календаря.
        let op = format!(
            r#"<m:DeleteItem DeleteType="MoveToDeletedItems" SendMeetingCancellations="{}"><m:ItemIds>{}</m:ItemIds></m:DeleteItem>"#,
            if flags.contains('m') { "SendToAllAndSaveCopy" } else { "SendToNone" },
            Self::event_ref(flags, id)
        );
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(()), Err)
    }

    /// Событие из файла календаря (.ics): то же событие (UID) в календаре заменяется.
    pub async fn import_ics(&mut self, cal: &crate::dav::Collection, ev: &crate::dav::IcsEvent) -> Result<bool> {
        let fid = cal.url.strip_prefix("ews:cal:").ok_or(AppError::NotFound)?.to_string();
        let start = parse_time(&ev.start).or_else(|| chrono::NaiveDate::parse_from_str(&ev.start, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)).map(|t| Utc.from_utc_datetime(&t))).unwrap_or_else(Utc::now);
        // Ошибка поиска не глотается: иначе событие добавилось бы второй раз вместо замены.
        let existing = self.events(cal, start - chrono::Duration::days(2), start + chrono::Duration::days(2)).await?;
        let mut replaced = false;
        if let Some(old) = existing.iter().find(|e| e.uid == ev.uid) {
            self.delete_event(&old.href).await?;
            replaced = true;
        }
        let op = format!(
            r#"<m:CreateItem SendMeetingInvitations="SendToNone"><m:SavedItemFolderId><t:FolderId Id="{}"/></m:SavedItemFolderId><m:Items><t:CalendarItem><t:MimeContent CharacterSet="UTF-8">{}</t:MimeContent></t:CalendarItem></m:Items></m:CreateItem>"#,
            xml_escape(&fid),
            base64::engine::general_purpose::STANDARD.encode(ev.ics.as_bytes())
        );
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(replaced), Err)
    }

    // ------------------------------------------------------------ контакты

    /// Адресные книги: «Контакты» ящика и общая адресная книга организации.
    pub fn books() -> Vec<crate::dav::Collection> {
        vec![
            crate::dav::Collection { url: CONTACTS_URL.into(), name: "Контакты".into(), color: None, writable: true },
            crate::dav::Collection { url: GAL_URL.into(), name: "Общая адресная книга".into(), color: None, writable: false },
        ]
    }

    async fn contact_items(&mut self, ids: &[(String, Option<String>)], with_body: bool) -> Result<Vec<crate::dav::Contact>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(BATCH) {
            let list: String = chunk.iter().map(|(id, _)| format!(r#"<t:ItemId Id="{}"/>"#, xml_escape(id))).collect();
            let body = if with_body { r#"<t:FieldURI FieldURI="item:Body"/>"# } else { "" };
            let op = format!(r#"<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:BodyType>Text</t:BodyType><t:AdditionalProperties>{CONTACT_PROPS}{body}</t:AdditionalProperties></m:ItemShape><m:ItemIds>{list}</m:ItemIds></m:GetItem>"#);
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            for r in responses(&d)? {
                if response_error(r).is_some() {
                    continue;
                }
                let Some(n) = descendants(r, "Contact").next() else { continue };
                let Some(idn) = child(n, "ItemId") else { continue };
                let id = idn.attribute("Id").unwrap_or_default();
                out.push(contact_of(n, id, idn.attribute("ChangeKey").map(str::to_string), text_of(n, "Body")));
            }
        }
        Ok(out)
    }

    /// Контакты папки «Контакты».
    pub async fn contacts(&mut self) -> Result<Vec<crate::dav::Contact>> {
        let mut ids = Vec::new();
        let mut offset = 0usize;
        loop {
            let op = format!(r#"<m:FindItem Traversal="Shallow"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI="item:ItemClass"/></t:AdditionalProperties></m:ItemShape><m:IndexedPageItemView MaxEntriesReturned="500" Offset="{offset}" BasePoint="Beginning"/><m:ParentFolderIds><t:DistinguishedFolderId Id="contacts"/></m:ParentFolderIds></m:FindItem>"#);
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            if let Some(e) = first_error(&responses(&d)?) {
                return Err(e);
            }
            let found: Vec<(String, Option<String>)> = d
                .descendants()
                .filter(|n| n.is_element() && n.tag_name().name() == "Contact")
                .filter_map(|n| child(n, "ItemId").map(|i| (i.attribute("Id").unwrap_or_default().to_string(), i.attribute("ChangeKey").map(str::to_string))))
                .collect();
            let n = d.descendants().filter(|n| n.is_element() && ITEM_TAGS.iter().chain(["Contact", "DistributionList"].iter()).any(|t| *t == n.tag_name().name()) && child(*n, "ItemId").is_some()).count();
            ids.extend(found);
            let last = descendants(d.root(), "RootFolder").next().and_then(|r| r.attribute("IncludesLastItemInRange")).is_none_or(|v| v == "true");
            offset += n;
            if last || n == 0 || offset > 20_000 {
                break;
            }
        }
        self.contact_items(&ids, false).await
    }

    pub async fn contact(&mut self, href: &str) -> Result<crate::dav::Contact> {
        if let Some(email) = href.strip_prefix("ews:gal:") {
            return self.resolve(email).await?.into_iter().find(|c| c.emails.iter().any(|e| e.eq_ignore_ascii_case(email))).ok_or(AppError::NotFound);
        }
        let (_, id) = parse_href(href, "c").ok_or(AppError::NotFound)?;
        self.contact_items(&[(id.to_string(), None)], true).await?.into_iter().next().ok_or(AppError::NotFound)
    }

    /// Поиск в общей адресной книге организации (ResolveNames).
    pub async fn resolve(&mut self, query: &str) -> Result<Vec<crate::dav::Contact>> {
        let q = query.trim();
        if q.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let op = format!(r#"<m:ResolveNames ReturnFullContactData="true" SearchScope="ActiveDirectoryContacts"><m:UnresolvedEntry>{}</m:UnresolvedEntry></m:ResolveNames>"#, xml_escape(q));
        let text = self.call(&op, crate::netio::API_LIMIT).await?;
        let d = doc(&text)?;
        let mut out = Vec::new();
        for r in responses(&d)? {
            // «Неоднозначное имя» — тоже результат (несколько совпадений).
            let code = text_of(r, "ResponseCode").unwrap_or_default();
            if code != "NoError" && code != "ErrorNameResolutionMultipleResults" {
                continue;
            }
            for res in descendants(r, "Resolution") {
                let (name, addr) = child(res, "Mailbox").map(mailbox).unwrap_or_default();
                if addr.is_empty() {
                    continue;
                }
                let c = child(res, "Contact");
                let href = format!("ews:gal:{addr}");
                out.push(crate::dav::Contact {
                    id: href.clone(),
                    book: "Общая адресная книга".into(),
                    href,
                    etag: None,
                    editable: false,
                    full_name: c.and_then(|c| text_of(c, "DisplayName")).filter(|s| !s.is_empty()).unwrap_or(if name.is_empty() { addr.clone() } else { name }),
                    emails: vec![addr],
                    phones: c.map(|c| entries(c, "PhoneNumbers").into_iter().map(|(_, v)| v).collect()).unwrap_or_default(),
                    org: c.and_then(|c| text_of(c, "Department").or_else(|| text_of(c, "CompanyName"))).filter(|s| !s.is_empty()),
                    title: c.and_then(|c| text_of(c, "JobTitle")).filter(|s| !s.is_empty()),
                    note: None,
                    address: c.and_then(address_of),
                });
            }
        }
        Ok(out)
    }

    fn contact_fields(input: &crate::dav::ContactInput) -> Result<Vec<(String, String)>> {
        let name = input.full_name.trim();
        if name.is_empty() {
            return Err(AppError::Other("Укажите имя контакта".into()));
        }
        let emails: Vec<&str> = input.emails.iter().map(|e| e.trim()).filter(|e| !e.is_empty()).collect();
        if let Some(bad) = emails.iter().find(|e| !e.contains('@')) {
            return Err(AppError::Other(format!("Некорректный e-mail: {bad}")));
        }
        let parts: Vec<&str> = name.split_whitespace().collect();
        let mut f: Vec<(String, String)> = vec![
            ("item:Body".into(), format!(r#"<t:Body BodyType="Text">{}</t:Body>"#, xml_escape(input.note.as_deref().unwrap_or_default()))),
            ("contacts:FileAs".into(), format!("<t:FileAs>{}</t:FileAs>", xml_escape(name))),
            ("contacts:DisplayName".into(), format!("<t:DisplayName>{}</t:DisplayName>", xml_escape(name))),
            ("contacts:GivenName".into(), format!("<t:GivenName>{}</t:GivenName>", xml_escape(parts.get(1).copied().unwrap_or_default()))),
            ("contacts:CompanyName".into(), format!("<t:CompanyName>{}</t:CompanyName>", xml_escape(input.org.as_deref().unwrap_or_default().trim()))),
        ];
        for (i, e) in emails.iter().take(3).enumerate() {
            f.push((format!("email:EmailAddress{}", i + 1), xml_escape(e)));
        }
        for (i, p) in input.phones.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).take(PHONE_KEYS.len()).enumerate() {
            f.push((format!("phone:{}", PHONE_KEYS[i]), xml_escape(p)));
        }
        f.push(("contacts:JobTitle".into(), format!("<t:JobTitle>{}</t:JobTitle>", xml_escape(input.title.as_deref().unwrap_or_default().trim()))));
        f.push(("contacts:Surname".into(), format!("<t:Surname>{}</t:Surname>", xml_escape(parts.first().copied().unwrap_or_default()))));
        Ok(f)
    }

    pub async fn create_contact(&mut self, input: &crate::dav::ContactInput) -> Result<()> {
        let f = Self::contact_fields(input)?;
        let simple = |k: &str| f.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).filter(|v| !empty_field(v)).unwrap_or_default();
        let emails: String = f.iter().filter_map(|(k, v)| k.strip_prefix("email:").map(|key| format!(r#"<t:Entry Key="{key}">{v}</t:Entry>"#))).collect();
        let phones: String = f.iter().filter_map(|(k, v)| k.strip_prefix("phone:").map(|key| format!(r#"<t:Entry Key="{key}">{v}</t:Entry>"#))).collect();
        let xml = format!(
            "{}{}{}{}{}{}{}{}{}",
            simple("item:Body"),
            simple("contacts:FileAs"),
            simple("contacts:DisplayName"),
            simple("contacts:GivenName"),
            simple("contacts:CompanyName"),
            if emails.is_empty() { String::new() } else { format!("<t:EmailAddresses>{emails}</t:EmailAddresses>") },
            if phones.is_empty() { String::new() } else { format!("<t:PhoneNumbers>{phones}</t:PhoneNumbers>") },
            simple("contacts:JobTitle"),
            simple("contacts:Surname"),
        );
        let op = format!(r#"<m:CreateItem><m:SavedItemFolderId><t:DistinguishedFolderId Id="contacts"/></m:SavedItemFolderId><m:Items><t:Contact>{xml}</t:Contact></m:Items></m:CreateItem>"#);
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(()), Err)
    }

    pub async fn update_contact(&mut self, href: &str, input: &crate::dav::ContactInput) -> Result<()> {
        let (_, id) = parse_href(href, "c").ok_or(AppError::NotFound)?;
        let f = Self::contact_fields(input)?;
        let mut updates = String::new();
        for (k, v) in &f {
            if k.starts_with("email:") || k.starts_with("phone:") {
                continue;
            }
            if empty_field(v) {
                updates.push_str(&format!(r#"<t:DeleteItemField><t:FieldURI FieldURI="{k}"/></t:DeleteItemField>"#));
            } else {
                updates.push_str(&format!(r#"<t:SetItemField><t:FieldURI FieldURI="{k}"/><t:Contact>{v}</t:Contact></t:SetItemField>"#));
            }
        }
        for i in 1..=3 {
            let key = format!("EmailAddress{i}");
            match f.iter().find(|(k, _)| *k == format!("email:{key}")) {
                Some((_, v)) => updates.push_str(&format!(r#"<t:SetItemField><t:IndexedFieldURI FieldURI="contacts:EmailAddress" FieldIndex="{key}"/><t:Contact><t:EmailAddresses><t:Entry Key="{key}">{v}</t:Entry></t:EmailAddresses></t:Contact></t:SetItemField>"#)),
                None => updates.push_str(&format!(r#"<t:DeleteItemField><t:IndexedFieldURI FieldURI="contacts:EmailAddress" FieldIndex="{key}"/></t:DeleteItemField>"#)),
            }
        }
        for key in PHONE_KEYS {
            match f.iter().find(|(k, _)| *k == format!("phone:{key}")) {
                Some((_, v)) => updates.push_str(&format!(r#"<t:SetItemField><t:IndexedFieldURI FieldURI="contacts:PhoneNumber" FieldIndex="{key}"/><t:Contact><t:PhoneNumbers><t:Entry Key="{key}">{v}</t:Entry></t:PhoneNumbers></t:Contact></t:SetItemField>"#)),
                None => updates.push_str(&format!(r#"<t:DeleteItemField><t:IndexedFieldURI FieldURI="contacts:PhoneNumber" FieldIndex="{key}"/></t:DeleteItemField>"#)),
            }
        }
        let op = format!(r#"<m:UpdateItem ConflictResolution="AlwaysOverwrite"><m:ItemChanges><t:ItemChange><t:ItemId Id="{}"/><t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>"#, xml_escape(id));
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        // Удаление отсутствующего поля — не ошибка.
        responses(&d)?.into_iter().find_map(|r| response_error(r).filter(|_| text_of(r, "ResponseCode").as_deref() != Some("ErrorInvalidPropertyDelete"))).map_or(Ok(()), Err)
    }

    pub async fn delete_contact(&mut self, href: &str) -> Result<()> {
        let (_, id) = parse_href(href, "c").ok_or(AppError::NotFound)?;
        let op = format!(r#"<m:DeleteItem DeleteType="MoveToDeletedItems"><m:ItemIds><t:ItemId Id="{}"/></m:ItemIds></m:DeleteItem>"#, xml_escape(id));
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(()), Err)
    }
}

/// Запрос AQS из разобранного поиска.
fn aqs_of(q: &crate::search::Query) -> String {
    let quote = |w: &str| format!("\"{}\"", w.replace('"', ""));
    let mut parts: Vec<String> = Vec::new();
    for w in &q.words {
        parts.push(quote(w));
    }
    for w in &q.from {
        parts.push(format!("from:{}", quote(w)));
    }
    for w in &q.to {
        parts.push(format!("(to:{0} OR cc:{0})", quote(w)));
    }
    for w in &q.subject {
        parts.push(format!("subject:{}", quote(w)));
    }
    for w in &q.attachment {
        parts.push(format!("attachment:{}", quote(w)));
    }
    if let Some(d) = &q.domain {
        parts.push(format!("(from:{0} OR to:{0})", quote(d)));
    }
    if q.has_attachment {
        parts.push("hasattachment:true".into());
    }
    if q.unread {
        parts.push("isread:false".into());
    }
    if q.flagged {
        parts.push("isflagged:true".into());
    }
    if q.important {
        parts.push("importance:high".into());
    }
    if let Some(a) = q.after {
        parts.push(format!("received>={}", a.format("%Y-%m-%d")));
    }
    if let Some(b) = q.before {
        parts.push(format!("received<{}", b.format("%Y-%m-%d")));
    }
    parts.join(" AND ")
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

fn ews_url_of(text: &str) -> Option<String> {
    let d = roxmltree::Document::parse(text).ok()?;
    let protocols: Vec<_> = d.descendants().filter(|n| n.tag_name().name() == "Protocol").collect();
    let pick = |ty: &str| protocols.iter().find(|p| text_of(**p, "Type").as_deref() == Some(ty)).and_then(|p| text_of(*p, "EwsUrl")).filter(|u| u.to_ascii_lowercase().starts_with("https://"));
    pick("EXCH").or_else(|| pick("EXPR"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternative_mailboxes_from_autodiscover() {
        let ad = r#"<?xml version="1.0"?><Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/responseschema/2006"><Response xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a"><Account><Protocol><Type>EXCH</Type><EwsUrl>https://mail.corp.ru/EWS/Exchange.asmx</EwsUrl></Protocol><AlternativeMailbox><Type>Archive</Type><DisplayName>Архив</DisplayName><SmtpAddress>archive@corp.ru</SmtpAddress></AlternativeMailbox><AlternativeMailbox><Type>Delegate</Type><DisplayName>Канцелярия</DisplayName><LegacyDN>/o=corp/ou=x</LegacyDN><Server>mbx1</Server><SmtpAddress>Office@Corp.ru</SmtpAddress></AlternativeMailbox><AlternativeMailbox><Type>Delegate</Type><DisplayName>Я</DisplayName><SmtpAddress>me@corp.ru</SmtpAddress></AlternativeMailbox><AlternativeMailbox><Type>TeamMailbox</Type><OwnerSmtpAddress>team@corp.ru</OwnerSmtpAddress></AlternativeMailbox><AlternativeMailbox><Type>Delegate</Type><DisplayName>Дубль</DisplayName><SmtpAddress>office@corp.ru</SmtpAddress></AlternativeMailbox></Account></Response></Autodiscover>"#;
        assert_eq!(
            alternative_mailboxes(ad, "me@corp.ru"),
            vec![("office@corp.ru".to_string(), "Канцелярия".to_string()), ("team@corp.ru".to_string(), "team@corp.ru".to_string())]
        );
        assert!(alternative_mailboxes("not xml", "me@corp.ru").is_empty());
    }

    #[test]
    fn mailbox_addresses_are_checked() {
        assert!(valid_mailbox("office@corp.ru"));
        for bad in ["office", "@corp.ru", "a@corp", "a b@corp.ru", "a@corp.ru>", "a@.ru", "a@corp.ru/x", "a@co\nrp.ru"] {
            assert!(!valid_mailbox(bad), "{bad}");
        }
    }

    #[test]
    fn connected_mailbox_folders_go_under_shared() {
        let raw: Vec<RawFolder> = vec![
            ("I".into(), "R".into(), "Входящие".into(), "IPF.Note".into(), 3, 1, 1),
            ("P".into(), "I".into(), "Заявки".into(), "IPF.Note".into(), 2, 0, 0),
            ("T".into(), "R".into(), "Удаленные".into(), "IPF.Note".into(), 0, 0, 0),
            ("X".into(), "Q".into(), "Чужая".into(), "IPF.Note".into(), 0, 0, 0),
        ];
        let roles = HashMap::from([("I".to_string(), FolderRole::Inbox), ("T".to_string(), FolderRole::Trash)]);
        let out = build_folders(&raw, &roles, "R", Some("shared/office@corp.ru"));
        let paths: Vec<_> = out.iter().map(|(f, _)| (f.path.as_str(), f.depth, f.system, f.parent.as_deref())).collect();
        assert_eq!(
            paths,
            vec![
                ("shared/office@corp.ru/INBOX", 2, true, Some("shared/office@corp.ru")),
                ("shared/office@corp.ru/INBOX/Заявки", 3, false, Some("shared/office@corp.ru/INBOX")),
                ("shared/office@corp.ru/Удаленные", 2, true, Some("shared/office@corp.ru")),
            ]
        );
        assert_eq!(out[1].1.id, "P");
        let own = build_folders(&raw, &roles, "R", None);
        assert_eq!(own[0].0.path, "INBOX");
        assert!(own[0].0.system && own[2].0.system && !own[1].0.system);
    }

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
    fn uids_follow_arrival_time_and_stay_unique() {
        let mut f = FolderMap::default();
        let t = Utc.with_ymd_and_hms(2026, 11, 3, 10, 0, 0).unwrap();
        let a = f.uid_for("A", Some(t));
        let b = f.uid_for("B", Some(t));
        let c = f.uid_for("C", Some(t + chrono::Duration::seconds(60)));
        assert_eq!(a, (t.timestamp() - EPOCH_2000) as u32);
        assert_eq!(b, a + 1);
        assert!(c > b);
        assert_eq!(f.uid_for("A", Some(t)), a);
        f.forget(a);
        assert_eq!(f.uid_for("D", Some(t)), a);
        let json = serde_json::to_vec(&f).unwrap();
        let mut back: FolderMap = serde_json::from_slice(&json).unwrap();
        back.rebuild();
        assert_eq!(back.uid_for("B", None), b);
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

    #[test]
    fn parses_items_and_flags() {
        let xml = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><m:GetItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages" xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types"><m:ResponseMessages><m:GetItemResponseMessage ResponseClass="Success"><m:ResponseCode>NoError</m:ResponseCode><m:Items><t:Message><t:ItemId Id="AAA=" ChangeKey="x"/><t:Subject>Отчёт</t:Subject><t:DateTimeReceived>2026-11-03T10:00:00Z</t:DateTimeReceived><t:Size>2048</t:Size><t:Categories><t:String>Работа</t:String><t:String>$label1</t:String></t:Categories><t:Importance>Normal</t:Importance><t:InReplyTo>&lt;a@b&gt;</t:InReplyTo><t:HasAttachments>true</t:HasAttachments><t:Attachments><t:FileAttachment><t:Name>смета.xlsx</t:Name><t:IsInline>false</t:IsInline></t:FileAttachment><t:FileAttachment><t:Name>logo.png</t:Name><t:IsInline>true</t:IsInline></t:FileAttachment></t:Attachments><t:ExtendedProperty><t:ExtendedFieldURI PropertyTag="0x1081" PropertyType="Integer"/><t:Value>102</t:Value></t:ExtendedProperty><t:Flag><t:FlagStatus>Flagged</t:FlagStatus></t:Flag><t:From><t:Mailbox><t:Name>Петров</t:Name><t:EmailAddress>petrov@corp.ru</t:EmailAddress></t:Mailbox></t:From><t:ToRecipients><t:Mailbox><t:Name>Иванов</t:Name><t:EmailAddress>ivanov@corp.ru</t:EmailAddress></t:Mailbox><t:Mailbox><t:Name>Внутренний</t:Name><t:EmailAddress>/O=CORP/OU=X</t:EmailAddress></t:Mailbox></t:ToRecipients><t:IsRead>false</t:IsRead><t:InternetMessageId>&lt;m1@corp.ru&gt;</t:InternetMessageId><t:References>&lt;r0@x&gt; &lt;a@b&gt;</t:References></t:Message></m:Items></m:GetItemResponseMessage></m:ResponseMessages></m:GetItemResponse></s:Body></s:Envelope>"#;
        let d = doc(xml).unwrap();
        let rs = responses(&d).unwrap();
        assert_eq!(rs.len(), 1);
        assert!(first_error(&rs).is_none());
        let n = items_of(rs[0]).into_iter().next().unwrap();
        assert_eq!(item_id(n).as_deref(), Some("AAA="));
        let s = EwsSession { conn: None, url: normalize_url("x.ru").unwrap(), login: String::new(), password: String::new(), key: String::new(), auth: Auth::Unknown, authed: false, version: "", folders: vec![], root: None, map: Arc::new(Mutex::new(AccountMap::default())) };
        let m = s.summary("INBOX", 7, n);
        assert_eq!((m.subject.as_str(), m.from_name.as_str(), m.from_address.as_str()), ("Отчёт", "Петров", "petrov@corp.ru"));
        assert_eq!(m.to, ["Иванов <ivanov@corp.ru>", "Внутренний"]);
        assert!(m.unread && m.flagged && m.important && m.answered);
        assert_eq!(m.keywords, ["Работа"]);
        assert_eq!(m.attachments, ["смета.xlsx"]);
        assert_eq!(m.message_id.as_deref(), Some("<m1@corp.ru>"));
        assert_eq!(m.thread, ["<r0@x>", "<a@b>"]);
        assert_eq!(m.size, 2048);
        assert_eq!(m.date.as_deref(), Some("2026-11-03T10:00:00+00:00"));
        let err = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><m:GetItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"><m:ResponseMessages><m:GetItemResponseMessage ResponseClass="Error"><m:MessageText>нет</m:MessageText><m:ResponseCode>ErrorItemNotFound</m:ResponseCode></m:GetItemResponseMessage></m:ResponseMessages></m:GetItemResponse></s:Body></s:Envelope>"#;
        let d = doc(err).unwrap();
        assert!(matches!(first_error(&responses(&d).unwrap()), Some(AppError::NotFound)));
    }

    #[test]
    fn aqs() {
        let q = crate::search::Query::parse("от:петров тема:\"план работ\" есть:вложение после:2026-10-01");
        assert_eq!(aqs_of(&q), "from:\"петров\" AND subject:\"план работ\" AND hasattachment:true AND received>=2026-10-01");
    }
}
