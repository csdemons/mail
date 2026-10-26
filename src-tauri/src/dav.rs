//! CalDAV / CardDAV: календарь и контакты (RuPost/SOGo, Radicale и др.).
//! Тот же логин и пароль, что и для почты. Только чтение.

use std::collections::HashMap;
use std::io::BufReader;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use reqwest::{Client, Method, StatusCode};
use serde::Serialize;
use url::Url;

use crate::error::{AppError, Result};

const NS_DAV: &str = "DAV:";
const NS_CAL: &str = "urn:ietf:params:xml:ns:caldav";
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
}

pub struct DavClient {
    http: Client,
    user: String,
    pass: String,
    pub log: Vec<String>,
}

/// Свойства одного `<response>` из `<multistatus>` (только со статусом 200).
#[derive(Debug, Default)]
struct DavResponse {
    href: String,
    /// (namespace, имя) → текст
    text: HashMap<(String, String), String>,
    /// (namespace, имя) → вложенные `<href>`
    hrefs: HashMap<(String, String), Vec<String>>,
    /// типы ресурса: (namespace, имя) из `<resourcetype>`
    resource_types: Vec<(String, String)>,
}

impl DavResponse {
    fn text(&self, ns: &str, name: &str) -> Option<&str> {
        self.text.get(&(ns.to_string(), name.to_string())).map(String::as_str)
    }
    fn href(&self, ns: &str, name: &str) -> Option<&str> {
        self.hrefs
            .get(&(ns.to_string(), name.to_string()))
            .and_then(|v| v.first())
            .map(String::as_str)
    }
    fn is(&self, ns: &str, name: &str) -> bool {
        self.resource_types.iter().any(|(n, l)| n == ns && l == name)
    }
}

fn parse_multistatus(xml: &str) -> Result<Vec<DavResponse>> {
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
                .map_or(true, |s| s.contains(" 200"));
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
    pub fn new(user: &str, pass: &str) -> Result<Self> {
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(45))
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("RRCKI Mail/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AppError::Other(format!("HTTP-клиент: {e}")))?;
        Ok(Self {
            http,
            user: user.to_string(),
            pass: pass.to_string(),
            log: Vec::new(),
        })
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
            let text = resp.text().await.unwrap_or_default();
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

        let props = "<d:resourcetype/><d:displayname/><a:calendar-color/><c:supported-calendar-component-set/>";
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
            out.push(Collection { url: url.to_string(), name, color });
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
                out.extend(parse_vcards(data, &book.name, &r.href));
            }
        }
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
                out.extend(parse_events(data, cal, start, end));
            }
        }
        Ok(out)
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

// ---------------------------------------------------------------- vCard

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub id: String,
    pub book: String,
    pub full_name: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub org: Option<String>,
    pub title: Option<String>,
    pub note: Option<String>,
}

fn unescape(v: &str) -> String {
    v.replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

pub fn parse_vcards(data: &str, book: &str, href: &str) -> Vec<Contact> {
    let parser = ical::VcardParser::new(BufReader::new(data.as_bytes()));
    let mut out = Vec::new();
    for (i, card) in parser.flatten().enumerate() {
        let mut c = Contact {
            id: format!("{href}#{i}"),
            book: book.to_string(),
            full_name: String::new(),
            emails: vec![],
            phones: vec![],
            org: None,
            title: None,
            note: None,
        };
        let mut n_value = None;
        for p in &card.properties {
            let Some(v) = p.value.as_deref().map(str::trim).filter(|v| !v.is_empty()) else { continue };
            match p.name.to_uppercase().as_str() {
                "FN" => c.full_name = unescape(v),
                "N" => n_value = Some(v.to_string()),
                "EMAIL" => c.emails.push(unescape(v)),
                "TEL" => c.phones.push(unescape(v).trim_start_matches("tel:").to_string()),
                "ORG" => c.org = Some(unescape(&v.replace(';', ", ")).trim_matches([',', ' ']).to_string()),
                "TITLE" => c.title = Some(unescape(v)),
                "NOTE" => c.note = Some(unescape(v)),
                _ => {}
            }
        }
        if c.full_name.is_empty() {
            if let Some(n) = n_value {
                // N: Фамилия;Имя;Отчество;…
                let parts: Vec<&str> = n.split(';').collect();
                let name = [parts.first(), parts.get(1), parts.get(2)]
                    .into_iter()
                    .flatten()
                    .map(|s| unescape(s.trim()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                c.full_name = name;
            }
        }
        if c.full_name.is_empty() {
            c.full_name = c.emails.first().cloned().unwrap_or_else(|| "(без имени)".into());
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------- iCalendar

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalEvent {
    pub id: String,
    pub calendar: String,
    pub color: Option<String>,
    pub title: String,
    pub location: Option<String>,
    pub description: Option<String>,
    pub organizer: Option<String>,
    /// RFC 3339 (UTC) для событий со временем, YYYY-MM-DD для «весь день».
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub recurring: bool,
}

fn param<'a>(p: &'a ical::property::Property, name: &str) -> Option<&'a str> {
    p.params
        .as_ref()?
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .and_then(|(_, v)| v.first())
        .map(String::as_str)
}

/// Windows-имена часовых поясов (Exchange/Outlook) → IANA.
fn windows_tz(name: &str) -> Option<&'static str> {
    Some(match name {
        "Russian Standard Time" => "Europe/Moscow",
        "Russia Time Zone 3" => "Europe/Samara",
        "Ekaterinburg Standard Time" => "Asia/Yekaterinburg",
        "N. Central Asia Standard Time" => "Asia/Novosibirsk",
        "North Asia Standard Time" => "Asia/Krasnoyarsk",
        "North Asia East Standard Time" => "Asia/Irkutsk",
        "Yakutsk Standard Time" => "Asia/Yakutsk",
        "Vladivostok Standard Time" => "Asia/Vladivostok",
        "Kaliningrad Standard Time" => "Europe/Kaliningrad",
        "UTC" | "Coordinated Universal Time" => "UTC",
        "W. Europe Standard Time" => "Europe/Berlin",
        "GMT Standard Time" => "Europe/London",
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy)]
enum When {
    Date(NaiveDate),
    Time(DateTime<Utc>),
}

impl When {
    fn utc(self) -> DateTime<Utc> {
        match self {
            When::Time(t) => t,
            When::Date(d) => Local
                .from_local_datetime(&d.and_hms_opt(0, 0, 0).expect("midnight"))
                .earliest()
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(|| Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).expect("midnight"))),
        }
    }
}

fn tz_of(tzid: Option<&str>) -> Option<chrono_tz::Tz> {
    let tzid = tzid?.trim_matches('"');
    tzid.parse::<chrono_tz::Tz>()
        .ok()
        .or_else(|| windows_tz(tzid).and_then(|n| n.parse().ok()))
}

fn parse_when(p: &ical::property::Property) -> Option<When> {
    let v = p.value.as_deref()?.trim();
    if param(p, "VALUE").is_some_and(|x| x.eq_ignore_ascii_case("DATE")) || v.len() == 8 {
        return NaiveDate::parse_from_str(&v[..8.min(v.len())], "%Y%m%d").ok().map(When::Date);
    }
    if let Some(stripped) = v.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        return Some(When::Time(Utc.from_utc_datetime(&naive)));
    }
    let naive = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    let local = match tz_of(param(p, "TZID")) {
        Some(tz) => tz.from_local_datetime(&naive).earliest().map(|t| t.with_timezone(&Utc)),
        None => Local.from_local_datetime(&naive).earliest().map(|t| t.with_timezone(&Utc)),
    };
    local.map(When::Time)
}

fn parse_duration(v: &str) -> Option<chrono::Duration> {
    // P[n]W | P[n]DT[n]H[n]M[n]S
    let v = v.trim();
    let (neg, v) = v.strip_prefix('-').map_or((false, v), |r| (true, r));
    let v = v.strip_prefix('+').unwrap_or(v).strip_prefix('P')?;
    let mut total = chrono::Duration::zero();
    let mut num = String::new();
    let mut in_time = false;
    for ch in v.chars() {
        match ch {
            '0'..='9' => num.push(ch),
            'T' => in_time = true,
            'W' | 'D' | 'H' | 'M' | 'S' => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match (ch, in_time) {
                    ('W', _) => chrono::Duration::weeks(n),
                    ('D', _) => chrono::Duration::days(n),
                    ('H', true) => chrono::Duration::hours(n),
                    ('M', true) => chrono::Duration::minutes(n),
                    ('S', true) => chrono::Duration::seconds(n),
                    _ => return None,
                };
            }
            _ => return None,
        }
    }
    Some(if neg { -total } else { total })
}

struct RawEvent {
    uid: String,
    title: String,
    location: Option<String>,
    description: Option<String>,
    organizer: Option<String>,
    start: When,
    start_prop: ical::property::Property,
    duration: chrono::Duration,
    rrule: Option<String>,
    exdates: Vec<DateTime<Utc>>,
    recurrence_id: Option<DateTime<Utc>>,
    cancelled: bool,
}

fn raw_event(ev: &ical::parser::ical::component::IcalEvent) -> Option<RawEvent> {
    let get = |name: &str| ev.properties.iter().find(|p| p.name.eq_ignore_ascii_case(name));
    let start_prop = get("DTSTART")?.clone();
    let start = parse_when(&start_prop)?;
    let duration = if let Some(end) = get("DTEND").and_then(parse_when) {
        end.utc() - start.utc()
    } else if let Some(d) = get("DURATION").and_then(|p| p.value.as_deref()).and_then(parse_duration) {
        d
    } else if matches!(start, When::Date(_)) {
        chrono::Duration::days(1)
    } else {
        chrono::Duration::zero()
    };
    let text = |name: &str| {
        get(name)
            .and_then(|p| p.value.as_deref())
            .map(unescape)
            .filter(|s| !s.trim().is_empty())
    };
    let exdates = ev
        .properties
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case("EXDATE"))
        .flat_map(|p| {
            p.value
                .as_deref()
                .unwrap_or_default()
                .split(',')
                .filter_map(|v| {
                    let mut single = p.clone();
                    single.value = Some(v.to_string());
                    parse_when(&single).map(When::utc)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    Some(RawEvent {
        uid: text("UID").unwrap_or_default(),
        title: text("SUMMARY").unwrap_or_else(|| "(без названия)".into()),
        location: text("LOCATION"),
        description: text("DESCRIPTION"),
        organizer: get("ORGANIZER").map(|p| {
            param(p, "CN")
                .map(|cn| cn.trim_matches('"').to_string())
                .unwrap_or_else(|| {
                    p.value
                        .as_deref()
                        .unwrap_or_default()
                        .trim_start_matches("mailto:")
                        .trim_start_matches("MAILTO:")
                        .to_string()
                })
        }),
        start,
        start_prop,
        duration,
        rrule: text("RRULE"),
        exdates,
        recurrence_id: get("RECURRENCE-ID").and_then(parse_when).map(When::utc),
        cancelled: text("STATUS").is_some_and(|s| s.eq_ignore_ascii_case("CANCELLED")),
    })
}

fn occurrence(ev: &RawEvent, cal: &Collection, start_utc: DateTime<Utc>, recurring: bool) -> CalEvent {
    let (start, end, all_day) = match ev.start {
        When::Date(d) => {
            let offset = start_utc - ev.start.utc();
            let s = d + offset;
            let days = ev.duration.num_days().max(1);
            (s.to_string(), (s + chrono::Duration::days(days)).to_string(), true)
        }
        When::Time(_) => (
            start_utc.to_rfc3339(),
            (start_utc + ev.duration).to_rfc3339(),
            false,
        ),
    };
    CalEvent {
        id: format!("{}@{}", ev.uid, start_utc.timestamp()),
        calendar: cal.name.clone(),
        color: cal.color.clone(),
        title: ev.title.clone(),
        location: ev.location.clone(),
        description: ev.description.clone(),
        organizer: ev.organizer.clone(),
        start,
        end,
        all_day,
        recurring,
    }
}

/// Развёртывает повторения (RRULE) в окне [from, to).
fn expand(ev: &RawEvent, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let Some(rule) = &ev.rrule else { return vec![ev.start.utc()] };
    let dtstart = match ev.start {
        When::Date(d) => format!("DTSTART:{}", d.format("%Y%m%dT000000")),
        When::Time(t) => match tz_of(param(&ev.start_prop, "TZID")) {
            Some(tz) => format!(
                "DTSTART;TZID={}:{}",
                tz.name(),
                t.with_timezone(&tz).format("%Y%m%dT%H%M%S")
            ),
            None => format!("DTSTART:{}", t.format("%Y%m%dT%H%M%SZ")),
        },
    };
    let Ok(set) = format!("{dtstart}\nRRULE:{rule}").parse::<rrule::RRuleSet>() else {
        return vec![ev.start.utc()];
    };
    let lower = (from - ev.duration - chrono::Duration::days(1)).with_timezone(&rrule::Tz::UTC);
    let upper = (to + chrono::Duration::days(1)).with_timezone(&rrule::Tz::UTC);
    set.after(lower)
        .before(upper)
        .all(1000)
        .dates
        .into_iter()
        .map(|d| d.with_timezone(&Utc))
        .collect()
}

pub fn parse_events(data: &str, cal: &Collection, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<CalEvent> {
    let mut masters = Vec::new();
    let mut overrides: Vec<RawEvent> = Vec::new();
    for calendar in ical::IcalParser::new(BufReader::new(data.as_bytes())).flatten() {
        for ev in &calendar.events {
            if let Some(raw) = raw_event(ev) {
                if raw.recurrence_id.is_some() {
                    overrides.push(raw);
                } else {
                    masters.push(raw);
                }
            }
        }
    }
    let in_range = |s: DateTime<Utc>, d: chrono::Duration| s < to && s + d.max(chrono::Duration::minutes(1)) > from;
    let mut out = Vec::new();
    for m in &masters {
        if m.cancelled {
            continue;
        }
        let recurring = m.rrule.is_some();
        for s in expand(m, from, to) {
            let excluded = m.exdates.contains(&s)
                || overrides.iter().any(|o| o.uid == m.uid && o.recurrence_id == Some(s));
            if !excluded && in_range(s, m.duration) {
                out.push(occurrence(m, cal, s, recurring));
            }
        }
    }
    for o in &overrides {
        let s = o.start.utc();
        if !o.cancelled && in_range(s, o.duration) {
            out.push(occurrence(o, cal, s, true));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cal() -> Collection {
        Collection { url: "https://x/cal/".into(), name: "Личный".into(), color: Some("#2474ed".into()) }
    }

    #[test]
    fn parses_vcard_with_cyrillic_and_n_fallback() {
        let data = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Иванов;Иван;Иванович;;\r\nEMAIL;TYPE=work:ivanov@nrcki.ru\r\nTEL:+7 495 000-00-00\r\nORG:НИЦ КИ;УИТ\r\nTITLE:Инженер\r\nEND:VCARD\r\n";
        let c = parse_vcards(data, "Личные", "/c/1.vcf");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].full_name, "Иванов Иван Иванович");
        assert_eq!(c[0].emails, vec!["ivanov@nrcki.ru"]);
        assert_eq!(c[0].org.as_deref(), Some("НИЦ КИ, УИТ"));
    }

    #[test]
    fn expands_weekly_event_with_exdate_and_override() {
        let data = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:planerka\r\nSUMMARY:Планёрка\r\nDTSTART;TZID=Europe/Moscow:20261005T100000\r\nDTEND;TZID=Europe/Moscow:20261005T103000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nEXDATE;TZID=Europe/Moscow:20261012T100000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:planerka\r\nRECURRENCE-ID;TZID=Europe/Moscow:20261019T100000\r\nSUMMARY:Планёрка (перенос)\r\nDTSTART;TZID=Europe/Moscow:20261019T120000\r\nDTEND;TZID=Europe/Moscow:20261019T123000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:allday\r\nSUMMARY:Командировка\r\nDTSTART;VALUE=DATE:20261014\r\nDTEND;VALUE=DATE:20261016\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let from = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2026, 10, 31, 0, 0, 0).unwrap();
        let mut ev = parse_events(data, &cal(), from, to);
        ev.sort_by(|a, b| a.start.cmp(&b.start));
        let view: Vec<(&str, &str)> = ev.iter().map(|e| (e.start.as_str(), e.title.as_str())).collect();
        assert_eq!(
            view,
            vec![
                ("2026-10-05T07:00:00+00:00", "Планёрка"),
                ("2026-10-14", "Командировка"),
                ("2026-10-19T09:00:00+00:00", "Планёрка (перенос)"),
                ("2026-10-26T07:00:00+00:00", "Планёрка"),
            ]
        );
        let allday = ev.iter().find(|e| e.all_day).unwrap();
        assert_eq!(allday.end, "2026-10-16");
        assert!(ev[0].recurring && ev[0].end == "2026-10-05T07:30:00+00:00");
    }

    #[test]
    fn maps_windows_timezones_and_durations() {
        assert_eq!(tz_of(Some("Russian Standard Time")).map(|t| t.name()), Some("Europe/Moscow"));
        assert_eq!(parse_duration("PT1H30M"), Some(chrono::Duration::minutes(90)));
        assert_eq!(parse_duration("P1W"), Some(chrono::Duration::days(7)));
    }

    #[test]
    fn parses_multistatus_props() {
        let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/SOGo/dav/u/Calendar/personal/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Личный</d:displayname></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat><d:propstat><d:prop><x:calendar-color xmlns:x="http://apple.com/ns/ical/"/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response></d:multistatus>"#;
        let r = parse_multistatus(xml).unwrap();
        assert_eq!(r[0].href, "/SOGo/dav/u/Calendar/personal/");
        assert!(r[0].is(NS_CAL, "calendar"));
        assert_eq!(r[0].text(NS_DAV, "displayname"), Some("Личный"));
    }
}
