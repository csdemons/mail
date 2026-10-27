//! Nextcloud: вход (Login Flow v2), Deck (доски/карточки), Talk (чат).
//! Авторизация — пароль приложения Nextcloud (а не пароль почты): его выдаёт
//! сервер после входа в браузере, он хранится в системном хранилище и может быть
//! отозван в настройках Nextcloud.

use std::time::Duration;

use reqwest::{Client, Method, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use url::Url;

use crate::error::{AppError, Result};

#[derive(Clone)]
pub struct NcCreds {
    pub base: Url,
    pub login: String,
    pub app_password: String,
}

impl std::fmt::Debug for NcCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NcCreds").field("base", &self.base.as_str()).field("login", &self.login).finish_non_exhaustive()
    }
}

pub fn normalize_base(raw: &str) -> Result<Url> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let mut url = Url::parse(&with_scheme).map_err(|_| AppError::Other(format!("Некорректный адрес Nextcloud: {raw}")))?;
    if url.scheme() != "https" && url.host_str() != Some("localhost") {
        return Err(AppError::Other("Nextcloud подключается только по HTTPS".into()));
    }
    if !url.path().ends_with('/') {
        let p = format!("{}/", url.path());
        url.set_path(&p);
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn http(timeout: Duration) -> Result<Client> {
    Client::builder()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(15))
        .user_agent(concat!("RRCKI Mail/", env!("CARGO_PKG_VERSION"), " (Desktop)"))
        .build()
        .map_err(|e| AppError::Other(format!("HTTP-клиент: {e}")))
}

fn net_error(e: reqwest::Error) -> AppError {
    let mut detail = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(cause) = src {
        detail.push_str(&format!(": {cause}"));
        src = cause.source();
    }
    AppError::Other(format!("Nextcloud недоступен: {detail}"))
}

fn status_error(status: StatusCode, what: &str) -> AppError {
    match status.as_u16() {
        401 => AppError::Other("Nextcloud: пароль приложения недействителен — подключите Nextcloud заново".into()),
        403 => AppError::Other(format!("{what}: нет прав")),
        404 => AppError::Other(format!("{what}: не найдено (возможно, приложение не установлено на сервере)")),
        412 => AppError::Other(format!("{what}: включено лобби, доступ только модераторам")),
        413 => AppError::Other("Сообщение слишком длинное".into()),
        _ => AppError::Other(format!("{what}: Nextcloud ответил HTTP {status}")),
    }
}

// ---------------------------------------------------------------- Login Flow v2

#[derive(Debug, Clone)]
pub struct LoginFlow {
    pub login_url: String,
    pub poll_endpoint: Url,
    pub poll_token: String,
}

pub async fn login_flow_start(base: &Url) -> Result<LoginFlow> {
    let client = http(Duration::from_secs(30))?;
    let url = base.join("index.php/login/v2").expect("valid path");
    let resp = client.post(url).send().await.map_err(net_error)?;
    if !resp.status().is_success() {
        return Err(status_error(resp.status(), "Вход в Nextcloud"));
    }
    let v: Value = resp.json().await.map_err(net_error)?;
    let s = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_string);
    let endpoint = s("/poll/endpoint").ok_or_else(|| AppError::Other("Nextcloud не вернул адрес проверки входа".into()))?;
    let poll_endpoint = Url::parse(&endpoint).map_err(|e| AppError::Other(e.to_string()))?;
    // Ответ сервера не должен уводить пароль приложения на чужой хост.
    if poll_endpoint.origin() != base.origin() {
        return Err(AppError::Other("Nextcloud вернул адрес на другом сервере — вход прерван".into()));
    }
    Ok(LoginFlow {
        login_url: s("/login").ok_or_else(|| AppError::Other("Nextcloud не вернул адрес входа".into()))?,
        poll_endpoint,
        poll_token: s("/poll/token").ok_or_else(|| AppError::Other("Nextcloud не вернул токен входа".into()))?,
    })
}

/// `Ok(None)` — пользователь ещё не подтвердил вход в браузере.
pub async fn login_flow_poll(base: &Url, flow: &LoginFlow) -> Result<Option<NcCreds>> {
    let client = http(Duration::from_secs(30))?;
    let resp = client
        .post(flow.poll_endpoint.clone())
        .form(&[("token", flow.poll_token.as_str())])
        .send()
        .await
        .map_err(net_error)?;
    if resp.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(status_error(resp.status(), "Вход в Nextcloud"));
    }
    let v: Value = resp.json().await.map_err(net_error)?;
    let login = v["loginName"].as_str().unwrap_or_default().to_string();
    let app_password = v["appPassword"].as_str().unwrap_or_default().to_string();
    if login.is_empty() || app_password.is_empty() {
        return Err(AppError::Other("Nextcloud вернул неполные данные входа".into()));
    }
    Ok(Some(NcCreds { base: base.clone(), login, app_password }))
}

// ---------------------------------------------------------------- клиент

pub struct Nc {
    client: Client,
    pub creds: NcCreds,
    /// ID пользователя Nextcloud (actorId в Talk) — может отличаться от логина.
    pub me: String,
}

impl Nc {
    pub fn new(creds: NcCreds, me: &str) -> Result<Self> {
        Ok(Self { client: http(Duration::from_secs(45))?, creds, me: me.to_string() })
    }

    /// Загрузка файлов: большие файлы по медленному каналу идут долго.
    pub fn upload(creds: NcCreds, me: &str) -> Result<Self> {
        Ok(Self { client: http(Duration::from_secs(30 * 60))?, creds, me: me.to_string() })
    }

    /// Long-poll Talk ждёт до 30 с — нужен клиент с запасом по времени.
    pub fn long_poll(creds: NcCreds, me: &str) -> Result<Self> {
        Ok(Self { client: http(Duration::from_secs(75))?, creds, me: me.to_string() })
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
        what: &str,
    ) -> Result<(StatusCode, reqwest::header::HeaderMap, Value)> {
        let url = self.creds.base.join(path).map_err(|e| AppError::Other(e.to_string()))?;
        let mut req = self
            .client
            .request(method, url)
            .basic_auth(&self.creds.login, Some(&self.creds.app_password))
            .header("OCS-APIRequest", "true")
            .header("Accept", "application/json")
            .query(query);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(net_error)?;
        let status = resp.status();
        let headers = resp.headers().clone();
        if status == StatusCode::NOT_MODIFIED {
            return Ok((status, headers, Value::Null));
        }
        if !status.is_success() {
            return Err(status_error(status, what));
        }
        let text = resp.text().await.map_err(net_error)?;
        let v = if text.trim().is_empty() { Value::Null } else {
            serde_json::from_str(&text).map_err(|_| AppError::Other(format!("{what}: Nextcloud вернул не JSON")))?
        };
        Ok((status, headers, v))
    }

    /// OCS-ответ: `{ ocs: { meta, data } }` → data.
    async fn ocs(&self, method: Method, path: &str, query: &[(&str, String)], body: Option<Value>, what: &str) -> Result<(StatusCode, reqwest::header::HeaderMap, Value)> {
        let (status, headers, v) = self.call(method, path, query, body, what).await?;
        let data = v.pointer("/ocs/data").cloned().unwrap_or(v);
        Ok((status, headers, data))
    }

    /// (ID пользователя, отображаемое имя). Заодно проверяет пароль приложения.
    pub async fn user_info(&self) -> Result<(String, String)> {
        let (_, _, d) = self.ocs(Method::GET, "ocs/v1.php/cloud/user", &[("format", "json".into())], None, "Профиль Nextcloud").await?;
        let id = d["id"].as_str().unwrap_or(&self.creds.login).to_string();
        let name = d["displayname"].as_str().or(d["display-name"].as_str()).unwrap_or(&id).to_string();
        Ok((id, name))
    }

    pub async fn revoke(&self) {
        let _ = self.ocs(Method::DELETE, "ocs/v2.php/core/apppassword", &[], None, "Отзыв пароля").await;
    }
}

// ---------------------------------------------------------------- Deck

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
    pub assigned: Vec<String>,
    pub owner: String,
    pub comments_unread: i64,
    pub attachment_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckStack {
    pub id: i64,
    pub title: String,
    pub order: i64,
    pub cards: Vec<DeckCard>,
}

fn int(v: &Value) -> i64 {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)
}
fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}
fn user_name(v: &Value) -> String {
    v.get("displayname").and_then(Value::as_str)
        .or_else(|| v.get("uid").and_then(Value::as_str))
        .or_else(|| v.as_str())
        .unwrap_or_default()
        .to_string()
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
        a.iter().map(|l| DeckLabel { id: int(&l["id"]), title: text(&l["title"]), color: text(&l["color"]) }).collect()
    }).unwrap_or_default();
    let assigned = v["assignedUsers"].as_array().map(|a| {
        a.iter().map(|u| user_name(u.get("participant").unwrap_or(u))).filter(|s| !s.is_empty()).collect()
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

    pub async fn deck_create_card(&self, board: i64, stack: i64, title: &str, description: &str, duedate: Option<&str>) -> Result<()> {
        let title = title.trim();
        if title.is_empty() { return Err(AppError::Other("Укажите название карточки".into())); }
        let mut body = json!({ "title": title.chars().take(255).collect::<String>(), "type": "plain", "order": 999, "description": description });
        if let Some(d) = duedate { body["duedate"] = json!(d); }
        self.call(Method::POST, &format!("{DECK}boards/{board}/stacks/{stack}/cards"), &[], Some(body), "Новая карточка").await?;
        Ok(())
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

    pub async fn deck_create_stack(&self, board: i64, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() { return Err(AppError::Other("Укажите название колонки".into())); }
        self.call(Method::POST, &format!("{DECK}boards/{board}/stacks"), &[], Some(json!({ "title": title, "order": 999 })), "Новая колонка").await?;
        Ok(())
    }
}

// ---------------------------------------------------------------- Talk

const TALK_ROOM: &str = "ocs/v2.php/apps/spreed/api/v4/room";
const TALK_CHAT: &str = "ocs/v2.php/apps/spreed/api/v1/chat/";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkRoom {
    pub token: String,
    pub name: String,
    /// 1 — личная, 2 — группа, 3 — публичная, 4 — журнал изменений, 6 — заметки
    pub kind: i64,
    pub unread: i64,
    pub unread_mention: bool,
    pub last_activity: i64,
    pub last_message: Option<String>,
    pub read_only: bool,
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
}

/// Порция сообщений и наибольший id, пришедший с сервера (включая служебные),
/// — от него продолжается опрос.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkChunk {
    pub messages: Vec<TalkMessage>,
    pub last_id: i64,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcUser {
    pub id: String,
    pub label: String,
    pub sub: String,
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
                    files.push(TalkFile { id: text(&p["id"]).or_int(&p["id"]), name: name.to_string(), size: int(&p["size"]), mimetype: text(&p["mimetype"]) });
                    String::new()
                }
                _ => name.to_string(),
            };
            out = out.replace(&format!("{{{key}}}"), &shown);
        }
    }
    (out.trim().to_string(), files)
}

trait OrInt {
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
fn edit_text(v: &Value) -> String {
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

fn reactions(v: &Value) -> Vec<TalkReaction> {
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

fn parse_chat(d: &Value, me: &str) -> Vec<TalkMessage> {
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
    pub async fn talk_rooms(&self) -> Result<Vec<TalkRoom>> {
        let (_, _, d) = self.ocs(Method::GET, TALK_ROOM, &[], None, "Беседы Talk").await?;
        let mut rooms: Vec<TalkRoom> = d.as_array().map(|a| a.iter().map(|r| TalkRoom {
            token: text(&r["token"]),
            name: r["displayName"].as_str().or(r["name"].as_str()).unwrap_or_default().to_string(),
            kind: int(&r["type"]),
            unread: int(&r["unreadMessages"]),
            unread_mention: r["unreadMention"].as_bool().unwrap_or(false),
            last_activity: int(&r["lastActivity"]),
            last_message: r["lastMessage"].is_object().then(|| {
                let (t, f) = render_message(&r["lastMessage"]);
                let t = match f.first() { Some(file) if t.is_empty() => format!("📎 {}", file.name), _ => t };
                let who = text(&r["lastMessage"]["actorDisplayName"]);
                if who.is_empty() { t } else { format!("{who}: {t}") }
            }),
            read_only: int(&r["readOnly"]) == 1,
        }).filter(|r| r.kind != 4).collect()).unwrap_or_default();
        rooms.sort_by_key(|r| std::cmp::Reverse(r.last_activity));
        Ok(rooms)
    }

    /// История: последние сообщения (или более старые, чем `before`). Помечает беседу прочитанной.
    pub async fn talk_history(&self, token: &str, before: Option<i64>) -> Result<TalkChunk> {
        let mut q = vec![("lookIntoFuture", "0".to_string()), ("limit", "100".to_string()), ("setReadMarker", "1".to_string())];
        if let Some(b) = before { q.push(("lastKnownMessageId", b.to_string())); }
        let (status, _, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{token}"), &q, None, "Сообщения").await?;
        if status == StatusCode::NOT_MODIFIED { return Ok(TalkChunk { messages: Vec::new(), last_id: 0 }); }
        Ok(TalkChunk { messages: parse_chat(&d, &self.me), last_id: max_id(&d) })
    }

    /// Ждёт новые сообщения до 30 с (long-poll). Пусто — новых нет.
    pub async fn talk_poll(&self, token: &str, last_known: i64) -> Result<TalkChunk> {
        let q = vec![("lookIntoFuture", "1".to_string()), ("lastKnownMessageId", last_known.to_string()),
            ("timeout", "30".to_string()), ("limit", "100".to_string()), ("setReadMarker", "1".to_string())];
        let (status, _, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{token}"), &q, None, "Новые сообщения").await?;
        if status == StatusCode::NOT_MODIFIED { return Ok(TalkChunk { messages: Vec::new(), last_id: 0 }); }
        Ok(TalkChunk { messages: parse_chat(&d, &self.me), last_id: max_id(&d) })
    }

    pub async fn talk_send(&self, token: &str, message: &str, reply_to: Option<i64>) -> Result<()> {
        let message = message.trim();
        if message.is_empty() { return Ok(()); }
        let mut body = json!({ "message": message });
        if let Some(r) = reply_to { body["replyTo"] = json!(r); }
        self.ocs(Method::POST, &format!("{TALK_CHAT}{token}"), &[], Some(body), "Отправка сообщения").await?;
        Ok(())
    }

    pub async fn talk_react(&self, token: &str, message_id: i64, emoji: &str, add: bool) -> Result<()> {
        let path = format!("ocs/v2.php/apps/spreed/api/v1/reaction/{token}/{message_id}");
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
        self.ocs(Method::PUT, &format!("{TALK_CHAT}{token}/{message_id}"), &[], Some(json!({ "message": message })), "Правка сообщения").await?;
        Ok(())
    }

    pub async fn talk_delete(&self, token: &str, message_id: i64) -> Result<()> {
        self.ocs(Method::DELETE, &format!("{TALK_CHAT}{token}/{message_id}"), &[], None, "Удаление сообщения").await?;
        Ok(())
    }

    /// Кого можно упомянуть в беседе (подсказка после «@»).
    pub async fn talk_mentions(&self, token: &str, search: &str) -> Result<Vec<TalkMention>> {
        let q = [("search", search.to_string()), ("limit", "8".to_string()), ("includeStatus", "false".to_string())];
        let (_, _, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{token}/mentions"), &q, None, "Упоминания").await?;
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
            ("shareTypes[]", "0".to_string()), ("limit", "10".to_string())];
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
                self.ocs(Method::POST, &format!("{TALK_ROOM}/{token}/participants"), &[], Some(json!({ "newParticipant": u, "source": "users" })), "Участник беседы").await?;
            }
        }
        Ok(token)
    }

    /// URL в WebDAV файлов пользователя: `remote.php/dav/files/<uid>/<path>`.
    fn files_url(&self, path: &str) -> Result<Url> {
        let mut url = self.creds.base.join("remote.php/dav/files/").map_err(|e| AppError::Other(e.to_string()))?;
        {
            let mut seg = url.path_segments_mut().map_err(|_| AppError::Other("Некорректный адрес Nextcloud".into()))?;
            seg.pop_if_empty().push(&self.me);
            for part in path.split('/').filter(|p| !p.is_empty()) {
                seg.push(part);
            }
        }
        Ok(url)
    }

    fn dav(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.creds.login, Some(&self.creds.app_password))
            .header("OCS-APIRequest", "true")
    }

    /// Папка вложений Talk из настроек пользователя (по умолчанию «/Talk»).
    async fn talk_attachment_folder(&self) -> String {
        let folder = self
            .ocs(Method::GET, "ocs/v2.php/cloud/capabilities", &[("format", "json".into())], None, "Возможности сервера")
            .await
            .ok()
            .and_then(|(_, _, d)| d.pointer("/capabilities/spreed/config/attachments/folder").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let folder = folder.trim_matches('/');
        if folder.is_empty() || folder.split('/').any(|p| p == ".." || p == ".") { "Talk".into() } else { folder.to_string() }
    }

    /// Отправляет файл в беседу так же, как веб-интерфейс Talk: загружает его
    /// в папку вложений пользователя и публикует в беседе (shareType 10).
    /// Имя не перезаписывает существующие файлы: «файл (1).pdf», «файл (2).pdf»…
    pub async fn talk_send_file(&self, token: &str, name: &str, data: Vec<u8>, caption: &str, reply_to: Option<i64>) -> Result<String> {
        let folder = self.talk_attachment_folder().await;
        let mut acc = String::new();
        for part in folder.split('/') {
            acc.push('/');
            acc.push_str(part);
            let resp = self
                .dav(Method::from_bytes(b"MKCOL").expect("MKCOL"), self.files_url(&acc)?)
                .send()
                .await
                .map_err(net_error)?;
            // 201 — создана, 405 — уже существует.
            if !(resp.status().is_success() || resp.status() == StatusCode::METHOD_NOT_ALLOWED) {
                return Err(status_error(resp.status(), "Папка вложений Talk"));
            }
        }
        let mut stored = None;
        for n in 0..100 {
            let candidate = numbered_name(name, n);
            let path = format!("{acc}/{candidate}");
            let resp = self
                .dav(Method::PUT, self.files_url(&path)?)
                .header("If-None-Match", "*")
                .header("Content-Type", mime_guess::from_path(name).first_or_octet_stream().essence_str())
                .body(data.clone())
                .send()
                .await
                .map_err(net_error)?;
            match resp.status() {
                s if s.is_success() => {
                    stored = Some(path);
                    break;
                }
                StatusCode::PRECONDITION_FAILED => continue,
                StatusCode::INSUFFICIENT_STORAGE => return Err(AppError::Other("В Nextcloud закончилось место (квота)".into())),
                StatusCode::PAYLOAD_TOO_LARGE => return Err(AppError::Other("Файл слишком большой для сервера Nextcloud".into())),
                s => return Err(status_error(s, "Загрузка файла")),
            }
        }
        let path = stored.ok_or_else(|| AppError::Other("Не удалось подобрать свободное имя файла".into()))?;
        let mut meta = json!({ "messageType": "comment" });
        if !caption.trim().is_empty() { meta["caption"] = json!(caption.trim()); }
        if let Some(r) = reply_to { meta["replyTo"] = json!(r); }
        let body = json!({ "shareType": 10, "shareWith": token, "path": path, "talkMetaData": meta.to_string() });
        self.ocs(Method::POST, "ocs/v2.php/apps/files_sharing/api/v1/shares", &[("format", "json".into())], Some(body), "Публикация файла в беседе").await?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_base_url() {
        assert_eq!(normalize_base("next.nrcki.ru").unwrap().as_str(), "https://next.nrcki.ru/");
        assert_eq!(normalize_base("https://next.nrcki.ru/nc").unwrap().as_str(), "https://next.nrcki.ru/nc/");
        assert!(normalize_base("http://next.nrcki.ru").is_err());
    }

    #[test]
    fn renders_talk_parameters() {
        let v = json!({
            "id": 7, "actorType": "users", "actorId": "yushkov_do@nrcki.ru", "actorDisplayName": "Юшков Дмитрий",
            "timestamp": 1793000000, "message": "{mention-user1}, посмотри {file}", "messageType": "comment", "systemMessage": "",
            "messageParameters": {
                "mention-user1": { "type": "user", "id": "ivanov", "name": "Иванов Иван" },
                "file": { "type": "file", "id": "42", "name": "Отчёт.docx" }
            },
            "parent": { "id": 5, "actorDisplayName": "Иванов Иван", "message": "Где отчёт?", "messageParameters": [] }
        });
        let m = parse_message(&v, "yushkov_do@nrcki.ru");
        assert_eq!(m.text, "@Иванов Иван, посмотри");
        assert_eq!(m.files, vec![TalkFile { id: "42".into(), name: "Отчёт.docx".into(), size: 0, mimetype: String::new() }]);
        // Сообщение «только файл»: текста нет, ID числом.
        let (t, f) = render_message(&json!({"message": "{file}", "messageParameters": {"file": {"type": "file", "id": 9, "name": "a.png", "size": 5, "mimetype": "image/png"}}}));
        assert!(t.is_empty() && f[0].id == "9" && f[0].size == 5);
        assert!(m.own && !m.system);
        assert_eq!(m.parent_text.as_deref(), Some("Где отчёт?"));
        // Пустые параметры приходят как [] — не должно падать.
        assert_eq!(render_message(&json!({"message": "Привет", "messageParameters": []})).0, "Привет");
    }

    #[test]
    fn talk_updates_reactions_and_edits() {
        let me = "test";
        let chat = json!([
            {"id": 10, "actorType": "users", "actorId": "test", "actorDisplayName": "Я", "timestamp": 1, "message": "Привет, {mention-user1}",
             "messageType": "comment", "systemMessage": "", "lastEditTime": 0,
             "messageParameters": {"mention-user1": {"type": "user", "id": "ivanov@nrcki.ru", "name": "Иванов", "mention-id": "ivanov@nrcki.ru"}},
             "reactions": {"👍": 2, "🎉": 1}, "reactionsSelf": ["👍"]},
            {"id": 11, "actorType": "users", "actorId": "ivanov", "actorDisplayName": "Иванов", "timestamp": 2, "message": "{actor} reacted",
             "messageType": "system", "systemMessage": "reaction",
             "parent": {"id": 10, "actorType": "users", "actorId": "test", "actorDisplayName": "Я", "timestamp": 1, "message": "Привет", "messageType": "comment",
                        "systemMessage": "", "lastEditTime": 5, "messageParameters": [], "reactions": {"👍": 3}, "reactionsSelf": ["👍"]}},
            {"id": 12, "actorType": "users", "actorId": "test", "actorDisplayName": "Я", "timestamp": 3, "message": "x", "messageType": "system", "systemMessage": "message_deleted",
             "parent": {"id": 9, "actorType": "users", "actorId": "test", "actorDisplayName": "Я", "timestamp": 0, "message": "Message deleted by you", "messageType": "comment_deleted", "systemMessage": "", "messageParameters": []}},
            {"id": 13, "actorType": "users", "actorId": "x", "message": "{actor} joined", "messageType": "system", "systemMessage": "user_added", "messageParameters": []}
        ]);
        let m = parse_chat(&chat, me);
        assert_eq!(m.iter().map(|x| (x.id, x.update)).collect::<Vec<_>>(), vec![(10, false), (13, false), (10, true), (9, true)]);
        assert_eq!(m[0].reactions, vec![TalkReaction { emoji: "👍".into(), count: 2, own: true }, TalkReaction { emoji: "🎉".into(), count: 1, own: false }]);
        assert_eq!(m[0].edit_text.as_deref(), Some("Привет, @\"ivanov@nrcki.ru\""));
        assert!(m[2].edited && m[2].reactions[0].count == 3);
        assert!(m[3].deleted && m[3].text == "Сообщение удалено" && m[3].edit_text.is_none());
        assert!(m[1].system);
    }

    #[test]
    fn numbers_duplicate_names() {
        assert_eq!(numbered_name("отчёт.pdf", 0), "отчёт.pdf");
        assert_eq!(numbered_name("отчёт.pdf", 2), "отчёт (2).pdf");
        assert_eq!(numbered_name(".bashrc", 1), ".bashrc (1)");
        assert_eq!(numbered_name("README", 1), "README (1)");
    }

    #[test]
    fn parses_deck_card_variants() {
        let c = parse_card(&json!({
            "id": "12", "stackId": 3, "title": "Обновить RuPost", "description": "", "order": 1,
            "duedate": "2026-10-30T00:00:00+00:00", "done": null, "archived": false,
            "labels": [{"id": 1, "title": "Срочно", "color": "ff0000", "boardId": "1"}],
            "assignedUsers": [{"participant": {"uid": "ivanov", "displayname": "Иванов"}}],
            "owner": {"uid": "yushkov_do", "displayname": "Юшков"}, "commentsUnread": 2
        }));
        assert_eq!((c.id, c.stack_id, c.done), (12, 3, false));
        assert_eq!(c.assigned, vec!["Иванов"]);
        assert_eq!(c.labels[0].title, "Срочно");
        let done = parse_card(&json!({"id": 1, "done": "2026-10-27T10:00:00+00:00", "owner": "u"}));
        assert!(done.done && done.owner == "u");
    }
}
