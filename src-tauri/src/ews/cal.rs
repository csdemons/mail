//! Календарь и контакты Exchange: коллекции как CalDAV, события, контакты,
//! глобальная адресная книга (GAL). Хранится в `ews:cal:<flags>:<ItemId>` и
//! `ews:contacts:<flags>:<ItemId>`.

use base64::Engine as _;
use chrono::{DateTime, Utc};

use crate::error::{AppError, Result};

use super::*;

// ---------------------------------------------------------------- календарь и контакты

/// Windows-имя часового пояса компьютера (для событий «на весь день»: Exchange
/// хранит их в поясе запроса). По смещению — пояса России, иначе UTC.
fn windows_tz() -> &'static str {
    match chrono::Local::now().offset().local_minus_utc() / 3600 {
        2 => "Kaliningrad Standard Time",
        3 => "Russian Standard Time",
        4 => "Russia Time Zone 3",
        5 => "Ekaterinburg Standard Time",
        6 => "Omsk Standard Time",
        7 => "North Asia Standard Time",
        8 => "North Asia East Standard Time",
        9 => "Yakutsk Standard Time",
        10 => "Vladivostok Standard Time",
        11 => "Magadan Standard Time",
        12 => "Russia Time Zone 11",
        _ => "UTC",
    }
}

/// Адрес коллекции календаря/контактов Exchange в общих типах программы.
pub const CONTACTS_URL: &str = "ews:contacts";
pub const GAL_URL: &str = "ews:gal";

fn ews_href(kind: &str, flags: &str, id: &str) -> String {
    format!("ews:{kind}:{flags}:{id}")
}

/// `ews:cal:<флаги>:<ItemId>` → (флаги, ItemId).
pub fn parse_href<'a>(href: &'a str, kind: &str) -> Option<(&'a str, &'a str)> {
    let rest = href.strip_prefix("ews:")?.strip_prefix(kind)?.strip_prefix(':')?;
    rest.split_once(':')
}

fn utc(s: &str) -> String {
    s.to_string()
}

/// Значение поля пустое (`<t:X></t:X>` или `<t:Body …></t:Body>`).
fn empty_field(xml: &str) -> bool {
    xml.find('>').is_some_and(|i| xml[i + 1..].starts_with("</"))
}

const CAL_PROPS: &str = r#"<t:FieldURI FieldURI="item:Subject"/><t:FieldURI FieldURI="calendar:Start"/><t:FieldURI FieldURI="calendar:End"/><t:FieldURI FieldURI="calendar:IsAllDayEvent"/><t:FieldURI FieldURI="calendar:Location"/><t:FieldURI FieldURI="calendar:Organizer"/><t:FieldURI FieldURI="calendar:IsRecurring"/><t:FieldURI FieldURI="calendar:CalendarItemType"/><t:FieldURI FieldURI="calendar:UID"/><t:FieldURI FieldURI="calendar:MyResponseType"/><t:FieldURI FieldURI="calendar:IsMeeting"/>"#;

const CONTACT_PROPS: &str = r#"<t:FieldURI FieldURI="contacts:DisplayName"/><t:FieldURI FieldURI="contacts:EmailAddresses"/><t:FieldURI FieldURI="contacts:PhoneNumbers"/><t:FieldURI FieldURI="contacts:CompanyName"/><t:FieldURI FieldURI="contacts:JobTitle"/><t:FieldURI FieldURI="contacts:Department"/><t:FieldURI FieldURI="contacts:PhysicalAddresses"/>"#;

const PHONE_KEYS: [&str; 4] = ["BusinessPhone", "MobilePhone", "HomePhone", "OtherTelephone"];

fn local_date(t: &str) -> Option<chrono::NaiveDate> {
    parse_time(t).map(|d| d.with_timezone(&chrono::Local).date_naive())
}

fn entries(n: roxmltree::Node<'_, '_>, name: &str) -> Vec<(String, String)> {
    child(n, name)
        .map(|c| c.children().filter(|e| e.is_element() && e.tag_name().name() == "Entry").filter_map(|e| Some((e.attribute("Key").unwrap_or_default().to_string(), e.text()?.trim().to_string()))).filter(|(_, v)| !v.is_empty()).collect())
        .unwrap_or_default()
}

fn address_of(n: roxmltree::Node<'_, '_>) -> Option<String> {
    let addrs = child(n, "PhysicalAddresses")?;
    let e = addrs.children().find(|e| e.is_element() && e.tag_name().name() == "Entry")?;
    let parts: Vec<String> = ["Street", "City", "State", "PostalCode", "CountryOrRegion"].iter().filter_map(|k| text_of(e, k)).filter(|v| !v.is_empty()).collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn contact_of(n: roxmltree::Node<'_, '_>, id: &str, change_key: Option<String>, body: Option<String>) -> crate::dav::Contact {
    let emails: Vec<String> = entries(n, "EmailAddresses").into_iter().map(|(_, v)| v.trim_start_matches("SMTP:").trim_start_matches("smtp:").to_string()).filter(|v| v.contains('@')).collect();
    let phones: Vec<String> = entries(n, "PhoneNumbers").into_iter().map(|(_, v)| v).collect();
    let name = text_of(n, "DisplayName").filter(|s| !s.is_empty()).or_else(|| emails.first().cloned()).unwrap_or_else(|| "(без имени)".into());
    let href = ews_href("c", "-", id);
    crate::dav::Contact {
        id: href.clone(),
        book: "Контакты".into(),
        href,
        etag: change_key,
        editable: true,
        full_name: name,
        emails,
        phones,
        org: text_of(n, "CompanyName").filter(|s| !s.is_empty()),
        title: text_of(n, "JobTitle").filter(|s| !s.is_empty()),
        note: body.filter(|b| !b.trim().is_empty()),
        address: address_of(n),
    }
}

impl EwsSession {
    /// Календари ящика (основной и вложенные), как коллекции CalDAV.
    pub async fn calendars(&mut self) -> Result<Vec<crate::dav::Collection>> {
        let op = r#"<m:FindFolder Traversal="Deep"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI="folder:DisplayName"/><t:FieldURI FieldURI="folder:FolderClass"/><t:ExtendedFieldURI PropertyTag="0x10F4" PropertyType="Boolean"/></t:AdditionalProperties></m:FolderShape><m:IndexedPageFolderView MaxEntriesReturned="500" Offset="0" BasePoint="Beginning"/><m:ParentFolderIds><t:DistinguishedFolderId Id="msgfolderroot"/></m:ParentFolderIds></m:FindFolder>"#;
        let text = self.call(op, crate::netio::API_LIMIT).await?;
        let d = doc(&text)?;
        if let Some(e) = first_error(&responses(&d)?) {
            return Err(e);
        }
        let mut out: Vec<crate::dav::Collection> = d
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name().ends_with("Folder") && child(*n, "FolderId").is_some())
            .filter(|f| text_of(*f, "FolderClass").is_some_and(|c| c == "IPF.Appointment" || c.starts_with("IPF.Appointment.")))
            .filter(|f| !descendants(*f, "ExtendedProperty").any(|e| text_of(e, "Value").as_deref() == Some("true")))
            .filter_map(|f| {
                let id = child(f, "FolderId")?.attribute("Id")?;
                Some(crate::dav::Collection { url: format!("ews:cal:{id}"), name: text_of(f, "DisplayName").unwrap_or_else(|| "Календарь".into()), color: None, writable: true })
            })
            .collect();
        // Основной календарь — первым.
        let main = r#"<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds><t:DistinguishedFolderId Id="calendar"/></m:FolderIds></m:GetFolder>"#;
        if let Ok(text) = self.call(main, crate::netio::SMALL_LIMIT).await {
            if let Ok(d) = doc(&text) {
                if let Some(id) = descendants(d.root(), "FolderId").next().and_then(|f| f.attribute("Id")) {
                    let url = format!("ews:cal:{id}");
                    out.sort_by_key(|c| c.url != url);
                }
            }
        }
        Ok(out)
    }

    /// События календаря за период (повторения развёрнуты сервером).
    pub async fn events(&mut self, cal: &crate::dav::Collection, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<crate::dav::CalEvent>> {
        let fid = cal.url.strip_prefix("ews:cal:").ok_or(AppError::NotFound)?.to_string();
        let mut out = Vec::new();
        // CalendarView — не больше двух лет за запрос.
        let mut from = start;
        while from < end {
            let to = (from + chrono::Duration::days(366)).min(end);
            let op = format!(
                r#"<m:FindItem Traversal="Shallow"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{CAL_PROPS}</t:AdditionalProperties></m:ItemShape><m:CalendarView MaxEntriesReturned="1000" StartDate="{}" EndDate="{}"/><m:ParentFolderIds><t:FolderId Id="{}"/></m:ParentFolderIds></m:FindItem>"#,
                from.format("%Y-%m-%dT%H:%M:%SZ"),
                to.format("%Y-%m-%dT%H:%M:%SZ"),
                xml_escape(&fid)
            );
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            if let Some(e) = first_error(&responses(&d)?) {
                return Err(e);
            }
            let items: Vec<_> = d.descendants().filter(|n| n.is_element() && n.tag_name().name() == "CalendarItem").collect();
            let mut ids = Vec::new();
            for n in &items {
                let Some(idn) = child(*n, "ItemId") else { continue };
                let id = idn.attribute("Id").unwrap_or_default().to_string();
                let ck = idn.attribute("ChangeKey").map(str::to_string);
                let all_day = text_of(*n, "IsAllDayEvent").as_deref() == Some("true");
                let (s, e) = (text_of(*n, "Start").unwrap_or_default(), text_of(*n, "End").unwrap_or_default());
                let (start_s, end_s) = if all_day {
                    let sd = local_date(&s).unwrap_or_default();
                    let ed = local_date(&e).filter(|x| *x > sd).unwrap_or(sd + chrono::Duration::days(1));
                    (sd.to_string(), ed.to_string())
                } else {
                    (parse_time(&s).map(|t| t.to_rfc3339()).unwrap_or(utc(&s)), parse_time(&e).map(|t| t.to_rfc3339()).unwrap_or(utc(&e)))
                };
                let kind = text_of(*n, "CalendarItemType").unwrap_or_default();
                let recurring = kind == "Occurrence" || kind == "Exception" || text_of(*n, "IsRecurring").as_deref() == Some("true");
                let organizer_meeting = text_of(*n, "MyResponseType").as_deref() == Some("Organizer") && text_of(*n, "IsMeeting").as_deref() == Some("true");
                let flags = format!("{}{}", if recurring && kind != "RecurringMaster" { "r" } else { "" }, if organizer_meeting { "m" } else { "" });
                let organizer = child(*n, "Organizer").and_then(|o| child(o, "Mailbox")).map(mailbox);
                ids.push(id.clone());
                out.push(crate::dav::CalEvent {
                    id: format!("{id}@{start_s}"),
                    uid: text_of(*n, "UID").unwrap_or_else(|| id.clone()),
                    href: ews_href("cal", if flags.is_empty() { "-" } else { &flags }, &id),
                    etag: ck,
                    editable: true,
                    calendar_url: cal.url.clone(),
                    calendar: cal.name.clone(),
                    color: cal.color.clone(),
                    title: text_of(*n, "Subject").filter(|s| !s.is_empty()).unwrap_or_else(|| "(без названия)".into()),
                    location: text_of(*n, "Location").filter(|s| !s.is_empty()),
                    description: None,
                    organizer: organizer.as_ref().map(|(n, a)| if n.is_empty() { a.clone() } else { n.clone() }),
                    participants: organizer.map(|(_, a)| a.to_lowercase()).filter(|a| a.contains('@')).into_iter().collect(),
                    start: start_s,
                    end: end_s,
                    all_day,
                    recurring,
                });
            }
            // Описание и участники — отдельным запросом (CalendarView их не отдаёт).
            let details = self.event_details(&ids).await.unwrap_or_default();
            for ev in out.iter_mut() {
                if let Some((_, id)) = parse_href(&ev.href, "cal") {
                    if let Some((desc, people)) = details.get(id) {
                        if ev.description.is_none() {
                            ev.description = desc.clone();
                        }
                        for p in people {
                            if !ev.participants.contains(p) {
                                ev.participants.push(p.clone());
                            }
                        }
                    }
                }
            }
            from = to;
        }
        Ok(out)
    }

    async fn event_details(&mut self, ids: &[String]) -> Result<HashMap<String, (Option<String>, Vec<String>)>> {
        let mut out = HashMap::new();
        for chunk in ids.chunks(BATCH) {
            let list: String = chunk.iter().map(|id| format!(r#"<t:ItemId Id="{}"/>"#, xml_escape(id))).collect();
            let op = format!(r#"<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:BodyType>Text</t:BodyType><t:AdditionalProperties><t:FieldURI FieldURI="item:Body"/><t:FieldURI FieldURI="calendar:RequiredAttendees"/><t:FieldURI FieldURI="calendar:OptionalAttendees"/></t:AdditionalProperties></m:ItemShape><m:ItemIds>{list}</m:ItemIds></m:GetItem>"#);
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            for (r, id) in responses(&d)?.into_iter().zip(chunk) {
                if response_error(r).is_some() {
                    continue;
                }
                let Some(n) = descendants(r, "CalendarItem").next() else { continue };
                let body = text_of(n, "Body").filter(|b| !b.trim().is_empty());
                let people: Vec<String> = ["RequiredAttendees", "OptionalAttendees"]
                    .iter()
                    .filter_map(|k| child(n, k))
                    .flat_map(|a| descendants(a, "Mailbox").map(mailbox).collect::<Vec<_>>())
                    .map(|(_, addr)| addr.to_lowercase())
                    .filter(|a| a.contains('@'))
                    .collect();
                out.insert(id.clone(), (body, people));
            }
        }
        Ok(out)
    }

    fn event_fields(input: &crate::dav::EventInput, with_time: bool) -> Result<(String, Vec<(&'static str, String)>)> {
        let title = input.title.trim();
        if title.is_empty() {
            return Err(AppError::Other("Укажите название события".into()));
        }
        let mut fields: Vec<(&'static str, String)> = vec![("item:Subject", format!("<t:Subject>{}</t:Subject>", xml_escape(title)))];
        fields.push(("item:Body", format!(r#"<t:Body BodyType="Text">{}</t:Body>"#, xml_escape(input.description.as_deref().unwrap_or_default()))));
        let mut tz = String::new();
        if with_time {
            let (start, end) = if input.all_day {
                let p = |v: &str| chrono::NaiveDate::parse_from_str(&v[..10.min(v.len())], "%Y-%m-%d").map_err(|_| AppError::Other(format!("Некорректная дата: {v}")));
                let s = p(&input.start)?;
                let e = p(&input.end)?.max(s + chrono::Duration::days(1));
                tz = windows_tz().to_string();
                (format!("{s}T00:00:00"), format!("{e}T00:00:00"))
            } else {
                let p = |v: &str| DateTime::parse_from_rfc3339(v).map(|d| d.with_timezone(&Utc)).map_err(|_| AppError::Other(format!("Некорректное время: {v}")));
                let (s, e) = (p(&input.start)?, p(&input.end)?);
                if e < s {
                    return Err(AppError::Other("Окончание раньше начала".into()));
                }
                (s.format("%Y-%m-%dT%H:%M:%SZ").to_string(), e.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            };
            fields.push(("calendar:Start", format!("<t:Start>{start}</t:Start>")));
            fields.push(("calendar:End", format!("<t:End>{end}</t:End>")));
            fields.push(("calendar:IsAllDayEvent", format!("<t:IsAllDayEvent>{}</t:IsAllDayEvent>", input.all_day)));
        }
        fields.push(("calendar:Location", format!("<t:Location>{}</t:Location>", xml_escape(input.location.as_deref().unwrap_or_default().trim()))));
        Ok((tz, fields))
    }

    /// Запрос с поясом компьютера (события «на весь день» задаются местным временем).
    async fn call_tz(&mut self, op: &str, tz: &str) -> Result<String> {
        if tz.is_empty() {
            return self.call(op, crate::netio::API_LIMIT).await;
        }
        let body = envelope(self.version, op).replacen("</soap:Header>", &format!(r#"<t:TimeZoneContext><t:TimeZoneDefinition Id="{tz}"/></t:TimeZoneContext></soap:Header>"#), 1);
        self.post_raw(&body, crate::netio::API_LIMIT).await
    }

    pub async fn create_event(&mut self, cal: &crate::dav::Collection, input: &crate::dav::EventInput) -> Result<()> {
        let fid = cal.url.strip_prefix("ews:cal:").ok_or(AppError::NotFound)?.to_string();
        let (tz, fields) = Self::event_fields(input, true)?;
        let xml: String = fields.iter().map(|(_, x)| x.as_str()).collect();
        let op = format!(r#"<m:CreateItem SendMeetingInvitations="SendToNone"><m:SavedItemFolderId><t:FolderId Id="{}"/></m:SavedItemFolderId><m:Items><t:CalendarItem>{xml}</t:CalendarItem></m:Items></m:CreateItem>"#, xml_escape(&fid));
        let text = self.call_tz(&op, &tz).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(()), Err)
    }

    /// Ссылка на событие в запросе: повторение — через всю серию.
    fn event_ref(flags: &str, id: &str) -> String {
        if flags.contains('r') {
            format!(r#"<t:RecurringMasterItemId OccurrenceId="{}"/>"#, xml_escape(id))
        } else {
            format!(r#"<t:ItemId Id="{}"/>"#, xml_escape(id))
        }
    }

    pub async fn update_event(&mut self, href: &str, input: &crate::dav::EventInput, change_time: bool) -> Result<()> {
        let (flags, id) = parse_href(href, "cal").ok_or(AppError::NotFound)?;
        let (tz, fields) = Self::event_fields(input, change_time)?;
        let updates: String = fields
            .iter()
            .map(|(uri, x)| {
                if (*uri == "calendar:Location" || *uri == "item:Body") && empty_field(x) {
                    format!(r#"<t:DeleteItemField><t:FieldURI FieldURI="{uri}"/></t:DeleteItemField>"#)
                } else {
                    format!(r#"<t:SetItemField><t:FieldURI FieldURI="{uri}"/><t:CalendarItem>{x}</t:CalendarItem></t:SetItemField>"#)
                }
            })
            .collect();
        let op = format!(
            r#"<m:UpdateItem ConflictResolution="AlwaysOverwrite" SendMeetingInvitationsOrCancellations="{}"><m:ItemChanges><t:ItemChange>{}<t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>"#,
            if flags.contains('m') { "SendToChangedAndSaveCopy" } else { "SendToNone" },
            Self::event_ref(flags, id)
        );
        let text = self.call_tz(&op, &tz).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(()), Err)
    }

    pub async fn delete_event(&mut self, href: &str) -> Result<()> {
        let (flags, id) = parse_href(href, "cal").ok_or(AppError::NotFound)?;
        // Своё совещание — участники получают отмену; чужое — просто убирается из календаря.
        let op = format!(
            r#"<m:DeleteItem DeleteType="MoveToDeletedItems" SendMeetingCancellations="{}"><m:ItemIds>{}</m:ItemIds></m:DeleteItem>"#,
            if flags.contains('m') { "SendToAllAndSaveCopy" } else { "SendToNone" },
            Self::event_ref(flags, id)
        );
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(()), Err)
    }

    /// Событие из файла календаря (.ics): то же событие (UID) в календаре заменяется.
    pub async fn import_ics(&mut self, cal: &crate::dav::Collection, ev: &crate::dav::IcsEvent) -> Result<bool> {
        let fid = cal.url.strip_prefix("ews:cal:").ok_or(AppError::NotFound)?.to_string();
        let start = parse_time(&ev.start).or_else(|| chrono::NaiveDate::parse_from_str(&ev.start, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)).map(|t| Utc.from_utc_datetime(&t))).unwrap_or_else(Utc::now);
        // Ошибка поиска не глотается: иначе событие добавилось бы второй раз вместо замены.
        let existing = self.events(cal, start - chrono::Duration::days(2), start + chrono::Duration::days(2)).await?;
        let mut replaced = false;
        if let Some(old) = existing.iter().find(|e| e.uid == ev.uid) {
            self.delete_event(&old.href).await?;
            replaced = true;
        }
        let op = format!(
            r#"<m:CreateItem SendMeetingInvitations="SendToNone"><m:SavedItemFolderId><t:FolderId Id="{}"/></m:SavedItemFolderId><m:Items><t:CalendarItem><t:MimeContent CharacterSet="UTF-8">{}</t:MimeContent></t:CalendarItem></m:Items></m:CreateItem>"#,
            xml_escape(&fid),
            base64::engine::general_purpose::STANDARD.encode(ev.ics.as_bytes())
        );
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(replaced), Err)
    }

    // ------------------------------------------------------------ контакты

    /// Адресные книги: «Контакты» ящика и общая адресная книга организации.
    pub fn books() -> Vec<crate::dav::Collection> {
        vec![
            crate::dav::Collection { url: CONTACTS_URL.into(), name: "Контакты".into(), color: None, writable: true },
            crate::dav::Collection { url: GAL_URL.into(), name: "Общая адресная книга".into(), color: None, writable: false },
        ]
    }

    async fn contact_items(&mut self, ids: &[(String, Option<String>)], with_body: bool) -> Result<Vec<crate::dav::Contact>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(BATCH) {
            let list: String = chunk.iter().map(|(id, _)| format!(r#"<t:ItemId Id="{}"/>"#, xml_escape(id))).collect();
            let body = if with_body { r#"<t:FieldURI FieldURI="item:Body"/>"# } else { "" };
            let op = format!(r#"<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:BodyType>Text</t:BodyType><t:AdditionalProperties>{CONTACT_PROPS}{body}</t:AdditionalProperties></m:ItemShape><m:ItemIds>{list}</m:ItemIds></m:GetItem>"#);
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            for r in responses(&d)? {
                if response_error(r).is_some() {
                    continue;
                }
                let Some(n) = descendants(r, "Contact").next() else { continue };
                let Some(idn) = child(n, "ItemId") else { continue };
                let id = idn.attribute("Id").unwrap_or_default();
                out.push(contact_of(n, id, idn.attribute("ChangeKey").map(str::to_string), text_of(n, "Body")));
            }
        }
        Ok(out)
    }

    /// Контакты папки «Контакты».
    pub async fn contacts(&mut self) -> Result<Vec<crate::dav::Contact>> {
        let mut ids = Vec::new();
        let mut offset = 0usize;
        loop {
            let op = format!(r#"<m:FindItem Traversal="Shallow"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI="item:ItemClass"/></t:AdditionalProperties></m:ItemShape><m:IndexedPageItemView MaxEntriesReturned="500" Offset="{offset}" BasePoint="Beginning"/><m:ParentFolderIds><t:DistinguishedFolderId Id="contacts"/></m:ParentFolderIds></m:FindItem>"#);
            let text = self.call(&op, crate::netio::API_LIMIT).await?;
            let d = doc(&text)?;
            if let Some(e) = first_error(&responses(&d)?) {
                return Err(e);
            }
            let found: Vec<(String, Option<String>)> = d
                .descendants()
                .filter(|n| n.is_element() && n.tag_name().name() == "Contact")
                .filter_map(|n| child(n, "ItemId").map(|i| (i.attribute("Id").unwrap_or_default().to_string(), i.attribute("ChangeKey").map(str::to_string))))
                .collect();
            let n = d.descendants().filter(|n| n.is_element() && ITEM_TAGS.iter().chain(["Contact", "DistributionList"].iter()).any(|t| *t == n.tag_name().name()) && child(*n, "ItemId").is_some()).count();
            ids.extend(found);
            let last = descendants(d.root(), "RootFolder").next().and_then(|r| r.attribute("IncludesLastItemInRange")).is_none_or(|v| v == "true");
            offset += n;
            if last || n == 0 || offset > 20_000 {
                break;
            }
        }
        self.contact_items(&ids, false).await
    }

    pub async fn contact(&mut self, href: &str) -> Result<crate::dav::Contact> {
        if let Some(email) = href.strip_prefix("ews:gal:") {
            return self.resolve(email).await?.into_iter().find(|c| c.emails.iter().any(|e| e.eq_ignore_ascii_case(email))).ok_or(AppError::NotFound);
        }
        let (_, id) = parse_href(href, "c").ok_or(AppError::NotFound)?;
        self.contact_items(&[(id.to_string(), None)], true).await?.into_iter().next().ok_or(AppError::NotFound)
    }

    /// Поиск в общей адресной книге организации (ResolveNames).
    pub async fn resolve(&mut self, query: &str) -> Result<Vec<crate::dav::Contact>> {
        let q = query.trim();
        if q.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let op = format!(r#"<m:ResolveNames ReturnFullContactData="true" SearchScope="ActiveDirectoryContacts"><m:UnresolvedEntry>{}</m:UnresolvedEntry></m:ResolveNames>"#, xml_escape(q));
        let text = self.call(&op, crate::netio::API_LIMIT).await?;
        let d = doc(&text)?;
        let mut out = Vec::new();
        for r in responses(&d)? {
            // «Неоднозначное имя» — тоже результат (несколько совпадений).
            let code = text_of(r, "ResponseCode").unwrap_or_default();
            if code != "NoError" && code != "ErrorNameResolutionMultipleResults" {
                continue;
            }
            for res in descendants(r, "Resolution") {
                let (name, addr) = child(res, "Mailbox").map(mailbox).unwrap_or_default();
                if addr.is_empty() {
                    continue;
                }
                let c = child(res, "Contact");
                let href = format!("ews:gal:{addr}");
                out.push(crate::dav::Contact {
                    id: href.clone(),
                    book: "Общая адресная книга".into(),
                    href,
                    etag: None,
                    editable: false,
                    full_name: c.and_then(|c| text_of(c, "DisplayName")).filter(|s| !s.is_empty()).unwrap_or(if name.is_empty() { addr.clone() } else { name }),
                    emails: vec![addr],
                    phones: c.map(|c| entries(c, "PhoneNumbers").into_iter().map(|(_, v)| v).collect()).unwrap_or_default(),
                    org: c.and_then(|c| text_of(c, "Department").or_else(|| text_of(c, "CompanyName"))).filter(|s| !s.is_empty()),
                    title: c.and_then(|c| text_of(c, "JobTitle")).filter(|s| !s.is_empty()),
                    note: None,
                    address: c.and_then(address_of),
                });
            }
        }
        Ok(out)
    }

    fn contact_fields(input: &crate::dav::ContactInput) -> Result<Vec<(String, String)>> {
        let name = input.full_name.trim();
        if name.is_empty() {
            return Err(AppError::Other("Укажите имя контакта".into()));
        }
        let emails: Vec<&str> = input.emails.iter().map(|e| e.trim()).filter(|e| !e.is_empty()).collect();
        if let Some(bad) = emails.iter().find(|e| !e.contains('@')) {
            return Err(AppError::Other(format!("Некорректный e-mail: {bad}")));
        }
        let parts: Vec<&str> = name.split_whitespace().collect();
        let mut f: Vec<(String, String)> = vec![
            ("item:Body".into(), format!(r#"<t:Body BodyType="Text">{}</t:Body>"#, xml_escape(input.note.as_deref().unwrap_or_default()))),
            ("contacts:FileAs".into(), format!("<t:FileAs>{}</t:FileAs>", xml_escape(name))),
            ("contacts:DisplayName".into(), format!("<t:DisplayName>{}</t:DisplayName>", xml_escape(name))),
            ("contacts:GivenName".into(), format!("<t:GivenName>{}</t:GivenName>", xml_escape(parts.get(1).copied().unwrap_or_default()))),
            ("contacts:CompanyName".into(), format!("<t:CompanyName>{}</t:CompanyName>", xml_escape(input.org.as_deref().unwrap_or_default().trim()))),
        ];
        for (i, e) in emails.iter().take(3).enumerate() {
            f.push((format!("email:EmailAddress{}", i + 1), xml_escape(e)));
        }
        for (i, p) in input.phones.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).take(PHONE_KEYS.len()).enumerate() {
            f.push((format!("phone:{}", PHONE_KEYS[i]), xml_escape(p)));
        }
        f.push(("contacts:JobTitle".into(), format!("<t:JobTitle>{}</t:JobTitle>", xml_escape(input.title.as_deref().unwrap_or_default().trim()))));
        f.push(("contacts:Surname".into(), format!("<t:Surname>{}</t:Surname>", xml_escape(parts.first().copied().unwrap_or_default()))));
        Ok(f)
    }

    pub async fn create_contact(&mut self, input: &crate::dav::ContactInput) -> Result<()> {
        let f = Self::contact_fields(input)?;
        let simple = |k: &str| f.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).filter(|v| !empty_field(v)).unwrap_or_default();
        let emails: String = f.iter().filter_map(|(k, v)| k.strip_prefix("email:").map(|key| format!(r#"<t:Entry Key="{key}">{v}</t:Entry>"#))).collect();
        let phones: String = f.iter().filter_map(|(k, v)| k.strip_prefix("phone:").map(|key| format!(r#"<t:Entry Key="{key}">{v}</t:Entry>"#))).collect();
        let xml = format!(
            "{}{}{}{}{}{}{}{}{}",
            simple("item:Body"),
            simple("contacts:FileAs"),
            simple("contacts:DisplayName"),
            simple("contacts:GivenName"),
            simple("contacts:CompanyName"),
            if emails.is_empty() { String::new() } else { format!("<t:EmailAddresses>{emails}</t:EmailAddresses>") },
            if phones.is_empty() { String::new() } else { format!("<t:PhoneNumbers>{phones}</t:PhoneNumbers>") },
            simple("contacts:JobTitle"),
            simple("contacts:Surname"),
        );
        let op = format!(r#"<m:CreateItem><m:SavedItemFolderId><t:DistinguishedFolderId Id="contacts"/></m:SavedItemFolderId><m:Items><t:Contact>{xml}</t:Contact></m:Items></m:CreateItem>"#);
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(()), Err)
    }

    pub async fn update_contact(&mut self, href: &str, input: &crate::dav::ContactInput) -> Result<()> {
        let (_, id) = parse_href(href, "c").ok_or(AppError::NotFound)?;
        let f = Self::contact_fields(input)?;
        let mut updates = String::new();
        for (k, v) in &f {
            if k.starts_with("email:") || k.starts_with("phone:") {
                continue;
            }
            if empty_field(v) {
                updates.push_str(&format!(r#"<t:DeleteItemField><t:FieldURI FieldURI="{k}"/></t:DeleteItemField>"#));
            } else {
                updates.push_str(&format!(r#"<t:SetItemField><t:FieldURI FieldURI="{k}"/><t:Contact>{v}</t:Contact></t:SetItemField>"#));
            }
        }
        for i in 1..=3 {
            let key = format!("EmailAddress{i}");
            match f.iter().find(|(k, _)| *k == format!("email:{key}")) {
                Some((_, v)) => updates.push_str(&format!(r#"<t:SetItemField><t:IndexedFieldURI FieldURI="contacts:EmailAddress" FieldIndex="{key}"/><t:Contact><t:EmailAddresses><t:Entry Key="{key}">{v}</t:Entry></t:EmailAddresses></t:Contact></t:SetItemField>"#)),
                None => updates.push_str(&format!(r#"<t:DeleteItemField><t:IndexedFieldURI FieldURI="contacts:EmailAddress" FieldIndex="{key}"/></t:DeleteItemField>"#)),
            }
        }
        for key in PHONE_KEYS {
            match f.iter().find(|(k, _)| *k == format!("phone:{key}")) {
                Some((_, v)) => updates.push_str(&format!(r#"<t:SetItemField><t:IndexedFieldURI FieldURI="contacts:PhoneNumber" FieldIndex="{key}"/><t:Contact><t:PhoneNumbers><t:Entry Key="{key}">{v}</t:Entry></t:PhoneNumbers></t:Contact></t:SetItemField>"#)),
                None => updates.push_str(&format!(r#"<t:DeleteItemField><t:IndexedFieldURI FieldURI="contacts:PhoneNumber" FieldIndex="{key}"/></t:DeleteItemField>"#)),
            }
        }
        let op = format!(r#"<m:UpdateItem ConflictResolution="AlwaysOverwrite"><m:ItemChanges><t:ItemChange><t:ItemId Id="{}"/><t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>"#, xml_escape(id));
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        // Удаление отсутствующего поля — не ошибка.
        responses(&d)?.into_iter().find_map(|r| response_error(r).filter(|_| text_of(r, "ResponseCode").as_deref() != Some("ErrorInvalidPropertyDelete"))).map_or(Ok(()), Err)
    }

    pub async fn delete_contact(&mut self, href: &str) -> Result<()> {
        let (_, id) = parse_href(href, "c").ok_or(AppError::NotFound)?;
        let op = format!(r#"<m:DeleteItem DeleteType="MoveToDeletedItems"><m:ItemIds><t:ItemId Id="{}"/></m:ItemIds></m:DeleteItem>"#, xml_escape(id));
        let text = self.call(&op, crate::netio::SMALL_LIMIT).await?;
        let d = doc(&text)?;
        first_error(&responses(&d)?).map_or(Ok(()), Err)
    }
}
