//! Клиент DAV: PROPFIND/REPORT, разбор multistatus, поиск коллекций и адресной книги.
//!
//! Часть модуля `dav` — общий обзор в `mod.rs`.

use super::prelude::*;
use super::*;

pub(crate) const NS_DAV: &str = "DAV:";
pub(crate) const NS_CAL: &str = "urn:ietf:params:xml:ns:caldav";
const NS_CARD: &str = "urn:ietf:params:xml:ns:carddav";
const NS_APPLE: &str = "http://apple.com/ns/ical/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Calendar,
    Contacts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub url: String,
    pub name: String,
    pub color: Option<String>,
    /// Можно ли создавать/менять записи (current-user-privilege-set).
    pub writable: bool,
}

pub struct DavClient {
    http: Client,
    user: String,
    pass: String,
    /// Сервер, которому можно отправлять логин и пароль (схема, хост, порт адреса
    /// календаря/контактов из настроек). Запросы и перенаправления на другие
    /// адреса не выполняются — пароль не уходит на посторонний сервер.
    origin: (String, String, u16),
    pub log: Vec<String>,
}

fn origin_of(u: &Url) -> (String, String, u16) {
    (u.scheme().to_string(), u.host_str().unwrap_or_default().to_ascii_lowercase(), u.port_or_known_default().unwrap_or(0))
}

/// Свойства одного `<response>` из `<multistatus>` (только со статусом 200).
#[derive(Debug, Default)]
pub(crate) struct DavResponse {
    pub(crate) href: String,
    /// (namespace, имя) → текст
    pub(crate) text: HashMap<(String, String), String>,
    /// (namespace, имя) → вложенные `<href>`
    pub(crate) hrefs: HashMap<(String, String), Vec<String>>,
    /// типы ресурса: (namespace, имя) из `<resourcetype>`
    pub(crate) resource_types: Vec<(String, String)>,
    /// (namespace, имя) → имена вложенных элементов (для privilege-set)
    pub(crate) elements: HashMap<(String, String), Vec<String>>,
}

impl DavResponse {
    pub(crate) fn text(&self, ns: &str, name: &str) -> Option<&str> {
        self.text.get(&(ns.to_string(), name.to_string())).map(String::as_str)
    }
    pub(crate) fn href(&self, ns: &str, name: &str) -> Option<&str> {
        self.hrefs
            .get(&(ns.to_string(), name.to_string()))
            .and_then(|v| v.first())
            .map(String::as_str)
    }
    /// `None` — свойство не вернулось (сервер не поддерживает), `Some(true)` — запись разрешена.
    pub(crate) fn writable(&self) -> Option<bool> {
        self.elements
            .get(&(NS_DAV.to_string(), "current-user-privilege-set".to_string()))
            .map(|names| {
                names
                    .iter()
                    .any(|n| matches!(n.as_str(), "all" | "write" | "write-content" | "bind"))
            })
    }
    pub(crate) fn is(&self, ns: &str, name: &str) -> bool {
        self.resource_types.iter().any(|(n, l)| n == ns && l == name)
    }
}

pub(crate) fn parse_multistatus(xml: &str) -> Result<Vec<DavResponse>> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| AppError::Other(format!("Некорректный ответ DAV-сервера: {e}")))?;
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.has_tag_name((NS_DAV, "response"))) {
        let mut r = DavResponse {
            href: resp
                .children()
                .find(|n| n.has_tag_name((NS_DAV, "href")))
                .and_then(|n| n.text())
                .unwrap_or_default()
                .trim()
                .to_string(),
            ..Default::default()
        };
        for propstat in resp.children().filter(|n| n.has_tag_name((NS_DAV, "propstat"))) {
            let ok = propstat
                .children()
                .find(|n| n.has_tag_name((NS_DAV, "status")))
                .and_then(|n| n.text())
                .is_none_or(|s| s.contains(" 200"));
            if !ok {
                continue;
            }
            let Some(prop) = propstat.children().find(|n| n.has_tag_name((NS_DAV, "prop"))) else {
                continue;
            };
            for p in prop.children().filter(|n| n.is_element()) {
                let key = (
                    p.tag_name().namespace().unwrap_or_default().to_string(),
                    p.tag_name().name().to_string(),
                );
                if key == (NS_DAV.to_string(), "resourcetype".to_string()) {
                    r.resource_types = p
                        .children()
                        .filter(|n| n.is_element())
                        .map(|n| {
                            (
                                n.tag_name().namespace().unwrap_or_default().to_string(),
                                n.tag_name().name().to_string(),
                            )
                        })
                        .collect();
                    continue;
                }
                r.elements.insert(
                    key.clone(),
                    p.descendants()
                        .skip(1)
                        .filter(|n| n.is_element())
                        .map(|n| n.tag_name().name().to_string())
                        .collect(),
                );
                let hrefs: Vec<String> = p
                    .descendants()
                    .filter(|n| n.has_tag_name((NS_DAV, "href")))
                    .filter_map(|n| n.text().map(|t| t.trim().to_string()))
                    .collect();
                if !hrefs.is_empty() {
                    r.hrefs.insert(key.clone(), hrefs);
                }
                let text: String = p.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect();
                r.text.insert(key, text);
            }
        }
        out.push(r);
    }
    Ok(out)
}

impl DavClient {
    /// `base` — адрес сервера календаря/контактов из настроек: только ему отправляется пароль.
    pub fn new(user: &str, pass: &str, base: &Url) -> Result<Self> {
        let http = crate::tls::client_builder()?
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(45))
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("Atom Workspace/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AppError::Other(format!("HTTP-клиент: {e}")))?;
        Ok(Self {
            http,
            user: user.to_string(),
            pass: pass.to_string(),
            origin: origin_of(base),
            log: Vec::new(),
        })
    }

    /// Можно ли отправить пароль на `url`: тот же хост и порт, та же схема
    /// (или переход http → https на том же хосте). https → http — нельзя.
    pub(crate) fn check_target(&mut self, url: &Url) -> Result<()> {
        let (scheme, host, port) = origin_of(url);
        let (o_scheme, o_host, o_port) = &self.origin;
        // Пароль почты — доменный: по http его не отправляем никогда.
        if scheme != "https" {
            self.log.push(format!("отказ: {url} — не https, пароль не отправлен"));
            return Err(AppError::Other(
                "Адрес календаря и контактов должен начинаться с https:// — по http пароль не отправляется".into(),
            ));
        }
        let same = host == *o_host && (port == *o_port || (o_scheme == "http" && port == 443));
        if same {
            return Ok(());
        }
        self.log.push(format!("отказ: {url} — другой сервер, пароль не отправлен"));
        Err(AppError::Other(format!(
            "Сервер календаря/контактов ссылается на другой адрес ({scheme}://{host}:{port}). Пароль туда не отправлен. \
             Если это ваш сервер, укажите его адрес в настройках «Календарь и контакты»."
        )))
    }

    /// Запрос с ручным следованием редиректам (PROPFIND/REPORT не должны
    /// превращаться в GET). Возвращает статус, итоговый URL и тело.
    async fn request(
        &mut self,
        method: &str,
        url: &Url,
        depth: Option<&str>,
        body: Option<&str>,
    ) -> Result<(StatusCode, Url, String)> {
        let method = Method::from_bytes(method.as_bytes()).expect("valid method");
        let mut url = url.clone();
        for _ in 0..6 {
            self.check_target(&url)?;
            let mut req = self
                .http
                .request(method.clone(), url.clone())
                .basic_auth(&self.user, Some(&self.pass));
            if let Some(d) = depth {
                req = req.header("Depth", d);
            }
            if let Some(b) = body {
                req = req
                    .header("Content-Type", "application/xml; charset=utf-8")
                    .body(b.to_string());
            }
            let resp = req.send().await.map_err(|e| {
                // reqwest прячет причину (TLS, DNS, отказ) в цепочке source().
                let mut detail = e.to_string();
                let mut src = std::error::Error::source(&e);
                while let Some(cause) = src {
                    detail.push_str(&format!(": {cause}"));
                    src = cause.source();
                }
                self.log.push(format!("ошибка: {method} {url}: {detail}"));
                AppError::Other(format!("Сервер календаря/контактов недоступен: {detail}"))
            })?;
            let status = resp.status();
            self.log.push(format!("{method} {url} → {status}"));
            if status.is_redirection() {
                let Some(loc) = resp.headers().get("Location").and_then(|v| v.to_str().ok()) else {
                    return Ok((status, url, String::new()));
                };
                url = url
                    .join(loc)
                    .map_err(|e| AppError::Other(format!("Некорректный редирект {loc}: {e}")))?;
                continue;
            }
            let text = crate::netio::text_limited(resp, crate::netio::API_LIMIT).await?;
            return Ok((status, url, text));
        }
        Err(AppError::Other("Слишком много перенаправлений".into()))
    }

    async fn propfind(&mut self, url: &Url, depth: &str, props: &str) -> Result<Option<(Url, Vec<DavResponse>)>> {
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="{NS_CAL}" xmlns:cr="{NS_CARD}" xmlns:a="{NS_APPLE}"><d:prop>{props}</d:prop></d:propfind>"#
        );
        let (status, final_url, text) = self.request("PROPFIND", url, Some(depth), Some(&body)).await?;
        if status == StatusCode::UNAUTHORIZED {
            return Err(AppError::Other(
                "Сервер календаря/контактов отклонил логин или пароль (HTTP 401)".into(),
            ));
        }
        if status != StatusCode::MULTI_STATUS {
            return Ok(None);
        }
        Ok(Some((final_url, parse_multistatus(&text)?)))
    }

    /// Автообнаружение коллекций: RFC 6764 (/.well-known), путь SOGo, корень.
    pub async fn discover(&mut self, base: &Url, kind: Kind) -> Result<Vec<Collection>> {
        let well_known = match kind {
            Kind::Calendar => "/.well-known/caldav",
            Kind::Contacts => "/.well-known/carddav",
        };
        let mut candidates = vec![];
        if base.path() != "/" {
            candidates.push(base.clone());
        }
        for path in [well_known.to_string(), format!("/SOGo/dav/{}/", self.user), "/".to_string()] {
            if let Ok(u) = base.join(&path) {
                candidates.push(u);
            }
        }

        let mut principal = None;
        for c in &candidates {
            if let Some((final_url, resps)) = self.propfind(c, "0", "<d:current-user-principal/>").await? {
                if let Some(href) = resps.iter().find_map(|r| r.href(NS_DAV, "current-user-principal")) {
                    principal = final_url.join(href).ok();
                    break;
                }
            }
        }
        let principal = principal.ok_or_else(|| {
            AppError::Other("Не удалось найти CalDAV/CardDAV на сервере (current-user-principal)".into())
        })?;
        self.log.push(format!("principal: {principal}"));

        let (ns, home_prop) = match kind {
            Kind::Calendar => (NS_CAL, "calendar-home-set"),
            Kind::Contacts => (NS_CARD, "addressbook-home-set"),
        };
        let prefix = if kind == Kind::Calendar { "c" } else { "cr" };
        let home = match self
            .propfind(&principal, "0", &format!("<{prefix}:{home_prop}/>"))
            .await?
        {
            Some((final_url, resps)) => resps
                .iter()
                .find_map(|r| r.href(ns, home_prop))
                .and_then(|h| final_url.join(h).ok()),
            None => None,
        }
        .ok_or_else(|| AppError::Other(format!("Сервер не сообщил {home_prop}")))?;
        self.log.push(format!("{home_prop}: {home}"));

        let props = "<d:resourcetype/><d:displayname/><a:calendar-color/><c:supported-calendar-component-set/><d:current-user-privilege-set/>";
        let Some((final_url, resps)) = self.propfind(&home, "1", props).await? else {
            return Ok(Vec::new());
        };
        let wanted = match kind {
            Kind::Calendar => (NS_CAL, "calendar"),
            Kind::Contacts => (NS_CARD, "addressbook"),
        };
        let mut out = Vec::new();
        for r in resps.iter().filter(|r| r.is(wanted.0, wanted.1)) {
            let Ok(url) = final_url.join(&r.href) else { continue };
            // SOGo показывает служебную «Public User <%{Email}>» — это не календарь пользователя.
            if url.path().contains("/anonymous_") {
                continue;
            }
            let name = r
                .text(NS_DAV, "displayname")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    url.path_segments()
                        .and_then(|mut s| s.rfind(|p| !p.is_empty()).map(str::to_string))
                        .unwrap_or_else(|| url.to_string())
                });
            let color = r
                .text(NS_APPLE, "calendar-color")
                .map(str::trim)
                .filter(|c| c.starts_with('#') && (c.len() == 7 || c.len() == 9))
                .map(|c| c[..7].to_string());
            // Корпоративная книга (GAL) и ресурсы SOGo только для чтения, даже если
            // сервер не прислал privilege-set.
            let readonly_path = kind == Kind::Contacts
                && (url.path().ends_with("/gal/") || url.path().contains("/resources_of_"));
            let writable = !readonly_path && r.writable().unwrap_or(true);
            out.push(Collection { url: url.to_string(), name, color, writable });
        }
        self.log.push(format!("найдено коллекций: {}", out.len()));
        Ok(out)
    }

    async fn report(&mut self, url: &Url, body: &str) -> Result<Option<Vec<DavResponse>>> {
        let (status, _, text) = self.request("REPORT", url, Some("1"), Some(body)).await?;
        if status != StatusCode::MULTI_STATUS {
            return Ok(None);
        }
        Ok(Some(parse_multistatus(&text)?))
    }

    pub async fn contacts(&mut self, book: &Collection) -> Result<Vec<Contact>> {
        let url = Url::parse(&book.url).map_err(|e| AppError::Other(e.to_string()))?;
        let query = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><cr:addressbook-query xmlns:d="DAV:" xmlns:cr="{NS_CARD}"><d:prop><d:getetag/><cr:address-data/></d:prop></cr:addressbook-query>"#
        );
        let resps = match self.report(&url, &query).await? {
            Some(r) => r,
            None => {
                // Запасной путь: список ресурсов + addressbook-multiget.
                let Some((_, listed)) = self.propfind(&url, "1", "<d:getetag/>").await? else {
                    return Ok(Vec::new());
                };
                let hrefs: String = listed
                    .iter()
                    .filter(|r| r.href.ends_with(".vcf"))
                    .map(|r| format!("<d:href>{}</d:href>", xml_escape(&r.href)))
                    .collect();
                let multiget = format!(
                    r#"<?xml version="1.0" encoding="utf-8"?><cr:addressbook-multiget xmlns:d="DAV:" xmlns:cr="{NS_CARD}"><d:prop><d:getetag/><cr:address-data/></d:prop>{hrefs}</cr:addressbook-multiget>"#
                );
                self.report(&url, &multiget).await?.unwrap_or_default()
            }
        };
        let mut out = Vec::new();
        for r in resps {
            if let Some(data) = r.text(NS_CARD, "address-data") {
                let origin = Origin {
                    href: url.join(&r.href).map(|u| u.to_string()).unwrap_or_else(|_| r.href.clone()),
                    etag: r.text(NS_DAV, "getetag").map(|e| e.trim().to_string()),
                    editable: book.writable,
                };
                out.extend(parse_vcards(data, &book.name, &origin));
            }
        }
        Ok(out)
    }

    /// Поиск в адресной книге на сервере. Нужен для корпоративной книги SOGo
    /// (LDAP): на запрос «все записи» она отвечает пустым списком и ищет
    /// только по строке. Ищем по имени, e-mail, фамилии и организации.
    pub async fn search_contacts(&mut self, book: &Collection, query: &str, limit: usize) -> Result<Vec<Contact>> {
        self.search_contacts_by(book, query, &["FN", "EMAIL", "N", "ORG"], limit).await
    }

    /// Тело addressbook-query с фильтром «содержит» по указанным свойствам.
    pub fn search_body(query: &str, props: &[&str], limit: usize) -> String {
        let q = xml_escape(query.trim());
        let filters: String = props
            .iter()
            .map(|p| format!(r#"<cr:prop-filter name="{p}"><cr:text-match collation="i;unicode-casemap" match-type="contains">{q}</cr:text-match></cr:prop-filter>"#))
            .collect();
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?><cr:addressbook-query xmlns:d="DAV:" xmlns:cr="{NS_CARD}"><d:prop><d:getetag/><cr:address-data/></d:prop><cr:filter test="anyof">{filters}</cr:filter><cr:limit><cr:nresults>{limit}</cr:nresults></cr:limit></cr:addressbook-query>"#
        )
    }

    /// Сырой ответ REPORT — для диагностики.
    pub async fn report_raw(&mut self, url: &str, body: &str) -> Result<(StatusCode, String)> {
        let url = Url::parse(url).map_err(|e| AppError::Other(e.to_string()))?;
        let (status, _, text) = self.request("REPORT", &url, Some("1"), Some(body)).await?;
        Ok((status, text))
    }

    pub async fn search_contacts_by(&mut self, book: &Collection, query: &str, props: &[&str], limit: usize) -> Result<Vec<Contact>> {
        let url = Url::parse(&book.url).map_err(|e| AppError::Other(e.to_string()))?;
        let body = Self::search_body(query, props, limit);
        let Some(resps) = self.report(&url, &body).await? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for r in resps {
            if let Some(data) = r.text(NS_CARD, "address-data") {
                let origin = Origin {
                    href: url.join(&r.href).map(|u| u.to_string()).unwrap_or_else(|_| r.href.clone()),
                    etag: r.text(NS_DAV, "getetag").map(|e| e.trim().to_string()),
                    editable: book.writable,
                };
                out.extend(parse_vcards(data, &book.name, &origin));
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    pub async fn events(
        &mut self,
        cal: &Collection,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<CalEvent>> {
        let url = Url::parse(&cal.url).map_err(|e| AppError::Other(e.to_string()))?;
        let fmt = |d: DateTime<Utc>| d.format("%Y%m%dT%H%M%SZ").to_string();
        let query = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query xmlns:d="DAV:" xmlns:c="{NS_CAL}"><d:prop><d:getetag/><c:calendar-data/></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{}" end="{}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#,
            fmt(start),
            fmt(end)
        );
        let Some(resps) = self.report(&url, &query).await? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for r in resps {
            if let Some(data) = r.text(NS_CAL, "calendar-data") {
                let origin = Origin {
                    href: url.join(&r.href).map(|u| u.to_string()).unwrap_or_else(|_| r.href.clone()),
                    etag: r.text(NS_DAV, "getetag").map(|e| e.trim().to_string()),
                    editable: cal.writable,
                };
                out.extend(parse_events(data, cal, start, end, &origin));
            }
        }
        Ok(out)
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

impl DavClient {
    async fn write_request(
        &mut self,
        method: &str,
        url: &Url,
        body: Option<(&str, &str)>,
        if_match: Option<&str>,
        if_none_match: bool,
    ) -> Result<(StatusCode, Option<String>, String)> {
        let method_v = Method::from_bytes(method.as_bytes()).expect("valid method");
        self.check_target(url)?;
        let mut req = self
            .http
            .request(method_v, url.clone())
            .basic_auth(&self.user, Some(&self.pass));
        if let Some((content_type, data)) = body {
            req = req.header("Content-Type", content_type).body(data.to_string());
        }
        if let Some(tag) = if_match {
            req = req.header("If-Match", tag);
        }
        if if_none_match {
            req = req.header("If-None-Match", "*");
        }
        let resp = req.send().await.map_err(|e| {
            AppError::Other(format!("Сервер календаря/контактов недоступен: {e}"))
        })?;
        let status = resp.status();
        self.log.push(format!("{method} {url} → {status}"));
        let etag = resp
            .headers()
            .get("ETag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let text = crate::netio::text_limited(resp, crate::netio::API_LIMIT).await?;
        Ok((status, etag, text))
    }

    /// Текущая версия ресурса и его ETag.
    pub async fn get(&mut self, href: &str) -> Result<(String, Option<String>)> {
        let url = Url::parse(href).map_err(|e| AppError::Other(e.to_string()))?;
        let (status, etag, text) = self.write_request("GET", &url, None, None, false).await?;
        if !status.is_success() {
            return Err(write_error(status, "Загрузка записи"));
        }
        Ok((text, etag))
    }

    /// Создание (`etag = None`, If-None-Match: *) или изменение (If-Match).
    pub async fn put(&mut self, href: &str, content_type: &str, data: &str, etag: Option<&str>) -> Result<()> {
        let url = Url::parse(href).map_err(|e| AppError::Other(e.to_string()))?;
        let (status, _, _) = self
            .write_request("PUT", &url, Some((content_type, data)), etag, etag.is_none())
            .await?;
        if !status.is_success() {
            return Err(write_error(status, "Сохранение"));
        }
        Ok(())
    }

    pub async fn delete(&mut self, href: &str, etag: Option<&str>) -> Result<()> {
        let url = Url::parse(href).map_err(|e| AppError::Other(e.to_string()))?;
        let (status, _, _) = self.write_request("DELETE", &url, None, etag, false).await?;
        if !status.is_success() && status != StatusCode::NOT_FOUND {
            return Err(write_error(status, "Удаление"));
        }
        Ok(())
    }
}

impl DavClient {
    /// Событие с этим UID уже есть в календаре: его адрес и ETag.
    pub async fn find_uid(&mut self, cal: &Collection, uid: &str) -> Result<Option<(String, Option<String>)>> {
        let url = Url::parse(&cal.url).map_err(|e| AppError::Other(e.to_string()))?;
        let query = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query xmlns:d="DAV:" xmlns:c="{NS_CAL}"><d:prop><d:getetag/><c:calendar-data/></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:prop-filter name="UID"><c:text-match collation="i;octet">{}</c:text-match></c:prop-filter></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#,
            xml_escape(uid)
        );
        let Some(resps) = self.report(&url, &query).await? else { return Ok(None) };
        for r in resps {
            // text-match ищет подстроку — проверяем UID точно.
            let exact = r.text(NS_CAL, "calendar-data").is_some_and(|d| d.lines().any(|l| l.trim_end_matches('\r').strip_prefix("UID:").is_some_and(|v| v.trim() == uid)));
            if exact {
                let href = url.join(&r.href).map(|u| u.to_string()).unwrap_or_else(|_| r.href.clone());
                return Ok(Some((href, r.text(NS_DAV, "getetag").map(|e| e.trim().to_string()))));
            }
        }
        Ok(None)
    }
}

