//! Контакты: разбор vCard и запись контакта в адресную книгу.
//!
//! Часть модуля `dav` — общий обзор в `mod.rs`.

use super::prelude::*;
use super::*;

/// Откуда запись: адрес ресурса, ETag (для безопасного изменения) и права.
#[derive(Debug, Clone, Default)]
pub struct Origin {
    pub href: String,
    pub etag: Option<String>,
    pub editable: bool,
}

// ---------------------------------------------------------------- vCard

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub id: String,
    pub book: String,
    pub href: String,
    pub etag: Option<String>,
    pub editable: bool,
    pub full_name: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub org: Option<String>,
    pub title: Option<String>,
    pub note: Option<String>,
    /// Адрес (ADR) одной строкой.
    pub address: Option<String>,
}

pub(crate) fn unescape(v: &str) -> String {
    v.replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

pub fn parse_vcards(data: &str, book: &str, origin: &Origin) -> Vec<Contact> {
    let href = origin.href.as_str();
    let parser = ical::VcardParser::new(BufReader::new(data.as_bytes()));
    let mut out = Vec::new();
    for (i, card) in parser.flatten().enumerate() {
        let mut c = Contact {
            id: format!("{href}#{i}"),
            book: book.to_string(),
            href: href.to_string(),
            etag: origin.etag.clone(),
            editable: origin.editable && i == 0,
            full_name: String::new(),
            emails: vec![],
            phones: vec![],
            org: None,
            title: None,
            note: None,
            address: None,
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
                "ADR" if c.address.is_none() => {
                    // ADR: п/я;доп.;улица;город;регион;индекс;страна
                    let a = v.split(';').map(|x| unescape(x.trim())).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(", ");
                    if !a.is_empty() { c.address = Some(a); }
                }
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

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactInput {
    pub full_name: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub org: Option<String>,
    pub title: Option<String>,
    pub note: Option<String>,
}

fn contact_lines(input: &ContactInput) -> Result<Vec<String>> {
    let name = input.full_name.trim();
    if name.is_empty() {
        return Err(AppError::Other("Укажите имя контакта".into()));
    }
    // N: Фамилия;Имя;Отчество;; — порядок «Фамилия Имя Отчество», как пишут в RRCKI.
    let parts: Vec<String> = name.split_whitespace().map(escape_text).collect();
    let n = format!(
        "N:{};{};{};;",
        parts.first().cloned().unwrap_or_default(),
        parts.get(1).cloned().unwrap_or_default(),
        if parts.len() > 2 { parts[2..].join(" ") } else { String::new() }
    );
    let mut lines = vec![format!("FN:{}", escape_text(name)), n];
    for e in input.emails.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
        if !e.contains('@') {
            return Err(AppError::Other(format!("Некорректный e-mail: {e}")));
        }
        lines.push(format!("EMAIL;TYPE=INTERNET:{}", escape_text(e)));
    }
    for p in input.phones.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        lines.push(format!("TEL;TYPE=WORK:{}", escape_text(p)));
    }
    if let Some(org) = input.org.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let units: Vec<String> = org.split(", ").map(escape_text).collect();
        lines.push(format!("ORG:{}", units.join(";")));
    }
    if let Some(t) = input.title.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        lines.push(format!("TITLE:{}", escape_text(t)));
    }
    if let Some(n) = input.note.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        lines.push(format!("NOTE:{}", escape_text(n)));
    }
    lines.push(format!("REV:{}", utc_stamp()));
    Ok(lines)
}

pub fn build_vcard(uid: &str, input: &ContactInput) -> Result<String> {
    let mut lines = vec![
        "BEGIN:VCARD".to_string(),
        "VERSION:3.0".into(),
        "PRODID:-//RRCKI//RRCKI Mail//RU".into(),
        format!("UID:{uid}"),
    ];
    lines.extend(contact_lines(input)?);
    lines.push("END:VCARD".into());
    Ok(join_lines(&lines))
}

/// Правит карточку, сохраняя поля, которые клиент не редактирует (фото, адреса …).
pub fn update_vcard(original: &str, input: &ContactInput) -> Result<String> {
    replace_props(
        original,
        "VCARD",
        |_| true,
        &["FN", "N", "EMAIL", "TEL", "ORG", "TITLE", "NOTE", "REV"],
        contact_lines(input)?,
    )
}

// ---------------------------------------------------------------- файл календаря (.ics) из письма

