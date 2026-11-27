//! CalDAV/CardDAV: поиск коллекций, чтение и запись событий и контактов, разбор ICS и vCard.
//!
//! Разделён по задачам: см. подмодули ниже.

mod prelude;
mod client;
mod contacts;
mod events;
mod ics;

pub use client::*;

pub use contacts::*;
pub use events::*;
pub use ics::*;
pub(crate) use contacts::{unescape};
pub(crate) use events::{RawEvent, When, raw_event, write_error};
pub(crate) use ics::{escape_text, join_lines, replace_props, utc_stamp};

#[cfg(test)]
mod tests {
    use super::*;
    use super::{NS_DAV, NS_CAL, parse_multistatus, tz_of, parse_duration, unfold};
    use super::prelude::*;

    #[test]
    fn password_only_to_configured_server() {
        let base = Url::parse("https://rupost/").unwrap();
        let mut c = DavClient::new("u", "p", &base).unwrap();
        for ok in ["https://rupost/SOGo/dav/u/", "https://RUPOST:443/x"] {
            assert!(c.check_target(&Url::parse(ok).unwrap()).is_ok(), "{ok}");
        }
        for bad in ["https://evil.example/", "http://rupost/", "https://rupost:8443/", "https://rupost.evil.example/"] {
            assert!(c.check_target(&Url::parse(bad).unwrap()).is_err(), "{bad}");
        }
        // http в настройках: сам http-адрес не получает пароль, допустим лишь https того же сервера.
        let mut c = DavClient::new("u", "p", &Url::parse("http://dav.local:8080/").unwrap()).unwrap();
        assert!(c.check_target(&Url::parse("http://dav.local:8080/").unwrap()).is_err());
        assert!(c.check_target(&Url::parse("https://dav.local/").unwrap()).is_ok());
        assert!(c.check_target(&Url::parse("http://other.local:8080/").unwrap()).is_err());
    }

    #[test]
    fn splits_calendar_file_by_uid() {
        let data = "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Outlook//RU\nMETHOD:REQUEST\nBEGIN:VTIMEZONE\nTZID:Russian Standard Time\nBEGIN:STANDARD\nDTSTART:16010101T000000\nTZOFFSETFROM:+0300\nTZOFFSETTO:+0300\nEND:STANDARD\nEND:VTIMEZONE\nBEGIN:VEVENT\nUID:meet-1\nSUMMARY:Совещание по проекту с очень дли\n нным названием\nDTSTART;TZID=Russian Standard Time:20261110T100000\nDTEND;TZID=Russian Standard Time:20261110T113000\nLOCATION:Переговорная 214\nRRULE:FREQ=WEEKLY;COUNT=4\nBEGIN:VALARM\nTRIGGER:-PT15M\nACTION:DISPLAY\nEND:VALARM\nEND:VEVENT\nBEGIN:VEVENT\nUID:meet-1\nRECURRENCE-ID;TZID=Russian Standard Time:20261117T100000\nSUMMARY:Перенос\nDTSTART;TZID=Russian Standard Time:20261118T100000\nDTEND;TZID=Russian Standard Time:20261118T110000\nEND:VEVENT\nBEGIN:VEVENT\nSUMMARY:Без UID\nDTSTART;VALUE=DATE:20261201\nEND:VEVENT\nEND:VCALENDAR\n";
        let ev = split_ics(data).unwrap();
        assert_eq!(ev.len(), 2);
        let a = &ev[0];
        assert_eq!(a.uid, "meet-1");
        assert_eq!(a.title, "Совещание по проекту с очень длинным названием");
        assert_eq!(a.start, "2026-11-10T07:00:00+00:00");
        assert_eq!(a.end, "2026-11-10T08:30:00+00:00");
        assert!(a.recurring && !a.all_day);
        assert_eq!(a.location.as_deref(), Some("Переговорная 214"));
        assert!(!a.ics.contains("METHOD:"), "METHOD kept");
        assert!(a.ics.contains("BEGIN:VTIMEZONE") && a.ics.contains("RECURRENCE-ID") && a.ics.contains("BEGIN:VALARM"));
        assert_eq!(a.ics.matches("BEGIN:VEVENT").count(), 2);
        assert!(a.ics.contains("SUMMARY:Совещание по проекту с очень дли\r\n нным названием"));
        let b = &ev[1];
        assert!(b.all_day && b.start == "2026-12-01" && b.end == "2026-12-02");
        assert!(b.ics.contains(&format!("UID:{}", b.uid)) && !b.uid.is_empty());
        // Записанное читается обратно как событие календаря.
        let back = parse_events(&a.ics, &cal(), Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap(), Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap(), &Origin::default());
        assert_eq!(back.len(), 4);
        assert!(back.iter().any(|e| e.title == "Перенос"));
        assert!(split_ics("BEGIN:VCALENDAR\nEND:VCALENDAR\n").is_err());
        assert!(split_ics("не календарь").is_err());
        assert_eq!(resource_name("meet-1"), "meet-1");
        assert_ne!(resource_name("../x"), "../x");
        assert_ne!(resource_name("a b"), "a b");
    }

    fn cal() -> Collection {
        Collection { url: "https://x/cal/".into(), name: "Личный".into(), color: Some("#2474ed".into()), writable: true }
    }

    #[test]
    fn parses_vcard_with_cyrillic_and_n_fallback() {
        let data = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Иванов;Иван;Иванович;;\r\nEMAIL;TYPE=work:ivanov@inst.test\r\nTEL:+7 495 000-00-00\r\nORG:НИЦ КИ;УИТ\r\nTITLE:Инженер\r\nEND:VCARD\r\n";
        let c = parse_vcards(data, "Личные", &Origin { href: "/c/1.vcf".into(), etag: None, editable: true });
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].full_name, "Иванов Иван Иванович");
        assert_eq!(c[0].emails, vec!["ivanov@inst.test"]);
        assert_eq!(c[0].org.as_deref(), Some("НИЦ КИ, УИТ"));
    }

    #[test]
    fn expands_weekly_event_with_exdate_and_override() {
        let data = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:planerka\r\nSUMMARY:Планёрка\r\nDTSTART;TZID=Europe/Moscow:20261005T100000\r\nDTEND;TZID=Europe/Moscow:20261005T103000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nEXDATE;TZID=Europe/Moscow:20261012T100000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:planerka\r\nRECURRENCE-ID;TZID=Europe/Moscow:20261019T100000\r\nSUMMARY:Планёрка (перенос)\r\nDTSTART;TZID=Europe/Moscow:20261019T120000\r\nDTEND;TZID=Europe/Moscow:20261019T123000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:allday\r\nSUMMARY:Командировка\r\nDTSTART;VALUE=DATE:20261014\r\nDTEND;VALUE=DATE:20261016\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let from = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2026, 10, 31, 0, 0, 0).unwrap();
        let mut ev = parse_events(data, &cal(), from, to, &Origin::default());
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
    fn builds_and_updates_event_preserving_other_props() {
        let input = EventInput {
            title: "Совещание, отдел; УИТ".into(),
            location: Some("Каб. 214".into()),
            description: Some("Строка 1\nСтрока 2".into()),
            start: "2026-10-27T07:00:00Z".into(),
            end: "2026-10-27T08:00:00+00:00".into(),
            all_day: false,
        };
        let ics = build_event_ics("u1", &input).unwrap();
        assert!(ics.contains("SUMMARY:Совещание\\, отдел\\; УИТ\r\n"));
        assert!(ics.contains("DTSTART:20261027T070000Z") && ics.contains("DTEND:20261027T080000Z"));
        let parsed = parse_events(&ics, &cal(), Utc.with_ymd_and_hms(2026, 10, 26, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 10, 28, 0, 0, 0).unwrap(), &Origin::default());
        assert_eq!(parsed[0].title, "Совещание, отдел; УИТ");
        assert_eq!(parsed[0].description.as_deref(), Some("Строка 1\nСтрока 2"));

        // Серия с напоминанием и переносом: меняем только текст, время и VALARM не трогаем.
        let series = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:s\r\nSUMMARY:Старое\r\nDTSTART:20261005T070000Z\r\nRRULE:FREQ=WEEKLY\r\nSEQUENCE:3\r\nATTENDEE;CN=Иванов:mailto:ivanov@inst.test\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Напоминание\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID:20261012T070000Z\r\nSUMMARY:Перенос\r\nDTSTART:20261012T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        // Участники встречи — для истории контрагента (адреса без mailto:, без повторов).
        let with_people = series.replace("SEQUENCE:3\r\n", "SEQUENCE:3\r\nORGANIZER;CN=Петров:MAILTO:Petrov@IHEP.ru\r\nATTENDEE:mailto:petrov@ihep.ru\r\n");
        let occ = parse_events(&with_people, &cal(), Utc.with_ymd_and_hms(2026, 10, 4, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 10, 6, 0, 0, 0).unwrap(), &Origin::default());
        assert_eq!(occ[0].participants, ["petrov@ihep.ru", "ivanov@inst.test"]);
        let upd = update_event_ics(series, &EventInput { title: "Новое".into(), location: None, description: None,
            start: "2030-01-01T00:00:00Z".into(), end: "2030-01-01T01:00:00Z".into(), all_day: false }, false).unwrap();
        assert!(upd.contains("SUMMARY:Новое") && upd.contains("SUMMARY:Перенос"));
        assert!(upd.contains("DTSTART:20261005T070000Z") && !upd.contains("2030"));
        assert!(upd.contains("RRULE:FREQ=WEEKLY") && upd.contains("ATTENDEE;CN=Иванов"));
        assert!(upd.contains("DESCRIPTION:Напоминание") && upd.contains("SEQUENCE:4"));

        let allday = build_event_ics("a", &EventInput { title: "Отпуск".into(), location: None, description: None,
            start: "2026-11-02".into(), end: "2026-11-02".into(), all_day: true }).unwrap();
        assert!(allday.contains("DTSTART;VALUE=DATE:20261102") && allday.contains("DTEND;VALUE=DATE:20261103"));
    }

    #[test]
    fn builds_and_updates_vcard() {
        let input = ContactInput {
            full_name: "Сидоров Пётр Ильич".into(),
            emails: vec!["sidorov@inst.test".into(), " ".into()],
            phones: vec!["+7 495 000-00-02".into()],
            org: Some("НИЦ КИ, УИТ".into()),
            title: Some("Инженер".into()),
            note: None,
        };
        let card = build_vcard("c1", &input).unwrap();
        assert!(card.contains("N:Сидоров;Пётр;Ильич;;") && card.contains("ORG:НИЦ КИ;УИТ"));
        let parsed = parse_vcards(&card, "b", &Origin::default());
        assert_eq!(parsed[0].full_name, "Сидоров Пётр Ильич");
        assert_eq!(parsed[0].org.as_deref(), Some("НИЦ КИ, УИТ"));
        assert_eq!(parsed[0].emails, vec!["sidorov@inst.test"]);

        let with_photo = card.replace("END:VCARD", "PHOTO;ENCODING=b;TYPE=JPEG:AAAA\r\nEND:VCARD");
        let upd = update_vcard(&with_photo, &ContactInput { phones: vec![], ..input.clone() }).unwrap();
        assert!(upd.contains("PHOTO;ENCODING=b") && !upd.contains("TEL") && upd.contains("UID:c1"));
        assert!(build_vcard("x", &ContactInput { emails: vec!["bad".into()], ..input }).is_err());
    }

    #[test]
    fn folds_long_utf8_lines() {
        let long = format!("DESCRIPTION:{}", "я".repeat(100));
        let folded = fold(&long);
        assert!(folded.split("\r\n").all(|l| l.len() <= 75));
        assert_eq!(unfold(&folded), vec![long]);
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

