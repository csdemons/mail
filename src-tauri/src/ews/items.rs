//! Письма: список, чтение, флаги, перемещение, удаление, отправка, поиск.
//! Для остальной программы ящик Exchange выглядит как IMAP (UID — время
//! получения с 2000 года), поэтому методы совпадают по сигнатурам с IMAP.

use base64::Engine as _;
use chrono::{TimeZone, Utc};

use crate::error::{AppError, Result};
use crate::imap::{FlagKind, FlagState, MessagePage, MessageSummary};

use super::*;

pub(super) const ITEM_TAGS: [&str; 7] = ["Message", "MeetingRequest", "MeetingResponse", "MeetingCancellation", "PostItem", "Item", "MeetingMessage"];

pub(super) fn items_of<'a, 'i>(n: roxmltree::Node<'a, 'i>) -> Vec<roxmltree::Node<'a, 'i>> {
    n.descendants().filter(|c| c.is_element() && ITEM_TAGS.contains(&c.tag_name().name()) && child(*c, "ItemId").is_some()).collect()
}

pub(super) fn item_id(n: roxmltree::Node<'_, '_>) -> Option<String> {
    child(n, "ItemId").and_then(|i| i.attribute("Id")).map(str::to_string)
}

pub(super) fn mailbox(n: roxmltree::Node<'_, '_>) -> (String, String) {
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Query;

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
        let q = Query::parse("от:петров тема:\"план работ\" есть:вложение после:2026-10-01");
        assert_eq!(aqs_of(&q), "from:\"петров\" AND subject:\"план работ\" AND hasattachment:true AND received>=2026-10-01");
    }
}
