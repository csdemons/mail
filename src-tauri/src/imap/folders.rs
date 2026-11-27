//! Папки: дерево, роли (входящие, отправленные, черновики…), создание и особые папки.
//!
//! Часть модуля `imap` — общий обзор в `mod.rs`.

use super::prelude::*;
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

pub(crate) fn role_by_name(leaf: &str) -> FolderRole {
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

/// Папка из общего/чужого ящика (под корнем `shared` и т. п.).
pub fn is_shared(folder: &Folder) -> bool {
    let root = folder.id.split(['/', '.']).next().unwrap_or_default();
    is_shared_root(root)
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
        let in_own_root = parent.as_deref().is_none_or(|p| p.eq_ignore_ascii_case("INBOX"));
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
    if let Some(e) = conn.ews() {
        return e.list_folders().await;
    }
    let mut names: Vec<_> = conn
        .imap()?
        .list(Some(""), Some("*"))
        .await?
        .try_collect()
        .await?;
    // INBOX есть у любого сервера; без него ответ LIST — признак
    // рассинхронизированного соединения.
    if !names.iter().any(|n| n.name().eq_ignore_ascii_case("INBOX")) {
        conn.reconnect().await?;
        names = conn
            .imap()?
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
        match conn.imap()?.status(&folder.id, "(MESSAGES UNSEEN)").await {
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

/// Разделитель уровней папок на сервере («/» или «.»).
pub async fn hierarchy_delimiter(conn: &mut Connection) -> Result<String> {
    if conn.is_ews() {
        return Ok("/".into());
    }
    let names: Vec<_> = conn.imap()?.list(Some(""), Some("")).await?.try_collect().await?;
    Ok(names.first().and_then(|n| n.delimiter()).unwrap_or("/").to_string())
}

/// Имя уровня папки: без разделителя и символов, недопустимых в IMAP.
pub fn folder_part(name: &str, delimiter: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || c == '*' || c == '%' || delimiter.contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() { "Без имени".to_string() } else { cleaned.chars().take(120).collect() }
}

/// Создаёт папку и все уровни над ней; возвращает имя на сервере (modified UTF-7).
pub async fn ensure_folder_path(conn: &mut Connection, parts: &[String], delimiter: &str) -> Result<String> {
    let mut name = String::new();
    for part in parts {
        if !name.is_empty() {
            name.push_str(delimiter);
        }
        name.push_str(&utf7_imap::encode_utf7_imap(folder_part(part, delimiter)));
        create_folder(conn, &name).await?;
    }
    Ok(name)
}

/// Служебная папка своего ящика: по атрибуту SPECIAL-USE, иначе по привычному имени.
pub async fn role_folder(conn: &mut Connection, role: FolderRole) -> Result<Option<String>> {
    if let Some(e) = conn.ews() {
        return e.role_folder(role).await;
    }
    let names: Vec<_> = conn.imap()?.list(Some(""), Some("*")).await?.try_collect().await?;
    let attr_matches = |a: &NameAttribute<'_>| match role {
        FolderRole::Sent => matches!(a, NameAttribute::Sent),
        FolderRole::Drafts => matches!(a, NameAttribute::Drafts),
        FolderRole::Trash => matches!(a, NameAttribute::Trash),
        FolderRole::Archive => matches!(a, NameAttribute::Archive),
        FolderRole::Junk => matches!(a, NameAttribute::Junk),
        _ => false,
    };
    let by_attr = names.iter().find(|n| n.attributes().iter().any(attr_matches)).map(|n| n.name().to_string());
    let by_name = names
        .iter()
        .find(|n| parent_of(n.name(), n.delimiter()).is_none() && role_by_name(n.name()) == role)
        .map(|n| n.name().to_string());
    Ok(by_attr.or(by_name))
}

/// Последние письма папки с заданной ролью («Входящие», «Отправленные»…) — для
/// общих папок нескольких учётных записей. `None` — такой папки в ящике нет.
pub async fn role_messages(conn: &mut Connection, role: FolderRole, limit: u32) -> Result<Option<MessagePage>> {
    let folder = match role {
        FolderRole::Inbox => Some("INBOX".to_string()),
        FolderRole::Other => None,
        r => role_folder(conn, r).await?,
    };
    let Some(folder) = folder else { return Ok(None) };
    list_messages(conn, &folder, None, limit).await.map(Some)
}

/// Роль папки по имени из интерфейса общих папок.
pub fn parse_role(name: &str) -> Option<FolderRole> {
    Some(match name {
        "inbox" => FolderRole::Inbox,
        "sent" => FolderRole::Sent,
        "drafts" => FolderRole::Drafts,
        "archive" => FolderRole::Archive,
        "junk" => FolderRole::Junk,
        "trash" => FolderRole::Trash,
        _ => return None,
    })
}

/// Папка шаблонов (как «Templates» в Thunderbird); создаётся при первом сохранении.
pub async fn templates_folder(conn: &mut Connection, create: bool) -> Result<Option<String>> {
    if conn.is_ews() {
        return if create { Err(AppError::Other("Шаблоны для учётной записи Exchange пока недоступны".into())) } else { Ok(None) };
    }
    let names: Vec<_> = conn.imap()?.list(Some(""), Some("*")).await?.try_collect().await?;
    let found = names
        .iter()
        .map(|n| n.name().to_string())
        .find(|n| {
            let leaf = utf7_imap::decode_utf7_imap(n.clone()).to_lowercase();
            leaf == "templates" || leaf == "шаблоны"
        });
    drop(names);
    if found.is_some() || !create {
        return Ok(found);
    }
    conn.imap()?.create("Templates").await?;
    Ok(Some("Templates".into()))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub folder: String,
    pub uid: u32,
    pub name: String,
    pub text: String,
    pub html: Option<String>,
}

pub async fn templates(conn: &mut Connection) -> Result<Vec<Template>> {
    let Some(folder) = templates_folder(conn, false).await? else { return Ok(Vec::new()) };
    conn.ensure_selected(&folder).await?;
    let uids: Vec<u32> = conn.imap()?.uid_search("UNDELETED").await?.into_iter().collect();
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let fetches: Vec<Fetch> = conn.imap()?.uid_fetch(uid_set(&uids[..uids.len().min(200)]), "(UID BODY.PEEK[])").await?.try_collect().await?;
    let parser = MessageParser::default();
    let mut out: Vec<Template> = fetches
        .iter()
        .filter_map(|f| {
            let msg = parser.parse(f.body()?)?;
            let html = msg.body_html(0).map(|h| crate::smtp::sanitize_html(&h));
            Some(Template {
                folder: folder.clone(),
                uid: f.uid?,
                name: msg.subject().unwrap_or("(без названия)").to_string(),
                text: msg.body_text(0).map(|b| b.into_owned()).unwrap_or_default().replace("\r\n", "\n"),
                html,
            })
        })
        .collect();
    out.sort_by_key(|t| t.name.to_lowercase());
    Ok(out)
}

pub async fn append_template(conn: &mut Connection, raw: &[u8]) -> Result<()> {
    let folder = templates_folder(conn, true).await?.expect("created");
    conn.selected = None;
    conn.imap()?.append(&folder, Some("(\\Seen)"), None, raw).await?;
    Ok(())
}

/// Папка «Спам» своего ящика (создаётся при необходимости).
pub async fn junk_folder(conn: &mut Connection) -> Result<String> {
    if let Some(f) = role_folder(conn, FolderRole::Junk).await? {
        return Ok(f);
    }
    conn.imap()?.create("Junk").await?;
    Ok("Junk".into())
}

/// Папка «Отложенные» (создаётся при первом использовании).
pub async fn snoozed_folder(conn: &mut Connection, create: bool) -> Result<Option<String>> {
    if conn.is_ews() {
        return if create { Err(AppError::Other("«Отложить» для учётной записи Exchange пока недоступно".into())) } else { Ok(None) };
    }
    let names: Vec<_> = conn.imap()?.list(Some(""), Some("*")).await?.try_collect().await?;
    let found = names.iter().map(|n| n.name().to_string()).find(|n| {
        let leaf = utf7_imap::decode_utf7_imap(n.clone()).to_lowercase();
        leaf == "отложенные" || leaf == "snoozed"
    });
    drop(names);
    if found.is_some() || !create {
        return Ok(found);
    }
    let name = utf7_imap::encode_utf7_imap("Отложенные".to_string());
    conn.imap()?.create(&name).await?;
    Ok(Some(name))
}

/// Создаёт папку (если её ещё нет) и подписывается на неё.
pub async fn create_folder(conn: &mut Connection, name: &str) -> Result<()> {
    if let Some(e) = conn.ews() {
        return e.create_folder(name).await;
    }
    if let Err(e) = conn.imap()?.create(name).await {
        // ALREADYEXISTS — не ошибка: папку могли создать в веб-почте.
        let text = e.to_string();
        if !text.to_ascii_uppercase().contains("ALREADYEXISTS") && !text.to_lowercase().contains("exists") {
            return Err(e.into());
        }
    }
    let _ = conn.imap()?.subscribe(name).await;
    Ok(())
}

