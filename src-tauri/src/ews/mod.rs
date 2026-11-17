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
//!
//! Структура модуля:
//! - здесь (`ews.rs`): общие типы, константы, соответствие UID ↔ ItemId,
//!   общие XML-хелперы;
//! - [`auth`] — TLS, NTLM, вход, Autodiscover, диагностика;
//! - [`folders`] — дерево папок и подключённые ящики;
//! - [`items`] — письма (список, чтение, флаги, отправка, поиск);
//! - [`cal`] — календарь и контакты;
//! - `mailboxes` (в `ews_mailboxes.rs`) — подключённые ящики Exchange.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{AppError, Result};
use crate::imap::FolderRole;

#[path = "../ews_mailboxes.rs"]
mod mailboxes;
pub use mailboxes::*;

mod auth;
mod folders;
mod items;
mod cal;

pub use auth::*;
pub use cal::*;
#[allow(unused_imports)]
use folders::*;
#[allow(unused_imports)]
use items::*;

const NS_T: &str = "http://schemas.microsoft.com/exchange/services/2006/types";
const NS_M: &str = "http://schemas.microsoft.com/exchange/services/2006/messages";
/// Большие ответы (исходники писем в base64).
const MIME_LIMIT: usize = 96 * 1024 * 1024;
/// Сколько элементов запрашивать за раз в GetItem.
const BATCH: usize = 50;
/// Начало отсчёта номеров писем.
const EPOCH_2000: i64 = 946_684_800;
/// Простаивающее соединение IIS закрывает (по умолчанию через 2 минуты) — раньше этого
/// открываем новое сами, а не узнаём об обрыве посреди запроса.
const IDLE_REOPEN: Duration = Duration::from_secs(50);

// ---------------------------------------------------------------- соответствие UID ↔ ItemId

#[derive(Serialize, Deserialize, Default)]
pub(super) struct FolderMap {
    /// UID → ItemId (Id без ChangeKey).
    pub(super) items: HashMap<u32, String>,
    #[serde(skip)]
    pub(super) rev: HashMap<String, u32>,
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
pub(super) struct AccountMap {
    /// По FolderId Exchange (не меняется при переименовании папки).
    pub(super) folders: HashMap<String, FolderMap>,
    #[serde(skip)]
    pub(super) dirty: bool,
    #[serde(skip)]
    pub(super) saved: Option<std::time::Instant>,
}

pub(super) type Shared = Arc<Mutex<AccountMap>>;

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

pub(super) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub(super) fn account_map(key: &str) -> Shared {
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

pub(super) fn save_map(key: &str, map: &Shared, force: bool) {
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

// ---------------------------------------------------------------- типы сеанса

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Auth {
    Unknown,
    Basic,
    /// Схема заголовка: `NTLM` или `Negotiate` (с токеном NTLM).
    Ntlm(&'static str),
}

/// Папка Exchange: путь (как у IMAP), FolderId, роль.
#[derive(Clone, Debug)]
pub(super) struct EwsFolder {
    pub(super) path: String,
    pub(super) id: String,
    pub(super) role: FolderRole,
}

pub struct EwsSession {
    /// Своё соединение (не пул): вход NTLM действует на сокете, где он прошёл.
    pub(super) conn: Option<Pinned>,
    pub(super) url: Url,
    pub(super) login: String,
    pub(super) password: String,
    pub(super) key: String,
    pub(super) auth: Auth,
    /// Соединение уже прошло NTLM (IIS помнит вход на соединении).
    pub(super) authed: bool,
    pub(super) version: &'static str,
    pub(super) folders: Vec<EwsFolder>,
    pub(super) root: Option<String>,
    pub(super) map: Shared,
}

// ---------------------------------------------------------------- соединение (тип)

/// Одно соединение HTTP/1.1 поверх TLS. Вход NTLM (как и в Outlook) привязан к TCP-
/// соединению: IIS помнит его на сокете. Пул соединений общего HTTP-клиента мог
/// отправить третий шаг входа по другому соединению — тогда вход «не проходил»
/// при верном пароле. Здесь все запросы сеанса идут по одному соединению.
pub(super) struct Pinned {
    pub(super) sender: hyper::client::conn::http1::SendRequest<http_body_util::Full<bytes::Bytes>>,
    /// Сертификат сервера (DER) — для привязки входа к TLS-каналу.
    pub(super) cert: Option<Vec<u8>>,
    pub(super) used: std::time::Instant,
}

/// Ответ сервера целиком (с ограничением размера).
pub(super) struct Resp {
    pub(super) status: http::StatusCode,
    pub(super) headers: http::HeaderMap,
    pub(super) body: Vec<u8>,
}

impl Resp {
    pub(super) fn text(self) -> String {
        match String::from_utf8(self.body) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        }
    }
}

// ---------------------------------------------------------------- разбор XML (общее)

pub(super) fn doc(text: &str) -> Result<roxmltree::Document<'_>> {
    roxmltree::Document::parse(text).map_err(|e| AppError::Other(format!("Ответ Exchange не разобран: {e}")))
}

pub(super) fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

pub(super) fn text_of(n: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    child(n, name).and_then(|c| c.text()).map(|s| s.trim().to_string())
}

pub(super) fn descendants<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = roxmltree::Node<'a, 'i>> + 'a {
    n.descendants().filter(move |c| c.is_element() && c.tag_name().name() == name)
}

/// Ответы операции (`*ResponseMessage`) и ошибка SOAP целиком.
pub(super) fn responses<'a, 'i>(d: &'a roxmltree::Document<'i>) -> Result<Vec<roxmltree::Node<'a, 'i>>> {
    if let Some(f) = d.descendants().find(|n| n.tag_name().name() == "Fault") {
        let msg = descendants(f, "faultstring").next().and_then(|n| n.text()).unwrap_or("ошибка SOAP");
        return Err(AppError::Other(format!("Exchange: {msg}")));
    }
    Ok(d.descendants().filter(|n| n.is_element() && n.tag_name().name().ends_with("ResponseMessage") && n.attribute("ResponseClass").is_some()).collect())
}

pub(super) fn response_error(r: roxmltree::Node<'_, '_>) -> Option<AppError> {
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
pub(super) fn first_error(rs: &[roxmltree::Node<'_, '_>]) -> Option<AppError> {
    rs.iter().find_map(|r| response_error(*r))
}

pub(super) fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim()).ok().map(|d| d.with_timezone(&Utc))
}

pub(super) fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub(super) fn envelope(version: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:t="{NS_T}" xmlns:m="{NS_M}"><soap:Header><t:RequestServerVersion Version="{version}"/></soap:Header><soap:Body>{body}</soap:Body></soap:Envelope>"#
    )
}

/// Полный текст ошибки (с причинами): главное часто спрятано в `source()`.
pub(super) fn error_chain(e: &dyn std::error::Error) -> String {
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
}
