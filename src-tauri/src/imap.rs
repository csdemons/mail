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
    creds: Credentials,
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
        creds: creds.clone(),
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

    /// Новое соединение вместо текущего (после рассинхронизации протокола).
    async fn reconnect(&mut self) -> Result<()> {
        log::warn!("IMAP: reconnecting after protocol desync");
        let fresh = connect(&self.creds).await?;
        let old = std::mem::replace(self, fresh);
        drop(old); // старое соединение в неизвестном состоянии — просто закрываем сокет
        Ok(())
    }

    /// SELECT с проверкой: ответ без обязательного UIDVALIDITY означает, что
    /// поток ответов рассинхронизирован (сервер прислал то, что библиотека не
    /// разобрала). В этом случае переподключаемся и повторяем один раз,
    /// иначе папка выглядела бы «пустой» без всякой ошибки.
    async fn select(&mut self, folder: &str) -> Result<async_imap::types::Mailbox> {
        for attempt in 0..2 {
            self.selected = None;
            let mailbox = self.session.select(folder).await.map_err(|e| match e {
                async_imap::error::Error::No(_) => AppError::NotFound,
                other => other.into(),
            })?;
            if mailbox.uid_validity.is_some() {
                self.selected = Some(folder.to_string());
                return Ok(mailbox);
            }
            if attempt == 0 {
                self.reconnect().await?;
            }
        }
        Err(AppError::Imap(format!(
            "сервер не вернул UIDVALIDITY для папки «{}»",
            utf7_imap::decode_utf7_imap(folder.to_string())
        )))
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    /// Имя на сервере (modified UTF-7) — его передаём обратно в команды.
    pub id: String,
    /// Отображаемое имя последнего уровня (декодированное).
    pub name: String,
    /// Полный путь для отображения.
    pub path: String,
    /// `id` родительской папки.
    pub parent: Option<String>,
    pub depth: usize,
    /// Роль (для иконки). Может быть и у вложенных/чужих папок.
    pub role: FolderRole,
    /// Служебная папка своего ящика верхнего уровня — показывается под
    /// стандартным названием («Входящие», «Отправленные» …).
    pub system: bool,
    /// `false` — только узел дерева (\\Noselect), открыть нельзя.
    pub selectable: bool,
    pub has_children: bool,
    pub total: u32,
    pub unseen: u32,
}

fn role_by_name(leaf: &str) -> FolderRole {
    match leaf.to_lowercase().as_str() {
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

/// Папка из ответа LIST.
#[derive(Debug, Clone)]
pub struct RawFolder {
    pub id: String,
    pub delimiter: Option<String>,
    /// Роль из атрибутов SPECIAL-USE (\\Sent, \\Trash …).
    pub special_use: Option<FolderRole>,
    pub selectable: bool,
}

/// Корень пространства имён общих/чужих ящиков (Dovecot/RuPost: `shared`).
fn is_shared_root(id: &str) -> bool {
    matches!(
        id.to_lowercase().as_str(),
        "shared" | "shared folders" | "other users" | "public folders"
    )
}

fn parent_of(id: &str, delimiter: Option<&str>) -> Option<String> {
    let d = delimiter.filter(|d| !d.is_empty())?;
    id.rfind(d).map(|pos| id[..pos].to_string()).filter(|p| !p.is_empty())
}

/// Строит дерево папок в порядке отображения (родитель, затем его дети).
/// Служебные папки своего ящика — сверху в привычном порядке, остальные по
/// алфавиту; невыбираемые корни (общие папки, другие пользователи) — в конце.
pub fn build_folder_tree(raw: Vec<RawFolder>) -> Vec<Folder> {
    use std::collections::{BTreeMap, HashMap};

    let mut by_id: BTreeMap<String, RawFolder> = BTreeMap::new();
    for f in raw {
        by_id.insert(f.id.clone(), f);
    }
    // Достраиваем отсутствующих в LIST предков как невыбираемые узлы.
    let ids: Vec<String> = by_id.keys().cloned().collect();
    for id in ids {
        let delim = by_id[&id].delimiter.clone();
        let mut cur = id;
        while let Some(parent) = parent_of(&cur, delim.as_deref()) {
            by_id.entry(parent.clone()).or_insert_with(|| RawFolder {
                id: parent.clone(),
                delimiter: delim.clone(),
                special_use: None,
                selectable: false,
            });
            cur = parent;
        }
    }

    let mut nodes: HashMap<String, Folder> = HashMap::new();
    let mut children: HashMap<Option<String>, Vec<String>> = HashMap::new();
    for f in by_id.values() {
        let parent = parent_of(&f.id, f.delimiter.as_deref());
        let path = utf7_imap::decode_utf7_imap(f.id.clone());
        let name = match f.delimiter.as_deref().filter(|d| !d.is_empty()) {
            Some(d) => path.rsplit(d).next().unwrap_or(&path).to_string(),
            None => path.clone(),
        };
        let delim = f.delimiter.as_deref().unwrap_or("/");
        let in_own_root = parent.as_deref().map_or(true, |p| p.eq_ignore_ascii_case("INBOX"));
        // Папка верхнего уровня подключённого ящика: shared/<ящик>/<папка>.
        let in_shared_mailbox = parent
            .as_deref()
            .and_then(|p| parent_of(p, Some(delim)))
            .is_some_and(|root| parent_of(&root, Some(delim)).is_none() && is_shared_root(&root));
        let role = if f.id.eq_ignore_ascii_case("INBOX") {
            FolderRole::Inbox
        } else if let Some(role) = f.special_use {
            role
        } else if in_own_root || in_shared_mailbox {
            role_by_name(&name)
        } else {
            FolderRole::Other
        };
        let system = f.selectable
            && ((in_own_root
                && (f.id.eq_ignore_ascii_case("INBOX")
                    || (role != FolderRole::Other && role != FolderRole::Inbox)))
                || (in_shared_mailbox && role != FolderRole::Other));
        let name = if parent.is_none() && !f.selectable && is_shared_root(&f.id) {
            "Подключённые ящики".to_string()
        } else {
            name
        };
        children.entry(parent.clone()).or_default().push(f.id.clone());
        nodes.insert(
            f.id.clone(),
            Folder {
                id: f.id.clone(),
                name,
                path,
                parent,
                depth: 0,
                role,
                system,
                selectable: f.selectable,
                has_children: false,
                total: 0,
                unseen: 0,
            },
        );
    }
    for (parent, kids) in &children {
        if let Some(p) = parent.as_ref().and_then(|p| nodes.get_mut(p)) {
            p.has_children = !kids.is_empty();
        }
    }

    let rank = |f: &Folder| -> (u8, String) {
        let r = if f.system {
            f.role as u8
        } else if f.parent.is_none() && !f.selectable {
            200
        } else {
            100
        };
        (r, f.name.to_lowercase())
    };
    for kids in children.values_mut() {
        kids.sort_by_key(|id| rank(&nodes[id]));
    }

    let mut out = Vec::with_capacity(nodes.len());
    let mut stack: Vec<(String, usize)> = children
        .get(&None)
        .map(|v| v.iter().rev().map(|id| (id.clone(), 0)).collect())
        .unwrap_or_default();
    while let Some((id, depth)) = stack.pop() {
        if let Some(kids) = children.get(&Some(id.clone())) {
            for kid in kids.iter().rev() {
                stack.push((kid.clone(), depth + 1));
            }
        }
        let mut node = nodes.remove(&id).expect("node exists");
        node.depth = depth;
        out.push(node);
    }
    out
}

pub async fn list_folders(conn: &mut Connection) -> Result<Vec<Folder>> {
    let mut names: Vec<_> = conn
        .session
        .list(Some(""), Some("*"))
        .await?
        .try_collect()
        .await?;
    // INBOX есть у любого сервера; без него ответ LIST — признак
    // рассинхронизированного соединения.
    if !names.iter().any(|n| n.name().eq_ignore_ascii_case("INBOX")) {
        conn.reconnect().await?;
        names = conn
            .session
            .list(Some(""), Some("*"))
            .await?
            .try_collect()
            .await?;
    }

    let mut raw = Vec::new();
    for name in &names {
        let mut special_use = None;
        let mut selectable = true;
        for attr in name.attributes() {
            match attr {
                NameAttribute::NoSelect => selectable = false,
                NameAttribute::Sent => special_use = Some(FolderRole::Sent),
                NameAttribute::Drafts => special_use = Some(FolderRole::Drafts),
                NameAttribute::Junk => special_use = Some(FolderRole::Junk),
                NameAttribute::Trash => special_use = Some(FolderRole::Trash),
                NameAttribute::Archive => special_use = Some(FolderRole::Archive),
                NameAttribute::Extension(ext) if ext.eq_ignore_ascii_case("\\NonExistent") => {
                    selectable = false
                }
                _ => {}
            }
        }
        raw.push(RawFolder {
            id: name.name().to_string(),
            delimiter: name.delimiter().map(str::to_string),
            special_use,
            selectable,
        });
    }
    drop(names);

    let mut folders = build_folder_tree(raw);
    for folder in folders.iter_mut().filter(|f| f.selectable) {
        // STATUS не должен ронять весь список, если одна папка недоступна.
        // Отказ сервера (NO/BAD) — просто нет счётчика. Любая другая ошибка
        // (неразобранный ответ, обрыв) портит соединение: переподключаемся.
        match conn.session.status(&folder.id, "(MESSAGES UNSEEN)").await {
            Ok(mb) => {
                folder.total = mb.exists;
                // RuPost иногда отдаёт мусорный UNSEEN (например, 4141792512 при 0 писем).
                folder.unseen = mb.unseen.unwrap_or(0).min(mb.exists);
            }
            Err(async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_)) => {}
            Err(e) => {
                log::warn!("STATUS {} failed: {e}", folder.id);
                conn.reconnect().await?;
            }
        }
    }
    conn.selected = None;
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
    /// Message-ID и References — для заголовков ответа.
    pub message_id: Option<String>,
    pub references: Option<String>,
    /// Reply-To, если отличается от отправителя.
    pub reply_to: Option<String>,
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

fn has_param(params: &async_imap::imap_proto::types::BodyParams<'_>, names: &[&str]) -> bool {
    params
        .as_ref()
        .is_some_and(|list| list.iter().any(|(k, _)| names.iter().any(|n| k.eq_ignore_ascii_case(n))))
}

/// Есть ли во вложенной структуре MIME части-файлы: часть с
/// `Content-Disposition: attachment`, нетекстовая часть с именем файла или
/// вложенное письмо (message/rfc822).
fn has_attachment_part(bs: &async_imap::imap_proto::types::BodyStructure<'_>) -> bool {
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

    let has_attachments = fetch.bodystructure().is_some_and(has_attachment_part);

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

/// Страница писем, новые сверху. Курсор — UID: `before_uid` = UID последнего
/// уже показанного письма. В отличие от номеров сообщений UID не сдвигаются,
/// когда в папку приходят или из неё удаляются письма, поэтому догрузка не
/// даёт пропусков и повторов. Загружаются только заголовки нужной страницы.
pub async fn list_messages(
    conn: &mut Connection,
    folder: &str,
    before_uid: Option<u32>,
    limit: u32,
) -> Result<MessagePage> {
    let mailbox = conn.select(folder).await?;
    let total = mailbox.exists;
    let limit = limit.clamp(1, 200) as usize;

    if total == 0 {
        return Ok(MessagePage { items: Vec::new(), total, has_more: false });
    }

    let mut uids: Vec<u32> = conn.session.uid_search("ALL").await?.into_iter().collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    if let Some(before) = before_uid {
        uids.retain(|uid| *uid < before);
    }
    let has_more = uids.len() > limit;
    uids.truncate(limit);
    if uids.is_empty() {
        return Ok(MessagePage { items: Vec::new(), total, has_more: false });
    }

    let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let fetches: Vec<Fetch> = conn
        .session
        .uid_fetch(
            set,
            "(UID FLAGS INTERNALDATE RFC822.SIZE BODYSTRUCTURE BODY.PEEK[HEADER])",
        )
        .await?
        .try_collect()
        .await?;

    let parser = MessageParser::default();
    let mut items: Vec<MessageSummary> = fetches
        .iter()
        .filter(|f| f.uid.is_some())
        .map(|f| {
            let parsed = f.header().and_then(|h| parser.parse_headers(h));
            summary_from(folder, f, parsed.as_ref())
        })
        .collect();
    items.sort_by_key(|m| std::cmp::Reverse(m.uid));

    Ok(MessagePage { items, total, has_more })
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
    let fetch = fetch_raw(conn, folder, uid).await?;
    let body = fetch.body().ok_or(AppError::NotFound)?;
    let parsed = MessageParser::default().parse(body);
    let summary = summary_from(folder, &fetch, parsed.as_ref());

    let (message_id, references, reply_to) = match parsed.as_ref() {
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
        message_id,
        references,
        reply_to,
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

/// Кладёт отправленное письмо в папку «Отправленные» (\\Sent или по имени).
/// Возвращает имя папки.
pub async fn append_sent(conn: &mut Connection, raw: &[u8]) -> Result<String> {
    let names: Vec<_> = conn
        .session
        .list(Some(""), Some("*"))
        .await?
        .try_collect()
        .await?;
    let by_attr = names
        .iter()
        .find(|n| n.attributes().iter().any(|a| matches!(a, NameAttribute::Sent)))
        .map(|n| n.name().to_string());
    let by_name = names
        .iter()
        .find(|n| parent_of(n.name(), n.delimiter()).is_none() && role_by_name(n.name()) == FolderRole::Sent)
        .map(|n| n.name().to_string());
    drop(names);
    let folder = by_attr
        .or(by_name)
        .ok_or_else(|| AppError::Other("Папка «Отправленные» не найдена".into()))?;
    conn.selected = None;
    conn.session.append(&folder, Some("(\\Seen)"), None, raw).await?;
    Ok(folder)
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

// ---------------------------------------------------------------- перемещение и удаление

fn uid_set(uids: &[u32]) -> String {
    uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

async fn has_capability(conn: &mut Connection, name: &str) -> bool {
    conn.capabilities().await.map(|c| c.iter().any(|x| x.eq_ignore_ascii_case(name))).unwrap_or(false)
}

/// Удаляет помеченные \Deleted письма: только указанные (UID EXPUNGE, если
/// сервер умеет UIDPLUS), иначе — все помеченные в папке.
async fn expunge_uids(conn: &mut Connection, uids: &[u32]) -> Result<()> {
    if has_capability(conn, "UIDPLUS").await {
        let _: Vec<_> = conn.session.uid_expunge(uid_set(uids)).await?.try_collect().await?;
    } else {
        let _: Vec<_> = conn.session.expunge().await?.try_collect().await?;
    }
    Ok(())
}

/// Перемещает письма в другую папку (UID MOVE, без него — COPY + удаление).
pub async fn move_messages(conn: &mut Connection, folder: &str, uids: &[u32], target: &str) -> Result<()> {
    if uids.is_empty() || folder == target {
        return Ok(());
    }
    let can_move = has_capability(conn, "MOVE").await;
    conn.ensure_selected(folder).await?;
    if can_move {
        conn.session.uid_mv(uid_set(uids), target).await?;
    } else {
        conn.session.uid_copy(uid_set(uids), target).await?;
        let _: Vec<Fetch> = conn.session.uid_store(uid_set(uids), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
        expunge_uids(conn, uids).await?;
    }
    Ok(())
}

/// Удаление: в «Корзину» (`trash`), а из самой корзины (или если её нет) — насовсем.
/// Возвращает `true`, если письма удалены безвозвратно.
pub async fn delete_messages(conn: &mut Connection, folder: &str, uids: &[u32], trash: Option<&str>) -> Result<bool> {
    if uids.is_empty() {
        return Ok(false);
    }
    match trash {
        Some(t) if t != folder => {
            move_messages(conn, folder, uids, t).await?;
            Ok(false)
        }
        _ => {
            conn.ensure_selected(folder).await?;
            let _: Vec<Fetch> = conn.session.uid_store(uid_set(uids), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
            expunge_uids(conn, uids).await?;
            Ok(true)
        }
    }
}

// ---------------------------------------------------------------- диагностика

/// Выполняет произвольную команду и возвращает все ответы сервера текстом.
#[doc(hidden)]
pub async fn raw_command(conn: &mut Connection, command: &str) -> Vec<String> {
    use async_imap::imap_proto::types::Response;
    let mut out = Vec::new();
    let tag = match conn.session.run_command(command).await {
        Ok(tag) => tag,
        Err(e) => return vec![format!("ошибка отправки: {e}")],
    };
    loop {
        match conn.session.read_response().await {
            Ok(Some(resp)) => {
                let done = matches!(resp.parsed(), Response::Done { tag: t, .. } if *t == tag);
                out.push(format!("{:?}", resp.parsed()));
                if done {
                    break;
                }
            }
            Ok(None) => {
                out.push("соединение закрыто".into());
                break;
            }
            Err(e) => {
                out.push(format!("ошибка чтения: {e}"));
                // Поток ответов сломан — дальше работаем через новое соединение.
                if let Err(e) = conn.reconnect().await {
                    out.push(format!("переподключение не удалось: {e}"));
                }
                break;
            }
        }
    }
    out
}

/// Отчёт для разбора проблем: что сервер отвечает на ключевые запросы.
/// Пароль в отчёт не попадает. Папка открывается только на чтение (EXAMINE).
pub async fn diagnostics(conn: &mut Connection, folder: Option<&str>) -> Result<String> {
    use std::fmt::Write;
    let mut r = String::new();
    let _ = writeln!(r, "RRCKI Mail {} — диагностика IMAP", env!("CARGO_PKG_VERSION"));

    let caps = conn.capabilities().await.unwrap_or_else(|e| vec![format!("ошибка: {e}")]);
    let _ = writeln!(r, "\n## CAPABILITY\n{}", caps.join(" "));

    let _ = writeln!(r, "\n## LIST \"\" \"*\"");
    let mut ids = Vec::new();
    let listed: std::result::Result<Vec<_>, String> = match conn.session.list(Some(""), Some("*")).await {
        Ok(stream) => stream.try_collect::<Vec<_>>().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match listed {
        Ok(names) => {
            for n in &names {
                let _ = writeln!(
                    r,
                    "{:?} | {:?} | {} | {}",
                    n.attributes(),
                    n.delimiter(),
                    n.name(),
                    utf7_imap::decode_utf7_imap(n.name().to_string())
                );
                if !n.attributes().iter().any(|a| matches!(a, NameAttribute::NoSelect)) {
                    ids.push(n.name().to_string());
                }
            }
        }
        Err(e) => {
            let _ = writeln!(r, "ошибка: {e}");
            conn.reconnect().await?;
        }
    }

    let _ = writeln!(r, "\n## STATUS (MESSAGES UNSEEN) по папкам");
    for id in &ids {
        match conn.session.status(id, "(MESSAGES UNSEEN)").await {
            Ok(mb) => {
                let _ = writeln!(r, "ok  {id}: {} / {:?}", mb.exists, mb.unseen);
            }
            Err(e) => {
                let fatal = !matches!(e, async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_));
                let _ = writeln!(r, "ERR {id}: {e}{}", if fatal { " — соединение пересоздано" } else { "" });
                if fatal {
                    conn.reconnect().await?;
                }
            }
        }
    }

    if let Some(folder) = folder {
        conn.selected = None;
        let _ = writeln!(r, "\n## EXAMINE {folder} ({})", utf7_imap::decode_utf7_imap(folder.to_string()));
        match conn.session.examine(folder).await {
            Ok(mb) => {
                let _ = writeln!(
                    r,
                    "exists={} recent={} unseen={:?} uidvalidity={:?} uidnext={:?}",
                    mb.exists, mb.recent, mb.unseen, mb.uid_validity, mb.uid_next
                );
                match conn.session.uid_search("ALL").await {
                    Ok(uids) => {
                        let mut uids: Vec<u32> = uids.into_iter().collect();
                        uids.sort_unstable();
                        let _ = writeln!(
                            r,
                            "UID SEARCH ALL: {} шт., первые {:?}, последние {:?}",
                            uids.len(),
                            &uids[..uids.len().min(5)],
                            &uids[uids.len().saturating_sub(5)..]
                        );
                        if let Some(last) = uids.last() {
                            let _ = writeln!(r, "UID FETCH {last} (FLAGS RFC822.SIZE BODYSTRUCTURE):");
                            for line in raw_command(
                                conn,
                                &format!("UID FETCH {last} (UID FLAGS RFC822.SIZE BODYSTRUCTURE)"),
                            )
                            .await
                            {
                                let _ = writeln!(r, "{}", line.chars().take(1500).collect::<String>());
                            }
                        }
                    }
                    Err(e) => {
                        let _ = writeln!(r, "UID SEARCH ALL: ошибка: {e}");
                    }
                }
                if mb.exists > 0 {
                    let _ = writeln!(r, "FETCH {}:{} (UID FLAGS):", mb.exists.saturating_sub(2).max(1), mb.exists);
                    for line in raw_command(
                        conn,
                        &format!("FETCH {}:{} (UID FLAGS)", mb.exists.saturating_sub(2).max(1), mb.exists),
                    )
                    .await
                    {
                        let _ = writeln!(r, "{line}");
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(r, "ошибка: {e}");
            }
        }
        conn.selected = None;
    }
    Ok(r)
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
        assert_eq!(sanitize_filename("con.txt"), "_con.txt");
        assert_eq!(sanitize_filename("COM1"), "_COM1");
        assert_eq!(sanitize_filename("report.pdf. . "), "report.pdf");
        assert_eq!(sanitize_filename("console.log"), "console.log");
        let long = format!("{}.docx", "я".repeat(300));
        let cut = sanitize_filename(&long);
        assert!(cut.ends_with(".docx") && cut.chars().count() <= 180);
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
        assert_eq!(role_by_name("Sent Items"), FolderRole::Sent);
        assert_eq!(role_by_name("Проекты"), FolderRole::Other);
    }

    fn raw(id: &str, special: Option<FolderRole>, selectable: bool) -> RawFolder {
        RawFolder {
            id: utf7_imap::encode_utf7_imap(id.to_string()),
            delimiter: Some("/".into()),
            special_use: special,
            selectable,
        }
    }

    #[test]
    fn labels_rupost_shared_mailboxes() {
        let list = vec![
            raw("INBOX", None, true),
            raw("shared", None, false),
            raw("shared/noc@nrcki.ru", None, false),
            raw("shared/noc@nrcki.ru/INBOX", None, true),
            raw("shared/noc@nrcki.ru/Sent", None, true),
            raw("shared/noc@nrcki.ru/imap", None, true),
            raw("shared/_soc@nrcki.ru/INBOX/MaxPatrol", None, true),
            raw("Archives", None, false),
            raw("Archives/Archives", Some(FolderRole::Archive), true),
        ];
        let tree = build_folder_tree(list);
        let view: Vec<(usize, &str, FolderRole, bool)> =
            tree.iter().map(|f| (f.depth, f.name.as_str(), f.role, f.system)).collect();
        assert_eq!(
            view,
            vec![
                (0, "INBOX", FolderRole::Inbox, true),
                (0, "Archives", FolderRole::Archive, false),
                (1, "Archives", FolderRole::Archive, false),
                (0, "Подключённые ящики", FolderRole::Other, false),
                (1, "_soc@nrcki.ru", FolderRole::Other, false),
                (2, "INBOX", FolderRole::Inbox, false),
                (3, "MaxPatrol", FolderRole::Other, false),
                (1, "noc@nrcki.ru", FolderRole::Other, false),
                (2, "INBOX", FolderRole::Inbox, true),
                (2, "Sent", FolderRole::Sent, true),
                (2, "imap", FolderRole::Other, false),
            ]
        );
    }

    #[test]
    fn builds_tree_with_shared_mailboxes() {
        let list = vec![
            raw("Other Users", None, false),
            raw("Other Users/uex2016-06", None, false),
            raw("Other Users/uex2016-06/INBOX", None, true),
            raw("Other Users/uex2016-06/Sent", Some(FolderRole::Sent), true),
            raw("Корзина/2024/Astra", None, true),
            raw("Корзина", Some(FolderRole::Trash), true),
            raw("Проекты", None, true),
            raw("INBOX/Входящие отдела", None, true),
            raw("Отправленные", Some(FolderRole::Sent), true),
            raw("INBOX", None, true),
            raw("Архив", None, true),
        ];
        let tree = build_folder_tree(list);
        let view: Vec<(usize, &str, bool, bool)> = tree
            .iter()
            .map(|f| (f.depth, f.name.as_str(), f.system, f.selectable))
            .collect();
        assert_eq!(
            view,
            vec![
                (0, "INBOX", true, true),
                (1, "Входящие отдела", false, true),
                (0, "Отправленные", true, true),
                (0, "Архив", true, true),
                (0, "Корзина", true, true),
                (1, "2024", false, false),
                (2, "Astra", false, true),
                (0, "Проекты", false, true),
                (0, "Подключённые ящики", false, false),
                (1, "uex2016-06", false, false),
                (2, "INBOX", true, true),
                (2, "Sent", true, true),
            ]
        );
        // Папки подключённого ящика подписываются стандартно («Входящие», «Отправленные»).
        assert_eq!(tree[11].role, FolderRole::Sent);
        assert!(tree[0].has_children && tree[4].has_children && !tree[2].has_children);
        assert_eq!(tree[1].parent.as_deref(), Some("INBOX"));
    }

    #[test]
    fn decodes_utf7_folder_names() {
        let encoded = utf7_imap::encode_utf7_imap("Отправленные".to_string());
        assert_eq!(utf7_imap::decode_utf7_imap(encoded), "Отправленные");
    }
}
