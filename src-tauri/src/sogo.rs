//! Поиск адресатов через веб-API SOGo (тот же, что подсказывает адреса в
//! веб-почте). Нужен как запасной путь: корпоративная книга SOGo (LDAP) может
//! не отвечать на CardDAV-поиск, а `allContactSearch` ищет по всем книгам сразу.
//!
//! Вход — теми же логином и паролем, сессия (cookie) живёт в памяти процесса.

use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use url::Url;

use crate::dav::Contact;
use crate::error::{AppError, Result};

#[derive(Clone)]
pub struct SogoSession {
    root: Url,
    user: String,
    cookie: String,
    xsrf: Option<String>,
}

impl std::fmt::Debug for SogoSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SogoSession").field("root", &self.root.as_str()).field("user", &self.user).finish_non_exhaustive()
    }
}

fn http() -> Result<Client> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(15))
        .user_agent(concat!("RRCKI Mail/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AppError::Other(format!("HTTP-клиент: {e}")))
}

fn net(e: reqwest::Error) -> AppError {
    let mut detail = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(cause) = src {
        detail.push_str(&format!(": {cause}"));
        src = cause.source();
    }
    AppError::Other(format!("SOGo: {detail}"))
}

/// `https://host/` → `https://host/SOGo/`
pub fn sogo_root(dav_base: &Url) -> Result<Url> {
    // Пароль уходит формой входа — только по https.
    if dav_base.scheme() != "https" {
        return Err(AppError::Other("Адрес сервера SOGo должен начинаться с https://".into()));
    }
    let mut u = dav_base.clone();
    u.set_path("/SOGo/");
    u.set_query(None);
    Ok(u)
}

/// Собирает `имя=значение` из всех Set-Cookie (без атрибутов).
fn cookies(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

pub async fn login(dav_base: &Url, user: &str, pass: &str, log: &mut Vec<String>) -> Result<SogoSession> {
    let root = sogo_root(dav_base)?;
    let connect = root.join("connect").map_err(|e| AppError::Other(e.to_string()))?;
    let client = http()?;
    // SOGo 4/5 принимает JSON; старые версии — форму.
    let attempts: [(&str, bool); 2] = [("json", true), ("form", false)];
    for (kind, as_json) in attempts {
        let req = client.post(connect.clone());
        let req = if as_json {
            req.json(&json!({ "userName": user, "password": pass, "rememberLogin": 0 }))
        } else {
            req.form(&[("userName", user), ("password", pass)])
        };
        let resp = req.send().await.map_err(net)?;
        let status = resp.status();
        let jar = cookies(resp.headers());
        log.push(format!("POST {connect} ({kind}) → {status}, cookies: {}", jar.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(", ")));
        let session = jar.iter().any(|(k, _)| k.to_ascii_lowercase().contains("sogo"));
        if status.is_success() && session {
            let xsrf = jar.iter().find(|(k, _)| k == "XSRF-TOKEN").map(|(_, v)| v.clone());
            let cookie = jar.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ");
            return Ok(SogoSession { root, user: user.to_string(), cookie, xsrf });
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED {
            return Err(AppError::Other("SOGo: вход в веб-интерфейс отклонён".into()));
        }
    }
    Err(AppError::Other("SOGo: веб-интерфейс не выдал сессию".into()))
}

impl SogoSession {
    fn url(&self, tail: &str) -> Result<Url> {
        let mut u = self.root.clone();
        {
            let mut seg = u.path_segments_mut().map_err(|_| AppError::Other("Некорректный адрес SOGo".into()))?;
            seg.pop_if_empty().push("so").push(&self.user);
            for p in tail.split('/') {
                seg.push(p);
            }
        }
        Ok(u)
    }

    /// Сырой ответ поиска — для диагностики.
    pub async fn search_raw(&self, query: &str) -> Result<(StatusCode, String)> {
        let mut url = self.url("Contacts/allContactSearch")?;
        url.query_pairs_mut().append_pair("search", query).append_pair("excludeGroups", "1");
        let mut req = http()?.get(url).header("Cookie", &self.cookie).header("Accept", "application/json");
        if let Some(x) = &self.xsrf {
            req = req.header("X-XSRF-TOKEN", x);
        }
        let resp = req.send().await.map_err(net)?;
        let status = resp.status();
        let text = resp.text().await.map_err(net)?;
        Ok((status, text))
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Contact>> {
        let (status, text) = self.search_raw(query).await?;
        if status.is_redirection() || status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(AppError::Other("SOGo: сессия истекла".into()));
        }
        if !status.is_success() {
            return Err(AppError::Other(format!("SOGo: поиск вернул HTTP {status}")));
        }
        let v: Value = serde_json::from_str(&text).map_err(|_| AppError::Other("SOGo: поиск вернул не JSON".into()))?;
        Ok(parse_search(&v))
    }

    /// Список книги целиком, как его открывает веб-интерфейс SOGo. Для LDAP-книг
    /// сервер часто отдаёт список только по шаблону («.» — все), поэтому пробуем
    /// несколько вариантов и берём первый непустой. `log` — для диагностики.
    pub async fn browse(&self, source: &str, log: &mut Vec<String>) -> Result<Vec<Contact>> {
        let client = http()?;
        let url = self.url(&format!("Contacts/{source}/view"))?;
        let attempts: [(&str, Value); 3] = [
            ("view .", json!({ "search": "name_or_address", "value": ".", "sort": "c_cn", "asc": 1 })),
            ("view *", json!({ "search": "name_or_address", "value": "*", "sort": "c_cn", "asc": 1 })),
            ("view пусто", json!({ "search": "name_or_address", "value": "", "sort": "c_cn", "asc": 1 })),
        ];
        for (label, body) in attempts {
            let mut req = client.post(url.clone()).header("Cookie", &self.cookie).header("Accept", "application/json").json(&body);
            if let Some(x) = &self.xsrf {
                req = req.header("X-XSRF-TOKEN", x);
            }
            let resp = req.send().await.map_err(net)?;
            let status = resp.status();
            let text = resp.text().await.map_err(net)?;
            let list = serde_json::from_str::<Value>(&text).map(|v| parse_list(&v, source)).unwrap_or_default();
            log.push(format!("{label} → {status}, записей: {}, ответ: {}", list.len(), text.chars().take(200).collect::<String>()));
            if !list.is_empty() {
                return Ok(list);
            }
        }
        for q in [".", "*"] {
            if let Ok((status, text)) = self.search_raw(q).await {
                let list: Vec<Contact> = serde_json::from_str::<Value>(&text).map(|v| parse_search(&v)).unwrap_or_default()
                    .into_iter().filter(|c| c.id.starts_with(&format!("sogo:{source}:"))).collect();
                log.push(format!("allContactSearch «{q}» → {status}, записей: {}", list.len()));
                if !list.is_empty() {
                    return Ok(list);
                }
            }
        }
        Ok(Vec::new())
    }

    fn req(&self, method: reqwest::Method, url: Url) -> Result<reqwest::RequestBuilder> {
        let mut r = http()?.request(method, url).header("Cookie", &self.cookie).header("Accept", "application/json");
        if let Some(x) = &self.xsrf {
            r = r.header("X-XSRF-TOKEN", x);
        }
        Ok(r)
    }

    /// Настройки пользователя SOGo (`Preferences/jsonDefaults`) — там же фильтры почты.
    pub async fn defaults(&self) -> Result<Value> {
        let resp = self.req(reqwest::Method::GET, self.url("Preferences/jsonDefaults")?)?.send().await.map_err(net)?;
        let status = resp.status();
        let text = resp.text().await.map_err(net)?;
        if !status.is_success() {
            return Err(AppError::Other(format!("SOGo: настройки недоступны (HTTP {status})")));
        }
        let v: Value = serde_json::from_str(&text).map_err(|_| AppError::Other("SOGo: настройки — не JSON".into()))?;
        if !v.is_object() {
            return Err(AppError::Other("SOGo: неожиданный формат настроек".into()));
        }
        Ok(v)
    }

    /// Сохраняет настройки целиком — как веб-интерфейс SOGo (он отправляет все
    /// настройки, сервер заменяет их и пересобирает Sieve-скрипт фильтров).
    pub async fn save_defaults(&self, defaults: &Value) -> Result<()> {
        let resp = self.req(reqwest::Method::POST, self.url("Preferences/save")?)?
            .json(&json!({ "defaults": defaults }))
            .send()
            .await
            .map_err(net)?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Other(format!("SOGo не сохранил фильтры (HTTP {status}): {}", text.chars().take(200).collect::<String>())));
        }
        Ok(())
    }

    /// Сырой ответ карточки контакта веб-интерфейса SOGo (`…/Contacts/<книга>/<id>/view`).
    pub async fn view_raw(&self, source: &str, id: &str) -> Result<(StatusCode, String)> {
        let url = self.url(&format!("Contacts/{source}/{id}/view"))?;
        let mut req = http()?.get(url).header("Cookie", &self.cookie).header("Accept", "application/json");
        if let Some(x) = &self.xsrf {
            req = req.header("X-XSRF-TOKEN", x);
        }
        let resp = req.send().await.map_err(net)?;
        let status = resp.status();
        Ok((status, resp.text().await.map_err(net)?))
    }

    pub async fn logout(&self) {
        if let (Ok(url), Ok(client)) = (self.url("logoff"), http()) {
            let _ = client.get(url).header("Cookie", &self.cookie).send().await;
        }
    }
}

fn strings(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => vec![s.trim().to_string()],
        Value::Array(a) => a
            .iter()
            .flat_map(|x| match x {
                Value::Object(_) => strings(x.get("value").unwrap_or(&Value::Null)),
                _ => strings(x),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn first(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| strings(v.get(*k).unwrap_or(&Value::Null)).into_iter().next())
}

/// Ответ `allContactSearch`: `{"contacts":[{"c_cn":…, "c_mail":…|[…], "c_o":…}]}`
/// (поля у SOGo 3/4/5 немного отличаются — берём то, что есть).
pub fn parse_search(v: &Value) -> Vec<Contact> {
    let list = v.get("contacts").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for c in &list {
        if c.get("isGroup").and_then(Value::as_bool).unwrap_or(false) || c.get("c_component").and_then(Value::as_str) == Some("vlist") {
            continue;
        }
        let mut emails = strings(c.get("c_mail").unwrap_or(&Value::Null));
        emails.extend(strings(c.get("emails").unwrap_or(&Value::Null)));
        emails.extend(strings(c.get("mail").unwrap_or(&Value::Null)));
        emails.dedup();
        if emails.is_empty() {
            continue;
        }
        let name = first(c, &["c_cn", "fn", "cn", "c_screenname", "displayName"]).unwrap_or_else(|| emails[0].clone());
        let id = first(c, &["c_name", "id", "c_uid"]).unwrap_or_else(|| emails[0].clone());
        let source = first(c, &["sourceID", "sourceid", "pid", "container"]).unwrap_or_else(|| "gal".into());
        out.push(Contact {
            id: format!("sogo:{source}:{id}"),
            book: "Корпоративная адресная книга".into(),
            href: format!("sogo:{source}:{id}"),
            etag: None,
            editable: false,
            full_name: name,
            emails,
            phones: {
                let mut p = strings(c.get("phones").unwrap_or(&Value::Null));
                p.extend(strings(c.get("c_telephonenumber").unwrap_or(&Value::Null)));
                p.dedup();
                p
            },
            org: first(c, &["c_o", "org", "o"]),
            title: first(c, &["c_title", "title"]),
            note: None,
            address: None,
        });
    }
    out
}

/// Проверка фильтров перед сохранением: массив объектов с правилами и действиями,
/// разумные размеры, без управляющих символов.
pub fn validate_filters(filters: &Value) -> Result<()> {
    let list = filters.as_array().ok_or_else(|| AppError::Other("Фильтры: ожидается список".into()))?;
    if list.len() > 200 {
        return Err(AppError::Other("Слишком много фильтров".into()));
    }
    let ok_str = |v: &Value| v.as_str().is_some_and(|s| s.chars().count() <= 500 && !s.chars().any(|c| c.is_control() && c != '\t'));
    for f in list {
        let o = f.as_object().ok_or_else(|| AppError::Other("Фильтр: ожидается объект".into()))?;
        if !o.get("name").is_some_and(|n| ok_str(n) && n.as_str().is_some_and(|x| !x.trim().is_empty())) {
            return Err(AppError::Other("У фильтра должно быть название".into()));
        }
        if !matches!(o.get("match").and_then(Value::as_str), Some("all" | "any" | "allmessages")) {
            return Err(AppError::Other("Фильтр: условие «все/любое/все письма»".into()));
        }
        for r in o.get("rules").and_then(Value::as_array).into_iter().flatten() {
            for k in ["field", "operator", "value"] {
                if !r.get(k).is_some_and(|v| ok_str(v) || v.is_number()) {
                    return Err(AppError::Other(format!("Фильтр «{}»: неполное условие", o["name"].as_str().unwrap_or_default())));
                }
            }
        }
        let actions = o.get("actions").and_then(Value::as_array).ok_or_else(|| AppError::Other("Фильтр без действий".into()))?;
        if actions.is_empty() {
            return Err(AppError::Other(format!("Фильтр «{}»: нужно хотя бы одно действие", o["name"].as_str().unwrap_or_default())));
        }
        for a in actions {
            if !a.get("method").is_some_and(ok_str) || a.get("argument").is_some_and(|v| !(v.is_null() || ok_str(v))) {
                return Err(AppError::Other("Фильтр: некорректное действие".into()));
            }
        }
    }
    Ok(())
}

/// Список книги из `…/view`: либо `{"headers": [[поля…], [значения…], …]}`
/// (SOGo 5), либо `{"cards": [{…}]}` / `{"contacts": [{…}]}`.
pub fn parse_list(v: &Value, source: &str) -> Vec<Contact> {
    let mut rows: Vec<Value> = Vec::new();
    if let Some(h) = v.get("headers").and_then(Value::as_array) {
        if let Some(names) = h.first().and_then(Value::as_array) {
            let names: Vec<String> = names.iter().map(|n| n.as_str().unwrap_or_default().to_string()).collect();
            for row in h.iter().skip(1).filter_map(Value::as_array) {
                let mut obj = serde_json::Map::new();
                for (k, val) in names.iter().zip(row) {
                    obj.insert(k.clone(), val.clone());
                }
                rows.push(Value::Object(obj));
            }
        }
    }
    for key in ["cards", "contacts"] {
        if let Some(a) = v.get(key).and_then(Value::as_array) {
            rows.extend(a.iter().cloned());
        }
    }
    let rows: Vec<Value> = rows.into_iter().map(|mut r| {
        if let Some(o) = r.as_object_mut() {
            o.entry("sourceID").or_insert_with(|| json!(source));
        }
        r
    }).collect();
    parse_search(&json!({ "contacts": rows }))
}

/// Карточка из веб-API SOGo: поля называются по-разному в разных версиях —
/// берём всё, что похоже на телефоны, должность, организацию и адрес.
pub fn parse_view(v: &Value, base: Contact) -> Contact {
    let mut c = base;
    let name = first(v, &["c_cn", "cn", "fn", "displayname"]);
    if let Some(n) = name { c.full_name = n; }
    let mut emails = strings(v.get("emails").unwrap_or(&Value::Null));
    emails.extend(strings(v.get("c_mail").unwrap_or(&Value::Null)));
    for e in emails {
        if !c.emails.iter().any(|x| x.eq_ignore_ascii_case(&e)) { c.emails.push(e); }
    }
    let mut phones = strings(v.get("phones").unwrap_or(&Value::Null));
    for k in ["c_telephonenumber", "telephonenumber", "mobile", "c_mobile", "homephone", "pager", "facsimiletelephonenumber"] {
        phones.extend(strings(v.get(k).unwrap_or(&Value::Null)));
    }
    for p in phones {
        if !c.phones.contains(&p) { c.phones.push(p); }
    }
    if let Some(t) = first(v, &["title", "c_title"]) { c.title = Some(t); }
    let orgs = strings(v.get("orgs").unwrap_or(&Value::Null));
    let mut org_parts: Vec<String> = first(v, &["c_o", "o", "org"]).into_iter().collect();
    org_parts.extend(orgs);
    for k in ["ou", "c_ou", "department", "division"] {
        org_parts.extend(strings(v.get(k).unwrap_or(&Value::Null)));
    }
    org_parts.dedup();
    if !org_parts.is_empty() { c.org = Some(org_parts.join(", ")); }
    if let Some(a) = v.get("addresses").and_then(Value::as_array).and_then(|a| a.first()) {
        let line = ["street", "street2", "locality", "region", "postalcode", "country"].iter()
            .filter_map(|k| a.get(*k).and_then(Value::as_str)).map(str::trim).filter(|x| !x.is_empty())
            .collect::<Vec<_>>().join(", ");
        if !line.is_empty() { c.address = Some(line); }
    }
    if let Some(n) = first(v, &["notes", "note", "description"]) { c.note = Some(n); }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sogo_search_variants() {
        let v = json!({"contacts": [
            {"c_name": "ivanov", "c_cn": "Иванов Иван", "c_mail": "ivanov@nrcki.ru", "c_o": "НИЦ КИ", "sourceid": "gal"},
            {"id": "p1", "c_cn": "Петрова Анна", "c_mail": ["petrova@nrcki.ru", "anna@nrcki.ru"], "isGroup": false},
            {"c_cn": "Все сотрудники", "c_mail": "all@nrcki.ru", "isGroup": true},
            {"c_cn": "Без почты"},
            {"cn": "Сидоров", "emails": [{"type": "work", "value": "sidorov@nrcki.ru"}]}
        ]});
        let c = parse_search(&v);
        assert_eq!(c.len(), 3);
        assert_eq!((c[0].full_name.as_str(), c[0].emails[0].as_str(), c[0].org.as_deref()), ("Иванов Иван", "ivanov@nrcki.ru", Some("НИЦ КИ")));
        assert_eq!(c[1].emails, vec!["petrova@nrcki.ru", "anna@nrcki.ru"]);
        assert_eq!(c[2].emails, vec!["sidorov@nrcki.ru"]);
        assert!(c.iter().all(|x| !x.editable && x.href.starts_with("sogo:")));
    }

    #[test]
    fn parses_sogo_view() {
        let base = parse_search(&json!({"contacts": [{"c_name": "ivanov", "c_cn": "Иванов", "c_mail": "ivanov@nrcki.ru"}]})).remove(0);
        let c = parse_view(&json!({
            "c_cn": "Иванов Иван Иванович", "emails": [{"type": "work", "value": "ivanov@nrcki.ru"}],
            "phones": [{"type": "work", "value": "+7 495 000-00-01"}, {"type": "cell", "value": "+7 900 000-00-00"}],
            "title": "Ведущий инженер", "orgs": [{"value": "УИТ"}], "c_o": "НИЦ КИ",
            "addresses": [{"street": "пл. Курчатова, 1", "locality": "Москва"}]
        }), base);
        assert_eq!(c.full_name, "Иванов Иван Иванович");
        assert_eq!(c.emails, vec!["ivanov@nrcki.ru"]);
        assert_eq!(c.phones.len(), 2);
        assert_eq!((c.title.as_deref(), c.org.as_deref()), (Some("Ведущий инженер"), Some("НИЦ КИ, УИТ")));
        assert_eq!(c.address.as_deref(), Some("пл. Курчатова, 1, Москва"));
    }

    #[test]
    fn parses_real_rupost_search_and_lists() {
        // Фрагмент реального ответа RuPost (allContactSearch).
        let c = parse_search(&json!({"contacts": [{"hasPhoto": 0, "c_cn": "Dmitry O. Yushkov", "c_component": "vcard", "sourceID": "gal",
            "containerName": "Корпоративная адресная книга", "id": "flay@nrcki.ru", "c_uid": "flay@nrcki.ru", "phones": [{"type": "pref", "value": ""}],
            "c_sn": "Yushkov", "c_telephonenumber": "", "c_name": "flay@nrcki.ru", "fn": "Dmitry O. Yushkov", "org": "", "c_o": "",
            "emails": [{"type": "pref", "value": "flay@nrcki.ru"}]}]}));
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].full_name.as_str(), c[0].id.as_str()), ("Dmitry O. Yushkov", "sogo:gal:flay@nrcki.ru"));
        assert!(c[0].phones.is_empty() && c[0].org.is_none());
        let h = parse_list(&json!({"headers": [["c_name", "c_cn", "c_mail", "c_o"], ["ivanov", "Иванов Иван", "ivanov@nrcki.ru", "УИТ"], ["petrova", "Петрова Анна", "petrova@nrcki.ru", ""]]}), "gal");
        assert_eq!(h.iter().map(|x| x.full_name.as_str()).collect::<Vec<_>>(), vec!["Иванов Иван", "Петрова Анна"]);
        assert_eq!(h[0].org.as_deref(), Some("УИТ"));
        assert_eq!(h[0].id, "sogo:gal:ivanov");
        assert_eq!(parse_list(&json!({"cards": [{"c_name": "x", "c_cn": "X", "c_mail": "x@nrcki.ru"}]}), "gal").len(), 1);
    }

    #[test]
    fn validates_filters() {
        let good = json!([{"name": "Мониторинг", "active": 1, "match": "any",
            "rules": [{"field": "from", "operator": "contains", "value": "zabbix@nrcki.ru"}],
            "actions": [{"method": "fileinto", "argument": "INBOX/Сервера/Monitoring"}, {"method": "stop"}]}]);
        assert!(validate_filters(&good).is_ok());
        assert!(validate_filters(&json!([{"name": "x", "match": "all", "rules": [], "actions": []}])).is_err());
        assert!(validate_filters(&json!([{"name": "", "match": "all", "actions": [{"method": "keep"}]}])).is_err());
        assert!(validate_filters(&json!([{"name": "x", "match": "some", "actions": [{"method": "keep"}]}])).is_err());
        assert!(validate_filters(&json!({"name": "x"})).is_err());
        assert!(validate_filters(&json!([{"name": "x", "match": "all", "rules": [{"field": "subject", "operator": "contains"}], "actions": [{"method": "keep"}]}])).is_err());
    }

    #[test]
    fn builds_sogo_urls() {
        let root = sogo_root(&Url::parse("https://rp.example.ru/").unwrap()).unwrap();
        assert_eq!(root.as_str(), "https://rp.example.ru/SOGo/");
        let s = SogoSession { root, user: "yushkov_do@example.ru".into(), cookie: String::new(), xsrf: None };
        assert_eq!(s.url("Contacts/allContactSearch").unwrap().as_str(), "https://rp.example.ru/SOGo/so/yushkov_do@example.ru/Contacts/allContactSearch");
    }
}
