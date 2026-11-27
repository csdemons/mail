//! Talk: беседы, сообщения, реакции, опросы, напоминания и файлы в беседе.
//!
//! Часть модуля `nextcloud` — общий обзор в `mod.rs`.

use super::*;

pub(crate) const TALK_ROOM: &str = "ocs/v2.php/apps/spreed/api/v4/room";
// Ветки, отложенные сообщения, архив и отметки бесед, «Заметки для себя».
pub(crate) const TALK_CHAT: &str = "ocs/v2.php/apps/spreed/api/v1/chat/";
pub(crate) const TALK_POLL: &str = "ocs/v2.php/apps/spreed/api/v1/poll/";
/// Порция истории беседы (более ранние — следующим запросом с lastKnownMessageId).
pub(crate) const TALK_PAGE: usize = 100;


/// Найденное сообщение беседы (поиск Nextcloud по сообщениям Talk).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkSearchHit {
    pub message_id: i64,
    pub author: String,
    pub text: String,
    pub timestamp: i64,
}

/// Участник беседы (список, как в Talk).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkParticipant {
    pub attendee_id: i64,
    /// users, groups, emails, guests, federated_users…
    pub actor_type: String,
    pub actor_id: String,
    pub name: String,
    pub role: i64,
    /// online, away, dnd, offline или пусто.
    pub status: String,
    pub status_message: String,
    pub me: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkMessage {
    pub id: i64,
    pub author: String,
    pub author_id: String,
    pub own: bool,
    pub timestamp: i64,
    pub text: String,
    pub system: bool,
    pub parent_text: Option<String>,
    pub parent_author: Option<String>,
    pub files: Vec<TalkFile>,
    /// Реакции: эмодзи, сколько и поставил ли я.
    pub reactions: Vec<TalkReaction>,
    pub edited: bool,
    pub deleted: bool,
    /// Текст для правки: упоминания в виде `@"id"`, как их понимает Talk.
    pub edit_text: Option<String>,
    /// Это обновление уже показанного сообщения (реакция, правка, удаление).
    pub update: bool,
    /// Опрос в сообщении (`{object}` типа talk-poll): id и вопрос.
    pub poll: Option<TalkPollRef>,
    /// Голосовое сообщение (messageType voice-message).
    pub voice: bool,
    /// Сообщение исчезнет в это время (unix, с); 0 — не исчезнет.
    pub expires: i64,
    /// Закреплено: кем и до какого времени (0 — бессрочно).
    pub pinned: bool,
    pub pinned_by: String,
    pub pinned_until: i64,
    /// Ветка (Talk 22): к какой относится (0 — ни к какой), это её начало, название, ответов.
    pub thread_id: i64,
    pub is_thread: bool,
    pub thread_title: String,
    pub thread_replies: i64,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TalkPollRef {
    pub id: i64,
    pub question: String,
}

/// Опрос Talk с результатами (что видно этому пользователю).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkPoll {
    pub id: i64,
    pub question: String,
    pub options: Vec<String>,
    /// Голоса по вариантам (если результаты видны).
    pub votes: Vec<i64>,
    pub voters: i64,
    /// За какие варианты голосовал я.
    pub voted: Vec<i64>,
    pub closed: bool,
    /// Результаты скрыты до закрытия опроса.
    pub hidden: bool,
    /// 0 — сколько угодно вариантов, 1 — один.
    pub max_votes: i64,
    pub author: String,
    pub own: bool,
}

/// Предпросмотр ссылки (общий сервис ссылок Nextcloud).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreview {
    pub url: String,
    pub title: String,
    pub description: String,
    /// Картинка страницы (data:URL), если сервер её дал.
    pub image: Option<String>,
    pub site: String,
}

/// Порция сообщений и наибольший id, пришедший с сервера (включая служебные),
/// — от него продолжается опрос.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkChunk {
    pub messages: Vec<TalkMessage>,
    pub last_id: i64,
    /// Заголовок X-Chat-Last-Common-Read: до какого сообщения прочитали все; 0 — нет данных.
    pub last_common_read: i64,
    /// История: сервер отдал полную порцию — вероятно, есть более ранние сообщения.
    pub more: bool,
}

fn common_read(h: &reqwest::header::HeaderMap) -> i64 {
    h.get("X-Chat-Last-Common-Read").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TalkReaction {
    pub emoji: String,
    pub count: i64,
    pub own: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkMention {
    /// Что вставить в сообщение: `@"id"`.
    pub insert: String,
    pub label: String,
    pub source: String,
}

/// Файл, опубликованный в беседе (параметр `{file}` сообщения).
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TalkFile {
    /// ID файла в Nextcloud — открывается по ссылке `/f/<id>`.
    pub id: String,
    pub name: String,
    pub size: i64,
    pub mimetype: String,
    /// Путь файла у текущего пользователя (обычно «Talk/имя»): по нему файл
    /// скачивается и открывается в просмотрщике.
    pub path: Option<String>,
    /// Сервер умеет делать уменьшенную копию (фото, PDF…).
    pub preview: bool,
}

/// Подставляет `{mention-user1}`, `{file}` и т.п. из messageParameters.
pub fn render_message(v: &Value) -> (String, Vec<TalkFile>) {
    let mut out = text(&v["message"]);
    let mut files = Vec::new();
    if let Some(params) = v["messageParameters"].as_object() {
        for (key, p) in params {
            let name = p["name"].as_str().unwrap_or_default();
            let ty = p["type"].as_str().unwrap_or_default();
            let shown = match ty {
                "user" | "guest" | "call" | "user-group" | "group" => format!("@{name}"),
                "file" => {
                    // Файл показывается отдельной плашкой под текстом.
                    let path = p["path"].as_str().map(|x| x.trim_start_matches('/').to_string()).filter(|x| !x.is_empty());
                    let preview = p["preview-available"].as_str() == Some("yes");
                    files.push(TalkFile { id: text(&p["id"]).or_int(&p["id"]), name: name.to_string(), size: int(&p["size"]), mimetype: text(&p["mimetype"]), path, preview });
                    String::new()
                }
                // Опрос показывается карточкой; в тексте — пусто.
                "talk-poll" => String::new(),
                _ => name.to_string(),
            };
            out = out.replace(&format!("{{{key}}}"), &shown);
        }
    }
    (out.trim().to_string(), files)
}

pub(crate) trait OrInt {
    fn or_int(self, v: &Value) -> String;
}
impl OrInt for String {
    /// ID приходит то строкой, то числом.
    fn or_int(self, v: &Value) -> String {
        if self.is_empty() { v.as_i64().map(|n| n.to_string()).unwrap_or_default() } else { self }
    }
}

/// «отчёт.pdf» → «отчёт (2).pdf»
pub fn numbered_name(name: &str, n: u32) -> String {
    if n == 0 { return name.to_string(); }
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} ({n}){}", &name[..i], &name[i..]),
        _ => format!("{name} ({n})"),
    }
}

/// Текст для правки: `{mention-user1}` → `@"id"`.
pub(crate) fn edit_text(v: &Value) -> String {
    let mut out = text(&v["message"]);
    if let Some(params) = v["messageParameters"].as_object() {
        for (key, p) in params {
            let ty = p["type"].as_str().unwrap_or_default();
            let id = p.get("mention-id").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| text(&p["id"]).or_int(&p["id"]));
            let shown = match ty {
                "user" | "guest" | "user-group" | "group" => format!("@\"{id}\""),
                "call" => "@all".to_string(),
                _ => p["name"].as_str().unwrap_or_default().to_string(),
            };
            out = out.replace(&format!("{{{key}}}"), &shown);
        }
    }
    out
}

pub(crate) fn reactions(v: &Value) -> Vec<TalkReaction> {
    let own: Vec<String> = v["reactionsSelf"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let mut out: Vec<TalkReaction> = v["reactions"]
        .as_object()
        .map(|m| m.iter().map(|(e, n)| TalkReaction { emoji: e.clone(), count: int(n), own: own.contains(e) }).filter(|r| r.count > 0).collect())
        .unwrap_or_default();
    out.sort_by_key(|r| std::cmp::Reverse(r.count));
    out
}

pub fn parse_message(v: &Value, me: &str) -> TalkMessage {
    let (text_out, files) = render_message(v);
    let parent = v.get("parent").filter(|p| p.is_object());
    let actor_id = text(&v["actorId"]);
    let deleted = v["messageType"].as_str() == Some("comment_deleted");
    let system = !deleted && (v["messageType"].as_str() == Some("system") || !v["systemMessage"].as_str().unwrap_or_default().is_empty());
    let own = actor_id.eq_ignore_ascii_case(me) && v["actorType"].as_str() == Some("users");
    TalkMessage {
        id: int(&v["id"]),
        author: text(&v["actorDisplayName"]),
        own,
        author_id: actor_id,
        timestamp: int(&v["timestamp"]),
        system,
        text: if deleted { "Сообщение удалено".into() } else { text_out },
        parent_text: parent.map(|p| render_message(p).0),
        parent_author: parent.map(|p| text(&p["actorDisplayName"])),
        edit_text: (own && !system && !deleted && files.is_empty()).then(|| edit_text(v)),
        files,
        reactions: if deleted { Vec::new() } else { reactions(v) },
        edited: int(&v["lastEditTime"]) > 0,
        deleted,
        update: false,
        poll: poll_ref(v),
        voice: v["messageType"].as_str() == Some("voice-message"),
        expires: int(&v["expirationTimestamp"]),
        pinned: v["metaData"]["pinnedAt"].is_number(),
        pinned_by: text(&v["metaData"]["pinnedActorDisplayName"]),
        pinned_until: int(&v["metaData"]["pinnedUntil"]),
        thread_id: int(&v["threadId"]),
        is_thread: v["isThread"].as_bool().unwrap_or(false),
        thread_title: text(&v["threadTitle"]),
        thread_replies: int(&v["threadReplies"]),
    }
}

fn poll_ref(v: &Value) -> Option<TalkPollRef> {
    let o = v["messageParameters"].as_object()?.values().find(|p| p["type"].as_str() == Some("talk-poll"))?;
    let id = o["id"].as_str().and_then(|x| x.parse().ok()).or_else(|| o["id"].as_i64())?;
    Some(TalkPollRef { id, question: text(&o["name"]) })
}

fn parse_poll(d: &Value, me: &str) -> TalkPoll {
    let options: Vec<String> = d["options"].as_array().map(|a| a.iter().map(text).collect()).unwrap_or_default();
    let votes = (0..options.len()).map(|i| int(&d["votes"][format!("option-{i}")])).collect();
    TalkPoll {
        id: int(&d["id"]),
        question: text(&d["question"]),
        votes,
        voters: int(&d["numVoters"]),
        voted: d["votedSelf"].as_array().map(|a| a.iter().map(int).collect()).unwrap_or_default(),
        closed: int(&d["status"]) == 1,
        hidden: int(&d["resultMode"]) == 1,
        max_votes: int(&d["maxVotes"]),
        author: text(&d["actorDisplayName"]),
        own: d["actorType"].as_str() == Some("users") && text(&d["actorId"]).eq_ignore_ascii_case(me),
        options,
    }
}

/// Служебные сообщения о реакции/правке/удалении несут изменённое сообщение
/// в `parent` — превращаем их в обновление этого сообщения.
fn as_update(v: &Value, me: &str) -> Option<TalkMessage> {
    let kind = v["systemMessage"].as_str().unwrap_or_default();
    if !matches!(kind, "reaction" | "reaction_deleted" | "reaction_revoked" | "message_deleted" | "message_edited") {
        return None;
    }
    let parent = v.get("parent").filter(|p| p.is_object())?;
    let mut m = parse_message(parent, me);
    m.update = true;
    Some(m)
}

fn max_id(d: &Value) -> i64 {
    d.as_array().map(|a| a.iter().map(|m| int(&m["id"])).max().unwrap_or(0)).unwrap_or(0)
}

pub(crate) fn parse_chat(d: &Value, me: &str) -> Vec<TalkMessage> {
    // Ключ сортировки: обычные сообщения по id, обновления — после них, в порядке
    // служебных сообщений (последняя правка одного сообщения побеждает).
    let mut keyed: Vec<((bool, i64), TalkMessage)> = d
        .as_array()
        .map(|a| a.iter().filter_map(|m| {
            if hidden_system(m) {
                as_update(m, me).map(|u| ((true, int(&m["id"])), u))
            } else {
                let p = parse_message(m, me);
                Some(((false, p.id), p))
            }
        }).collect())
        .unwrap_or_default();
    keyed.sort_by_key(|(k, _)| *k);
    keyed.into_iter().map(|(_, m)| m).collect()
}

fn hidden_system(v: &Value) -> bool {
    // Служебные события звонков/реакций, которые в ленте не нужны.
    matches!(v["systemMessage"].as_str().unwrap_or_default(),
        "reaction" | "reaction_deleted" | "reaction_revoked" | "message_deleted" | "message_edited" | "history_cleared")
}

impl Nc {

    /// История: последние сообщения (или более старые, чем `before`). Отметку ставит видимая лента.
    pub async fn talk_history(&self, token: &str, before: Option<i64>) -> Result<TalkChunk> {
        let mut q = vec![("lookIntoFuture", "0".to_string()), ("limit", TALK_PAGE.to_string()),
            ("setReadMarker", "0".to_string()), ("markNotificationsAsRead", "0".to_string())];
        if let Some(b) = before { q.push(("lastKnownMessageId", b.to_string())); }
        let (status, h, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{}", seg(token)), &q, None, "Сообщения").await?;
        if status == StatusCode::NOT_MODIFIED { return Ok(TalkChunk { messages: Vec::new(), last_id: 0, last_common_read: common_read(&h), more: false }); }
        let more = d.as_array().is_some_and(|a| a.len() >= TALK_PAGE);
        Ok(TalkChunk { messages: parse_chat(&d, &self.me), last_id: max_id(&d), last_common_read: common_read(&h), more })
    }

    /// Ждёт новые сообщения до 30 с (long-poll). Пусто — новых нет.
    /// `mark_read` — сразу отметить прочитанным (программа отмечает сама, только если беседа ещё открыта).
    pub async fn talk_poll(&self, token: &str, last_known: i64, mark_read: bool) -> Result<TalkChunk> {
        let q = vec![("lookIntoFuture", "1".to_string()), ("lastKnownMessageId", last_known.to_string()),
            ("timeout", "30".to_string()), ("limit", "100".to_string()),
            ("setReadMarker", if mark_read { "1" } else { "0" }.to_string()),
            ("markNotificationsAsRead", if mark_read { "1" } else { "0" }.to_string())];
        let (status, h, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{}", seg(token)), &q, None, "Новые сообщения").await?;
        if status == StatusCode::NOT_MODIFIED { return Ok(TalkChunk { messages: Vec::new(), last_id: 0, last_common_read: common_read(&h), more: false }); }
        Ok(TalkChunk { messages: parse_chat(&d, &self.me), last_id: max_id(&d), last_common_read: common_read(&h), more: false })
    }

    /// `silent` — без звука уведомления у получателей («тихая отправка»); `thread` — в ветку
    /// (номер) или новая ветка с названием (Talk 22).
    pub async fn talk_send(&self, token: &str, message: &str, reply_to: Option<i64>, silent: bool, thread: &TalkThreadTarget) -> Result<()> {
        let message = message.trim();
        if message.is_empty() { return Ok(()); }
        let mut body = json!({ "message": message });
        if let Some(r) = reply_to { body["replyTo"] = json!(r); }
        if silent { body["silent"] = json!(true); }
        thread.apply(&mut body, reply_to.is_some());
        self.ocs(Method::POST, &format!("{TALK_CHAT}{}", seg(token)), &[], Some(body), "Отправка сообщения").await?;
        Ok(())
    }

    /// Отметить беседу избранной (или снять отметку) — то же, что звёздочка в Talk.
    pub async fn talk_favorite(&self, token: &str, on: bool) -> Result<()> {
        let path = format!("{TALK_ROOM}/{}/favorite", seg(token));
        self.ocs(if on { Method::POST } else { Method::DELETE }, &path, &[], None, "Избранное").await?;
        Ok(())
    }

    /// Поиск по сообщениям беседы на сервере (провайдер «talk-message-current» общего поиска
    /// Nextcloud; если его нет — общий «talk-message» с отбором по беседе).
    pub async fn talk_search(&self, token: &str, term: &str) -> Result<Vec<TalkSearchHit>> {
        let term = term.trim();
        if term.chars().count() < 2 { return Ok(Vec::new()); }
        let from = format!("/call/{token}");
        let q = |t: &str| vec![("term", t.to_string()), ("from", from.clone()), ("limit", "50".to_string())];
        let current = self.ocs(Method::GET, "ocs/v2.php/search/providers/talk-message-current/search", &q(term), None, "Поиск в беседе").await;
        let (d, filter) = match current {
            Ok((_, _, d)) => (d, false),
            Err(_) => (self.ocs(Method::GET, "ocs/v2.php/search/providers/talk-message/search", &q(term), None, "Поиск в беседе").await?.2, true),
        };
        let hits = d["entries"].as_array().map(|a| a.iter().filter(|e| !filter || e["attributes"]["conversation"].as_str() == Some(token)).filter_map(|e| {
            let a = &e["attributes"];
            let id = a["messageId"].as_str().and_then(|v| v.parse().ok()).or_else(|| a["messageId"].as_i64())?;
            Some(TalkSearchHit {
                message_id: id,
                author: text(&e["title"]),
                text: text(&e["subline"]),
                timestamp: a["timestamp"].as_str().and_then(|v| v.parse().ok()).or_else(|| a["timestamp"].as_i64()).unwrap_or(0),
            })
        }).collect()).unwrap_or_default();
        Ok(hits)
    }

    // --- Общий поиск ---

    /// Файлы по имени (WebDAV SEARCH по всему хранилищу), новые сверху.
    pub async fn files_search(&self, term: &str, limit: usize) -> Result<Vec<NcFile>> {
        let term = term.trim();
        if term.chars().count() < 2 { return Ok(Vec::new()); }
        let url = self.creds.base.join("remote.php/dav/").map_err(|e| AppError::Other(e.to_string()))?;
        let resp = self
            .dav(Method::from_bytes(b"SEARCH").expect("SEARCH"), url)
            .header("Content-Type", "text/xml; charset=utf-8")
            .body(dav_search_body(&self.me, term, limit))
            .send()
            .await
            .map_err(net_error)?;
        if resp.status().as_u16() != 207 {
            return Err(status_error(resp.status(), "Поиск файлов"));
        }
        let xml = crate::netio::text_limited(resp, crate::netio::API_LIMIT).await?;
        let root = self.files_root_path()?;
        let decoded_root = percent_encoding::percent_decode_str(&root).decode_utf8_lossy().to_string();
        let mut list = parse_file_list(&xml, &decoded_root)?;
        list.retain(|f| f.path != "/");
        list.truncate(limit);
        Ok(list)
    }

    /// Все файлы папки и её подпапок (до `depth` уровней); нет папки — пусто.
    pub async fn files_tree(&self, path: &str, depth: u32) -> Result<Vec<NcFile>> {
        let mut out = Vec::new();
        let mut level = vec![clean_path(path)?];
        for _ in 0..depth.max(1) {
            let mut next = Vec::new();
            for dir in level {
                let list = match self.files_list(&dir).await {
                    Ok(l) => l,
                    Err(AppError::Other(e)) if e.contains("404") || e.contains("не найден") => continue,
                    Err(e) => return Err(e),
                };
                for f in list {
                    if f.is_dir { next.push(f.path.clone()) } else { out.push(f) }
                }
            }
            level = next;
            if level.is_empty() { break }
        }
        Ok(out)
    }

    /// Карточки Deck, в названии или описании которых есть `term` (все доски, кроме архивных).
    pub async fn deck_search(&self, term: &str, limit: usize) -> Result<Vec<DeckHit>> {
        let t = fold(term.trim());
        if t.chars().count() < 2 { return Ok(Vec::new()); }
        let boards: Vec<DeckBoard> = self.deck_boards().await?.into_iter().filter(|b| !b.archived).collect();
        let stacks = futures::future::join_all(boards.iter().map(|b| self.deck_stacks(b.id))).await;
        let mut out = Vec::new();
        for (b, st) in boards.iter().zip(stacks) {
            let Ok(st) = st else { continue };
            for s in st {
                for c in s.cards {
                    if fold(&c.title).contains(&t) || fold(&c.description).contains(&t) {
                        out.push(DeckHit { board_id: b.id, board: b.title.clone(), stack_id: s.id, stack: s.title.clone(), card: c });
                    }
                }
            }
        }
        out.sort_by_key(|h| std::cmp::Reverse(h.card.last_modified.max(h.card.created_at)));
        out.truncate(limit);
        Ok(out)
    }

    /// Чат: беседы по названию и сообщения во всех беседах (поиск Nextcloud), новые сверху.
    pub async fn chat_search(&self, term: &str, limit: usize) -> Result<Vec<ChatHit>> {
        let term = term.trim();
        if term.chars().count() < 2 { return Ok(Vec::new()); }
        let rooms = self.talk_rooms().await?;
        let t = fold(term);
        let mut out: Vec<ChatHit> = rooms
            .iter()
            .filter(|r| fold(&r.name).contains(&t))
            .map(|r| ChatHit { token: r.token.clone(), room: r.name.clone(), message_id: None, author: String::new(), text: "Беседа".into(), timestamp: 0 })
            .collect();
        let q = [("term", term.to_string()), ("limit", limit.to_string())];
        let (_, _, d) = self.ocs(Method::GET, "ocs/v2.php/search/providers/talk-message/search", &q, None, "Поиск в чате").await?;
        for e in d["entries"].as_array().into_iter().flatten() {
            let a = &e["attributes"];
            let num = |v: &Value| v.as_str().and_then(|x| x.parse().ok()).or_else(|| v.as_i64());
            let token = text(&a["conversation"]);
            if token.is_empty() { continue }
            let room = rooms.iter().find(|r| r.token == token).map(|r| r.name.clone()).unwrap_or_default();
            out.push(ChatHit { token, room, message_id: num(&a["messageId"]), author: text(&e["title"]), text: text(&e["subline"]), timestamp: num(&a["timestamp"]).unwrap_or(0) });
        }
        out.sort_by_key(|h| std::cmp::Reverse(h.timestamp));
        out.truncate(limit);
        Ok(out)
    }

    // --- Опросы ---
    pub async fn talk_poll_create(&self, token: &str, question: &str, options: &[String], max_votes: i64, hidden: bool) -> Result<TalkPoll> {
        let question = question.trim();
        let options: Vec<&str> = options.iter().map(|o| o.trim()).filter(|o| !o.is_empty()).collect();
        if question.is_empty() || options.len() < 2 {
            return Err(AppError::Other("Нужны вопрос и хотя бы два варианта ответа".into()));
        }
        let body = json!({ "question": question, "options": options, "resultMode": i64::from(hidden), "maxVotes": max_votes.clamp(0, options.len() as i64) });
        let (_, _, d) = self.ocs(Method::POST, &format!("{TALK_POLL}{}", seg(token)), &[], Some(body), "Создание опроса").await?;
        Ok(parse_poll(&d, &self.me))
    }
    pub async fn talk_poll_get(&self, token: &str, id: i64) -> Result<TalkPoll> {
        let (_, _, d) = self.ocs(Method::GET, &format!("{TALK_POLL}{}/{id}", seg(token)), &[], None, "Опрос").await?;
        Ok(parse_poll(&d, &self.me))
    }
    pub async fn talk_poll_vote(&self, token: &str, id: i64, options: &[i64]) -> Result<TalkPoll> {
        let (_, _, d) = self.ocs(Method::POST, &format!("{TALK_POLL}{}/{id}", seg(token)), &[], Some(json!({ "optionIds": options })), "Голосование").await?;
        Ok(parse_poll(&d, &self.me))
    }
    pub async fn talk_poll_close(&self, token: &str, id: i64) -> Result<TalkPoll> {
        let (_, _, d) = self.ocs(Method::DELETE, &format!("{TALK_POLL}{}/{id}", seg(token)), &[], None, "Закрытие опроса").await?;
        Ok(parse_poll(&d, &self.me))
    }

    /// Медиа и файлы беседы (как вкладка «Общие элементы» в Talk): media, file, audio, voice, other…
    pub async fn talk_shared(&self, token: &str, kind: &str, before: Option<i64>) -> Result<Vec<TalkMessage>> {
        let kind = match kind { "media" | "file" | "audio" | "voice" | "other" | "location" | "deckcard" | "poll" | "recording" | "pinned" => kind, _ => "media" };
        let mut q = vec![("objectType", kind.to_string()), ("limit", "100".to_string())];
        if let Some(b) = before { q.push(("lastKnownMessageId", b.to_string())); }
        let (_, _, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{}/share", seg(token)), &q, None, "Медиа беседы").await?;
        let items: Vec<&Value> = match &d { Value::Object(m) => m.values().collect(), Value::Array(a) => a.iter().collect(), _ => Vec::new() };
        let mut out: Vec<TalkMessage> = items.into_iter().map(|m| parse_message(m, &self.me)).filter(|m| !m.deleted).collect();
        out.sort_by_key(|m| std::cmp::Reverse(m.id));
        Ok(out)
    }

    /// Закрепить сообщение (модератор; в личной беседе — оба): до `until` (unix, с), 0 — бессрочно.
    pub async fn talk_pin(&self, token: &str, id: i64, until: i64) -> Result<()> {
        let path = format!("{TALK_CHAT}{}/{id}/pin", seg(token));
        self.ocs(Method::POST, &path, &[], Some(json!({ "pinUntil": until.max(0) })), "Закрепление сообщения").await.map_err(|e| match e {
            AppError::Other(m) if m.contains("HTTP 400") => AppError::Other("Не удалось закрепить: срок уже прошёл или это служебное сообщение".into()),
            AppError::Other(m) if m.contains("нет прав") => AppError::Other("Закреплять сообщения могут модераторы беседы".into()),
            AppError::Other(m) if m.contains("не найдено") => AppError::Other("Сервер Talk не умеет закреплять сообщения (нужен Talk 23 / Nextcloud 33 или новее)".into()),
            e => e,
        })?;
        Ok(())
    }

    /// Открепить у всех (модератор) или скрыть закреплённое только у себя.
    pub async fn talk_unpin(&self, token: &str, id: i64, only_me: bool) -> Result<()> {
        let path = format!("{TALK_CHAT}{}/{id}/pin{}", seg(token), if only_me { "/self" } else { "" });
        self.ocs(Method::DELETE, &path, &[], None, "Открепление сообщения").await?;
        Ok(())
    }

    /// Напоминание о сообщении: когда (unix, с) или None — нет.
    pub async fn talk_reminder(&self, token: &str, id: i64) -> Result<Option<i64>> {
        let path = format!("{TALK_CHAT}{}/{id}/reminder", seg(token));
        match self.ocs(Method::GET, &path, &[], None, "Напоминание").await {
            Ok((_, _, d)) => Ok(Some(int(&d["timestamp"])).filter(|t| *t > 0)),
            Err(_) => Ok(None),
        }
    }

    /// Напомнить о сообщении в это время (уведомление придёт от Talk) или убрать напоминание.
    pub async fn talk_set_reminder(&self, token: &str, id: i64, at: Option<i64>) -> Result<()> {
        let path = format!("{TALK_CHAT}{}/{id}/reminder", seg(token));
        match at {
            Some(t) => { self.ocs(Method::POST, &path, &[], Some(json!({ "timestamp": t })), "Напоминание").await?; }
            None => { self.ocs(Method::DELETE, &path, &[], None, "Напоминание").await?; }
        }
        Ok(())
    }

    /// Профиль сотрудника (то, что он открыл в профиле Nextcloud) и статус.
    pub async fn user_profile(&self, user: &str) -> Result<NcProfile> {
        let (_, _, d) = self.ocs(Method::GET, &format!("ocs/v2.php/profile/{}", seg(user)), &[], None, "Профиль").await?;
        let mut p = NcProfile {
            id: user.to_string(),
            name: [text(&d["displayname"]), text(&d["displayName"])].into_iter().find(|x| !x.is_empty()).unwrap_or_else(|| user.to_string()),
            email: text(&d["email"]),
            phone: text(&d["phone"]),
            organisation: text(&d["organisation"]),
            role: text(&d["role"]),
            headline: text(&d["headline"]),
            address: text(&d["address"]),
            status: String::new(),
            status_message: String::new(),
        };
        if let Ok((_, _, st)) = self.ocs(Method::GET, &format!("ocs/v2.php/apps/user_status/api/v1/statuses/{}", seg(user)), &[], None, "Статус").await {
            p.status = text(&st["status"]);
            p.status_message = [text(&st["icon"]), text(&st["message"])].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
        }
        Ok(p)
    }

    /// Занятость сотрудника по календарю Nextcloud (запрос free/busy, RFC 6638):
    /// занятые интервалы в unix-секундах.
    pub async fn free_busy(&self, email: &str, from: i64, to: i64) -> Result<Vec<BusySlot>> {
        let email = email.trim();
        if !email.contains('@') || email.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ':' | ';' | ',')) {
            return Err(AppError::Other("У сотрудника не указан адрес почты — занятость не узнать".into()));
        }
        let (_, _, me) = self.ocs(Method::GET, "ocs/v1.php/cloud/user", &[("format", "json".into())], None, "Профиль Nextcloud").await?;
        let my_email = text(&me["email"]);
        if my_email.is_empty() {
            return Err(AppError::Other("В вашем профиле Nextcloud не указан адрес почты — без него сервер не отвечает о занятости".into()));
        }
        let fmt = |t: i64| chrono::DateTime::from_timestamp(t, 0).unwrap_or_default().format("%Y%m%dT%H%M%SZ").to_string();
        let body = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Atom Workspace//RU\r\nMETHOD:REQUEST\r\nBEGIN:VFREEBUSY\r\nUID:{}\r\nDTSTAMP:{}\r\nDTSTART:{}\r\nDTEND:{}\r\nORGANIZER:mailto:{}\r\nATTENDEE:mailto:{}\r\nEND:VFREEBUSY\r\nEND:VCALENDAR\r\n",
            crate::dav::new_uid(),
            fmt(chrono::Utc::now().timestamp()),
            fmt(from),
            fmt(to),
            my_email,
            email
        );
        let url = self.creds.base.join(&format!("remote.php/dav/calendars/{}/outbox/", seg(&self.me))).map_err(|e| AppError::Other(e.to_string()))?;
        let resp = self.dav(Method::POST, url).header("Content-Type", "text/calendar; charset=utf-8").body(body).send().await.map_err(net_error)?;
        if !resp.status().is_success() {
            return Err(status_error(resp.status(), "Занятость"));
        }
        let xml = crate::netio::text_limited(resp, crate::netio::API_LIMIT).await?;
        parse_free_busy(&xml, from, to)
    }

    /// Предпросмотр ссылки через сервис ссылок Nextcloud (сервер сам загружает страницу;
    /// картинку берём только с самого Nextcloud — посторонние сайты программа не трогает).
    pub async fn link_preview(&self, url: &str) -> Result<Option<LinkPreview>> {
        let u = url::Url::parse(url).map_err(|_| AppError::Other("Неверная ссылка".into()))?;
        if !matches!(u.scheme(), "http" | "https") { return Ok(None); }
        let (_, _, d) = self.ocs(Method::GET, "ocs/v2.php/references/resolve", &[("reference", url.to_string())], None, "Предпросмотр ссылки").await?;
        let r = &d["references"][url];
        let o = if r["openGraphObject"].is_object() { &r["openGraphObject"] } else { &r["richObject"] };
        let title = text(&o["name"]);
        if title.is_empty() { return Ok(None); }
        let thumb = text(&o["thumb"]);
        let base = self.creds.base.as_str();
        let rel = if let Some(r) = thumb.strip_prefix(base) { Some(r.to_string()) } else if thumb.starts_with('/') && !thumb.starts_with("//") { Some(thumb.trim_start_matches('/').to_string()) } else { None };
        let image = match rel { Some(r) if !r.is_empty() => self.image(&r).await.ok().flatten(), _ => None };
        Ok(Some(LinkPreview {
            url: url.to_string(),
            title,
            description: text(&o["description"]).chars().take(300).collect(),
            image,
            site: u.host_str().unwrap_or_default().trim_start_matches("www.").to_string(),
        }))
    }

    /// Участники беседы с ролями и статусом (как боковая панель Talk).
    pub async fn talk_participants(&self, token: &str) -> Result<Vec<TalkParticipant>> {
        let path = format!("{TALK_ROOM}/{}/participants", seg(token));
        let (_, _, d) = self.ocs(Method::GET, &path, &[("includeStatus", "true".into())], None, "Участники").await?;
        let mut list: Vec<TalkParticipant> = d.as_array().map(|a| a.iter().map(|p| {
            let actor_type = text(&p["actorType"]);
            let actor_id = text(&p["actorId"]);
            TalkParticipant {
                attendee_id: int(&p["attendeeId"]),
                me: actor_type == "users" && actor_id == self.me,
                name: p["displayName"].as_str().filter(|n| !n.is_empty()).unwrap_or(&actor_id).to_string(),
                role: int(&p["participantType"]),
                status: text(&p["status"]),
                status_message: text(&p["statusMessage"]),
                actor_type,
                actor_id,
            }
        }).collect()).unwrap_or_default();
        // Владельцы и модераторы сверху, затем по имени.
        list.sort_by_key(|p| (!matches!(p.role, 1 | 2 | 6), p.name.to_lowercase()));
        Ok(list)
    }

    /// Управление беседой (модератор/владелец) и своё участие — тонкие обёртки над API Talk.
    pub async fn talk_manage(&self, token: &str, action: &str, value: &Value) -> Result<()> {
        let room = format!("{TALK_ROOM}/{}", seg(token));
        let attendee = || -> Result<Value> {
            let id = value["attendeeId"].as_i64().ok_or_else(|| AppError::Other("Не указан участник".into()))?;
            Ok(json!({ "attendeeId": id }))
        };
        let (method, path, body, what) = match action {
            "add" => {
                let who = value["id"].as_str().unwrap_or_default().trim().to_string();
                if who.is_empty() { return Err(AppError::Other("Не указан участник".into())); }
                let source = value["source"].as_str().filter(|s| matches!(*s, "groups" | "emails" | "circles")).unwrap_or("users");
                (Method::POST, format!("{room}/participants"), Some(json!({ "newParticipant": who, "source": source })), "Добавление участника")
            }
            "remove" => (Method::DELETE, format!("{room}/attendees"), Some(attendee()?), "Удаление участника"),
            "promote" => (Method::POST, format!("{room}/moderators"), Some(attendee()?), "Назначение модератора"),
            "demote" => (Method::DELETE, format!("{room}/moderators"), Some(attendee()?), "Снятие модератора"),
            "rename" => {
                let name = value["name"].as_str().unwrap_or_default().trim().to_string();
                if name.is_empty() { return Err(AppError::Other("Название не может быть пустым".into())); }
                (Method::PUT, room.clone(), Some(json!({ "roomName": name })), "Переименование")
            }
            "description" => (Method::PUT, format!("{room}/description"), Some(json!({ "description": value["description"].as_str().unwrap_or_default() })), "Описание"),
            "read_only" => (Method::PUT, format!("{room}/read-only"), Some(json!({ "state": i64::from(value["on"].as_bool().unwrap_or(false)) })), "Только чтение"),
            "public" => (if value["on"].as_bool().unwrap_or(false) { Method::POST } else { Method::DELETE }, format!("{room}/public"), None, "Доступ по ссылке"),
            "notify" => {
                let level = value["level"].as_i64().filter(|l| (0..=3).contains(l)).unwrap_or(0);
                (Method::POST, format!("{room}/notify"), Some(json!({ "level": level })), "Уведомления")
            }
            "read" => {
                // Без номера — «прочитано всё» (lastReadMessage: null, как в Talk).
                let body = value["lastId"].as_i64().map_or_else(|| json!({ "lastReadMessage": null }), |last| json!({ "lastReadMessage": last }));
                (Method::POST, format!("{TALK_CHAT}{}/read", seg(token)), Some(body), "Прочитано")
            }
            "unread" => (Method::DELETE, format!("{TALK_CHAT}{}/read", seg(token)), None, "Непрочитано"),
            "expiration" => {
                let secs = value["seconds"].as_i64().filter(|v| (0..=31_536_000).contains(v)).unwrap_or(0);
                (Method::POST, format!("{room}/message-expiration"), Some(json!({ "seconds": secs })), "Исчезающие сообщения")
            }
            "archive" | "important" | "sensitive" => {
                let on = value["on"].as_bool().unwrap_or(false);
                let what = match action { "archive" => "Архив", "important" => "Важная беседа", _ => "Конфиденциальная беседа" };
                (if on { Method::POST } else { Method::DELETE }, format!("{room}/{action}"), None, what)
            }
            "notify_calls" => (Method::POST, format!("{room}/notify-calls"), Some(json!({ "level": i64::from(value["on"].as_bool().unwrap_or(true)) })), "Уведомления о звонках"),
            "leave" => (Method::DELETE, format!("{room}/participants/self"), None, "Выход из беседы"),
            "delete" => (Method::DELETE, room.clone(), None, "Удаление беседы"),
            other => return Err(AppError::Other(format!("Неизвестное действие с беседой: {other}"))),
        };
        self.ocs(method, &path, &[], body, what).await?;
        Ok(())
    }

    pub async fn talk_react(&self, token: &str, message_id: i64, emoji: &str, add: bool) -> Result<()> {
        let path = format!("ocs/v2.php/apps/spreed/api/v1/reaction/{}/{message_id}", seg(token));
        if add {
            self.ocs(Method::POST, &path, &[], Some(json!({ "reaction": emoji })), "Реакция").await?;
        } else {
            self.ocs(Method::DELETE, &path, &[("reaction", emoji.to_string())], None, "Реакция").await?;
        }
        Ok(())
    }

    pub async fn talk_edit(&self, token: &str, message_id: i64, message: &str) -> Result<()> {
        let message = message.trim();
        if message.is_empty() { return Err(AppError::Other("Сообщение не может быть пустым".into())); }
        self.ocs(Method::PUT, &format!("{TALK_CHAT}{}/{message_id}", seg(token)), &[], Some(json!({ "message": message })), "Правка сообщения").await?;
        Ok(())
    }

    pub async fn talk_delete(&self, token: &str, message_id: i64) -> Result<()> {
        self.ocs(Method::DELETE, &format!("{TALK_CHAT}{}/{message_id}", seg(token)), &[], None, "Удаление сообщения").await?;
        Ok(())
    }

    /// Кого можно упомянуть в беседе (подсказка после «@»).
    pub async fn talk_mentions(&self, token: &str, search: &str) -> Result<Vec<TalkMention>> {
        let q = [("search", search.to_string()), ("limit", "8".to_string()), ("includeStatus", "false".to_string())];
        let (_, _, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{}/mentions", seg(token)), &q, None, "Упоминания").await?;
        Ok(d.as_array().map(|a| a.iter().map(|m| {
            let source = text(&m["source"]);
            let id = m.get("mentionId").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| text(&m["id"]));
            TalkMention {
                insert: if source == "calls" { "@all".into() } else { format!("@\"{id}\"") },
                label: text(&m["label"]),
                source,
            }
        }).collect()).unwrap_or_default())
    }

    /// Поиск сотрудников Nextcloud (для новой беседы).
    pub async fn search_users(&self, search: &str) -> Result<Vec<NcUser>> {
        let q = [("search", search.to_string()), ("itemType", "call".to_string()), ("itemId", "new".to_string()),
            ("shareTypes[]", "0".to_string()), ("limit", "25".to_string())];
        let (_, _, d) = self.ocs(Method::GET, "ocs/v2.php/core/autocomplete/get", &q, None, "Поиск пользователей").await?;
        Ok(d.as_array().map(|a| a.iter().map(|u| NcUser {
            id: text(&u["id"]),
            label: text(&u["label"]),
            sub: text(&u["subline"]),
        }).filter(|u| !u.id.is_empty() && !u.id.eq_ignore_ascii_case(&self.me)).collect()).unwrap_or_default())
    }

    /// Личная беседа с пользователем (Talk вернёт существующую, если она есть) или
    /// групповая с названием и участниками. Возвращает token.
    pub async fn talk_create(&self, users: &[String], name: Option<&str>) -> Result<String> {
        if users.is_empty() { return Err(AppError::Other("Выберите собеседника".into())); }
        let group = name.is_some_and(|n| !n.trim().is_empty()) || users.len() > 1;
        let body = if group {
            json!({ "roomType": 2, "roomName": name.unwrap_or("Беседа").trim() })
        } else {
            json!({ "roomType": 1, "invite": users[0] })
        };
        let (_, _, d) = self.ocs(Method::POST, TALK_ROOM, &[], Some(body), "Новая беседа").await?;
        let token = text(&d["token"]);
        if token.is_empty() { return Err(AppError::Other("Talk не вернул беседу".into())); }
        if group {
            for u in users {
                self.ocs(Method::POST, &format!("{TALK_ROOM}/{}/participants", seg(&token)), &[], Some(json!({ "newParticipant": u, "source": "users" })), "Участник беседы").await?;
            }
        }
        Ok(token)
    }

    /// URL в WebDAV файлов пользователя: `remote.php/dav/files/<uid>/<path>`.
    pub(crate) fn files_url(&self, path: &str) -> Result<Url> {
        let mut url = self.creds.base.join("remote.php/dav/files/").map_err(|e| AppError::Other(e.to_string()))?;
        {
            let mut seg = url.path_segments_mut().map_err(|_| AppError::Other("Некорректный адрес Nextcloud".into()))?;
            seg.pop_if_empty().push(&self.me);
            let mut any = false;
            for part in path.split('/').filter(|p| !p.is_empty()) {
                seg.push(part);
                any = true;
            }
            // Корень файлов — каталог: со слэшем на конце.
            if !any {
                seg.push("");
            }
        }
        Ok(url)
    }

    pub(crate) fn dav(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.creds.login, Some(&self.creds.app_password))
            .header("OCS-APIRequest", "true")
    }

    /// Папка вложений Talk из настроек пользователя (по умолчанию «/Talk»).
    pub(crate) async fn talk_attachment_folder(&self) -> String {
        let folder = self
            .ocs(Method::GET, "ocs/v2.php/cloud/capabilities", &[("format", "json".into())], None, "Возможности сервера")
            .await
            .ok()
            .and_then(|(_, _, d)| d.pointer("/capabilities/spreed/config/attachments/folder").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let folder = folder.trim_matches('/');
        if folder.is_empty() || folder.split('/').any(|p| p == ".." || p == ".") { "Talk".into() } else { folder.to_string() }
    }
}

/// Как опубликовать файл в беседе.
#[derive(Debug, Default, Clone, Copy)]
pub struct TalkFileOpts {
    pub reply_to: Option<i64>,
    /// Голосовое сообщение (проигрыватель вместо файла).
    pub voice: bool,
    /// Без звука уведомления.
    pub silent: bool,
    /// В ветку (Talk 22).
    pub thread_id: Option<i64>,
}

