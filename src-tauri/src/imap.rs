//! IMAP: подключение, папки, список писем, чтение, флаги, вложения.

use std::time::Duration;

use async_imap::imap_proto::types::NameAttribute;
use async_imap::types::{Fetch, Flag};
use futures::TryStreamExt;
use mail_parser::{MessageParser, MimeHeaders};
use serde::Serialize;
use tokio::net::TcpStream;
use tokio_native_tls::TlsStream;

use crate::error::{AppError, Result};

pub type ImapSession = async_imap::Session<TlsStream<TcpStream>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Максимальный размер письма, которое загружаем целиком.
pub const MAX_MESSAGE_MB: u32 = 50;

#[derive(Clone)]
pub struct Credentials {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

pub struct Connection {
    session: ImapSession,
    selected: Option<String>,
}

/// TLS-подключение (IMAPS) и вход. Сертификат проверяется через системное
/// хранилище: SChannel в Windows, OpenSSL + системные CA в Linux, поэтому
/// корпоративные корневые сертификаты подхватываются автоматически.
pub async fn connect(creds: &Credentials) -> Result<Connection> {
    let tcp = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((creds.host.as_str(), creds.port)),
    )
    .await
    .map_err(|_| AppError::Timeout)?
    .map_err(|e| AppError::Connect(format!("{}:{}", creds.host, creds.port), e.to_string()))?;

    let connector = native_tls::TlsConnector::builder()
        .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
        .build()
        .map_err(|e| AppError::Tls(e.to_string()))?;
    let connector = tokio_native_tls::TlsConnector::from(connector);

    let tls = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(&creds.host, tcp))
        .await
        .map_err(|_| AppError::Timeout)?
        .map_err(|e| AppError::Tls(e.to_string()))?;

    let mut client = async_imap::Client::new(tls);
    // Приветствие сервера (* OK ...).
    tokio::time::timeout(CONNECT_TIMEOUT, client.read_response())
        .await
        .map_err(|_| AppError::Timeout)?
        .map_err(|_| AppError::ConnectionLost)?
        .ok_or(AppError::ConnectionLost)?;

    let session = tokio::time::timeout(
        CONNECT_TIMEOUT,
        client.login(&creds.username, &creds.password),
    )
    .await
    .map_err(|_| AppError::Timeout)?
    .map_err(|(err, _client)| match err {
        async_imap::error::Error::No(_) => AppError::AuthFailed,
        other => other.into(),
    })?;

    Ok(Connection {
        session,
        selected: None,
    })
}

impl Connection {
    /// Список возможностей сервера (CAPABILITY) — для диагностики.
    pub async fn capabilities(&mut self) -> Result<Vec<String>> {
        let caps = self.session.capabilities().await?;
        let mut list: Vec<String> = caps.iter().map(|c| format!("{c:?}")).collect();
        list.sort();
        Ok(list)
    }

    pub async fn logout(mut self) {
        let _ = self.session.logout().await;
    }

    async fn ensure_selected(&mut self, folder: &str) -> Result<()> {
        if self.selected.as_deref() != Some(folder) {
            self.select(folder).await?;
        }
        Ok(())
    }

    async fn select(&mut self, folder: &str) -> Result<async_imap::types::Mailbox> {
        self.selected = None;
        let mailbox = self.session.select(folder).await.map_err(|e| match e {
            async_imap::error::Error::No(_) => AppError::NotFound,
            other => other.into(),
        })?;
        self.selected = Some(folder.to_string());
        Ok(mailbox)
    }
}

// ---------------------------------------------------------------- папки

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderRole {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    Other,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    /// Имя на сервере (modified UTF-7) — его передаём обратно в команды.
    pub id: String,
    /// Отображаемое имя последнего уровня.
    pub name: String,
    /// Полный путь для отображения.
    pub path: String,
    pub depth: usize,
    pub role: FolderRole,
    /// `false` — только узел дерева (\\Noselect), открыть нельзя.
    pub selectable: bool,
    pub total: u32,
    pub unseen: u32,
}

fn role_by_name(path: &str) -> FolderRole {
    let lower = path.to_lowercase();
    let leaf = lower.rsplit(['/', '.']).next().unwrap_or(&lower);
    match leaf {
        "inbox" | "входящие" => FolderRole::Inbox,
        "drafts" | "draft" | "черновики" => FolderRole::Drafts,
        "sent" | "sent items" | "sent messages" | "отправленные" => FolderRole::Sent,
        "archive" | "archives" | "архив" => FolderRole::Archive,
        "junk" | "spam" | "junk e-mail" | "junk email" | "спам" | "нежелательная почта" => {
            FolderRole::Junk
        }
        "trash" | "deleted items" | "deleted messages" | "корзина" | "удаленные" | "удалённые" => {
            FolderRole::Trash
        }
        _ => FolderRole::Other,
    }
}

pub async fn list_folders(conn: &mut Connection) -> Result<Vec<Folder>> {
    struct Raw {
        id: String,
        delimiter: Option<String>,
        role: Option<FolderRole>,
        selectable: bool,
    }

    let names: Vec<_> = conn
        .session
        .list(Some(""), Some("*"))
        .await?
        .try_collect()
        .await?;

    let mut raw = Vec::new();
    for name in &names {
        let mut role = None;
        let mut selectable = true;
        for attr in name.attributes() {
            match attr {
                NameAttribute::NoSelect => selectable = false,
                NameAttribute::Sent => role = Some(FolderRole::Sent),
                NameAttribute::Drafts => role = Some(FolderRole::Drafts),
                NameAttribute::Junk => role = Some(FolderRole::Junk),
                NameAttribute::Trash => role = Some(FolderRole::Trash),
                NameAttribute::Archive => role = Some(FolderRole::Archive),
                NameAttribute::Extension(ext) if ext.eq_ignore_ascii_case("\\NonExistent") => {
                    selectable = false
                }
                _ => {}
            }
        }
        raw.push(Raw {
            id: name.name().to_string(),
            delimiter: name.delimiter().map(str::to_string),
            role,
            selectable,
        });
    }
    drop(names);

    let mut folders = Vec::with_capacity(raw.len());
    for item in raw {
        let path = utf7_imap::decode_utf7_imap(item.id.clone());
        let (name, depth) = match item.delimiter.as_deref() {
            Some(d) if !d.is_empty() => {
                let parts: Vec<&str> = path.split(d).collect();
                (parts.last().unwrap_or(&"").to_string(), parts.len() - 1)
            }
            _ => (path.clone(), 0),
        };
        let role = if item.id.eq_ignore_ascii_case("INBOX") {
            FolderRole::Inbox
        } else {
            item.role.unwrap_or_else(|| role_by_name(&path))
        };
        // STATUS не должен ронять весь список, если одна папка недоступна.
        let (total, unseen) = if !item.selectable {
            (0, 0)
        } else {
            match conn.session.status(&item.id, "(MESSAGES UNSEEN)").await {
                Ok(mb) => (mb.exists, mb.unseen.unwrap_or(0)),
                Err(e) => {
                    let e: AppError = e.into();
                    if e.is_connection() {
                        return Err(e);
                    }
                    (0, 0)
                }
            }
        };
        folders.push(Folder {
            id: item.id,
            name,
            path,
            depth,
            role,
            selectable: item.selectable,
            total,
            unseen,
        });
    }

    // Служебные папки сверху в привычном порядке, остальные — по алфавиту.
    folders.sort_by(|a, b| {
        a.role
            .cmp(&b.role)
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
    });
    Ok(folders)
}

// ---------------------------------------------------------------- письма

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSummary {
    pub folder: String,
    pub uid: u32,
    pub from_name: String,
    pub from_address: String,
    pub to: Vec<String>,
    pub subject: String,
    /// RFC 3339; время получения сервером, иначе заголовок Date.
    pub date: Option<String>,
    pub unread: bool,
    pub flagged: bool,
    pub answered: bool,
    pub has_attachments: bool,
    pub size: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    pub items: Vec<MessageSummary>,
    pub total: u32,
    pub has_more: bool,
}

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
    pub body_text: String,
    pub attachments: Vec<AttachmentInfo>,
}

fn addresses(addr: Option<&mail_parser::Address<'_>>) -> Vec<String> {
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
fn repair_display_address(name: String, address: String) -> (String, String) {
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

fn summary_from(folder: &str, fetch: &Fetch, msg: Option<&mail_parser::Message<'_>>) -> MessageSummary {
    let flags: Vec<Flag<'_>> = fetch.flags().collect();
    let has = |f: &Flag<'_>| flags.iter().any(|x| x == f);

    let (from_name, from_address) = msg
        .and_then(|m| m.from())
        .and_then(|a| a.first())
        .map(|a| {
            (
                a.name().unwrap_or_default().trim().to_string(),
                a.address().unwrap_or_default().to_string(),
            )
        })
        .unwrap_or_default();
    let (from_name, from_address) = repair_display_address(from_name, from_address);

    let date = fetch
        .internal_date()
        .map(|d| d.to_rfc3339())
        .or_else(|| msg.and_then(|m| m.date()).map(|d| d.to_rfc3339()));

    let has_attachments = msg
        .and_then(|m| m.content_type())
        .map(|ct| {
            ct.ctype().eq_ignore_ascii_case("multipart")
                && ct.subtype().is_some_and(|s| s.eq_ignore_ascii_case("mixed"))
        })
        .unwrap_or(false);

    MessageSummary {
        folder: folder.to_string(),
        uid: fetch.uid.unwrap_or(0),
        from_name,
        from_address,
        to: addresses(msg.and_then(|m| m.to())),
        subject: msg
            .and_then(|m| m.subject())
            .unwrap_or_default()
            .trim()
            .to_string(),
        date,
        unread: !has(&Flag::Seen),
        flagged: has(&Flag::Flagged),
        answered: has(&Flag::Answered),
        has_attachments,
        size: fetch.size.unwrap_or(0),
    }
}

/// Страница писем, новые сверху. Загружаются только заголовки нужной страницы,
/// а не весь ящик.
pub async fn list_messages(
    conn: &mut Connection,
    folder: &str,
    offset: u32,
    limit: u32,
) -> Result<MessagePage> {
    let mailbox = conn.select(folder).await?;
    let total = mailbox.exists;
    let limit = limit.clamp(1, 200);

    if total == 0 || offset >= total {
        return Ok(MessagePage { items: Vec::new(), total, has_more: false });
    }

    let end = total - offset;
    let start = end.saturating_sub(limit - 1).max(1);

    let fetches: Vec<Fetch> = conn
        .session
        .fetch(
            format!("{start}:{end}"),
            "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])",
        )
        .await?
        .try_collect()
        .await?;

    let parser = MessageParser::default();
    let mut items: Vec<(u32, MessageSummary)> = fetches
        .iter()
        .filter(|f| f.uid.is_some())
        .map(|f| {
            let parsed = f.header().and_then(|h| parser.parse_headers(h));
            (f.message, summary_from(folder, f, parsed.as_ref()))
        })
        .collect();
    items.sort_by_key(|(seq, _)| std::cmp::Reverse(*seq));

    Ok(MessagePage {
        items: items.into_iter().map(|(_, s)| s).collect(),
        total,
        has_more: start > 1,
    })
}

async fn fetch_raw(conn: &mut Connection, folder: &str, uid: u32) -> Result<Fetch> {
    conn.ensure_selected(folder).await?;

    let sizes: Vec<Fetch> = conn
        .session
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
        .session
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
    Ok(fetches.swap_remove(pos))
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
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "attachment".to_string()
    } else {
        cleaned
    }
}

pub async fn get_message(conn: &mut Connection, folder: &str, uid: u32) -> Result<MessageDetail> {
    let fetch = fetch_raw(conn, folder, uid).await?;
    let body = fetch.body().ok_or(AppError::NotFound)?;
    let parsed = MessageParser::default().parse(body);
    let summary = summary_from(folder, &fetch, parsed.as_ref());

    let (cc, body_text, attachments) = match parsed.as_ref() {
        Some(msg) => {
            let attachments: Vec<AttachmentInfo> = msg
                .attachments()
                .enumerate()
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
                attachments,
            )
        }
        None => (Vec::new(), String::from_utf8_lossy(body).into_owned(), Vec::new()),
    };

    let mut summary = summary;
    summary.has_attachments = !attachments.is_empty();

    Ok(MessageDetail {
        summary,
        cc,
        body_text: body_text.replace("\r\n", "\n"),
        attachments,
    })
}

/// Возвращает (имя файла, содержимое) вложения.
pub async fn get_attachment(
    conn: &mut Connection,
    folder: &str,
    uid: u32,
    index: usize,
) -> Result<(String, Vec<u8>)> {
    let fetch = fetch_raw(conn, folder, uid).await?;
    let body = fetch.body().ok_or(AppError::NotFound)?;
    let msg = MessageParser::default().parse(body).ok_or(AppError::NotFound)?;
    let part = msg.attachments().nth(index).ok_or(AppError::NotFound)?;
    Ok((attachment_name(part, index), part.contents().to_vec()))
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlagKind {
    Seen,
    Flagged,
}

pub async fn set_flag(
    conn: &mut Connection,
    folder: &str,
    uid: u32,
    flag: FlagKind,
    value: bool,
) -> Result<()> {
    conn.ensure_selected(folder).await?;
    let name = match flag {
        FlagKind::Seen => "\\Seen",
        FlagKind::Flagged => "\\Flagged",
    };
    let op = if value { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
    let _: Vec<Fetch> = conn
        .session
        .uid_store(uid.to_string(), format!("{op} ({name})"))
        .await?
        .try_collect()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_filenames() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\Users\\x\\отчёт.docx"), "отчёт.docx");
        assert_eq!(sanitize_filename("a<b>:c?.txt"), "a_b__c_.txt");
        assert_eq!(sanitize_filename(".."), "attachment");
        assert_eq!(sanitize_filename(""), "attachment");
    }

    #[test]
    fn repairs_encoded_from() {
        assert_eq!(
            repair_display_address("Робот <robot@rrcki.ru>".into(), String::new()),
            ("Робот".into(), "robot@rrcki.ru".into())
        );
        assert_eq!(
            repair_display_address("\"Иванов И.\" <ivanov@nrcki.ru>".into(), String::new()),
            ("Иванов И.".into(), "ivanov@nrcki.ru".into())
        );
        assert_eq!(
            repair_display_address("Имя".into(), "a@b.ru".into()),
            ("Имя".into(), "a@b.ru".into())
        );
    }

    #[test]
    fn detects_roles_by_name() {
        assert_eq!(role_by_name("Отправленные"), FolderRole::Sent);
        assert_eq!(role_by_name("INBOX/Корзина"), FolderRole::Trash);
        assert_eq!(role_by_name("Sent Items"), FolderRole::Sent);
        assert_eq!(role_by_name("Проекты"), FolderRole::Other);
    }

    #[test]
    fn decodes_utf7_folder_names() {
        let encoded = utf7_imap::encode_utf7_imap("Отправленные".to_string());
        assert_eq!(utf7_imap::decode_utf7_imap(encoded), "Отправленные");
    }
}
