//! Talk: встречи из беседы и общие встречи (календарь Nextcloud), «в отпуске», все напоминания,
//! сводка непрочитанного и перевод (ИИ на сервере, TaskProcessing), модерация (бан, зал
//! ожидания, комнаты для групп). Часть клиента `Nc` (подмодуль `nextcloud`).

use reqwest::Method;
use serde::Serialize;
use serde_json::{json, Value};

use super::{int, render_message, seg, text, Nc, TALK_CHAT, TALK_ROOM};
use crate::error::{AppError, Result};

const TALK_V1: &str = "ocs/v2.php/apps/spreed/api/v1";

/// Календарь Nextcloud, в который можно записать встречу.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NcCalendar {
    /// Последняя часть адреса (`personal`), её ждёт Talk.
    pub uri: String,
    pub name: String,
    pub color: String,
}

/// Встреча из календаря (общая с собеседником).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkEvent {
    pub name: String,
    pub start: i64,
    pub end: i64,
    pub link: String,
}

/// Напоминание о сообщении (из всех бесед).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkReminderItem {
    pub token: String,
    pub message_id: i64,
    pub at: i64,
    pub author: String,
    pub text: String,
}

/// Сотрудник в отпуске (отсутствие из календаря Nextcloud).
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NcAbsence {
    /// Последний день отсутствия «ГГГГ-ММ-ДД» (или пусто, если сервер его не дал).
    pub until: String,
    pub message: String,
    pub replacement: String,
}

/// Задача ИИ на сервере: в очереди, идёт, готова (с результатом) или не удалась.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcTask {
    pub status: String,
    pub output: String,
}

/// Сводка непрочитанного: задача ИИ и, если сообщений много, с какого места следующая часть.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkSummaryTask {
    pub task_id: i64,
    pub next_offset: i64,
}

/// Перевод: готовый текст (старый API) или задача ИИ, результат которой нужно подождать.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkTranslation {
    pub text: Option<String>,
    pub task_id: Option<i64>,
}

/// Заблокированный в беседе участник.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkBan {
    pub id: i64,
    pub actor_type: String,
    pub actor_id: String,
    pub name: String,
    pub by: String,
    pub time: i64,
    pub note: String,
}

/// Календари из ответа PROPFIND `calendars/<uid>/`: только те, куда можно записать события.
pub fn parse_calendars(xml: &str) -> Vec<NcCalendar> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    let mut out = Vec::new();
    for r in doc.descendants().filter(|n| n.tag_name().name() == "response") {
        let has = |name: &str| r.descendants().any(|n| n.tag_name().name() == name);
        let href = r.descendants().find(|n| n.tag_name().name() == "href").and_then(|n| n.text()).unwrap_or("");
        let uri = href.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string();
        if !has("calendar") || uri.is_empty() || has("deleted-calendar") {
            continue;
        }
        // Только календари с событиями (не задачи) и с правом записи, если сервер о них сообщил.
        let comps: Vec<&str> = r.descendants().filter(|n| n.tag_name().name() == "comp").filter_map(|n| n.attribute("name")).collect();
        if !comps.is_empty() && !comps.contains(&"VEVENT") {
            continue;
        }
        let privs = r.descendants().find(|n| n.tag_name().name() == "current-user-privilege-set");
        if privs.is_some_and(|p| !p.descendants().any(|n| matches!(n.tag_name().name(), "write" | "write-content" | "all"))) {
            continue;
        }
        let prop = |name: &str| r.descendants().find(|n| n.tag_name().name() == name).and_then(|n| n.text()).unwrap_or("").trim().to_string();
        let name = prop("displayname");
        out.push(NcCalendar { name: if name.is_empty() { uri.clone() } else { name }, color: prop("calendar-color"), uri });
    }
    out
}

/// Отсутствие: даты бывают числом (unix) или строкой «ГГГГ-ММ-ДД».
fn parse_absence(d: &Value) -> Option<NcAbsence> {
    if !d.is_object() || d.as_object().is_some_and(|o| o.is_empty()) {
        return None;
    }
    let until = match &d["endDate"] {
        Value::Number(n) => n.as_i64().and_then(|t| chrono::DateTime::from_timestamp(t, 0)).map(|t| t.format("%Y-%m-%d").to_string()).unwrap_or_default(),
        Value::String(s) => s.chars().take(10).collect(),
        _ => String::new(),
    };
    Some(NcAbsence { until, message: text(&d["shortMessage"]), replacement: text(&d["replacementUserDisplayName"]) })
}

impl Nc {
    /// Календари пользователя в Nextcloud, куда можно записать встречу.
    pub async fn nc_calendars(&self) -> Result<Vec<NcCalendar>> {
        let url = self.creds.base.join(&format!("remote.php/dav/calendars/{}/", seg(&self.me))).map_err(|e| AppError::Other(e.to_string()))?;
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/"><d:prop><d:resourcetype/><d:displayname/><a:calendar-color/><c:supported-calendar-component-set/><d:current-user-privilege-set/></d:prop></d:propfind>"#;
        let resp = self
            .dav(Method::from_bytes(b"PROPFIND").expect("PROPFIND"), url)
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body)
            .send()
            .await
            .map_err(super::net_error)?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Календари Nextcloud недоступны ({})", resp.status())));
        }
        let xml = crate::netio::text_limited(resp, crate::netio::API_LIMIT).await?;
        Ok(parse_calendars(&xml))
    }

    /// Запланировать встречу беседы: событие в календаре `calendar`, приглашения участникам
    /// (`attendees` — номера участников; `None` — все, пустой список — только я).
    #[allow(clippy::too_many_arguments)]
    pub async fn talk_schedule_meeting(&self, token: &str, calendar: &str, start: i64, end: i64, title: &str, description: &str, attendees: Option<Vec<i64>>) -> Result<()> {
        if calendar.is_empty() || calendar.contains(['/', '\\']) {
            return Err(AppError::Other("Выберите календарь".into()));
        }
        let now = chrono::Utc::now().timestamp();
        if start < now - 300 {
            return Err(AppError::Other("Время встречи уже прошло".into()));
        }
        if end <= start || end - start > 24 * 3600 {
            return Err(AppError::Other("Встреча должна закончиться позже начала и длиться не больше суток".into()));
        }
        let mut body = json!({ "calendarUri": calendar, "start": start, "end": end });
        if !title.trim().is_empty() {
            body["title"] = json!(title.trim().chars().take(250).collect::<String>());
        }
        if !description.trim().is_empty() {
            body["description"] = json!(description.trim().chars().take(4000).collect::<String>());
        }
        if let Some(ids) = attendees {
            body["attendeeIds"] = json!(ids);
        }
        self.ocs(Method::POST, &format!("{TALK_ROOM}/{}/meeting", seg(token)), &[], Some(body), "Встреча в календаре").await?;
        Ok(())
    }

    /// Ближайшие (до трёх за 7 дней) общие встречи с собеседником личной беседы.
    pub async fn talk_mutual_events(&self, token: &str) -> Result<Vec<TalkEvent>> {
        let (_, _, d) = self.ocs(Method::GET, &format!("{TALK_ROOM}/{}/mutual-events", seg(token)), &[], None, "Общие встречи").await?;
        Ok(d.as_array().map(|a| a.iter().map(|e| TalkEvent { name: text(&e["eventName"]), start: int(&e["start"]), end: int(&e["end"]), link: text(&e["eventLink"]) }).collect()).unwrap_or_default())
    }

    /// Все предстоящие напоминания о сообщениях (из всех бесед), ближайшие сверху.
    pub async fn talk_upcoming_reminders(&self) -> Result<Vec<TalkReminderItem>> {
        let (_, _, d) = self.ocs(Method::GET, &format!("{TALK_V1}/chat/upcoming-reminders"), &[], None, "Напоминания").await?;
        let mut out: Vec<TalkReminderItem> = d
            .as_array()
            .map(|a| a.iter().map(|r| TalkReminderItem {
                token: text(&r["roomToken"]),
                message_id: int(&r["messageId"]),
                at: int(&r["reminderTimestamp"]),
                author: text(&r["actorDisplayName"]),
                text: render_message(r).0,
            }).collect())
            .unwrap_or_default();
        out.sort_by_key(|r| r.at);
        Ok(out)
    }

    /// Отсутствие сотрудника сейчас (отпуск из календаря Nextcloud); `None` — на месте или неизвестно.
    pub async fn nc_absence(&self, uid: &str) -> Option<NcAbsence> {
        let path = format!("ocs/v2.php/apps/dav/api/v1/outOfOffice/{}/now", seg(uid));
        let (_, _, d) = self.ocs(Method::GET, &path, &[], None, "Отсутствие").await.ok()?;
        parse_absence(&d)
    }

    /// Сводка непрочитанного начиная с сообщения `from` (задача ИИ); `None` — сообщений нет.
    pub async fn talk_summarize(&self, token: &str, from: i64) -> Result<Option<TalkSummaryTask>> {
        let r = self.ocs(Method::POST, &format!("{TALK_CHAT}{}/summarize", seg(token)), &[], Some(json!({ "fromMessageId": from.max(1) })), "Сводка").await;
        let (status, _, d) = r.map_err(|_| AppError::Other("Сводка недоступна: на сервере Nextcloud не настроен ИИ-ассистент или беседа закрыта для сводок".into()))?;
        if status == reqwest::StatusCode::NO_CONTENT || !d.is_object() {
            return Ok(None);
        }
        Ok(Some(TalkSummaryTask { task_id: int(&d["taskId"]), next_offset: int(&d["nextOffset"]) }))
    }

    /// Состояние задачи ИИ (сводка, перевод).
    pub async fn nc_task(&self, id: i64) -> Result<NcTask> {
        let (_, _, d) = self.ocs(Method::GET, &format!("ocs/v2.php/taskprocessing/task/{id}"), &[], None, "Задача ИИ").await?;
        let t = &d["task"];
        let output = match &t["output"]["output"] {
            Value::String(s) => s.clone(),
            other => text(other),
        };
        Ok(NcTask { status: text(&t["status"]).trim_start_matches("STATUS_").to_lowercase(), output })
    }

    /// Перевести текст сообщения на язык `to` (`ru`, `en`…); язык оригинала определяет сервер.
    pub async fn talk_translate(&self, mode: &str, message: &str, to: &str) -> Result<TalkTranslation> {
        if message.trim().is_empty() {
            return Err(AppError::Other("Нечего переводить".into()));
        }
        if to.is_empty() || to.len() > 10 || !to.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(AppError::Other("Неверный язык перевода".into()));
        }
        let fail = |_| AppError::Other("Перевод недоступен: на сервере Nextcloud не настроен переводчик".into());
        if mode == "legacy" {
            let (_, _, d) = self.ocs(Method::POST, "ocs/v2.php/translation/translate", &[], Some(json!({ "text": message, "fromLanguage": null, "toLanguage": to })), "Перевод").await.map_err(fail)?;
            return Ok(TalkTranslation { text: Some(text(&d["text"])), task_id: None });
        }
        let body = json!({ "type": "core:text2text:translate", "appId": "spreed", "customId": "atom-translate",
            "input": { "input": message, "origin_language": "detect_language", "target_language": to } });
        let (_, _, d) = self.ocs(Method::POST, "ocs/v2.php/taskprocessing/schedule", &[], Some(body), "Перевод").await.map_err(fail)?;
        Ok(TalkTranslation { text: None, task_id: Some(int(&d["task"]["id"])) })
    }

    /// Заблокированные в беседе.
    pub async fn talk_bans(&self, token: &str) -> Result<Vec<TalkBan>> {
        let (_, _, d) = self.ocs(Method::GET, &format!("{TALK_V1}/ban/{}", seg(token)), &[], None, "Заблокированные").await?;
        Ok(d.as_array().map(|a| a.iter().map(|b| TalkBan {
            id: int(&b["id"]),
            actor_type: text(&b["bannedActorType"]),
            actor_id: text(&b["bannedActorId"]),
            name: text(&b["bannedDisplayName"]),
            by: text(&b["moderatorDisplayName"]),
            time: int(&b["bannedTime"]),
            note: text(&b["internalNote"]),
        }).collect()).unwrap_or_default())
    }

    /// Заблокировать участника (он удаляется из беседы и не может вернуться по ссылке).
    pub async fn talk_ban(&self, token: &str, actor_type: &str, actor_id: &str, note: &str) -> Result<()> {
        if !matches!(actor_type, "users" | "guests" | "emails") || actor_id.trim().is_empty() {
            return Err(AppError::Other("Неверный участник".into()));
        }
        let body = json!({ "actorType": actor_type, "actorId": actor_id, "internalNote": note.chars().take(4000).collect::<String>() });
        self.ocs(Method::POST, &format!("{TALK_V1}/ban/{}", seg(token)), &[], Some(body), "Блокировка").await?;
        Ok(())
    }

    pub async fn talk_unban(&self, token: &str, ban_id: i64) -> Result<()> {
        self.ocs(Method::DELETE, &format!("{TALK_V1}/ban/{}/{ban_id}", seg(token)), &[], None, "Снятие блокировки").await?;
        Ok(())
    }

    /// Зал ожидания: участники (не модераторы) ждут, пока его откроют; `timer` — когда откроется сам.
    pub async fn talk_lobby(&self, token: &str, on: bool, timer: Option<i64>) -> Result<()> {
        let mut body = json!({ "state": i64::from(on) });
        if let Some(t) = timer.filter(|t| *t > 0 && on) {
            body["timer"] = json!(t);
        }
        self.ocs(Method::PUT, &format!("{TALK_ROOM}/{}/webinar/lobby", seg(token)), &[], Some(body), "Зал ожидания").await?;
        Ok(())
    }

    /// Комнаты для групп: `configure` (mode 1 — авто, 3 — по выбору; amount 1–20), `start`, `stop`,
    /// `broadcast` (сообщение во все комнаты), `remove`.
    pub async fn talk_breakout(&self, token: &str, action: &str, value: &Value) -> Result<()> {
        let base = format!("{TALK_V1}/breakout-rooms/{}", seg(token));
        let (method, path, body, what) = match action {
            "configure" => {
                let mode = value["mode"].as_i64().filter(|m| matches!(m, 1 | 3)).unwrap_or(1);
                let amount = value["amount"].as_i64().filter(|a| (1..=20).contains(a)).ok_or_else(|| AppError::Other("Комнат может быть от 1 до 20".into()))?;
                (Method::POST, base, Some(json!({ "mode": mode, "amount": amount, "attendeeMap": "[]" })), "Комнаты для групп")
            }
            "start" => (Method::POST, format!("{base}/rooms"), None, "Запуск комнат"),
            "stop" => (Method::DELETE, format!("{base}/rooms"), None, "Остановка комнат"),
            "broadcast" => {
                let msg = value["message"].as_str().unwrap_or_default().trim().to_string();
                if msg.is_empty() { return Err(AppError::Other("Пустое сообщение".into())); }
                (Method::POST, format!("{base}/broadcast"), Some(json!({ "message": msg })), "Сообщение во все комнаты")
            }
            "remove" => (Method::DELETE, base, None, "Удаление комнат"),
            other => return Err(AppError::Other(format!("Неизвестное действие: {other}"))),
        };
        self.ocs(method, &path, &[], body, what).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writable_event_calendars_only() {
        let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/">
<d:response><d:href>/remote.php/dav/calendars/test/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
<d:response><d:href>/remote.php/dav/calendars/test/personal/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Личный</d:displayname><a:calendar-color>#0082c9</a:calendar-color><c:supported-calendar-component-set><c:comp name="VEVENT"/></c:supported-calendar-component-set><d:current-user-privilege-set><d:privilege><d:write/></d:privilege></d:current-user-privilege-set></d:prop></d:propstat></d:response>
<d:response><d:href>/remote.php/dav/calendars/test/tasks/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Задачи</d:displayname><c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set></d:prop></d:propstat></d:response>
<d:response><d:href>/remote.php/dav/calendars/test/shared_ro/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Чужой</d:displayname><d:current-user-privilege-set><d:privilege><d:read/></d:privilege></d:current-user-privilege-set></d:prop></d:propstat></d:response>
</d:multistatus>"#;
        assert_eq!(parse_calendars(xml), vec![NcCalendar { uri: "personal".into(), name: "Личный".into(), color: "#0082c9".into() }]);
    }

    #[test]
    fn absence_dates() {
        let a = parse_absence(&json!({ "endDate": 1795219200, "shortMessage": "Отпуск", "replacementUserDisplayName": "Петрова Анна" })).unwrap();
        assert_eq!((a.until.as_str(), a.message.as_str(), a.replacement.as_str()), ("2026-11-21", "Отпуск", "Петрова Анна"));
        assert_eq!(parse_absence(&json!({ "endDate": "2026-12-01", "shortMessage": "" })).unwrap().until, "2026-12-01");
        assert!(parse_absence(&json!(null)).is_none() && parse_absence(&json!({})).is_none() && parse_absence(&json!([])).is_none());
    }
}
