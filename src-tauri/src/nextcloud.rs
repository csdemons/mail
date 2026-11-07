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
        // Перенаправления — только в пределах того же сервера (схема, имя, порт):
        // пароль приложения и токены входа не уходят на чужой адрес или по http.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let same = attempt.previous().first().is_some_and(|first| first.origin() == attempt.url().origin());
            if same && attempt.previous().len() < 5 { attempt.follow() } else { attempt.stop() }
        }))
        .build()
        .map_err(|e| AppError::Other(format!("HTTP-клиент: {e}")))
}

/// Один сегмент пути URL (ID пользователя может содержать пробелы, @, кириллицу).
fn seg(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'@' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
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
    let v: Value = crate::netio::json_limited(resp, crate::netio::SMALL_LIMIT).await?;
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
    let v: Value = crate::netio::json_limited(resp, crate::netio::SMALL_LIMIT).await?;
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
        let text = crate::netio::text_limited(resp, crate::netio::API_LIMIT).await?;
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

    /// Картинка (аватар) по пути Nextcloud → `data:`-URL. `None` — картинки нет.
    pub async fn image(&self, path: &str) -> Result<Option<String>> {
        use base64::Engine as _;
        let url = self.creds.base.join(path).map_err(|e| AppError::Other(e.to_string()))?;
        let resp = self
            .client
            .get(url)
            .basic_auth(&self.creds.login, Some(&self.creds.app_password))
            .header("OCS-APIRequest", "true")
            .send()
            .await
            .map_err(net_error)?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
            .unwrap_or_default();
        if !ct.starts_with("image/") {
            return Ok(None);
        }
        // Аватар больше 2 МБ не нужен: загрузка прерывается на пределе.
        let Ok(bytes) = crate::netio::read_limited(resp, 2 * 1024 * 1024).await else {
            return Ok(None);
        };
        Ok(Some(format!("data:{ct};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes))))
    }

    /// Аватар пользователя (тот же, что в веб-интерфейсе; без своего — цветная буква Nextcloud).
    pub async fn user_avatar(&self, uid: &str, size: u32) -> Result<Option<String>> {
        let path = format!("index.php/avatar/{}/{size}", seg(uid));
        self.image(&path).await
    }

    /// Аватар беседы Talk (группы; у личной беседы — аватар собеседника).
    pub async fn room_avatar(&self, token: &str) -> Result<Option<String>> {
        let path = format!("ocs/v2.php/apps/spreed/api/v1/room/{}/avatar", seg(token));
        self.image(&path).await
    }

    /// Уменьшенная копия файла (фото в чате) — `core/preview` по ID файла.
    pub async fn file_preview(&self, file_id: &str, size: u32) -> Result<Option<String>> {
        if file_id.is_empty() || !file_id.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(None);
        }
        self.image(&format!("index.php/core/preview?fileId={file_id}&x={size}&y={size}&a=1&forceIcon=0")).await
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
const TALK_POLL: &str = "ocs/v2.php/apps/spreed/api/v1/poll/";
/// Порция истории беседы (более ранние — следующим запросом с lastKnownMessageId).
const TALK_PAGE: usize = 100;

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
    /// Личная беседа: ID собеседника (для аватара).
    pub peer: Option<String>,
    /// Избранная беседа (как «Избранное» в Talk) — показывается выше остальных.
    pub favorite: bool,
    /// Последнее сообщение, прочитанное всеми участниками (для «✓✓»); 0 — неизвестно.
    pub last_common_read: i64,
    /// Моя роль: 1 — владелец, 2 — модератор, 3 — участник, 4 — гость, 5 — сам вошёл, 6 — гость-модератор.
    pub role: i64,
    /// Я владелец или модератор — могу управлять беседой.
    pub moderator: bool,
    pub description: String,
    /// Уведомления: 1 — всегда, 2 — при упоминании, 3 — никогда (0 — по умолчанию).
    pub notify_level: i64,
    pub can_leave: bool,
    pub can_delete: bool,
    /// Исчезающие сообщения: через сколько секунд удаляются; 0 — выключено.
    pub message_expiration: i64,
}

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
        poll: poll_ref(v),
        voice: v["messageType"].as_str() == Some("voice-message"),
        expires: int(&v["expirationTimestamp"]),
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
            peer: (int(&r["type"]) == 1).then(|| text(&r["name"])).filter(|n| !n.is_empty()),
            favorite: r["isFavorite"].as_bool().unwrap_or(false),
            last_common_read: int(&r["lastCommonReadMessage"]),
            role: int(&r["participantType"]),
            moderator: matches!(int(&r["participantType"]), 1 | 2 | 6),
            description: text(&r["description"]),
            notify_level: int(&r["notificationLevel"]),
            can_leave: r["canLeaveConversation"].as_bool().unwrap_or(true),
            can_delete: r["canDeleteConversation"].as_bool().unwrap_or(false),
            message_expiration: int(&r["messageExpiration"]),
        }).filter(|r| r.kind != 4).collect()).unwrap_or_default();
        // Как в Talk: избранные сверху, внутри — по последней активности.
        rooms.sort_by_key(|r| (std::cmp::Reverse(r.favorite), std::cmp::Reverse(r.last_activity)));
        Ok(rooms)
    }

    /// История: последние сообщения (или более старые, чем `before`). Помечает беседу прочитанной.
    pub async fn talk_history(&self, token: &str, before: Option<i64>) -> Result<TalkChunk> {
        let mut q = vec![("lookIntoFuture", "0".to_string()), ("limit", TALK_PAGE.to_string()), ("setReadMarker", "1".to_string())];
        if let Some(b) = before { q.push(("lastKnownMessageId", b.to_string())); }
        let (status, h, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{}", seg(token)), &q, None, "Сообщения").await?;
        if status == StatusCode::NOT_MODIFIED { return Ok(TalkChunk { messages: Vec::new(), last_id: 0, last_common_read: common_read(&h), more: false }); }
        let more = d.as_array().is_some_and(|a| a.len() >= TALK_PAGE);
        Ok(TalkChunk { messages: parse_chat(&d, &self.me), last_id: max_id(&d), last_common_read: common_read(&h), more })
    }

    /// Ждёт новые сообщения до 30 с (long-poll). Пусто — новых нет.
    pub async fn talk_poll(&self, token: &str, last_known: i64) -> Result<TalkChunk> {
        let q = vec![("lookIntoFuture", "1".to_string()), ("lastKnownMessageId", last_known.to_string()),
            ("timeout", "30".to_string()), ("limit", "100".to_string()), ("setReadMarker", "1".to_string())];
        let (status, h, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{}", seg(token)), &q, None, "Новые сообщения").await?;
        if status == StatusCode::NOT_MODIFIED { return Ok(TalkChunk { messages: Vec::new(), last_id: 0, last_common_read: common_read(&h), more: false }); }
        Ok(TalkChunk { messages: parse_chat(&d, &self.me), last_id: max_id(&d), last_common_read: common_read(&h), more: false })
    }

    /// `silent` — без звука уведомления у получателей («тихая отправка»).
    pub async fn talk_send(&self, token: &str, message: &str, reply_to: Option<i64>, silent: bool) -> Result<()> {
        let message = message.trim();
        if message.is_empty() { return Ok(()); }
        let mut body = json!({ "message": message });
        if let Some(r) = reply_to { body["replyTo"] = json!(r); }
        if silent { body["silent"] = json!(true); }
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
        let kind = match kind { "media" | "file" | "audio" | "voice" | "other" | "location" | "deckcard" | "poll" | "recording" => kind, _ => "media" };
        let mut q = vec![("objectType", kind.to_string()), ("limit", "100".to_string())];
        if let Some(b) = before { q.push(("lastKnownMessageId", b.to_string())); }
        let (_, _, d) = self.ocs(Method::GET, &format!("{TALK_CHAT}{}/share", seg(token)), &q, None, "Медиа беседы").await?;
        let items: Vec<&Value> = match &d { Value::Object(m) => m.values().collect(), Value::Array(a) => a.iter().collect(), _ => Vec::new() };
        let mut out: Vec<TalkMessage> = items.into_iter().map(|m| parse_message(m, &self.me)).filter(|m| !m.deleted).collect();
        out.sort_by_key(|m| std::cmp::Reverse(m.id));
        Ok(out)
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
    fn files_url(&self, path: &str) -> Result<Url> {
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
}

/// Как опубликовать файл в беседе.
#[derive(Debug, Default, Clone, Copy)]
pub struct TalkFileOpts {
    pub reply_to: Option<i64>,
    /// Голосовое сообщение (проигрыватель вместо файла).
    pub voice: bool,
    /// Без звука уведомления.
    pub silent: bool,
}

impl Nc {
    /// Отправляет файл в беседу так же, как веб-интерфейс Talk: загружает его
    /// в папку вложений пользователя и публикует в беседе (shareType 10).
    /// Имя не перезаписывает существующие файлы: «файл (1).pdf», «файл (2).pdf»…
    /// `voice` — голосовое сообщение (Talk показывает проигрыватель), `silent` — без звука уведомления.
    pub async fn talk_send_file(&self, token: &str, name: &str, src: UploadSource, caption: &str, opts: TalkFileOpts) -> Result<String> {
        let TalkFileOpts { reply_to, voice, silent } = opts;
        let folder = self.talk_attachment_folder().await;
        let path = self.upload_unique_src(&folder, name, src).await?;
        let mut meta = json!({ "messageType": if voice { "voice-message" } else { "comment" } });
        if silent { meta["silent"] = json!(true); }
        if !caption.trim().is_empty() { meta["caption"] = json!(caption.trim()); }
        if let Some(r) = reply_to { meta["replyTo"] = json!(r); }
        let body = json!({ "shareType": 10, "shareWith": token, "path": path, "talkMetaData": meta.to_string() });
        self.ocs(Method::POST, "ocs/v2.php/apps/files_sharing/api/v1/shares", &[("format", "json".into())], Some(body), "Публикация файла в беседе").await?;
        Ok(path)
    }
}

// ---------------------------------------------------------------- Файлы

/// Файл или папка в Nextcloud. `path` — от корня файлов пользователя, с «/».
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NcFile {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: i64,
    pub modified: String,
    pub mime: String,
    pub file_id: Option<i64>,
    pub shared: bool,
    pub favorite: bool,
    pub can_write: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NcShare {
    pub id: String,
    /// 0 — пользователь, 1 — группа, 3 — публичная ссылка, 4 — e-mail, 10 — Talk.
    pub share_type: i64,
    pub share_with: String,
    pub share_with_name: String,
    pub url: String,
    pub expiration: String,
    pub has_password: bool,
    pub can_edit: bool,
}

/// Путь внутри файлов пользователя: только «/a/b», без «..» и пустых частей.
pub fn clean_path(path: &str) -> Result<String> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.iter().any(|p| *p == ".." || *p == "." || p.contains('\\') || p.chars().any(char::is_control)) {
        return Err(AppError::Other("Некорректный путь".into()));
    }
    Ok(format!("/{}", parts.join("/")))
}

fn valid_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() || n == "." || n == ".." || n.contains(['/', '\\']) || n.chars().any(char::is_control) {
        return Err(AppError::Other("Недопустимое имя".into()));
    }
    Ok(n.to_string())
}

const PROPFIND_FILES: &str = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/><d:getcontenttype/><oc:fileid/><oc:size/><oc:favorite/><oc:share-types/><oc:permissions/></d:prop></d:propfind>"#;

/// Разбор ответа PROPFIND по файлам: `root` — путь URL каталога files/<uid>/.
pub fn parse_file_list(xml: &str, root: &str) -> Result<Vec<NcFile>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| AppError::Other(format!("Ответ WebDAV: {e}")))?;
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.has_tag_name(("DAV:", "response"))) {
        let href = resp.descendants().find(|n| n.has_tag_name(("DAV:", "href"))).and_then(|n| n.text()).unwrap_or_default();
        let decoded = percent_encoding::percent_decode_str(href).decode_utf8_lossy().to_string();
        let Some(rel) = decoded.strip_prefix(root) else { continue };
        let path = format!("/{}", rel.trim_matches('/'));
        // Только свойства со статусом 200.
        let ok = resp.children().filter(|n| n.has_tag_name(("DAV:", "propstat"))).find(|ps| {
            ps.descendants().find(|n| n.has_tag_name(("DAV:", "status"))).and_then(|n| n.text()).is_some_and(|t| t.contains(" 200"))
        });
        let Some(ps) = ok else { continue };
        let prop = |ns: &str, name: &str| ps.descendants().find(|n| n.has_tag_name((ns, name))).and_then(|n| n.text()).unwrap_or_default().trim().to_string();
        let is_dir = ps.descendants().any(|n| n.has_tag_name(("DAV:", "collection")));
        let size_str = if is_dir { prop("http://owncloud.org/ns", "size") } else { prop("DAV:", "getcontentlength") };
        let perms = prop("http://owncloud.org/ns", "permissions");
        let shared = ps.descendants().find(|n| n.has_tag_name(("http://owncloud.org/ns", "share-types"))).is_some_and(|n| n.children().any(|c| c.is_element()));
        out.push(NcFile {
            name: path.rsplit('/').next().unwrap_or_default().to_string(),
            path,
            is_dir,
            size: size_str.parse().unwrap_or(0),
            modified: prop("DAV:", "getlastmodified"),
            mime: if is_dir { String::new() } else { prop("DAV:", "getcontenttype") },
            file_id: prop("http://owncloud.org/ns", "fileid").parse().ok(),
            shared,
            favorite: prop("http://owncloud.org/ns", "favorite") == "1",
            can_write: perms.is_empty() || perms.contains('W') || perms.contains('C') || perms.contains('K'),
        });
    }
    Ok(out)
}

fn parse_share(v: &Value) -> NcShare {
    NcShare {
        id: text(&v["id"]).or_int(&v["id"]),
        share_type: int(&v["share_type"]),
        share_with: text(&v["share_with"]),
        share_with_name: v["share_with_displayname"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| text(&v["share_with"])),
        url: text(&v["url"]),
        expiration: text(&v["expiration"]),
        has_password: v["password"].as_str().is_some_and(|p| !p.is_empty()) || v["share_with"].as_str().is_some_and(|p| !p.is_empty() && int(&v["share_type"]) == 3),
        can_edit: int(&v["permissions"]) & 2 != 0,
    }
}

impl Nc {
    fn files_root_path(&self) -> Result<String> {
        Ok(self.files_url("/")?.path().trim_end_matches('/').to_string() + "/")
    }

    pub async fn files_list(&self, path: &str) -> Result<Vec<NcFile>> {
        let path = clean_path(path)?;
        let resp = self
            .dav(Method::from_bytes(b"PROPFIND").expect("PROPFIND"), self.files_url(&path)?)
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_FILES)
            .send()
            .await
            .map_err(net_error)?;
        if resp.status().as_u16() != 207 {
            return Err(status_error(resp.status(), "Файлы"));
        }
        let xml = crate::netio::text_limited(resp, crate::netio::API_LIMIT).await?;
        let root = self.files_root_path()?;
        let decoded_root = percent_encoding::percent_decode_str(&root).decode_utf8_lossy().to_string();
        let mut list: Vec<NcFile> = parse_file_list(&xml, &decoded_root)?.into_iter().filter(|f| f.path != path).collect();
        list.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(list)
    }

    /// Создаёт папку и все недостающие родительские.
    pub async fn ensure_dir(&self, path: &str) -> Result<String> {
        let path = clean_path(path)?;
        let mut acc = String::new();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            acc.push('/');
            acc.push_str(part);
            let resp = self.dav(Method::from_bytes(b"MKCOL").expect("MKCOL"), self.files_url(&acc)?).send().await.map_err(net_error)?;
            // 201 — создана, 405 — уже существует.
            if !(resp.status().is_success() || resp.status() == StatusCode::METHOD_NOT_ALLOWED) {
                return Err(status_error(resp.status(), "Папка"));
            }
        }
        Ok(path)
    }

    pub async fn files_mkdir(&self, parent: &str, name: &str) -> Result<String> {
        let path = format!("{}/{}", clean_path(parent)?.trim_end_matches('/'), valid_name(name)?);
        let resp = self.dav(Method::from_bytes(b"MKCOL").expect("MKCOL"), self.files_url(&path)?).send().await.map_err(net_error)?;
        match resp.status() {
            s if s.is_success() => Ok(path),
            StatusCode::METHOD_NOT_ALLOWED => Err(AppError::Other("Папка с таким именем уже есть".into())),
            s => Err(status_error(s, "Новая папка")),
        }
    }

    /// Загружает файл в папку, не перезаписывая существующие: «файл (1).pdf»…
    pub async fn upload_unique(&self, dir: &str, name: &str, data: Vec<u8>) -> Result<String> {
        self.upload_unique_src(dir, name, UploadSource::Bytes(bytes::Bytes::from(data))).await
    }

    /// Загрузка без перезаписи: «имя (1).ext» и т.д. Файл с диска передаётся потоком
    /// (не читается в память целиком — важно на терминальных серверах).
    pub async fn upload_unique_src(&self, dir: &str, name: &str, src: UploadSource) -> Result<String> {
        let dir = self.ensure_dir(dir).await?;
        let name = valid_name(name)?;
        for n in 0..100 {
            let path = format!("{}/{}", dir.trim_end_matches('/'), numbered_name(&name, n));
            let (body, len) = match &src {
                UploadSource::Bytes(b) => (reqwest::Body::from(b.clone()), b.len() as u64),
                UploadSource::File(p) => {
                    let f = tokio::fs::File::open(p).await?;
                    let len = f.metadata().await?.len();
                    (reqwest::Body::from(f), len)
                }
            };
            let resp = self
                .dav(Method::PUT, self.files_url(&path)?)
                .header("If-None-Match", "*")
                .header("Content-Type", mime_guess::from_path(&name).first_or_octet_stream().essence_str())
                .header(reqwest::header::CONTENT_LENGTH, len)
                .body(body)
                .send()
                .await
                .map_err(net_error)?;
            match resp.status() {
                s if s.is_success() => return Ok(path),
                StatusCode::PRECONDITION_FAILED => continue,
                StatusCode::INSUFFICIENT_STORAGE => return Err(AppError::Other("В Nextcloud закончилось место (квота)".into())),
                StatusCode::PAYLOAD_TOO_LARGE => return Err(AppError::Other("Файл слишком большой для сервера Nextcloud".into())),
                s => return Err(status_error(s, "Загрузка файла")),
            }
        }
        Err(AppError::Other("Не удалось подобрать свободное имя файла".into()))
    }

    /// Скачивает файл потоком прямо на диск (без копии целиком в памяти).
    pub async fn files_download_to(&self, path: &str, target: &std::path::Path) -> Result<u64> {
        use tokio::io::AsyncWriteExt as _;
        let mut resp = self.dav(Method::GET, self.files_url(&clean_path(path)?)?).send().await.map_err(net_error)?;
        if !resp.status().is_success() {
            return Err(status_error(resp.status(), "Скачивание"));
        }
        let mut file = tokio::fs::File::create(target).await?;
        let mut total = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(net_error)? {
            file.write_all(&chunk).await?;
            total += chunk.len() as u64;
        }
        file.flush().await?;
        Ok(total)
    }

    /// Содержимое файла для просмотра; больше `max` байт — ошибка (загрузка прерывается).
    pub async fn files_read(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        let resp = self.dav(Method::GET, self.files_url(&clean_path(path)?)?).send().await.map_err(net_error)?;
        if !resp.status().is_success() {
            return Err(status_error(resp.status(), "Открытие файла"));
        }
        if resp.content_length().is_some_and(|n| n > max as u64) {
            return Err(AppError::Other(format!("Файл больше {} МБ — сохраните его на компьютер", max / (1024 * 1024))));
        }
        crate::netio::read_limited(resp, max)
            .await
            .map_err(|_| AppError::Other(format!("Файл больше {} МБ — сохраните его на компьютер", max / (1024 * 1024))))
    }

    /// Удаление (в Nextcloud — в корзину).
    pub async fn files_delete(&self, path: &str) -> Result<()> {
        let path = clean_path(path)?;
        if path == "/" { return Err(AppError::Other("Нельзя удалить корень".into())); }
        let resp = self.dav(Method::DELETE, self.files_url(&path)?).send().await.map_err(net_error)?;
        if !resp.status().is_success() {
            return Err(status_error(resp.status(), "Удаление"));
        }
        Ok(())
    }

    pub async fn files_rename(&self, path: &str, new_name: &str) -> Result<String> {
        let path = clean_path(path)?;
        let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        let target = format!("{parent}/{}", valid_name(new_name)?);
        let resp = self
            .dav(Method::from_bytes(b"MOVE").expect("MOVE"), self.files_url(&path)?)
            .header("Destination", self.files_url(&target)?.as_str())
            .header("Overwrite", "F")
            .send()
            .await
            .map_err(net_error)?;
        match resp.status() {
            s if s.is_success() => Ok(target),
            StatusCode::PRECONDITION_FAILED => Err(AppError::Other("Файл с таким именем уже есть".into())),
            s => Err(status_error(s, "Переименование")),
        }
    }

    pub async fn shares_list(&self, path: &str) -> Result<Vec<NcShare>> {
        let q = [("path", clean_path(path)?), ("reshares", "true".into()), ("format", "json".into())];
        let (_, _, d) = self.ocs(Method::GET, "ocs/v2.php/apps/files_sharing/api/v1/shares", &q, None, "Общий доступ").await?;
        Ok(d.as_array().map(|a| a.iter().map(parse_share).collect()).unwrap_or_default())
    }

    /// Публичная ссылка (только чтение), с паролем и сроком при необходимости.
    pub async fn share_link(&self, path: &str, password: Option<&str>, expire: Option<&str>) -> Result<NcShare> {
        let mut body = json!({ "path": clean_path(path)?, "shareType": 3, "permissions": 1 });
        if let Some(p) = password.filter(|p| !p.trim().is_empty()) { body["password"] = json!(p); }
        if let Some(e) = expire.filter(|e| !e.trim().is_empty()) { body["expireDate"] = json!(e); }
        let (_, _, d) = self.ocs(Method::POST, "ocs/v2.php/apps/files_sharing/api/v1/shares", &[("format", "json".into())], Some(body), "Публичная ссылка").await?;
        Ok(parse_share(&d))
    }

    /// Редакторы документов, доступные через Direct Editing (ONLYOFFICE / Euro-Office,
    /// Nextcloud Text). Нет приложения или API — пустой список, не ошибка.
    pub async fn editors(&self) -> NcEditors {
        let Ok((_, _, d)) = self.ocs(Method::GET, "ocs/v2.php/apps/files/api/v1/directEditing", &[("format", "json".into())], None, "Редакторы документов").await else {
            return NcEditors::default();
        };
        let strings = |v: &Value| -> Vec<String> { v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        let objects = |v: &Value| -> Vec<Value> {
            match v {
                Value::Object(m) => m.values().cloned().collect(),
                Value::Array(a) => a.clone(),
                _ => Vec::new(),
            }
        };
        NcEditors {
            editors: objects(&d["editors"]).iter().map(|e| NcEditor {
                id: text(&e["id"]),
                name: text(&e["name"]),
                mimetypes: strings(&e["mimetypes"]),
                optional_mimetypes: strings(&e["optionalMimetypes"]),
            }).filter(|e| !e.id.is_empty()).collect(),
            creators: objects(&d["creators"]).iter().map(|c| NcCreator {
                id: text(&c["id"]),
                editor: text(&c["editor"]),
                name: text(&c["name"]),
                extension: text(&c["extension"]).trim_start_matches('.').to_ascii_lowercase(),
            }).filter(|c| !c.id.is_empty()).collect(),
        }
    }

    /// Одноразовая ссылка на редактор для файла (вход не нужен — как в мобильных приложениях).
    pub async fn direct_open(&self, path: &str, editor_id: &str) -> Result<String> {
        let body = json!({ "path": clean_path(path)?, "editorId": editor_id });
        let (_, _, d) = self.ocs(Method::POST, "ocs/v2.php/apps/files/api/v1/directEditing/open", &[("format", "json".into())], Some(body), "Открытие в редакторе").await?;
        self.own_url(&text(&d["url"]))
    }

    /// Новый документ через редактор (шаблон сервера) — ссылка на редактор.
    pub async fn direct_create(&self, path: &str, editor_id: &str, creator_id: &str) -> Result<String> {
        let body = json!({ "path": clean_path(path)?, "editorId": editor_id, "creatorId": creator_id });
        let (_, _, d) = self.ocs(Method::POST, "ocs/v2.php/apps/files/api/v1/directEditing/create", &[("format", "json".into())], Some(body), "Новый документ").await?;
        self.own_url(&text(&d["url"]))
    }

    /// Ссылка должна вести на этот же Nextcloud — чужой адрес в окно программы не открываем.
    fn own_url(&self, url: &str) -> Result<String> {
        let u = url::Url::parse(url).map_err(|_| AppError::Other("Редактор вернул некорректную ссылку".into()))?;
        let b = &self.creds.base;
        if u.scheme() != b.scheme() || u.host_str() != b.host_str() || u.port_or_known_default() != b.port_or_known_default() {
            return Err(AppError::Other("Редактор вернул ссылку на другой сервер".into()));
        }
        Ok(u.to_string())
    }

    /// ID файла (для ссылки `index.php/f/<id>`).
    pub async fn file_id(&self, path: &str) -> Result<i64> {
        let path = clean_path(path)?;
        let resp = self
            .dav(Method::from_bytes(b"PROPFIND").expect("PROPFIND"), self.files_url(&path)?)
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_FILES)
            .send()
            .await
            .map_err(net_error)?;
        if resp.status().as_u16() != 207 {
            return Err(status_error(resp.status(), "Файл"));
        }
        let xml = crate::netio::text_limited(resp, crate::netio::API_LIMIT).await?;
        let root = self.files_root_path()?;
        let decoded_root = percent_encoding::percent_decode_str(&root).decode_utf8_lossy().to_string();
        parse_file_list(&xml, &decoded_root)?
            .into_iter()
            .find_map(|f| f.file_id)
            .ok_or_else(|| AppError::Other("Nextcloud не сообщил ID файла".into()))
    }

    /// Публичная ссылка для письма: существующая без пароля и со сроком не меньше
    /// `days`/2 дней — или новая со сроком `days` дней (0 — без срока, по политике).
    pub async fn link_for(&self, path: &str, days: u32) -> Result<String> {
        let today = chrono::Local::now().date_naive();
        let min_left = chrono::Duration::days(i64::from(days / 2).max(1));
        let usable = |s: &NcShare| {
            s.share_type == 3 && !s.has_password && !s.url.is_empty() && {
                let exp = chrono::NaiveDate::parse_from_str(s.expiration.get(..10).unwrap_or(""), "%Y-%m-%d").ok();
                match (days, exp) {
                    (0, _) => true,
                    (_, Some(e)) => e - today >= min_left,
                    (_, None) => false, // ссылка без срока — для письма не используем
                }
            }
        };
        if let Some(s) = self.shares_list(path).await?.into_iter().find(usable) {
            return Ok(s.url);
        }
        let expire = (days > 0).then(|| (today + chrono::Duration::days(i64::from(days))).format("%Y-%m-%d").to_string());
        Ok(self.share_link(path, None, expire.as_deref()).await?.url)
    }

    pub async fn share_user(&self, path: &str, uid: &str, can_edit: bool) -> Result<NcShare> {
        let body = json!({ "path": clean_path(path)?, "shareType": 0, "shareWith": uid, "permissions": if can_edit { 31 } else { 1 } });
        let (_, _, d) = self.ocs(Method::POST, "ocs/v2.php/apps/files_sharing/api/v1/shares", &[("format", "json".into())], Some(body), "Доступ сотруднику").await?;
        Ok(parse_share(&d))
    }

    pub async fn share_delete(&self, id: &str) -> Result<()> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '_') {
            return Err(AppError::Other("Некорректный идентификатор доступа".into()));
        }
        self.ocs(Method::DELETE, &format!("ocs/v2.php/apps/files_sharing/api/v1/shares/{id}"), &[("format", "json".into())], None, "Закрытие доступа").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_window_stays_on_own_servers() {
        let base = url::Url::parse("https://cloud.inst.test/index.php/apps/onlyoffice/direct/x").unwrap();
        let ok = |s: &str| editor_url_allowed(&base, &url::Url::parse(s).unwrap());
        assert!(ok("https://cloud.inst.test/index.php/f/5"));
        assert!(ok("https://eurooffice.inst.test/web-apps/x"));
        assert!(ok("about:blank"));
        assert!(!ok("https://evil.example/login"));
        assert!(!ok("https://inst.test.evil.example/"));
        assert!(!ok("http://eurooffice.inst.test/"));
        let local = url::Url::parse("https://localhost:8443/x").unwrap();
        assert!(!editor_url_allowed(&local, &url::Url::parse("https://example.org/").unwrap()));
        // Служебные адреса: только about:blank и blob: своих серверов.
        assert!(!ok("data:text/html,<form>login</form>"));
        assert!(!ok("javascript:alert(1)"));
        assert!(!ok("about:config"));
        assert!(ok("blob:https://eurooffice.inst.test/1234-5678"));
        assert!(!ok("blob:https://evil.example/1234"));
        // Порт, логин в адресе, похожие имена.
        assert!(!ok("https://eurooffice.inst.test:8443/"));
        assert!(!ok("https://cloud.inst.test@evil.example/"));
        assert!(!ok("https://evilinst.test/"));
        assert!(!ok("https://cloud.inst.test.evil.example/"));
        assert!(ok("https://INST.test/"));
        // IP-адрес и публичный домен второго уровня — только сам сервер.
        let ip = url::Url::parse("https://10.0.0.5/x").unwrap();
        assert!(editor_url_allowed(&ip, &url::Url::parse("https://10.0.0.5/y").unwrap()));
        assert!(!editor_url_allowed(&ip, &url::Url::parse("https://1.0.0.5/").unwrap()));
        let uk = url::Url::parse("https://nc.example.co.uk/").unwrap();
        assert!(editor_url_allowed(&uk, &url::Url::parse("https://docs.example.co.uk/").unwrap()));
        let suffix = url::Url::parse("https://firm.co.uk/").unwrap();
        assert!(!editor_url_allowed(&suffix, &url::Url::parse("https://other.co.uk/").unwrap()));
    }

    #[test]
    fn encodes_path_segment() {
        assert_eq!(seg("ivanov@inst.test"), "ivanov@inst.test");
        assert_eq!(seg("Иван Петров/../x"), "%D0%98%D0%B2%D0%B0%D0%BD%20%D0%9F%D0%B5%D1%82%D1%80%D0%BE%D0%B2%2F..%2Fx");
    }

    #[test]
    fn normalizes_base_url() {
        assert_eq!(normalize_base("next.inst.test").unwrap().as_str(), "https://next.inst.test/");
        assert_eq!(normalize_base("https://next.inst.test/nc").unwrap().as_str(), "https://next.inst.test/nc/");
        assert!(normalize_base("http://next.inst.test").is_err());
    }

    #[test]
    fn renders_talk_parameters() {
        let v = json!({
            "id": 7, "actorType": "users", "actorId": "yushkov_do@inst.test", "actorDisplayName": "Юшков Дмитрий",
            "timestamp": 1793000000, "message": "{mention-user1}, посмотри {file}", "messageType": "comment", "systemMessage": "",
            "messageParameters": {
                "mention-user1": { "type": "user", "id": "ivanov", "name": "Иванов Иван" },
                "file": { "type": "file", "id": "42", "name": "Отчёт.docx" }
            },
            "parent": { "id": 5, "actorDisplayName": "Иванов Иван", "message": "Где отчёт?", "messageParameters": [] }
        });
        let m = parse_message(&v, "yushkov_do@inst.test");
        assert_eq!(m.text, "@Иванов Иван, посмотри");
        assert_eq!(m.files, vec![TalkFile { id: "42".into(), name: "Отчёт.docx".into(), size: 0, mimetype: String::new(), path: None, preview: false }]);
        // Сообщение «только файл»: текста нет, ID числом.
        let (t, f) = render_message(&json!({"message": "{file}", "messageParameters": {"file": {"type": "file", "id": 9, "name": "a.png", "size": 5, "mimetype": "image/png", "path": "/Talk/a.png", "preview-available": "yes"}}}));
        assert!(t.is_empty() && f[0].id == "9" && f[0].size == 5);
        assert!(f[0].preview && f[0].path.as_deref() == Some("Talk/a.png"));
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
             "messageParameters": {"mention-user1": {"type": "user", "id": "ivanov@inst.test", "name": "Иванов", "mention-id": "ivanov@inst.test"}},
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
        assert_eq!(m[0].edit_text.as_deref(), Some("Привет, @\"ivanov@inst.test\""));
        assert!(m[2].edited && m[2].reactions[0].count == 3);
        assert!(m[3].deleted && m[3].text == "Сообщение удалено" && m[3].edit_text.is_none());
        assert!(m[1].system);
    }

    #[test]
    fn parses_board_details() {
        let d = parse_board_details(&json!({
            "owner": {"uid": "admin", "displayname": "Администратор"},
            "labels": [{"id": 1, "title": "Срочно", "color": "e9322d"}],
            "users": [{"uid": "ivanov", "displayname": "Иванов"}],
            "acl": [{"type": 0, "participant": {"uid": "petrova", "displayname": "Петрова"}},
                    {"type": 1, "participant": {"uid": "staff", "displayname": "Сотрудники"}},
                    {"type": 0, "participant": {"uid": "ivanov", "displayname": "Иванов"}}]
        }));
        assert_eq!(d.labels[0].title, "Срочно");
        assert_eq!(d.users.iter().map(|u| u.uid.as_str()).collect::<Vec<_>>(), vec!["admin", "ivanov", "petrova"]);
    }

    #[test]
    fn parses_webdav_listing_and_paths() {
        let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">
          <d:response><d:href>/remote.php/dav/files/test%40corp.test/Docs/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><oc:size>300</oc:size><oc:fileid>5</oc:fileid></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          <d:response><d:href>/remote.php/dav/files/test%40corp.test/Docs/%D0%9E%D1%82%D1%87%D1%91%D1%82%20(1).pdf</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>120</d:getcontentlength><d:getcontenttype>application/pdf</d:getcontenttype><oc:fileid>7</oc:fileid><oc:permissions>RGDNVW</oc:permissions><oc:share-types><oc:share-type>3</oc:share-type></oc:share-types></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat><d:propstat><d:prop><oc:favorite/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>
          <d:response><d:href>/remote.php/dav/files/test%40corp.test/Docs/Sub/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><oc:permissions>RGD</oc:permissions></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
        </d:multistatus>"#;
        let f = parse_file_list(xml, "/remote.php/dav/files/test@corp.test/").unwrap();
        assert_eq!(f.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(), vec!["/Docs", "/Docs/Отчёт (1).pdf", "/Docs/Sub"]);
        assert!(f[0].is_dir && f[0].size == 300);
        assert_eq!((f[1].name.as_str(), f[1].size, f[1].file_id, f[1].shared, f[1].can_write), ("Отчёт (1).pdf", 120, Some(7), true, true));
        assert!(!f[2].can_write);
        assert_eq!(clean_path("Docs//a/").unwrap(), "/Docs/a");
        assert!(clean_path("/Docs/../etc").is_err());
        assert!(valid_name("a/b").is_err() && valid_name("  Отчёт.pdf ").unwrap() == "Отчёт.pdf");
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
        assert_eq!(c.assigned, vec![DeckUser { uid: "ivanov".into(), name: "Иванов".into() }]);
        assert_eq!(c.labels[0].title, "Срочно");
        let done = parse_card(&json!({"id": 1, "done": "2026-10-27T10:00:00+00:00", "owner": "u"}));
        assert!(done.done && done.owner == "u");
    }
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcEditors {
    pub editors: Vec<NcEditor>,
    pub creators: Vec<NcCreator>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcEditor {
    pub id: String,
    pub name: String,
    pub mimetypes: Vec<String>,
    pub optional_mimetypes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcCreator {
    pub id: String,
    pub editor: String,
    pub name: String,
    pub extension: String,
}

impl NcEditors {
    /// Редактор для типа файла: сначала «основные» типы, затем дополнительные.
    pub fn for_mime(&self, mime: &str) -> Option<&NcEditor> {
        self.editors
            .iter()
            .find(|e| e.mimetypes.iter().any(|m| m == mime))
            .or_else(|| self.editors.iter().find(|e| e.optional_mimetypes.iter().any(|m| m == mime)))
    }
}

/// Тип файла по расширению — для офисных форматов (Nextcloud может не сообщить MIME).
pub fn office_mime(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "doc" => "application/msword",
        "xls" => "application/vnd.ms-excel",
        "ppt" => "application/vnd.ms-powerpoint",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "rtf" => "application/rtf",
        "csv" => "text/csv",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "pdf" => "application/pdf",
        _ => return None,
    })
}

/// Что загружать: байты (вложение письма, шаблон) или файл с диска (потоком).
pub enum UploadSource {
    Bytes(bytes::Bytes),
    File(std::path::PathBuf),
}

/// Адрес можно открыть в окне редактора: тот же сервер, что Nextcloud (схема, хост и
/// порт), или по https на стандартном порту — поддомен родительского домена Nextcloud
/// (сервер документов, например eurooffice.<домен>). Для IP-адресов, localhost и
/// адресов в «публичных» доменах второго уровня (co.uk, com.ru, msk.ru …) — только
/// сам сервер. Из служебных адресов — только about:blank/about:srcdoc и blob: своих
/// серверов; data: и javascript: запрещены (подложная страница прямо в окне).
pub fn editor_url_allowed(base: &url::Url, u: &url::Url) -> bool {
    match u.scheme() {
        "about" => return matches!(u.as_str(), "about:blank" | "about:srcdoc"),
        "blob" => return url::Url::parse(u.path()).is_ok_and(|inner| inner.scheme() != "blob" && editor_url_allowed(base, &inner)),
        "https" | "http" => {}
        _ => return false,
    }
    let (Some(host), Some(base_host)) = (u.host_str(), base.host_str()) else { return false };
    if u.scheme() == base.scheme() && host == base_host && u.port_or_known_default() == base.port_or_known_default() {
        return true;
    }
    if u.scheme() != "https" || u.port().is_some_and(|p| p != 443) || !u.username().is_empty() {
        return false;
    }
    // Родительский домен Nextcloud (cloud.inst.test → inst.test) — только для доменных имён.
    if !matches!(base.host(), Some(url::Host::Domain(_))) {
        return false;
    }
    let Some(parent) = base_host.split_once('.').map(|(_, p)| p).filter(|p| p.contains('.')) else { return false };
    let labels: Vec<&str> = parent.split('.').collect();
    let public_suffix = labels.len() == 2 && matches!(labels[0], "co" | "com" | "org" | "net" | "gov" | "edu" | "ac" | "msk" | "spb" | "nov" | "gob" | "or" | "ne");
    !public_suffix && (host == parent || host.ends_with(&format!(".{parent}")))
}

