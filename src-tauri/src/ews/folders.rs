//! Почтовые папки Exchange: дерево (как у IMAP), служебные папки, подключённые
//! ящики (через `shared/<адрес>/…`), создание папки.

use std::collections::HashMap;

use crate::error::{AppError, Result};
use crate::imap::{Folder, FolderRole};

use super::*;

const DISTINGUISHED: [(&str, FolderRole); 5] = [
    ("inbox", FolderRole::Inbox),
    ("drafts", FolderRole::Drafts),
    ("sentitems", FolderRole::Sent),
    ("deleteditems", FolderRole::Trash),
    ("junkemail", FolderRole::Junk),
];

/// Имя папки для пути: «/» внутри имени заменяется, чтобы не путать с уровнями.
pub(super) fn segment(name: &str) -> String {
    name.replace('/', "\u{2215}")
}

/// Корень подключённых ящиков в дереве папок — как `shared/<ящик>/…` у IMAP.
pub(super) const SHARED_ROOT: &str = "shared";
type RawFolder = (String, String, String, String, u32, u32, u32);

/// Ссылка на служебную папку — своего ящика или подключённого.
pub(super) fn distinguished(id: &str, mailbox: Option<&str>) -> String {
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
        let mut unavailable: Vec<(String, String)> = Vec::new();
        for (address, name) in boxes {
            let prefix = format!("{SHARED_ROOT}/{}", segment(&address));
            let (root, roles, known) = match self.mailbox_roots(Some(&address)).await {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("mailbox {address}: {e}");
                    unavailable.push((address, e.to_string()));
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
                unavailable.push((address, "нет доступа ни к одной почтовой папке".into()));
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
        set_unavailable(&self.key, unavailable);
        if !shared.is_empty() {
            out.push(node_folder(SHARED_ROOT, "Подключённые ящики", 0));
            out.extend(shared);
        }
        self.folders = cache;
        Ok(out)
    }

    /// FolderId папки по её пути (папки загружаются при первом обращении).
    pub(super) async fn folder_id(&mut self, folder: &str) -> Result<String> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
