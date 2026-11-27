//! Список писем: сводки, страницы, флаги в списке, идентификаторы, импорт и выгрузка.
//!
//! Часть модуля `imap` — общий обзор в `mod.rs`.

use super::prelude::*;
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// «Важное»: ключевое слово `$label1` (метка «Важное» в SOGo/Thunderbird) или `$Important`.
    pub important: bool,
    pub answered: bool,
    /// Ключевые слова IMAP: категории ($label2… / свои), $MDNSent и т. п.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Message-ID письма — для цепочек (`msgId`: у полного письма есть своё `messageId`).
    #[serde(default, rename = "msgId", skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// Предки в цепочке: In-Reply-To и References (первый — начало цепочки).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thread: Vec<String>,
    pub has_attachments: bool,
    /// Имена файлов-вложений (из BODYSTRUCTURE) — для поиска по вложениям.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    pub size: u32,
    /// Начало текста письма для строки списка — если письмо хранится на компьютере
    /// (подставляется из локального хранилища, на сервер не ходит).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    pub items: Vec<MessageSummary>,
    pub total: u32,
    pub has_more: bool,
}

/// Флаги письма и ключевые слова (категории, «уведомление о прочтении отправлено» и т. п.).
#[derive(Debug, Clone, Default)]
pub struct FlagState {
    pub unread: bool,
    pub flagged: bool,
    pub important: bool,
    pub answered: bool,
    pub keywords: Vec<String>,
}

fn flag_state(fetch: &Fetch) -> FlagState {
    let flags: Vec<Flag<'_>> = fetch.flags().collect();
    let has = |f: &Flag<'_>| flags.iter().any(|x| x == f);
    let is_important = |k: &str| k.eq_ignore_ascii_case(IMPORTANT) || k.eq_ignore_ascii_case("$Important");
    FlagState {
        unread: !has(&Flag::Seen),
        flagged: has(&Flag::Flagged),
        important: flags.iter().any(|f| matches!(f, Flag::Custom(k) if is_important(k))),
        answered: has(&Flag::Answered),
        keywords: flags
            .iter()
            .filter_map(|f| match f {
                Flag::Custom(k) if !is_important(k) && !k.starts_with('\\') => Some(k.to_string()),
                _ => None,
            })
            .collect(),
    }
}

impl MessageSummary {
    pub fn set_flags(&mut self, f: FlagState) {
        self.unread = f.unread;
        self.flagged = f.flagged;
        self.important = f.important;
        self.answered = f.answered;
        self.keywords = f.keywords;
    }
}

/// References (по порядку, от начала цепочки), затем In-Reply-To.
fn thread_ids(m: &mail_parser::Message<'_>) -> Vec<String> {
    let list = |h: &mail_parser::HeaderValue<'_>| -> Vec<String> {
        match h {
            mail_parser::HeaderValue::Text(t) => vec![t.to_string()],
            mail_parser::HeaderValue::TextList(l) => l.iter().map(|t| t.to_string()).collect(),
            _ => vec![],
        }
    };
    let mut ids: Vec<String> = list(m.references()).into_iter().chain(list(m.in_reply_to())).map(|id| format!("<{}>", id.trim_matches(['<', '>']))).collect();
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    ids.truncate(20);
    ids
}

pub(crate) fn summary_from(folder: &str, fetch: &Fetch, msg: Option<&mail_parser::Message<'_>>) -> MessageSummary {

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
    let attachments = fetch.bodystructure().map(attachment_names).unwrap_or_default();

    let mut summary = MessageSummary {
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
        unread: false,
        flagged: false,
        important: false,
        answered: false,
        keywords: Vec::new(),
        message_id: msg.and_then(|m| m.message_id()).map(|id| format!("<{}>", id.trim_matches(['<', '>']))),
        thread: msg.map(thread_ids).unwrap_or_default(),
        has_attachments,
        attachments,
        size: fetch.size.unwrap_or(0),
        snippet: None,
    };
    summary.set_flags(flag_state(fetch));
    summary
}

/// Новые непрочитанные письма папки с UID больше `after_uid` (новые сверху, не больше
/// `limit`) и последний UID папки — для уведомлений. Флаг \\Seen не меняется.
pub async fn new_unseen(conn: &mut Connection, folder: &str, after_uid: u32, limit: u32) -> Result<(Vec<MessageSummary>, u32)> {
    if let Some(e) = conn.ews() {
        return e.new_unseen(folder, after_uid, limit).await;
    }
    let mailbox = conn.select(folder).await?;
    let last_uid = mailbox.uid_next.map(|n| n.saturating_sub(1)).unwrap_or(0);
    if limit == 0 || mailbox.exists == 0 {
        return Ok((Vec::new(), last_uid));
    }
    let mut uids: Vec<u32> = conn
        .imap()?
        .uid_search(format!("UNSEEN UID {}:*", after_uid.saturating_add(1)))
        .await?
        .into_iter()
        .filter(|u| *u > after_uid)
        .collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.truncate(limit.min(10) as usize);
    let last_uid = last_uid.max(uids.first().copied().unwrap_or(0));
    if uids.is_empty() {
        return Ok((Vec::new(), last_uid));
    }
    let fetches: Vec<Fetch> = conn
        .imap()?
        .uid_fetch(uid_set(&uids), "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])")
        .await?
        .try_collect()
        .await?;
    let parser = MessageParser::default();
    let mut items: Vec<MessageSummary> = fetches
        .iter()
        .filter(|f| f.uid.is_some_and(|u| u > after_uid))
        .map(|f| summary_from(folder, f, f.header().and_then(|h| parser.parse_headers(h)).as_ref()))
        .filter(|m| m.unread)
        .collect();
    items.sort_by_key(|m| std::cmp::Reverse(m.uid));
    Ok((items, last_uid))
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
    if let Some(e) = conn.ews() {
        return e.list_messages(folder, before_uid, limit).await;
    }
    let mailbox = conn.select(folder).await?;
    let total = mailbox.exists;
    let limit = limit.clamp(1, 200) as usize;

    if total == 0 {
        return Ok(MessagePage { items: Vec::new(), total, has_more: false });
    }

    let mut uids: Vec<u32> = conn.imap()?.uid_search("ALL").await?.into_iter().collect();
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
        .imap()?
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

/// UIDVALIDITY папки и флаги всех писем (только чтение: EXAMINE).
pub async fn folder_flags(conn: &mut Connection, folder: &str) -> Result<(u32, Vec<(u32, FlagState)>)> {
    if conn.is_ews() {
        conn.selected = Some(folder.to_string());
        return conn.ews().expect("ews").folder_flags(folder).await;
    }
    conn.selected = None;
    let mailbox = conn.imap()?.examine(folder).await.map_err(|e| match e {
        async_imap::error::Error::No(_) => AppError::NotFound,
        other => other.into(),
    })?;
    let validity = mailbox.uid_validity.ok_or_else(|| AppError::Imap("нет UIDVALIDITY".into()))?;
    if mailbox.exists == 0 {
        return Ok((validity, Vec::new()));
    }
    let fetches: Vec<Fetch> = conn.imap()?.uid_fetch("1:*", "(UID FLAGS)").await?.try_collect().await?;
    Ok((validity, fetches.iter().filter_map(|f| f.uid.map(|u| (u, flag_state(f)))).collect()))
}

/// Заголовки писем для индекса (папка уже открыта `folder_flags`).
pub async fn index_summaries(conn: &mut Connection, folder: &str, uids: &[u32]) -> Result<Vec<MessageSummary>> {
    if let Some(e) = conn.ews() {
        return e.summaries_of(folder, uids).await;
    }
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let fetches: Vec<Fetch> = conn
        .imap()?
        .uid_fetch(uid_set(uids), "(UID FLAGS INTERNALDATE RFC822.SIZE BODYSTRUCTURE BODY.PEEK[HEADER.FIELDS (FROM TO SUBJECT DATE MESSAGE-ID IN-REPLY-TO REFERENCES)])")
        .await?
        .try_collect()
        .await?;
    let parser = MessageParser::default();
    Ok(fetches
        .iter()
        .filter(|f| f.uid.is_some())
        .map(|f| {
            let parsed = f.header().and_then(|h| parser.parse_headers(h));
            summary_from(folder, f, parsed.as_ref())
        })
        .collect())
}

/// Исходники писем (папка уже открыта `folder_flags`). Сервер отдаёт не больше
/// `max_each + 1` байт каждого письма (частичная выборка `BODY.PEEK[]<0.N>`), поэтому
/// размер ответа ограничен на уровне IMAP, даже если сервер сообщил неверный размер;
/// письма больше `max_each` пропускаются.
pub async fn raw_bodies(conn: &mut Connection, uids: &[u32], max_each: u32) -> Result<Vec<(u32, Vec<u8>)>> {
    if conn.is_ews() {
        let folder = conn.selected.clone().ok_or(AppError::NotFound)?;
        return conn.ews().expect("ews").mimes(&folder, uids, max_each).await;
    }
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let query = format!("(UID BODY.PEEK[]<0.{}>)", u64::from(max_each) + 1);
    let fetches: Vec<Fetch> = conn.imap()?.uid_fetch(uid_set(uids), query).await?.try_collect().await?;
    Ok(fetches
        .iter()
        .filter_map(|f| {
            let (uid, body) = (f.uid?, f.body()?);
            (body.len() <= max_each as usize).then(|| (uid, body.to_vec()))
        })
        .collect())
}

/// Непрочитанные во «Входящих» (для других учётных записей — без SELECT).
pub async fn inbox_unseen(conn: &mut Connection) -> Result<u32> {
    if let Some(e) = conn.ews() {
        return e.unread("INBOX").await;
    }
    let mb = conn.imap()?.status("INBOX", "(MESSAGES UNSEEN)").await?;
    Ok(mb.unseen.unwrap_or(0).min(mb.exists))
}

// ---------------------------------------------------------------- импорт и резервная копия

/// Message-ID писем папки — чтобы повторный импорт не создавал копий.
pub async fn message_ids(conn: &mut Connection, folder: &str) -> Result<std::collections::HashSet<String>> {
    conn.selected = None;
    let mailbox = conn.imap()?.examine(folder).await?;
    let mut out = std::collections::HashSet::new();
    if mailbox.exists == 0 {
        return Ok(out);
    }
    let fetches: Vec<Fetch> = conn.imap()?.fetch("1:*", "(BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])").await?.try_collect().await?;
    let parser = MessageParser::default();
    for f in &fetches {
        if let Some(id) = f.header().and_then(|h| parser.parse_headers(h)).and_then(|m| m.message_id().map(str::to_string)) {
            out.insert(id);
        }
    }
    Ok(out)
}

/// Кладёт письмо в папку с флагами и датой получения.
pub async fn append_imported(conn: &mut Connection, folder: &str, seen: bool, flagged: bool, answered: bool, date: Option<i64>, raw: &[u8]) -> Result<()> {
    let flags: Vec<&str> = [(seen, "\\Seen"), (flagged, "\\Flagged"), (answered, "\\Answered")].iter().filter(|(on, _)| *on).map(|(_, f)| *f).collect();
    let flags = format!("({})", flags.join(" "));
    let date = date.and_then(|d| chrono::DateTime::from_timestamp(d, 0)).map(|d| format!("\"{}\"", d.format("%d-%b-%Y %H:%M:%S +0000")));
    conn.selected = None;
    conn.imap()?.append(folder, Some(&flags), date.as_deref(), raw).await?;
    Ok(())
}

/// UID и размеры всех писем папки (папка открывается только для чтения).
pub async fn all_uids(conn: &mut Connection, folder: &str) -> Result<Vec<(u32, u32)>> {
    conn.selected = None;
    let mailbox = conn.imap()?.examine(folder).await.map_err(|e| match e {
        async_imap::error::Error::No(_) => AppError::NotFound,
        other => other.into(),
    })?;
    if mailbox.exists == 0 {
        return Ok(Vec::new());
    }
    let fetches: Vec<Fetch> = conn.imap()?.uid_fetch("1:*", "(UID RFC822.SIZE)").await?.try_collect().await?;
    let mut out: Vec<(u32, u32)> = fetches.iter().filter_map(|f| Some((f.uid?, f.size.unwrap_or(0)))).collect();
    out.sort_unstable();
    Ok(out)
}

/// Письмо для резервной копии.
pub struct Exported {
    pub raw: Vec<u8>,
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub date: Option<i64>,
}

/// Письма целиком с флагами и датой (папка открыта `all_uids`).
pub async fn export_batch(conn: &mut Connection, uids: &[u32]) -> Result<Vec<Exported>> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    // Не больше MAX_MESSAGE_MB на письмо — даже если сервер сообщил неверный размер.
    let max = MAX_MESSAGE_MB as usize * 1024 * 1024;
    let query = format!("(UID FLAGS INTERNALDATE BODY.PEEK[]<0.{}>)", max + 1);
    let mut fetches: Vec<Fetch> = conn.imap()?.uid_fetch(uid_set(uids), query).await?.try_collect().await?;
    fetches.sort_by_key(|f| f.uid);
    Ok(fetches
        .iter()
        .filter_map(|f| {
            let flags = flag_state(f);
            let body = f.body().filter(|b| b.len() <= max)?;
            Some(Exported { raw: body.to_vec(), seen: !flags.unread, answered: flags.answered, flagged: flags.flagged, date: f.internal_date().map(|d| d.timestamp()) })
        })
        .collect())
}

/// Поиск по тексту писем на сервере (IMAP SEARCH TEXT, UTF-8).
pub async fn search_server(conn: &mut Connection, folder: &str, query: &crate::search::Query) -> Result<Vec<u32>> {
    if let Some(e) = conn.ews() {
        return e.search(folder, query).await;
    }
    let Some(criteria) = query.imap_criteria() else { return Ok(Vec::new()) };
    conn.ensure_selected(folder).await?;
    let cmd = if query.needs_utf8() { format!("CHARSET UTF-8 {criteria}") } else { criteria };
    let mut uids: Vec<u32> = conn.imap()?.uid_search(cmd).await?.into_iter().collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    Ok(uids)
}

/// Заголовки найденных писем (папка открыта `search_server`).
pub async fn summaries_of(conn: &mut Connection, folder: &str, uids: &[u32]) -> Result<Vec<MessageSummary>> {
    if let Some(e) = conn.ews() {
        return e.summaries_of(folder, uids).await;
    }
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    conn.ensure_selected(folder).await?;
    index_summaries(conn, folder, uids).await
}

/// Кладёт отправленное письмо в папку «Отправленные» (\\Sent или по имени).
/// Возвращает имя папки.
/// Адрес из заголовка From (строчными).
pub fn from_address(raw: &[u8]) -> Option<String> {
    let msg = mail_parser::MessageParser::default().parse_headers(raw)?;
    msg.from()?.first()?.address().map(|a| a.trim().to_lowercase())
}

/// Где теперь письма из очереди действий без сети: для каждого — UID в папке
/// `folder` (проверка по Message-ID, а если UID уже другой — поиск по Message-ID)
/// или `None`, если письма в папке нет. Папки нет — все `None`.
pub async fn locate(conn: &mut Connection, folder: &str, targets: &[crate::offline::Target]) -> Result<Vec<Option<u32>>> {
    if let Some(e) = conn.ews() {
        return e.locate(folder, targets).await;
    }
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    // Свежий SELECT: нужен текущий UIDVALIDITY — сравнить с тем, что был при постановке в очередь.
    let validity = match conn.select(folder).await {
        Ok(mb) => mb.uid_validity,
        Err(AppError::NotFound) => return Ok(vec![None; targets.len()]),
        Err(e) => return Err(e),
    };
    let uids: Vec<u32> = targets.iter().map(|t| t.uid).filter(|u| *u > 0).collect();
    let fetches: Vec<Fetch> = if uids.is_empty() {
        Vec::new()
    } else {
        conn.imap()?.uid_fetch(uid_set(&uids), "(UID BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])").await?.try_collect().await?
    };
    let parser = MessageParser::default();
    let found: Vec<(u32, Option<String>)> = fetches
        .iter()
        .filter_map(|f| {
            let id = f.header().and_then(|h| parser.parse_headers(h)).and_then(|m| m.message_id().map(str::to_string));
            f.uid.map(|u| (u, id))
        })
        .collect();
    let mut out = Vec::with_capacity(targets.len());
    for t in targets {
        let direct = crate::offline::resolve(t, &found, None, validity);
        if direct.is_some() {
            out.push(direct);
            continue;
        }
        let id = t.msg_id.as_deref().map(crate::offline::norm_id).filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\'));
        let searched = match id {
            Some(id) => conn.imap()?.uid_search(format!("HEADER Message-ID \"{id}\"")).await?.into_iter().max(),
            None => None,
        };
        out.push(searched);
    }
    Ok(out)
}

/// UIDNEXT папки (STATUS, без открытия): по нему видно, пришли ли новые письма.
pub async fn uid_next(conn: &mut Connection, folder: &str) -> Result<u32> {
    if let Some(e) = conn.ews() {
        return e.uid_next(folder).await;
    }
    let mb = conn.imap()?.status(folder, "(UIDNEXT UIDVALIDITY)").await?;
    Ok(mb.uid_next.unwrap_or(0))
}

/// UID писем папки начиная с `from` (новые письма).
pub async fn uids_from(conn: &mut Connection, folder: &str, from: u32) -> Result<Vec<u32>> {
    if let Some(e) = conn.ews() {
        return e.uids_from(folder, from).await;
    }
    conn.ensure_selected(folder).await?;
    let mut uids: Vec<u32> = conn.imap()?.uid_search(format!("UID {from}:*")).await?.into_iter().filter(|u| *u >= from).collect();
    uids.sort_unstable();
    Ok(uids)
}

