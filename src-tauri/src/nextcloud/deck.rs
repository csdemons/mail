//! Deck: доски, колонки, карточки, метки, комментарии и вложения.
//!
//! Часть модуля `nextcloud` — общий обзор в `mod.rs`.

use super::*;

const DECK: &str = "index.php/apps/deck/api/v1.0/";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckBoard {
    pub id: i64,
    pub title: String,
    pub color: String,
    pub archived: bool,
    pub can_edit: bool,
    pub can_manage: bool,
    pub owner: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckLabel {
    pub id: i64,
    pub title: String,
    pub color: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckCard {
    pub id: i64,
    pub stack_id: i64,
    pub title: String,
    pub description: String,
    pub order: i64,
    pub duedate: Option<String>,
    pub done: bool,
    pub archived: bool,
    pub labels: Vec<DeckLabel>,
    pub assigned: Vec<DeckUser>,
    pub owner: String,
    pub comments_unread: i64,
    pub attachment_count: i64,
    /// Когда создана и последний раз изменена (Unix-время, секунды).
    pub created_at: i64,
    pub last_modified: i64,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeckUser {
    pub uid: String,
    pub name: String,
}

/// Метки и участники доски — для назначения на карточку.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckBoardDetails {
    pub labels: Vec<DeckLabel>,
    pub users: Vec<DeckUser>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckComment {
    pub id: i64,
    pub author: String,
    pub author_id: String,
    pub own: bool,
    pub message: String,
    pub created: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckAttachment {
    pub id: i64,
    pub name: String,
    pub size: i64,
    pub mimetype: String,
    /// ID файла в Nextcloud (открывается по `/f/<id>`), если вложение — файл.
    pub file_id: Option<i64>,
    /// Тип вложения в Deck: `file` (файл Nextcloud) или `deck_file` (старый формат).
    pub kind: String,
}

/// Событие в истории карточки (приложение Activity).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckActivity {
    pub id: i64,
    pub author: String,
    pub author_id: String,
    pub subject: String,
    pub date: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckStack {
    pub id: i64,
    pub title: String,
    pub order: i64,
    pub cards: Vec<DeckCard>,
}

pub(crate) fn int(v: &Value) -> i64 {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)
}
pub(crate) fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}
fn user_name(v: &Value) -> String {
    v.get("displayname").and_then(Value::as_str)
        .or_else(|| v.get("uid").and_then(Value::as_str))
        .or_else(|| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn deck_user(v: &Value) -> DeckUser {
    let uid = v.get("uid").and_then(Value::as_str).or_else(|| v.as_str()).unwrap_or_default().to_string();
    let name = user_name(v);
    DeckUser { name: if name.is_empty() { uid.clone() } else { name }, uid }
}

fn parse_label(l: &Value) -> DeckLabel {
    DeckLabel { id: int(&l["id"]), title: text(&l["title"]), color: text(&l["color"]) }
}

pub fn parse_board_details(v: &Value) -> DeckBoardDetails {
    let mut users: Vec<DeckUser> = Vec::new();
    let mut add = |u: DeckUser| if !u.uid.is_empty() && !users.iter().any(|x| x.uid == u.uid) { users.push(u) };
    add(deck_user(&v["owner"]));
    for u in v["users"].as_array().into_iter().flatten() {
        add(deck_user(u));
    }
    // Участники-пользователи из списка доступа (type 0); группы не раскрываем.
    for a in v["acl"].as_array().into_iter().flatten() {
        if int(&a["type"]) == 0 {
            add(deck_user(&a["participant"]));
        }
    }
    users.sort_by_key(|u| u.name.to_lowercase());
    DeckBoardDetails {
        labels: v["labels"].as_array().map(|a| a.iter().map(parse_label).collect()).unwrap_or_default(),
        users,
    }
}

pub fn parse_board(v: &Value) -> DeckBoard {
    let p = &v["permissions"];
    DeckBoard {
        id: int(&v["id"]),
        title: text(&v["title"]),
        color: text(&v["color"]),
        archived: v["archived"].as_bool().unwrap_or(false),
        can_edit: p["PERMISSION_EDIT"].as_bool().unwrap_or(false),
        can_manage: p["PERMISSION_MANAGE"].as_bool().unwrap_or(false),
        owner: user_name(&v["owner"]),
    }
}

pub fn parse_card(v: &Value) -> DeckCard {
    let labels = v["labels"].as_array().map(|a| {
        a.iter().map(parse_label).collect()
    }).unwrap_or_default();
    let assigned = v["assignedUsers"].as_array().map(|a| {
        a.iter().map(|u| deck_user(u.get("participant").unwrap_or(u))).filter(|u| !u.uid.is_empty()).collect()
    }).unwrap_or_default();
    DeckCard {
        id: int(&v["id"]),
        stack_id: int(&v["stackId"]),
        title: text(&v["title"]),
        description: text(&v["description"]),
        order: int(&v["order"]),
        duedate: v["duedate"].as_str().map(str::to_string).filter(|s| !s.is_empty()),
        done: !v["done"].is_null() && v["done"] != Value::Bool(false),
        archived: v["archived"].as_bool().unwrap_or(false),
        labels,
        assigned,
        owner: user_name(&v["owner"]),
        comments_unread: int(&v["commentsUnread"]),
        attachment_count: int(&v["attachmentCount"]),
        created_at: int(&v["createdAt"]),
        last_modified: int(&v["lastModified"]),
    }
}

impl Nc {
    pub async fn deck_boards(&self) -> Result<Vec<DeckBoard>> {
        let (_, _, v) = self.call(Method::GET, &format!("{DECK}boards"), &[], None, "Доски Deck").await?;
        let mut boards: Vec<DeckBoard> = v.as_array().map(|a| a.iter().filter(|b| b["deletedAt"].as_i64().unwrap_or(0) == 0).map(parse_board).collect()).unwrap_or_default();
        boards.sort_by(|a, b| a.archived.cmp(&b.archived).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase())));
        Ok(boards)
    }

    pub async fn deck_stacks(&self, board: i64) -> Result<Vec<DeckStack>> {
        let (_, _, v) = self.call(Method::GET, &format!("{DECK}boards/{board}/stacks"), &[], None, "Колонки доски").await?;
        let mut stacks: Vec<DeckStack> = v.as_array().map(|a| a.iter().map(|s| {
            let mut cards: Vec<DeckCard> = s["cards"].as_array().map(|c| c.iter().map(parse_card).filter(|c| !c.archived).collect()).unwrap_or_default();
            cards.sort_by_key(|c| (c.order, c.id));
            DeckStack { id: int(&s["id"]), title: text(&s["title"]), order: int(&s["order"]), cards }
        }).collect()).unwrap_or_default();
        stacks.sort_by_key(|s| (s.order, s.id));
        Ok(stacks)
    }

    pub async fn deck_create_card(&self, board: i64, stack: i64, title: &str, description: &str, duedate: Option<&str>) -> Result<i64> {
        let title = title.trim();
        if title.is_empty() { return Err(AppError::Other("Укажите название карточки".into())); }
        let mut body = json!({ "title": title.chars().take(255).collect::<String>(), "type": "plain", "order": 999, "description": description });
        if let Some(d) = duedate { body["duedate"] = json!(d); }
        let (_, _, v) = self.call(Method::POST, &format!("{DECK}boards/{board}/stacks/{stack}/cards"), &[], Some(body), "Новая карточка").await?;
        Ok(int(&v["id"]))
    }

    /// PUT карточки требует все поля — берём текущую версию и меняем нужное.
    #[allow(clippy::too_many_arguments)]
    pub async fn deck_update_card(&self, board: i64, stack: i64, card: i64, title: &str, description: &str, duedate: Option<&str>, done: bool) -> Result<()> {
        let path = format!("{DECK}boards/{board}/stacks/{stack}/cards/{card}");
        let (_, _, cur) = self.call(Method::GET, &path, &[], None, "Карточка").await?;
        let owner = cur["owner"].get("uid").cloned().unwrap_or_else(|| cur["owner"].clone());
        let done_value = if done {
            cur["done"].as_str().map(|s| json!(s)).unwrap_or_else(|| json!(chrono::Utc::now().to_rfc3339()))
        } else { Value::Null };
        let body = json!({
            "title": title.trim(), "description": description, "type": cur["type"].as_str().unwrap_or("plain"),
            "owner": owner, "order": cur["order"], "duedate": duedate, "archived": cur["archived"].as_bool().unwrap_or(false),
            "done": done_value,
        });
        self.call(Method::PUT, &path, &[], Some(body), "Сохранение карточки").await?;
        Ok(())
    }

    pub async fn deck_move_card(&self, board: i64, _stack: i64, card: i64, target_stack: i64, order: i64) -> Result<()> {
        // Nextcloud подставляет параметры из пути поверх тела запроса: если в пути
        // старая колонка, `stackId` из тела игнорируется и карточка «переезжает»
        // в ту же колонку. Поэтому целевая колонка — и в пути, и в теле.
        self.call(Method::PUT, &format!("{DECK}boards/{board}/stacks/{target_stack}/cards/{card}/reorder"), &[],
            Some(json!({ "order": order, "stackId": target_stack })), "Перемещение карточки").await?;
        Ok(())
    }

    pub async fn deck_delete_card(&self, board: i64, stack: i64, card: i64) -> Result<()> {
        self.call(Method::DELETE, &format!("{DECK}boards/{board}/stacks/{stack}/cards/{card}"), &[], None, "Удаление карточки").await?;
        Ok(())
    }

    pub async fn deck_board_details(&self, board: i64) -> Result<DeckBoardDetails> {
        let (_, _, v) = self.call(Method::GET, &format!("{DECK}boards/{board}"), &[], None, "Доска").await?;
        Ok(parse_board_details(&v))
    }

    /// Назначить/снять исполнителя.
    pub async fn deck_assign(&self, board: i64, stack: i64, card: i64, uid: &str, assign: bool) -> Result<()> {
        let action = if assign { "assignUser" } else { "unassignUser" };
        self.call(Method::PUT, &format!("{DECK}boards/{board}/stacks/{stack}/cards/{card}/{action}"), &[],
            Some(json!({ "userId": uid })), "Исполнитель").await?;
        Ok(())
    }

    /// Поставить/снять метку.
    pub async fn deck_label(&self, board: i64, stack: i64, card: i64, label: i64, add: bool) -> Result<()> {
        let action = if add { "assignLabel" } else { "removeLabel" };
        self.call(Method::PUT, &format!("{DECK}boards/{board}/stacks/{stack}/cards/{card}/{action}"), &[],
            Some(json!({ "labelId": label })), "Метка").await?;
        Ok(())
    }

    /// Комментарии карточки (OCS API Deck), от старых к новым.
    pub async fn deck_comments(&self, card: i64) -> Result<Vec<DeckComment>> {
        let (_, _, d) = self.ocs(Method::GET, &format!("ocs/v2.php/apps/deck/api/v1.0/cards/{card}/comments"),
            &[("limit", "100".into()), ("offset", "0".into())], None, "Комментарии").await?;
        let mut out: Vec<DeckComment> = d.as_array().map(|a| a.iter().map(|c| {
            let author_id = text(&c["actorId"]);
            DeckComment {
                id: int(&c["id"]),
                author: c["actorDisplayName"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| author_id.clone()),
                own: author_id.eq_ignore_ascii_case(&self.me),
                author_id,
                message: text(&c["message"]),
                created: text(&c["creationDateTime"]),
            }
        }).collect()).unwrap_or_default();
        out.sort_by_key(|c| c.id);
        Ok(out)
    }

    pub async fn deck_comment_add(&self, card: i64, message: &str) -> Result<()> {
        let message = message.trim();
        if message.is_empty() { return Ok(()); }
        if message.chars().count() > 1000 { return Err(AppError::Other("Комментарий длиннее 1000 символов".into())); }
        self.ocs(Method::POST, &format!("ocs/v2.php/apps/deck/api/v1.0/cards/{card}/comments"), &[],
            Some(json!({ "message": message })), "Комментарий").await?;
        Ok(())
    }

    pub async fn deck_comment_delete(&self, card: i64, comment: i64) -> Result<()> {
        self.ocs(Method::DELETE, &format!("ocs/v2.php/apps/deck/api/v1.0/cards/{card}/comments/{comment}"), &[], None, "Удаление комментария").await?;
        Ok(())
    }

    /// Вложения карточки (API 1.1 — включая файлы из Nextcloud).
    pub async fn deck_attachments(&self, board: i64, stack: i64, card: i64) -> Result<Vec<DeckAttachment>> {
        let (_, _, v) = self.call(Method::GET, &format!("index.php/apps/deck/api/v1.1/boards/{board}/stacks/{stack}/cards/{card}/attachments"),
            &[], None, "Вложения").await?;
        Ok(v.as_array().map(|a| a.iter().filter(|x| int(&x["deletedAt"]) == 0).map(|x| {
            let ext = &x["extendedData"];
            let name = ext.pointer("/info/filename").and_then(Value::as_str).map(|f| {
                match ext.pointer("/info/extension").and_then(Value::as_str) { Some(e) if !e.is_empty() => format!("{f}.{e}"), _ => f.to_string() }
            }).unwrap_or_else(|| text(&x["data"]));
            let file_id = ext["fileid"].as_i64().or_else(|| (x["type"].as_str() == Some("file")).then(|| int(&x["id"])));
            DeckAttachment { id: int(&x["id"]), name, size: int(&ext["filesize"]), mimetype: text(&ext["mimetype"]), file_id, kind: x["type"].as_str().unwrap_or("file").to_string() }
        }).collect()).unwrap_or_default())
    }

    /// Добавить файл к карточке (как перетаскивание файла в карточку в веб-Deck).
    pub async fn deck_attachment_upload(&self, board: i64, stack: i64, card: i64, name: &str, data: Vec<u8>) -> Result<()> {
        let url = self
            .creds
            .base
            .join(&format!("index.php/apps/deck/api/v1.1/boards/{board}/stacks/{stack}/cards/{card}/attachments"))
            .map_err(|e| AppError::Other(e.to_string()))?;
        let part = reqwest::multipart::Part::bytes(data).file_name(name.to_string()).mime_str("application/octet-stream").map_err(|e| AppError::Other(e.to_string()))?;
        let form = reqwest::multipart::Form::new().text("type", "file").part("file", part);
        let resp = self
            .client
            .post(url)
            .basic_auth(&self.creds.login, Some(&self.creds.app_password))
            .header("OCS-APIRequest", "true")
            .header("Accept", "application/json")
            .multipart(form)
            .send()
            .await
            .map_err(net_error)?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = crate::netio::text_limited(resp, 64 * 1024).await.unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["message"].as_str().map(str::to_string)).unwrap_or_default();
            return Err(if msg.is_empty() { status_error(status, "Вложение") } else { AppError::Other(format!("Вложение: {msg}")) });
        }
        Ok(())
    }

    pub async fn deck_attachment_delete(&self, board: i64, stack: i64, card: i64, attachment: i64, kind: &str) -> Result<()> {
        let kind = if kind == "deck_file" { "deck_file" } else { "file" };
        self.call(Method::DELETE, &format!("index.php/apps/deck/api/v1.1/boards/{board}/stacks/{stack}/cards/{card}/attachments/{attachment}"),
            &[("type", kind.into())], None, "Удаление вложения").await?;
        Ok(())
    }

    /// Убрать карточку в архив доски (или вернуть из архива).
    pub async fn deck_archive(&self, board: i64, stack: i64, card: i64, archive: bool) -> Result<()> {
        let action = if archive { "archive" } else { "unarchive" };
        self.call(Method::PUT, &format!("{DECK}boards/{board}/stacks/{stack}/cards/{card}/{action}"), &[], None, "Архив").await?;
        Ok(())
    }

    /// История карточки из приложения Activity. Нет приложения — пустой список.
    pub async fn deck_activity(&self, card: i64) -> Result<Vec<DeckActivity>> {
        let query = [("format", "json".to_string()), ("object_type", "deck_card".to_string()), ("object_id", card.to_string()), ("limit", "50".to_string())];
        let Ok((_, _, d)) = self.ocs(Method::GET, "ocs/v2.php/apps/activity/api/v2/activity/filter", &query, None, "История").await else {
            return Ok(Vec::new());
        };
        Ok(d.as_array()
            .map(|a| {
                a.iter()
                    .map(|x| DeckActivity {
                        id: int(&x["activity_id"]),
                        author_id: text(&x["user"]),
                        author: x.pointer("/subject_rich/1/user/name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| text(&x["user"])),
                        subject: text(&x["subject"]),
                        date: text(&x["datetime"]),
                    })
                    .filter(|a| !a.subject.is_empty())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Новая доска; `stacks` — колонки, которые сразу создать (по порядку).
    pub async fn deck_create_board(&self, title: &str, color: &str, stacks: &[String]) -> Result<DeckBoard> {
        let title: String = title.trim().chars().take(100).collect();
        if title.is_empty() { return Err(AppError::Other("Укажите название доски".into())); }
        let color = color.trim().trim_start_matches('#');
        let color = if color.len() == 6 && color.chars().all(|c| c.is_ascii_hexdigit()) { color.to_lowercase() } else { "0082c9".into() };
        let (_, _, v) = self.call(Method::POST, &format!("{DECK}boards"), &[], Some(json!({ "title": title, "color": color })), "Новая доска").await?;
        let board = parse_board(&v);
        if board.id == 0 { return Err(AppError::Other("Nextcloud не вернул новую доску".into())); }
        for (i, st) in stacks.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).take(20).enumerate() {
            let st: String = st.chars().take(100).collect();
            self.call(Method::POST, &format!("{DECK}boards/{}/stacks", board.id), &[], Some(json!({ "title": st, "order": i })), "Новая колонка").await?;
        }
        Ok(board)
    }

    pub async fn deck_create_stack(&self, board: i64, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() { return Err(AppError::Other("Укажите название колонки".into())); }
        self.call(Method::POST, &format!("{DECK}boards/{board}/stacks"), &[], Some(json!({ "title": title, "order": 999 })), "Новая колонка").await?;
        Ok(())
    }
}

// ---------------------------------------------------------------- Talk

