//! Профиль и занятость, пользователи, редакторы документов и загрузка файлов.
//!
//! Часть модуля `nextcloud` — общий обзор в `mod.rs`.

use serde::Serialize;

use super::prelude::*;


/// Профиль сотрудника (поля, которые он открыл в профиле Nextcloud) и статус.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcProfile {
    pub id: String,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub organisation: String,
    pub role: String,
    pub headline: String,
    pub address: String,
    pub status: String,
    pub status_message: String,
}

/// Занятый интервал: начало и конец (unix, с), BUSY / BUSY-TENTATIVE / BUSY-UNAVAILABLE.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BusySlot {
    pub start: i64,
    pub end: i64,
    pub kind: String,
}

/// Ответ outbox (schedule-response) → занятые интервалы; сервер не знает сотрудника — ошибка.
pub fn parse_free_busy(xml: &str, from: i64, to: i64) -> Result<Vec<BusySlot>> {
    let doc = roxmltree::Document::parse(xml).map_err(|_| AppError::Other("Сервер ответил о занятости непонятно".into()))?;
    let status = doc.descendants().find(|n| n.tag_name().name() == "request-status").and_then(|n| n.text()).unwrap_or("").trim().to_string();
    if !status.starts_with('2') {
        return Err(AppError::Other(if status.starts_with("3.7") {
            "Сотрудник не найден в календаре Nextcloud".into()
        } else {
            format!("Календарь не ответил о занятости ({status})")
        }));
    }
    let data = doc.descendants().find(|n| n.tag_name().name() == "calendar-data").and_then(|n| n.text()).unwrap_or("");
    let unfolded = data.replace("\r\n ", "").replace("\n ", "").replace("\r\n\t", "").replace("\n\t", "");
    let ts = |v: &str| chrono::NaiveDateTime::parse_from_str(v.trim().trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok().map(|d| d.and_utc().timestamp());
    let mut out = Vec::new();
    for line in unfolded.lines() {
        let Some(rest) = line.trim_end_matches('\r').strip_prefix("FREEBUSY") else { continue };
        let Some((params, periods)) = rest.split_once(':') else { continue };
        let kind = params.split(';').find_map(|p| p.strip_prefix("FBTYPE=")).unwrap_or("BUSY").to_uppercase();
        if kind == "FREE" {
            continue;
        }
        for period in periods.split(',') {
            let Some((a, b)) = period.split_once('/') else { continue };
            let Some(start) = ts(a) else { continue };
            let end = if let Some(dur) = b.strip_prefix('P').or_else(|| b.strip_prefix("+P")) {
                start + iso_duration(dur)
            } else if let Some(e) = ts(b) {
                e
            } else {
                continue;
            };
            if end > start && end > from && start < to {
                out.push(BusySlot { start: start.max(from), end: end.min(to), kind: kind.clone() });
            }
        }
    }
    out.sort_by_key(|b| b.start);
    Ok(out)
}

/// «T1H30M», «1D», «2W» (без «P») → секунды.
fn iso_duration(d: &str) -> i64 {
    let (mut total, mut num) = (0i64, 0i64);
    for c in d.chars() {
        match c {
            '0'..='9' => num = num * 10 + i64::from(c as u8 - b'0'),
            'W' => { total += num * 7 * 86400; num = 0 }
            'D' => { total += num * 86400; num = 0 }
            'H' => { total += num * 3600; num = 0 }
            'M' => { total += num * 60; num = 0 }
            'S' => { total += num; num = 0 }
            _ => {}
        }
    }
    total
}

#[cfg(test)]
mod free_busy_tests {
    use super::*;

    #[test]
    pub(crate) fn busy_periods_are_parsed_and_clipped() {
        let xml = "<?xml version=\"1.0\"?><cal:schedule-response xmlns:d=\"DAV:\" xmlns:cal=\"urn:ietf:params:xml:ns:caldav\"><cal:response><cal:recipient><d:href>mailto:a@b.ru</d:href></cal:recipient><cal:request-status>2.0;Success</cal:request-status><cal:calendar-data>BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nBEGIN:VFREEBUSY\r\nFREEBUSY:20261112T090000Z/20261112T100000Z,20261112T130000Z/PT1H30M\r\nFREEBUSY;FBTYPE=BUSY-TENTATIVE:20261113T07\r\n 0000Z/20261113T080000Z\r\nFREEBUSY;FBTYPE=FREE:20261113T090000Z/20261113T100000Z\r\nEND:VFREEBUSY\r\nEND:VCALENDAR\r\n</cal:calendar-data></cal:response></cal:schedule-response>";
        let from = chrono::NaiveDate::from_ymd_opt(2026, 11, 12).unwrap().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
        let slots = parse_free_busy(xml, from, from + 7 * 86400).unwrap();
        assert_eq!(slots.len(), 3, "{slots:?}");
        assert_eq!(slots[0].end - slots[0].start, 3600);
        assert_eq!(slots[1].end - slots[1].start, 5400);
        assert_eq!(slots[2].kind, "BUSY-TENTATIVE");
        assert_eq!(slots[2].start, from + 86400 + 7 * 3600);
        let unknown = xml.replace("2.0;Success", "3.7;Invalid calendar user");
        assert!(parse_free_busy(&unknown, from, from + 86400).unwrap_err().to_string().contains("не найден"));
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcUser {
    pub id: String,
    pub label: String,
    pub sub: String,
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


