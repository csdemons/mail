//! Talk: ветки (Talk 22), отложенные сообщения (Talk 23), архив и отметки бесед,
//! «Заметки для себя», возможности сервера. Часть клиента `Nc` (подмодуль `nextcloud`).

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{int, parse_message, seg, text, Nc, TalkChunk, TalkMessage, TALK_CHAT, TALK_ROOM};
use crate::error::{AppError, Result};

/// Куда отправить сообщение: в беседу, в ветку или новой веткой.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkThreadTarget {
    /// Ответ в ветку (без цитаты конкретного сообщения).
    pub thread_id: Option<i64>,
    /// Начать ветку с таким названием (только не ответом).
    pub thread_title: Option<String>,
}

impl TalkThreadTarget {
    /// Поля запроса Talk. Ответ на сообщение сам попадает в его ветку — номер не нужен.
    pub fn apply(&self, body: &mut Value, replying: bool) {
        if replying {
            return;
        }
        if let Some(t) = self.thread_id.filter(|t| *t > 0) {
            body["threadId"] = json!(t);
        } else if let Some(title) = self.thread_title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            body["threadTitle"] = json!(title.chars().take(200).collect::<String>());
        }
    }
}

/// Ветка беседы: начало, последнее сообщение, ответов, моя подписка.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkThread {
    pub id: i64,
    pub title: String,
    pub replies: i64,
    pub last_activity: i64,
    /// Уведомления по ветке: 0 — как в беседе, 1 — все, 2 — упоминания, 3 — никогда.
    pub notify_level: i64,
    pub first: Option<TalkMessage>,
    pub last: Option<TalkMessage>,
}

/// Отложенное сообщение (видно только автору до отправки).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkScheduled {
    pub id: String,
    pub text: String,
    pub send_at: i64,
    pub silent: bool,
    pub thread_id: i64,
    pub thread_title: String,
    pub parent_author: Option<String>,
    pub parent_text: Option<String>,
}

/// Возможности Talk на сервере, которые нужны программе.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkFeatures {
    pub threads: bool,
    pub scheduled: bool,
    pub archive: bool,
    pub important: bool,
    pub sensitive: bool,
    pub note_to_self: bool,
}

fn parse_thread(v: &Value, me: &str) -> TalkThread {
    let msg = |m: &Value| m.is_object().then(|| parse_message(m, me));
    TalkThread {
        id: int(&v["thread"]["id"]),
        title: text(&v["thread"]["title"]),
        replies: int(&v["thread"]["numReplies"]),
        last_activity: int(&v["thread"]["lastActivity"]),
        notify_level: int(&v["attendee"]["notificationLevel"]),
        first: msg(&v["first"]),
        last: msg(&v["last"]),
    }
}

fn parse_scheduled(v: &Value) -> TalkScheduled {
    let parent = v.get("parent").filter(|p| p.is_object());
    TalkScheduled {
        id: v["id"].as_str().map(str::to_string).unwrap_or_else(|| int(&v["id"]).to_string()),
        text: text(&v["message"]),
        send_at: int(&v["sendAt"]),
        silent: v["silent"].as_bool().unwrap_or(false),
        thread_id: int(&v["threadId"]).max(0),
        thread_title: text(&v["threadTitle"]),
        parent_author: parent.map(|p| text(&p["actorDisplayName"])),
        parent_text: parent.map(|p| super::render_message(p).0),
    }
}

/// Номер отложенного сообщения (Snowflake): только цифры.
fn scheduled_id(id: &str) -> Result<&str> {
    if id.is_empty() || id.len() > 30 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AppError::Other("Неверное отложенное сообщение".into()));
    }
    Ok(id)
}

/// Время отправки: в будущем и не дальше года.
fn check_send_at(at: i64) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    if at <= now + 30 {
        return Err(AppError::Other("Время отправки уже прошло — выберите время позже".into()));
    }
    if at > now + 366 * 86400 {
        return Err(AppError::Other("Отложить можно не больше чем на год".into()));
    }
    Ok(())
}

impl Nc {
    /// Что из новых возможностей Talk есть на сервере (`spreed.features`).
    pub async fn talk_features(&self) -> Result<TalkFeatures> {
        let (_, _, d) = self.ocs(Method::GET, "ocs/v2.php/cloud/capabilities", &[("format", "json".into())], None, "Возможности Talk").await?;
        let list: Vec<String> = d.pointer("/capabilities/spreed/features").and_then(Value::as_array).map(|a| a.iter().map(text).collect()).unwrap_or_default();
        let has = |f: &str| list.iter().any(|x| x == f);
        Ok(TalkFeatures {
            threads: has("threads"),
            scheduled: has("scheduled-messages"),
            archive: has("archived-conversations-v2"),
            important: has("important-conversations"),
            sensitive: has("sensitive-conversations"),
            note_to_self: has("note-to-self"),
        })
    }

    /// Недавние ветки беседы (новые сверху).
    pub async fn talk_threads(&self, token: &str) -> Result<Vec<TalkThread>> {
        let path = format!("{TALK_CHAT}{}/threads/recent", seg(token));
        let (_, _, d) = self.ocs(Method::GET, &path, &[("limit", "50".into())], None, "Ветки беседы").await?;
        Ok(d.as_array().map(|a| a.iter().map(|t| parse_thread(t, &self.me)).collect()).unwrap_or_default())
    }

    /// Ветка: сведения о ней.
    pub async fn talk_thread(&self, token: &str, thread_id: i64) -> Result<TalkThread> {
        let path = format!("{TALK_CHAT}{}/threads/{thread_id}", seg(token));
        let (_, _, d) = self.ocs(Method::GET, &path, &[], None, "Ветка").await?;
        Ok(parse_thread(&d, &self.me))
    }

    /// Сообщения ветки: последние (или старее `before`). Отметку прочтения беседы не трогает.
    pub async fn talk_thread_history(&self, token: &str, thread_id: i64, before: Option<i64>) -> Result<TalkChunk> {
        let mut q = vec![("lookIntoFuture", "0".to_string()), ("limit", "100".to_string()), ("setReadMarker", "0".to_string()), ("threadId", thread_id.to_string())];
        if let Some(b) = before {
            q.push(("lastKnownMessageId", b.to_string()));
        }
        let (status, _, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{}", seg(token)), &q, None, "Сообщения ветки").await?;
        if status == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(TalkChunk { messages: Vec::new(), last_id: 0, last_common_read: 0, more: false });
        }
        let more = d.as_array().is_some_and(|a| a.len() >= 100);
        let mut messages = super::parse_chat(&d, &self.me);
        messages.retain(|m| !m.update);
        messages.sort_by_key(|m| m.id);
        let last_id = messages.iter().map(|m| m.id).max().unwrap_or(0);
        Ok(TalkChunk { messages, last_id, last_common_read: 0, more })
    }

    /// Уведомления по ветке (0 — как в беседе, 1 — все, 2 — упоминания, 3 — никогда).
    pub async fn talk_thread_notify(&self, token: &str, thread_id: i64, level: i64) -> Result<()> {
        if !(0..=3).contains(&level) {
            return Err(AppError::Other("Неверный уровень уведомлений".into()));
        }
        let path = format!("{TALK_CHAT}{}/threads/{thread_id}/notify", seg(token));
        self.ocs(Method::POST, &path, &[], Some(json!({ "level": level })), "Уведомления ветки").await?;
        Ok(())
    }

    /// Переименовать ветку.
    pub async fn talk_thread_rename(&self, token: &str, thread_id: i64, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::Other("Название ветки не может быть пустым".into()));
        }
        let path = format!("{TALK_CHAT}{}/threads/{thread_id}", seg(token));
        self.ocs(Method::PUT, &path, &[], Some(json!({ "threadTitle": title.chars().take(200).collect::<String>() })), "Название ветки").await?;
        Ok(())
    }

    /// Отложенные сообщения этой беседы (мои).
    pub async fn talk_scheduled(&self, token: &str) -> Result<Vec<TalkScheduled>> {
        let (_, _, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{}/schedule", seg(token)), &[], None, "Отложенные сообщения").await?;
        let mut list: Vec<TalkScheduled> = d.as_array().map(|a| a.iter().map(parse_scheduled).collect()).unwrap_or_default();
        list.sort_by_key(|m| m.send_at);
        Ok(list)
    }

    /// Отложить сообщение до `send_at` (unix, с).
    pub async fn talk_schedule(&self, token: &str, message: &str, send_at: i64, reply_to: Option<i64>, silent: bool, thread: &TalkThreadTarget) -> Result<TalkScheduled> {
        let message = message.trim();
        if message.is_empty() {
            return Err(AppError::Other("Пустое сообщение".into()));
        }
        check_send_at(send_at)?;
        let mut body = json!({ "message": message, "sendAt": send_at, "silent": silent });
        if let Some(r) = reply_to {
            body["replyTo"] = json!(r);
        }
        thread.apply(&mut body, reply_to.is_some());
        let (_, _, d) = self.ocs(Method::POST, &format!("{TALK_CHAT}{}/schedule", seg(token)), &[], Some(body), "Отложенное сообщение").await?;
        Ok(parse_scheduled(&d))
    }

    /// Изменить текст или время отложенного сообщения.
    pub async fn talk_schedule_edit(&self, token: &str, id: &str, message: &str, send_at: i64, silent: bool) -> Result<TalkScheduled> {
        let id = scheduled_id(id)?;
        let message = message.trim();
        if message.is_empty() {
            return Err(AppError::Other("Пустое сообщение".into()));
        }
        check_send_at(send_at)?;
        let body = json!({ "message": message, "sendAt": send_at, "silent": silent });
        let (_, _, d) = self.ocs(Method::POST, &format!("{TALK_CHAT}{}/schedule/{id}", seg(token)), &[], Some(body), "Отложенное сообщение").await?;
        Ok(parse_scheduled(&d))
    }

    /// Отменить отложенное сообщение.
    pub async fn talk_schedule_delete(&self, token: &str, id: &str) -> Result<()> {
        let id = scheduled_id(id)?;
        self.ocs(Method::DELETE, &format!("{TALK_CHAT}{}/schedule/{id}", seg(token)), &[], None, "Отмена отложенного сообщения").await?;
        Ok(())
    }

    /// «Заметки для себя»: беседа с самим собой (сервер создаёт её при первом запросе).
    pub async fn talk_note_to_self(&self) -> Result<String> {
        let (_, _, d) = self.ocs(Method::GET, &format!("{TALK_ROOM}/note-to-self"), &[], None, "Заметки для себя").await?;
        let token = text(&d["token"]);
        if token.is_empty() {
            return Err(AppError::Other("Сервер не создал «Заметки для себя»".into()));
        }
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_target_fields() {
        let mut b = json!({});
        TalkThreadTarget { thread_id: Some(7), thread_title: Some("x".into()) }.apply(&mut b, false);
        assert_eq!(b, json!({ "threadId": 7 }));
        let mut b = json!({});
        TalkThreadTarget { thread_id: None, thread_title: Some("  План  ".into()) }.apply(&mut b, false);
        assert_eq!(b, json!({ "threadTitle": "План" }));
        let mut b = json!({});
        TalkThreadTarget { thread_id: Some(7), thread_title: None }.apply(&mut b, true);
        assert_eq!(b, json!({}), "a reply goes to its parent's thread by itself");
        let mut b = json!({});
        TalkThreadTarget::default().apply(&mut b, false);
        assert_eq!(b, json!({}));
    }

    #[test]
    fn scheduled_ids_and_times_are_checked() {
        assert!(scheduled_id("123456789012345").is_ok());
        assert!(scheduled_id("12/../x").is_err() && scheduled_id("").is_err());
        let now = chrono::Utc::now().timestamp();
        assert!(check_send_at(now - 10).is_err());
        assert!(check_send_at(now + 3600).is_ok());
        assert!(check_send_at(now + 400 * 86400).is_err());
    }

    #[test]
    fn scheduled_message_is_parsed() {
        let v = json!({ "id": "7340000000000000001", "message": "Привет", "sendAt": 1900000000, "silent": true, "threadId": 0,
            "parent": { "actorDisplayName": "Иванов", "message": "Вопрос", "messageParameters": [] } });
        let s = parse_scheduled(&v);
        assert_eq!((s.id.as_str(), s.text.as_str(), s.send_at, s.silent), ("7340000000000000001", "Привет", 1900000000, true));
        assert_eq!(s.parent_author.as_deref(), Some("Иванов"));
    }
}
