//! Запись ICS: сборка и правка события, разбор нескольких событий из файла.
//!
//! Часть модуля `dav` — общий обзор в `mod.rs`.

use super::prelude::*;
use super::*;

/// Экранирование значений iCalendar/vCard (RFC 5545 §3.3.11).
pub(crate) fn escape_text(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace(['\r', '\n'], "\\n")
}

/// Перенос строк длиннее 75 октетов, не разрывая символы UTF-8.
pub(crate) fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut len = 0;
    for ch in line.chars() {
        let w = ch.len_utf8();
        if len + w > 75 {
            out.push_str("\r\n ");
            len = 1;
        }
        out.push(ch);
        len += w;
    }
    out
}

pub(crate) fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(cont) = raw.strip_prefix(' ').or_else(|| raw.strip_prefix('\t')) {
            if let Some(last) = lines.last_mut() {
                last.push_str(cont);
                continue;
            }
        }
        if !raw.is_empty() {
            lines.push(raw.to_string());
        }
    }
    lines
}

fn prop_name(line: &str) -> String {
    line.split([';', ':']).next().unwrap_or_default().to_ascii_uppercase()
}

pub(crate) fn join_lines(lines: &[String]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(&fold(l));
        out.push_str("\r\n");
    }
    out
}

/// Заменяет свойства `names` в первом блоке `component`, подходящем под `pick`.
pub(crate) fn replace_props(
    text: &str,
    component: &str,
    pick: impl Fn(&[String]) -> bool,
    names: &[&str],
    new_lines: Vec<String>,
) -> Result<String> {
    let lines = unfold(text);
    let begin = format!("BEGIN:{component}");
    let end = format!("END:{component}");
    let mut i = 0;
    while i < lines.len() {
        if lines[i].eq_ignore_ascii_case(&begin) {
            let Some(rel_end) = lines[i..].iter().position(|l| l.eq_ignore_ascii_case(&end)) else {
                break;
            };
            let block_end = i + rel_end;
            if pick(&lines[i..=block_end]) {
                let mut out: Vec<String> = lines[..=i].to_vec();
                // Свойства вложенных компонентов (VALARM) не трогаем.
                let mut depth = 0;
                for l in &lines[i + 1..block_end] {
                    if l.to_ascii_uppercase().starts_with("BEGIN:") {
                        depth += 1;
                    }
                    let drop = depth == 0 && names.contains(&prop_name(l).as_str());
                    if l.to_ascii_uppercase().starts_with("END:") {
                        depth -= 1;
                    }
                    if !drop {
                        out.push(l.clone());
                    }
                }
                out.extend(new_lines);
                out.extend_from_slice(&lines[block_end..]);
                return Ok(join_lines(&out));
            }
            i = block_end;
        }
        i += 1;
    }
    Err(AppError::Other(format!("В записи нет компонента {component}")))
}

pub(crate) fn utc_stamp() -> String {
    Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventInput {
    pub title: String,
    pub location: Option<String>,
    pub description: Option<String>,
    /// RFC 3339 или YYYY-MM-DD для «весь день».
    pub start: String,
    /// Для «весь день» — день после последнего (не включительно).
    pub end: String,
    pub all_day: bool,
}

fn event_time_lines(input: &EventInput) -> Result<Vec<String>> {
    if input.all_day {
        let parse = |v: &str| {
            NaiveDate::parse_from_str(&v[..10.min(v.len())], "%Y-%m-%d")
                .map_err(|_| AppError::Other(format!("Некорректная дата: {v}")))
        };
        let start = parse(&input.start)?;
        let mut end = parse(&input.end)?;
        if end <= start {
            end = start + chrono::Duration::days(1);
        }
        Ok(vec![
            format!("DTSTART;VALUE=DATE:{}", start.format("%Y%m%d")),
            format!("DTEND;VALUE=DATE:{}", end.format("%Y%m%d")),
        ])
    } else {
        let parse = |v: &str| {
            DateTime::parse_from_rfc3339(v)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|_| AppError::Other(format!("Некорректное время: {v}")))
        };
        let start = parse(&input.start)?;
        let end = parse(&input.end)?;
        if end < start {
            return Err(AppError::Other("Окончание раньше начала".into()));
        }
        Ok(vec![
            format!("DTSTART:{}", start.format("%Y%m%dT%H%M%SZ")),
            format!("DTEND:{}", end.format("%Y%m%dT%H%M%SZ")),
        ])
    }
}

fn event_text_lines(input: &EventInput) -> Result<Vec<String>> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err(AppError::Other("Укажите название события".into()));
    }
    let mut lines = vec![format!("SUMMARY:{}", escape_text(title))];
    if let Some(l) = input.location.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        lines.push(format!("LOCATION:{}", escape_text(l)));
    }
    if let Some(d) = input.description.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        lines.push(format!("DESCRIPTION:{}", escape_text(d)));
    }
    Ok(lines)
}

pub fn build_event_ics(uid: &str, input: &EventInput) -> Result<String> {
    let stamp = utc_stamp();
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".into(),
        "PRODID:-//RRCKI//RRCKI Mail//RU".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{uid}"),
        format!("DTSTAMP:{stamp}"),
        format!("CREATED:{stamp}"),
        format!("LAST-MODIFIED:{stamp}"),
        "SEQUENCE:0".into(),
    ];
    lines.extend(event_text_lines(input)?);
    lines.extend(event_time_lines(input)?);
    lines.push("END:VEVENT".into());
    lines.push("END:VCALENDAR".into());
    Ok(join_lines(&lines))
}

/// Правит основное событие (без RECURRENCE-ID), сохраняя остальное: участников,
/// напоминания, правила повтора. `change_time = false` — время не меняется
/// (для повторяющихся событий, чтобы не сломать серию).
pub fn update_event_ics(original: &str, input: &EventInput, change_time: bool) -> Result<String> {
    let lines = unfold(original);
    let sequence = lines
        .iter()
        .filter(|l| prop_name(l) == "SEQUENCE")
        .filter_map(|l| l.split_once(':').and_then(|(_, v)| v.trim().parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    let mut names = vec!["SUMMARY", "LOCATION", "DESCRIPTION", "DTSTAMP", "LAST-MODIFIED", "SEQUENCE"];
    let mut new_lines = event_text_lines(input)?;
    if change_time {
        names.extend(["DTSTART", "DTEND", "DURATION"]);
        new_lines.extend(event_time_lines(input)?);
    }
    let stamp = utc_stamp();
    new_lines.push(format!("DTSTAMP:{stamp}"));
    new_lines.push(format!("LAST-MODIFIED:{stamp}"));
    new_lines.push(format!("SEQUENCE:{}", sequence + 1));
    replace_props(
        original,
        "VEVENT",
        |block| !block.iter().any(|l| prop_name(l) == "RECURRENCE-ID"),
        &names,
        new_lines,
    )
}

/// Событие из файла календаря: что показать перед добавлением и что записать
/// (`ics` — отдельный календарь с одним UID: событие, его исключения и нужные часовые пояса).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IcsEvent {
    pub uid: String,
    pub title: String,
    /// RFC 3339 (UTC) или YYYY-MM-DD для событий на весь день.
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub location: Option<String>,
    pub recurring: bool,
    #[serde(skip)]
    pub ics: String,
}

/// Сколько событий принимается из одного файла.
pub const MAX_ICS_EVENTS: usize = 500;

/// Разложить файл календаря на события по UID. Свойство METHOD (приглашение
/// iTIP) убирается — в календаре на сервере его быть не должно; событию без UID
/// он назначается.
pub fn split_ics(data: &str) -> Result<Vec<IcsEvent>> {
    // Логические строки: продолжения (начинаются с пробела/табуляции) — к предыдущей.
    let mut lines: Vec<String> = Vec::new();
    for raw in data.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if (line.starts_with(' ') || line.starts_with('\t')) && !lines.is_empty() {
            let last = lines.last_mut().expect("not empty");
            last.push_str("\r\n");
            last.push_str(line);
        } else if !line.trim().is_empty() {
            lines.push(line.to_string());
        }
    }
    let name_of = |l: &str| l.split([':', ';']).next().unwrap_or_default().trim().to_ascii_uppercase();
    let mut header: Vec<String> = Vec::new();
    let mut timezones: Vec<String> = Vec::new();
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut depth = 0usize;
    let mut block: Vec<String> = Vec::new();
    let mut kind = String::new();
    for l in lines {
        let name = name_of(&l);
        let value = l.split_once(':').map(|(_, v)| v.trim().to_ascii_uppercase()).unwrap_or_default();
        if name == "BEGIN" {
            depth += 1;
            if depth == 2 {
                kind = value.clone();
                block.clear();
            }
            if depth >= 2 {
                block.push(l);
            }
            continue;
        }
        if name == "END" {
            if depth >= 2 {
                block.push(l);
            }
            if depth == 2 {
                match kind.as_str() {
                    "VTIMEZONE" => timezones.push(block.join("\r\n")),
                    "VEVENT" => {
                        let uid = block
                            .iter()
                            .find(|x| name_of(x) == "UID")
                            .and_then(|x| x.split_once(':'))
                            .map(|(_, v)| v.replace("\r\n ", "").replace("\r\n\t", "").trim().to_string())
                            .filter(|u| !u.is_empty());
                        let uid = match uid {
                            Some(u) => u,
                            None => {
                                let u = new_uid();
                                block.insert(1, format!("UID:{u}"));
                                u
                            }
                        };
                        match groups.iter_mut().find(|(u, _)| *u == uid) {
                            Some((_, g)) => g.push(block.join("\r\n")),
                            None => {
                                if groups.len() >= MAX_ICS_EVENTS {
                                    return Err(AppError::Other(format!("В файле больше {MAX_ICS_EVENTS} событий")));
                                }
                                groups.push((uid, vec![block.join("\r\n")]));
                            }
                        }
                    }
                    _ => {}
                }
            }
            depth = depth.saturating_sub(1);
            continue;
        }
        if depth == 1 && matches!(name.as_str(), "VERSION" | "PRODID" | "CALSCALE") {
            header.push(l);
        } else if depth >= 2 {
            block.push(l);
        }
    }
    if groups.is_empty() {
        return Err(AppError::Other("В файле нет событий календаря".into()));
    }
    if !header.iter().any(|h| name_of(h) == "VERSION") {
        header.insert(0, "VERSION:2.0".into());
    }
    if !header.iter().any(|h| name_of(h) == "PRODID") {
        header.push("PRODID:-//NRC Kurchatov Institute//Atom Workspace//RU".into());
    }
    let mut out = Vec::new();
    for (uid, events) in groups {
        let mut ics = String::from("BEGIN:VCALENDAR\r\n");
        for h in &header {
            ics.push_str(h);
            ics.push_str("\r\n");
        }
        for tz in &timezones {
            ics.push_str(tz);
            ics.push_str("\r\n");
        }
        for e in &events {
            ics.push_str(e);
            ics.push_str("\r\n");
        }
        ics.push_str("END:VCALENDAR\r\n");
        // Сводка — по основному событию (без RECURRENCE-ID).
        let parsed: Vec<RawEvent> = ical::IcalParser::new(BufReader::new(ics.as_bytes()))
            .flatten()
            .flat_map(|c| c.events.iter().filter_map(raw_event).collect::<Vec<_>>())
            .collect();
        let Some(main) = parsed.iter().find(|e| e.recurrence_id.is_none()).or(parsed.first()) else { continue };
        let (start, end, all_day) = match main.start {
            When::Date(d) => (d.to_string(), (d + chrono::Duration::days(main.duration.num_days().max(1))).to_string(), true),
            When::Time(t) => (t.to_rfc3339(), (t + main.duration).to_rfc3339(), false),
        };
        out.push(IcsEvent {
            uid,
            title: main.title.clone(),
            start,
            end,
            all_day,
            location: main.location.clone(),
            recurring: main.rrule.is_some(),
            ics,
        });
    }
    if out.is_empty() {
        return Err(AppError::Other("События в файле не удалось разобрать (нет даты начала)".into()));
    }
    Ok(out)
}

/// Имя файла для события на сервере: UID, если он безопасен для адреса, иначе новый.
pub fn resource_name(uid: &str) -> String {
    if !uid.is_empty() && uid.len() <= 120 && uid.chars().all(|c| c.is_ascii_alphanumeric() || "-_.@".contains(c)) && !uid.starts_with('.') {
        uid.to_string()
    } else {
        new_uid()
    }
}

pub fn new_uid() -> String {
    uuid::Uuid::new_v4().to_string()
}

