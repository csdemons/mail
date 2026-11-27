//! События: разбор ICS (часовые пояса, повторы, длительности) и их чтение.
//!
//! Часть модуля `dav` — общий обзор в `mod.rs`.

use super::prelude::*;
use super::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalEvent {
    pub id: String,
    pub uid: String,
    pub href: String,
    pub etag: Option<String>,
    pub editable: bool,
    /// URL календаря — для формы редактирования.
    pub calendar_url: String,
    pub calendar: String,
    pub color: Option<String>,
    pub title: String,
    pub location: Option<String>,
    pub description: Option<String>,
    pub organizer: Option<String>,
    /// Адреса организатора и участников (нижний регистр) — для истории контрагента.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<String>,
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
pub(crate) enum When {
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

pub(crate) fn tz_of(tzid: Option<&str>) -> Option<chrono_tz::Tz> {
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

pub(crate) fn parse_duration(v: &str) -> Option<chrono::Duration> {
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

pub(crate) struct RawEvent {
    pub(crate) uid: String,
    pub(crate) title: String,
    pub(crate) location: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) organizer: Option<String>,
    pub(crate) participants: Vec<String>,
    pub(crate) start: When,
    pub(crate) start_prop: ical::property::Property,
    pub(crate) duration: chrono::Duration,
    pub(crate) rrule: Option<String>,
    pub(crate) exdates: Vec<DateTime<Utc>>,
    pub(crate) recurrence_id: Option<DateTime<Utc>>,
    pub(crate) cancelled: bool,
}

pub(crate) fn raw_event(ev: &ical::parser::ical::component::IcalEvent) -> Option<RawEvent> {
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
        participants: ev
            .properties
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case("ORGANIZER") || p.name.eq_ignore_ascii_case("ATTENDEE"))
            .filter_map(|p| p.value.as_deref())
            .map(|v| {
                let v = v.trim();
                let v = match v.get(..7) { Some(p) if p.eq_ignore_ascii_case("mailto:") => &v[7..], _ => v };
                v.trim().to_lowercase()
            })
            .filter(|v| v.contains('@'))
            .fold(Vec::new(), |mut acc, v| {
                if !acc.contains(&v) {
                    acc.push(v);
                }
                acc
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

fn occurrence(
    ev: &RawEvent,
    cal: &Collection,
    origin: &Origin,
    start_utc: DateTime<Utc>,
    recurring: bool,
) -> CalEvent {
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
        uid: ev.uid.clone(),
        href: origin.href.clone(),
        etag: origin.etag.clone(),
        editable: origin.editable,
        calendar_url: cal.url.clone(),
        calendar: cal.name.clone(),
        color: cal.color.clone(),
        title: ev.title.clone(),
        location: ev.location.clone(),
        description: ev.description.clone(),
        organizer: ev.organizer.clone(),
        participants: ev.participants.clone(),
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

pub fn parse_events(
    data: &str,
    cal: &Collection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    origin: &Origin,
) -> Vec<CalEvent> {
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
                out.push(occurrence(m, cal, origin, s, recurring));
            }
        }
    }
    for o in &overrides {
        let s = o.start.utc();
        if !o.cancelled && in_range(s, o.duration) {
            out.push(occurrence(o, cal, origin, s, true));
        }
    }
    out
}

// ---------------------------------------------------------------- запись

pub(crate) fn write_error(status: StatusCode, what: &str) -> AppError {
    match status.as_u16() {
        401 => AppError::Other("Сервер отклонил логин или пароль (HTTP 401)".into()),
        403 => AppError::Other(format!("Нет прав на изменение: {what}")),
        404 => AppError::Other(format!("{what}: запись не найдена на сервере — обновите список")),
        409 | 412 => AppError::Other(format!(
            "{what}: запись изменилась на сервере (например, в веб-интерфейсе) — обновите и повторите"
        )),
        507 => AppError::Other("На сервере недостаточно места".into()),
        _ => AppError::Other(format!("{what}: сервер ответил HTTP {status}")),
    }
}

