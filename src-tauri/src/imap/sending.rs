//! Отправка и её следы: «Отправленные», черновики, отложенные, шаблоны, напоминания,
//! флаги и перемещение писем.
//!
//! Часть модуля `imap` — общий обзор в `mod.rs`.

use super::prelude::*;
use super::*;

pub async fn append_sent(conn: &mut Connection, raw: &[u8]) -> Result<String> {
    if let Some(e) = conn.ews() {
        let folder = e.role_folder(FolderRole::Sent).await?.ok_or_else(|| AppError::Other("Папка «Отправленные» не найдена".into()))?;
        e.append(&folder, raw, true).await?;
        return Ok(folder);
    }
    // От имени общего ящика — копия в его «Отправленные», если она есть и доступна.
    if let Some(from) = from_address(raw) {
        if let Ok(folders) = list_folders(conn).await {
            let prefix = format!("shared/{from}/").to_lowercase();
            if let Some(f) = folders.iter().find(|f| f.role == FolderRole::Sent && f.path.to_lowercase().starts_with(&prefix)) {
                conn.selected = None;
                if conn.imap()?.append(&f.id, Some("(\\Seen)"), None, raw).await.is_ok() {
                    return Ok(f.id.clone());
                }
            }
        }
    }
    let folder = role_folder(conn, FolderRole::Sent).await?.ok_or_else(|| AppError::Other("Папка «Отправленные» не найдена".into()))?;
    conn.selected = None;
    conn.imap()?.append(&folder, Some("(\\Seen)"), None, raw).await?;
    Ok(folder)
}

/// Отправить письмо средствами почтового сервера (Exchange: с копией в
/// «Отправленные»). У ящиков IMAP так не отправляется — для них SMTP.
pub async fn send_via_server(conn: &mut Connection, raw: &[u8]) -> Result<()> {
    match conn.ews() {
        Some(e) => e.send(raw).await,
        None => Err(AppError::Other("Отправка через IMAP не поддерживается".into())),
    }
}

/// Положить письмо в «Черновики» (отложенная отправка). Возвращает имя папки.
pub async fn append_draft(conn: &mut Connection, raw: &[u8]) -> Result<String> {
    if let Some(e) = conn.ews() {
        let folder = e.role_folder(FolderRole::Drafts).await?.ok_or_else(|| AppError::Other("Папка «Черновики» не найдена".into()))?;
        e.append(&folder, raw, false).await?;
        return Ok(folder);
    }
    let folder = match role_folder(conn, FolderRole::Drafts).await? {
        Some(f) => f,
        None => {
            conn.imap()?.create("Drafts").await?;
            "Drafts".to_string()
        }
    };
    conn.selected = None;
    conn.imap()?.append(&folder, Some("(\\Seen \\Draft)"), None, raw).await?;
    Ok(folder)
}

/// Заголовок черновика, который программа сохраняет сама: по нему находятся и заменяются
/// прежние версии того же письма.
pub const DRAFT_KEY_HEADER: &str = "X-Atom-Draft-Key";

/// Ключ черновика: только латиница, цифры и «-» (попадает в команду IMAP SEARCH).
pub fn valid_draft_key(key: &str) -> bool {
    (8..=64).contains(&key.len()) && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Письма «Черновиков» с этим ключом (новые в конце).
async fn draft_versions(conn: &mut Connection, folder: &str, key: &str) -> Result<Vec<u32>> {
    conn.ensure_selected(folder).await?;
    let mut uids: Vec<u32> = conn.imap()?.uid_search(format!("UNDELETED HEADER {DRAFT_KEY_HEADER} \"{key}\"")).await?.into_iter().collect();
    uids.sort_unstable();
    Ok(uids)
}

/// Сохранить черновик: новая версия — в «Черновики», прежние версии (тот же ключ) и
/// черновик, с которого начали правку (`replace`), — удалить. Возвращает папку.
pub async fn save_draft(conn: &mut Connection, raw: &[u8], key: &str, replace: Option<(&str, u32)>) -> Result<String> {
    if !valid_draft_key(key) {
        return Err(AppError::Other("Некорректный ключ черновика".into()));
    }
    if conn.is_ews() {
        return Err(AppError::Other("Для Exchange черновик пока сохраняется только на этом компьютере".into()));
    }
    let folder = append_draft(conn, raw).await?;
    let mut uids = draft_versions(conn, &folder, key).await?;
    uids.pop(); // только что сохранённая
    if let Some((f, u)) = replace {
        if f == folder && !uids.contains(&u) {
            uids.push(u);
        }
    }
    if !uids.is_empty() {
        let _: Vec<Fetch> = conn.imap()?.uid_store(uid_set(&uids), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
        expunge_uids(conn, &uids).await?;
    }
    if let Some((f, u)) = replace.filter(|(f, _)| *f != folder) {
        remove_message(conn, f, u).await?;
    }
    Ok(folder)
}

/// Удалить все версии черновика (отправлен или отброшен).
pub async fn discard_draft(conn: &mut Connection, key: &str, replace: Option<(&str, u32)>) -> Result<()> {
    if conn.is_ews() || !valid_draft_key(key) {
        if let Some((f, u)) = replace {
            remove_message(conn, f, u).await?;
        }
        return Ok(());
    }
    let Some(folder) = role_folder(conn, FolderRole::Drafts).await? else { return Ok(()) };
    let mut uids = draft_versions(conn, &folder, key).await?;
    if let Some((f, u)) = replace {
        if f == folder && !uids.contains(&u) {
            uids.push(u);
        } else if f != folder {
            remove_message(conn, f, u).await?;
        }
    }
    if !uids.is_empty() {
        conn.ensure_selected(&folder).await?;
        let _: Vec<Fetch> = conn.imap()?.uid_store(uid_set(&uids), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
        expunge_uids(conn, &uids).await?;
    }
    Ok(())
}

/// Заголовок отложенной отправки (время в RFC 3339).
pub const SEND_AT_HEADER: &str = "X-Atom-Send-At";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scheduled {
    pub folder: String,
    pub uid: u32,
    pub send_at: String,
    pub subject: String,
    pub to: Vec<String>,
    /// `pending` — ждёт; `sending` — сейчас отправляется (другой копией программы);
    /// `unknown` — неизвестно, принял ли SMTP-сервер письмо (автоматически не повторяется);
    /// `sent` — отправлено, но ещё не убрано из «Черновиков».
    pub status: &'static str,
}

/// Метка: SMTP-сервер принял письмо (повторно оно не отправляется).
pub const SENT_MARK: &str = "atomsent";
/// Метка: результат отправки неизвестен (обрыв связи после начала передачи).
pub const UNKNOWN_MARK: &str = "atomunknown";
/// Захват старше этого срока означает, что отправка оборвалась на полпути.
const CLAIM_TTL_SECS: i64 = 600;

/// Время захвата из метки `atomclaim<секунды>…`.
fn claim_time(keyword: &str) -> Option<i64> {
    keyword.get("atomclaim".len()..)?.chars().take(10).collect::<String>().parse().ok()
}

/// Состояние отложенного письма по его меткам (в нижнем регистре).
pub fn send_status(keywords: &[String], now: i64) -> &'static str {
    if keywords.iter().any(|k| k == SENT_MARK) {
        return "sent";
    }
    if keywords.iter().any(|k| k == UNKNOWN_MARK) {
        return "unknown";
    }
    let claims: Vec<i64> = keywords.iter().filter(|k| k.starts_with("atomclaim")).map(|k| claim_time(k).unwrap_or(0)).collect();
    if claims.is_empty() {
        "pending"
    } else if claims.iter().any(|&t| t >= now - CLAIM_TTL_SECS) {
        "sending"
    } else {
        // Захват остался, а письмо не отправлено и не освобождено: программа
        // прервалась во время отправки — ушло ли письмо, неизвестно.
        "unknown"
    }
}

fn custom_keywords(f: &Fetch) -> Vec<String> {
    f.flags()
        .filter_map(|x| match x {
            Flag::Custom(k) => Some(k.to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// Письма в «Черновиках», ожидающие отложенной отправки.
pub async fn scheduled(conn: &mut Connection) -> Result<Vec<Scheduled>> {
    if conn.is_ews() {
        return Ok(Vec::new());
    }
    let Some(folder) = role_folder(conn, FolderRole::Drafts).await? else { return Ok(Vec::new()) };
    conn.ensure_selected(&folder).await?;
    let uids: Vec<u32> = conn.imap()?.uid_search(format!("UNDELETED HEADER {SEND_AT_HEADER} \"\"")).await?.into_iter().collect();
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let fetches: Vec<Fetch> = conn
        .imap()?
        .uid_fetch(uid_set(&uids), format!("(UID FLAGS BODY.PEEK[HEADER.FIELDS ({SEND_AT_HEADER} SUBJECT TO)])"))
        .await?
        .try_collect()
        .await?;
    let parser = MessageParser::default();
    let now = chrono::Utc::now().timestamp();
    let mut out: Vec<Scheduled> = fetches
        .iter()
        .filter_map(|f| {
            let uid = f.uid?;
            let msg = parser.parse_headers(f.header()?)?;
            let send_at = msg.header_raw(SEND_AT_HEADER)?.trim().to_string();
            chrono::DateTime::parse_from_rfc3339(&send_at).ok()?;
            Some(Scheduled {
                folder: folder.clone(),
                uid,
                send_at,
                subject: msg.subject().unwrap_or_default().to_string(),
                to: addresses(msg.to()),
                status: send_status(&custom_keywords(f), now),
            })
        })
        .collect();
    out.sort_by(|a, b| a.send_at.cmp(&b.send_at));
    Ok(out)
}

/// Захват отложенного письма для отправки, чтобы две запущенные копии программы
/// (два компьютера) не отправили его дважды: каждая ставит своё ключевое слово,
/// ждёт и смотрит, чья метка меньше. Возвращает true, если отправляет эта копия.
pub async fn claim(conn: &mut Connection, folder: &str, uid: u32, token: &str) -> Result<bool> {
    conn.ensure_selected(folder).await?;
    let _: Vec<Fetch> = conn.imap()?.uid_store(uid.to_string(), format!("+FLAGS.SILENT (atomclaim{token})")).await?.try_collect().await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let fetches: Vec<Fetch> = conn.imap()?.uid_fetch(uid.to_string(), "(UID FLAGS)").await?.try_collect().await?;
    let Some(f) = fetches.iter().find(|f| f.uid == Some(uid)) else { return Ok(false) };
    if f.flags().any(|x| matches!(x, Flag::Deleted)) {
        return Ok(false);
    }
    let keywords = custom_keywords(f);
    drop(fetches);
    let own = format!("atomclaim{token}").to_ascii_lowercase();
    // Уже отправлено или результат неизвестен — второй раз не отправляем. Метка
    // начинается со времени захвата: чужой «зависший» захват (старше 10 минут)
    // означает прерванную отправку — такое письмо тоже не отправляется заново
    // автоматически (его состояние «неизвестно», решает пользователь).
    let fresh_after = chrono::Utc::now().timestamp() - CLAIM_TTL_SECS;
    let settled = keywords.iter().any(|k| k == SENT_MARK || k == UNKNOWN_MARK);
    let interrupted = keywords.iter().any(|k| k.starts_with("atomclaim") && *k != own && claim_time(k).unwrap_or(0) < fresh_after);
    if settled || interrupted {
        let _: Vec<Fetch> = conn.imap()?.uid_store(uid.to_string(), format!("-FLAGS.SILENT (atomclaim{token})")).await?.try_collect().await?;
        return Ok(false);
    }
    let mut claims: Vec<String> = keywords.into_iter().filter(|k| k.starts_with("atomclaim")).collect();
    claims.sort();
    Ok(claims.first().is_some_and(|c| *c == own))
}

/// Пользователь сам решил отправить письмо с неизвестным результатом ещё раз:
/// снимаются метка «неизвестно» и все захваты.
pub async fn clear_send_marks(conn: &mut Connection, folder: &str, uid: u32) -> Result<()> {
    conn.ensure_selected(folder).await?;
    let fetches: Vec<Fetch> = conn.imap()?.uid_fetch(uid.to_string(), "(UID FLAGS)").await?.try_collect().await?;
    let Some(f) = fetches.iter().find(|f| f.uid == Some(uid)) else { return Err(AppError::NotFound) };
    let marks: Vec<String> = f
        .flags()
        .filter_map(|x| match x {
            Flag::Custom(k) if k.eq_ignore_ascii_case(UNKNOWN_MARK) || k.to_ascii_lowercase().starts_with("atomclaim") => Some(k.to_string()),
            _ => None,
        })
        .collect();
    drop(fetches);
    if marks.is_empty() {
        return Ok(());
    }
    let _: Vec<Fetch> = conn.imap()?.uid_store(uid.to_string(), format!("-FLAGS.SILENT ({})", marks.join(" "))).await?.try_collect().await?;
    Ok(())
}

/// Удалить письмо из папки (после отправки отложенного) — только это письмо.
pub async fn remove_message(conn: &mut Connection, folder: &str, uid: u32) -> Result<()> {
    if let Some(e) = conn.ews() {
        return e.delete_messages(folder, &[uid], None).await.map(|_| ());
    }
    conn.ensure_selected(folder).await?;
    let _: Vec<Fetch> = conn.imap()?.uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
    expunge_uids(conn, &[uid]).await
}

// ---------------------------------------------------------------- шаблоны и сроки флажков

/// Префикс ключевого слова со сроком флажка: `atomdue-ГГГГММДДЧЧММ` (UTC).
pub const DUE_PREFIX: &str = "atomdue-";

pub fn due_keyword(at: &chrono::DateTime<chrono::Utc>) -> String {
    format!("{DUE_PREFIX}{}", at.format("%Y%m%d%H%M"))
}

pub fn due_of(keywords: &[String]) -> Option<chrono::DateTime<chrono::Utc>> {
    keywords.iter().find_map(|k| {
        let rest = k.strip_prefix(DUE_PREFIX)?;
        chrono::NaiveDateTime::parse_from_str(rest, "%Y%m%d%H%M").ok().map(|n| n.and_utc())
    })
}

/// Флажок со сроком (или без срока): прежние сроки снимаются.
pub async fn set_due(conn: &mut Connection, folder: &str, uid: u32, due: Option<chrono::DateTime<chrono::Utc>>) -> Result<()> {
    if let Some(e) = conn.ews() {
        let old: Vec<String> = e.summaries_of(folder, &[uid]).await?.into_iter().next().map(|m| m.keywords.into_iter().filter(|k| k.starts_with(DUE_PREFIX)).collect()).unwrap_or_default();
        for k in old {
            e.set_keyword(folder, &[uid], &k, false).await?;
        }
        e.set_flag(folder, &[uid], FlagKind::Flagged, true).await?;
        if let Some(at) = due {
            e.set_keyword(folder, &[uid], &due_keyword(&at), true).await?;
        }
        return Ok(());
    }
    conn.ensure_selected(folder).await?;
    let fetches: Vec<Fetch> = conn.imap()?.uid_fetch(uid.to_string(), "(UID FLAGS)").await?.try_collect().await?;
    let old: Vec<String> = fetches
        .iter()
        .find(|f| f.uid == Some(uid))
        .map(|f| f.flags().filter_map(|x| match x { Flag::Custom(k) if k.starts_with(DUE_PREFIX) => Some(k.to_string()), _ => None }).collect())
        .unwrap_or_default();
    if !old.is_empty() {
        let _: Vec<Fetch> = conn.imap()?.uid_store(uid.to_string(), format!("-FLAGS.SILENT ({})", old.join(" "))).await?.try_collect().await?;
    }
    let mut add = vec!["\\Flagged".to_string()];
    if let Some(at) = due {
        add.push(due_keyword(&at));
    }
    let _: Vec<Fetch> = conn.imap()?.uid_store(uid.to_string(), format!("+FLAGS.SILENT ({})", add.join(" "))).await?.try_collect().await?;
    Ok(())
}

/// Письма с флажком и сроком в папке.
pub async fn due_in(conn: &mut Connection, folder: &str) -> Result<Vec<MessageSummary>> {
    if let Some(e) = conn.ews() {
        let q = crate::search::Query { flagged: true, ..Default::default() };
        let uids = e.search(folder, &q).await?;
        let items = e.summaries_of(folder, &uids).await?;
        return Ok(items.into_iter().filter(|m| m.keywords.iter().any(|k| k.starts_with(DUE_PREFIX))).collect());
    }
    conn.ensure_selected(folder).await?;
    let uids: Vec<u32> = conn.imap()?.uid_search("FLAGGED UNDELETED").await?.into_iter().collect();
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let flags: Vec<Fetch> = conn.imap()?.uid_fetch(uid_set(&uids), "(UID FLAGS)").await?.try_collect().await?;
    let with_due: Vec<u32> = flags
        .iter()
        .filter(|f| f.flags().any(|x| matches!(x, Flag::Custom(k) if k.starts_with(DUE_PREFIX))))
        .filter_map(|f| f.uid)
        .collect();
    index_summaries(conn, folder, &with_due).await
}

// ---------------------------------------------------------------- спам и «отложить»

/// Префикс ключевого слова «вернуть во Входящие»: `atomsnooze-ГГГГММДДЧЧММ` (UTC).
pub const SNOOZE_PREFIX: &str = "atomsnooze-";

/// Письма «Отложенных», срок которых наступил.
pub async fn snoozed_due(conn: &mut Connection, folder: &str, now: chrono::DateTime<chrono::Utc>) -> Result<Vec<u32>> {
    conn.ensure_selected(folder).await?;
    let uids: Vec<u32> = conn.imap()?.uid_search("UNDELETED").await?.into_iter().collect();
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let fetches: Vec<Fetch> = conn.imap()?.uid_fetch(uid_set(&uids), "(UID FLAGS)").await?.try_collect().await?;
    Ok(fetches
        .iter()
        .filter_map(|f| {
            let until = f.flags().find_map(|x| match x {
                Flag::Custom(k) => k.strip_prefix(SNOOZE_PREFIX).and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y%m%d%H%M").ok()),
                _ => None,
            })?;
            (until.and_utc() <= now).then_some(f.uid?)
        })
        .collect())
}

/// Снять все ключевые слова «отложено» у писем.
pub async fn clear_snooze(conn: &mut Connection, folder: &str, uids: &[u32]) -> Result<()> {
    conn.ensure_selected(folder).await?;
    let fetches: Vec<Fetch> = conn.imap()?.uid_fetch(uid_set(uids), "(UID FLAGS)").await?.try_collect().await?;
    let kws: std::collections::BTreeSet<String> = fetches
        .iter()
        .flat_map(|f| f.flags().filter_map(|x| match x { Flag::Custom(k) if k.starts_with(SNOOZE_PREFIX) => Some(k.to_string()), _ => None }).collect::<Vec<_>>())
        .collect();
    if !kws.is_empty() {
        let list: Vec<String> = kws.into_iter().collect();
        let _: Vec<Fetch> = conn.imap()?.uid_store(uid_set(uids), format!("-FLAGS.SILENT ({})", list.join(" "))).await?.try_collect().await?;
    }
    Ok(())
}

/// Поставить или снять ключевое слово (категория, $MDNSent) у писем.
pub async fn set_keyword(conn: &mut Connection, folder: &str, uids: &[u32], keyword: &str, value: bool) -> Result<()> {
    if !valid_keyword(keyword) {
        return Err(AppError::Other("Недопустимое имя метки".into()));
    }
    if let Some(e) = conn.ews() {
        return e.set_keyword(folder, uids, keyword, value).await;
    }
    if !valid_keyword(keyword) {
        return Err(AppError::Other("Недопустимое имя метки".into()));
    }
    conn.ensure_selected(folder).await?;
    let op = if value { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
    let _: Vec<Fetch> = conn.imap()?.uid_store(uid_set(uids), format!("{op} ({keyword})")).await?.try_collect().await?;
    Ok(())
}

/// Ключевое слово IMAP (atom): без пробелов, скобок, кавычек и т. п.; не системный флаг.
pub fn valid_keyword(k: &str) -> bool {
    !k.is_empty() && k.len() <= 64 && !k.starts_with('\\') && k.bytes().all(|b| b.is_ascii_graphic() && !b"(){%*\"\\]".contains(&b))
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlagKind {
    Seen,
    Flagged,
    Important,
}

/// Ключевое слово «Важное» — совместимо с меткой «Важное» веб-почты SOGo.
pub const IMPORTANT: &str = "$label1";

/// Флаг сразу у нескольких писем.
pub async fn set_flag_many(conn: &mut Connection, folder: &str, uids: &[u32], flag: FlagKind, value: bool) -> Result<()> {
    if let Some(e) = conn.ews() {
        return e.set_flag(folder, uids, flag, value).await;
    }
    for uid in uids {
        set_flag(conn, folder, *uid, flag, value).await?;
    }
    Ok(())
}

pub async fn set_flag(
    conn: &mut Connection,
    folder: &str,
    uid: u32,
    flag: FlagKind,
    value: bool,
) -> Result<()> {
    if let Some(e) = conn.ews() {
        return e.set_flag(folder, &[uid], flag, value).await;
    }
    conn.ensure_selected(folder).await?;
    let name = match flag {
        FlagKind::Seen => "\\Seen",
        FlagKind::Flagged => "\\Flagged",
        FlagKind::Important if value => IMPORTANT,
        FlagKind::Important => "$label1 $Important",
    };
    let op = if value { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
    let _: Vec<Fetch> = conn
        .imap()?
        .uid_store(uid.to_string(), format!("{op} ({name})"))
        .await?
        .try_collect()
        .await?;
    Ok(())
}

// ---------------------------------------------------------------- перемещение и удаление

pub(crate) fn uid_set(uids: &[u32]) -> String {
    uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

/// Удаляет помеченные \Deleted письма: только указанные (UID EXPUNGE, если
/// сервер умеет UIDPLUS), иначе — все помеченные в папке.
/// Окончательно удаляет только письма `uids` (у них уже стоит \\Deleted).
async fn expunge_uids(conn: &mut Connection, uids: &[u32]) -> Result<()> {
    // E2E: проверка запасного пути на сервере, который UIDPLUS поддерживает. В релизе этого нет.
    #[cfg(debug_assertions)]
    let force_plain = std::env::var_os("RRCKI_E2E_NO_UIDPLUS").is_some();
    #[cfg(not(debug_assertions))]
    let force_plain = false;
    if !force_plain && has_capability(conn, "UIDPLUS").await {
        let _: Vec<_> = conn.imap()?.uid_expunge(uid_set(uids)).await?.try_collect().await?;
        return Ok(());
    }
    // Без UIDPLUS есть только EXPUNGE, а он удаляет ВСЕ письма папки с \\Deleted —
    // в том числе помеченные к удалению другим клиентом (Outlook, веб-почта).
    // Поэтому на время EXPUNGE снимаем \\Deleted с чужих писем и затем возвращаем.
    let ours: std::collections::HashSet<u32> = uids.iter().copied().collect();
    let others: Vec<u32> = conn.imap()?.uid_search("DELETED").await?.into_iter().filter(|u| !ours.contains(u)).collect();
    if !others.is_empty() {
        let _: Vec<Fetch> = conn.imap()?.uid_store(uid_set(&others), "-FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
    }
    let result: Result<()> = async {
        let _: Vec<_> = conn.imap()?.expunge().await?.try_collect().await?;
        Ok(())
    }
    .await;
    if !others.is_empty() {
        let restored: Result<Vec<Fetch>> = async {
            Ok(conn.imap()?.uid_store(uid_set(&others), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?)
        }
        .await;
        if let Err(e) = restored {
            log::warn!("не удалось вернуть \\Deleted письмам {:?}: {e}", others);
        }
    }
    result
}

/// Перемещает письма в другую папку (UID MOVE, без него — COPY + удаление).
pub async fn move_messages(conn: &mut Connection, folder: &str, uids: &[u32], target: &str) -> Result<()> {
    if let Some(e) = conn.ews() {
        return e.move_messages(folder, uids, target).await;
    }
    if uids.is_empty() || folder == target {
        return Ok(());
    }
    let can_move = has_capability(conn, "MOVE").await;
    conn.ensure_selected(folder).await?;
    if can_move {
        conn.imap()?.uid_mv(uid_set(uids), target).await?;
    } else {
        conn.imap()?.uid_copy(uid_set(uids), target).await?;
        let _: Vec<Fetch> = conn.imap()?.uid_store(uid_set(uids), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
        expunge_uids(conn, uids).await?;
    }
    Ok(())
}

/// Удаление: в «Корзину» (`trash`), а из самой корзины (или если её нет) — насовсем.
/// Возвращает `true`, если письма удалены безвозвратно.
pub async fn delete_messages(conn: &mut Connection, folder: &str, uids: &[u32], trash: Option<&str>) -> Result<bool> {
    if let Some(e) = conn.ews() {
        return e.delete_messages(folder, uids, trash).await;
    }
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
            let _: Vec<Fetch> = conn.imap()?.uid_store(uid_set(uids), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
            expunge_uids(conn, uids).await?;
            Ok(true)
        }
    }
}

// ---------------------------------------------------------------- диагностика

