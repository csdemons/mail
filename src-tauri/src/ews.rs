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
use reqwest::StatusCode;
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
    http: reqwest::Client,
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

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(1)
        .http1_only()
        .tls_info(true)
        .user_agent(concat!("AtomMail/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AppError::Other(format!("HTTP-клиент: {e}")))
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

fn www_auth(resp: &reqwest::Response) -> Vec<String> {
    resp.headers().get_all("www-authenticate").iter().filter_map(|v| v.to_str().ok().map(str::to_string)).collect()
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
    async fn http_send(&self, body: &str, auth: Option<String>) -> Result<reqwest::Response> {
        let mut req = self.http.post(self.url.clone()).header("Content-Type", "text/xml; charset=utf-8").body(body.to_string());
        if let Some(a) = auth {
            req = req.header("Authorization", a);
        }
        req.send().await.map_err(|e| {
            if e.is_timeout() {
                AppError::Timeout
            } else if e.is_connect() {
                AppError::Unreachable(self.url.host_str().unwrap_or_default().to_string(), e.to_string())
            } else {
                AppError::ConnectionLost
            }
        })
    }

    /// Выполнить запрос SOAP, при необходимости войти (NTLM или Basic).
    async fn post_raw(&mut self, body: &str, limit: usize) -> Result<String> {
        for _ in 0..3 {
            let resp = match self.auth {
                Auth::Basic => {
                    let token = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", self.login, self.password));
                    self.http_send(body, Some(format!("Basic {token}"))).await?
                }
                Auth::Ntlm(scheme) if !self.authed => self.ntlm(body, scheme).await?,
                _ => self.http_send(body, None).await?,
            };
            match resp.status() {
                StatusCode::UNAUTHORIZED => {
                    let offered = www_auth(&resp);
                    let _ = crate::netio::read_limited(resp, crate::netio::SMALL_LIMIT).await;
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
                        _ => return Err(AppError::AuthFailed),
                    }
                }
                StatusCode::OK | StatusCode::INTERNAL_SERVER_ERROR => {
                    if matches!(self.auth, Auth::Ntlm(_)) {
                        self.authed = true;
                    }
                    let ok = resp.status() == StatusCode::OK;
                    let text = crate::netio::text_limited(resp, limit).await?;
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
    async fn ntlm(&mut self, body: &str, scheme: &'static str) -> Result<reqwest::Response> {
        let neg = crate::ntlm::negotiate();
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        // Первый шаг — без тела: большое письмо не передаётся дважды.
        let first = self
            .http
            .post(self.url.clone())
            .header("Content-Type", "text/xml; charset=utf-8")
            .header("Authorization", format!("{scheme} {}", b64(&neg)))
            .body("")
            .send()
            .await
            .map_err(|e| if e.is_timeout() { AppError::Timeout } else if e.is_connect() { AppError::Unreachable(self.url.host_str().unwrap_or_default().to_string(), e.to_string()) } else { AppError::ConnectionLost })?;
        let cert = first.extensions().get::<reqwest::tls::TlsInfo>().and_then(|i| i.peer_certificate()).map(<[u8]>::to_vec);
        let headers = www_auth(&first);
        let _ = crate::netio::read_limited(first, crate::netio::SMALL_LIMIT).await;
        let challenge = ntlm_token(&headers, scheme).and_then(|t| crate::ntlm::parse_challenge(&t)).ok_or(AppError::AuthFailed)?;
        let (user, domain) = crate::ntlm::split_login(&self.login);
        let mut cc = [0u8; 8];
        getrandom::getrandom(&mut cc).map_err(|e| AppError::Other(e.to_string()))?;
        let spn = format!("HTTP/{}", self.url.host_str().unwrap_or_default());
        let auth = crate::ntlm::authenticate(&challenge, &neg, &user, &domain, &self.password, "", cert.as_deref(), Some(&spn), cc, crate::ntlm::filetime_now());
        self.http_send(body, Some(format!("{scheme} {}", b64(&auth)))).await
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
        http: http_client()?,
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

impl EwsSession {
    /// Список почтовых папок (дерево, как у IMAP).
    pub async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        let ids: String = DISTINGUISHED.iter().map(|(id, _)| format!(r#"<t:DistinguishedFolderId Id="{id}"/>"#)).collect();
        let text = self.call(&format!(r#"<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds>{ids}</m:FolderIds></m:GetFolder>"#), crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        let mut roles: HashMap<String, FolderRole> = HashMap::new();
        for (r, (_, role)) in responses(&d)?.into_iter().zip(DISTINGUISHED.iter()) {
            if response_error(r).is_none() {
                if let Some(id) = descendants(r, "FolderId").next().and_then(|f| f.attribute("Id")) {
                    roles.insert(id.to_string(), *role);
                }
            }
        }
        let mut raw: Vec<(String, String, String, String, u32, u32, u32)> = Vec::new();
        let mut offset = 0;
        loop {
            let op = format!(
                r#"<m:FindFolder Traversal="Deep"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI="folder:DisplayName"/><t:FieldURI FieldURI="folder:ParentFolderId"/><t:FieldURI FieldURI="folder:FolderClass"/><t:FieldURI FieldURI="folder:TotalCount"/><t:FieldURI FieldURI="folder:UnreadCount"/><t:FieldURI FieldURI="folder:ChildFolderCount"/><t:ExtendedFieldURI PropertyTag="0x10F4" PropertyType="Boolean"/></t:AdditionalProperties></m:FolderShape><m:IndexedPageFolderView MaxEntriesReturned="500" Offset="{offset}" BasePoint="Beginning"/><m:ParentFolderIds><t:DistinguishedFolderId Id="msgfolderroot"/></m:ParentFolderIds></m:FindFolder>"#
            );
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            let rs = responses(&d)?;
            if let Some(e) = first_error(&rs) {
                return Err(e);
            }
            let folders: Vec<_> = d.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Folder" && child(*n, "FolderId").is_some()).collect();
            for f in &folders {
                let hidden = descendants(*f, "ExtendedProperty").any(|e| text_of(e, "Value").as_deref() == Some("true"));
                let class = text_of(*f, "FolderClass").unwrap_or_default();
                if hidden || !(class.is_empty() || class == "IPF.Note" || class.starts_with("IPF.Note.")) {
                    continue;
                }
                let id = child(*f, "FolderId").and_then(|x| x.attribute("Id")).unwrap_or_default().to_string();
                let parent = child(*f, "ParentFolderId").and_then(|x| x.attribute("Id")).unwrap_or_default().to_string();
                let num = |name: &str| text_of(*f, name).and_then(|v| v.parse().ok()).unwrap_or(0);
                raw.push((id, parent, text_of(*f, "DisplayName").unwrap_or_default(), class, num("TotalCount"), num("UnreadCount"), num("ChildFolderCount")));
            }
            let last = descendants(d.root(), "RootFolder").next().and_then(|r| r.attribute("IncludesLastItemInRange")).is_none_or(|v| v == "true");
            offset += folders.len();
            if last || folders.is_empty() || offset > 20_000 {
                break;
            }
        }
        // Пути: от корня ящика; «Входящие» — INBOX.
        let root = self.root.clone().unwrap_or_default();
        let by_id: HashMap<&str, usize> = raw.iter().enumerate().map(|(i, r)| (r.0.as_str(), i)).collect();
        let path_of = |i: usize| -> Option<String> {
            let mut parts = Vec::new();
            let mut cur = i;
            for _ in 0..32 {
                let r = &raw[cur];
                parts.push(if roles.get(&r.0) == Some(&FolderRole::Inbox) { "INBOX".to_string() } else { segment(&r.2) });
                if r.1 == root {
                    parts.reverse();
                    return Some(parts.join("/"));
                }
                cur = *by_id.get(r.1.as_str())?;
            }
            None
        };
        let mut out = Vec::new();
        let mut cache = Vec::new();
        for (i, r) in raw.iter().enumerate() {
            let Some(path) = path_of(i) else { continue };
            let role = roles.get(&r.0).copied().unwrap_or(FolderRole::Other);
            let id = utf7_imap::encode_utf7_imap(path.clone());
            let depth = path.matches('/').count();
            let parent = path.rsplit_once('/').map(|(p, _)| utf7_imap::encode_utf7_imap(p.to_string()));
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            out.push(Folder {
                id: id.clone(),
                name: if role == FolderRole::Inbox { "INBOX".into() } else { name },
                path: path.clone(),
                parent,
                depth,
                role,
                system: role != FolderRole::Other && depth == 0,
                selectable: true,
                has_children: r.6 > 0,
                total: r.4,
                unseen: r.5,
            });
            cache.push(EwsFolder { path: id, id: r.0.clone(), role });
        }
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
        let top_role: HashMap<String, FolderRole> = out.iter().filter(|f| f.depth == 0).map(|f| (f.path.clone(), f.role)).collect();
        let key = |f: &Folder| {
            let top = f.path.split('/').next().unwrap_or_default().to_string();
            (rank(top_role.get(&top).copied().unwrap_or(FolderRole::Other)), f.path.to_lowercase())
        };
        out.sort_by_key(key);
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
        Ok(self.folders.iter().find(|f| f.role == role).map(|f| f.path.clone()))
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
    pub async fn send(&mut self, raw: &[u8]) -> Result<()> {
        let op = format!(
            r#"<m:CreateItem MessageDisposition="SendAndSaveCopy"><m:SavedItemFolderId><t:DistinguishedFolderId Id="sentitems"/></m:SavedItemFolderId><m:Items><t:Message><t:MimeContent CharacterSet="UTF-8">{}</t:MimeContent></t:Message></m:Items></m:CreateItem>"#,
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
        first_error(&responses(&d)?).map_or(Ok(()), Err)
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
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006"><Request><EMailAddress>{}</EMailAddress><AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema></Request></Autodiscover>"#,
        xml_escape(email)
    );
    #[allow(unused_mut)]
    let mut candidates = vec![format!("https://autodiscover.{domain}/autodiscover/autodiscover.xml"), format!("https://{domain}/autodiscover/autodiscover.xml")];
    // E2E (отладочная сборка): тестовый сервер вместо адресов в домене почты.
    #[cfg(debug_assertions)]
    if let Ok(u) = std::env::var("RRCKI_E2E_AUTODISCOVER_EWS") {
        candidates = vec![u];
    }
    let mut last = AppError::Other("Сервер Exchange не найден автоматически — укажите его адрес".into());
    for c in candidates {
        let Ok(url) = Url::parse(&c) else { continue };
        let mut s = EwsSession {
            http: http_client()?,
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
        };
        match tokio::time::timeout(Duration::from_secs(25), s.post_raw(&body, crate::netio::SMALL_LIMIT)).await {
            Ok(Ok(text)) => {
                if let Some(url) = ews_url_of(&text) {
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
        let s = EwsSession { http: http_client().unwrap(), url: normalize_url("x.ru").unwrap(), login: String::new(), password: String::new(), key: String::new(), auth: Auth::Unknown, authed: false, version: "", folders: vec![], root: None, map: Arc::new(Mutex::new(AccountMap::default())) };
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
