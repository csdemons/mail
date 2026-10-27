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
        let name = first(c, &["c_cn", "cn", "fn", "displayName"]).unwrap_or_else(|| emails[0].clone());
        let id = first(c, &["c_name", "id", "c_uid"]).unwrap_or_else(|| emails[0].clone());
        let source = first(c, &["sourceid", "container"]).unwrap_or_else(|| "gal".into());
        out.push(Contact {
            id: format!("sogo:{source}:{id}"),
            book: "Корпоративная адресная книга".into(),
            href: format!("sogo:{source}:{id}"),
            etag: None,
            editable: false,
            full_name: name,
            emails,
            phones: strings(c.get("c_telephonenumber").unwrap_or(&Value::Null)),
            org: first(c, &["c_o", "o", "org"]),
            title: first(c, &["c_title", "title"]),
            note: None,
        });
    }
    out
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
    fn builds_sogo_urls() {
        let root = sogo_root(&Url::parse("https://rp.example.ru/").unwrap()).unwrap();
        assert_eq!(root.as_str(), "https://rp.example.ru/SOGo/");
        let s = SogoSession { root, user: "yushkov_do@example.ru".into(), cookie: String::new(), xsrf: None };
        assert_eq!(s.url("Contacts/allContactSearch").unwrap().as_str(), "https://rp.example.ru/SOGo/so/yushkov_do@example.ru/Contacts/allContactSearch");
    }
}
