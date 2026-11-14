//! Календарь и контакты (CalDAV/CardDAV, SOGo, Exchange), приглашения .ics.

use super::*;

/// Календари Exchange (один раз за сеанс, как у CalDAV).
pub(crate) async fn ews_calendars(ctx: &AccountCtx) -> Result<Vec<dav::Collection>> {
    if let Some(c) = ctx.calendars.lock().await.clone() {
        return Ok(c);
    }
    let found = with_ctx!(ctx, c => async { ews_of(c)?.calendars().await })?;
    *ctx.calendars.lock().await = Some(found.clone());
    Ok(found)
}

pub(crate) async fn dav_client(ctx: &AccountCtx) -> Result<(dav::DavClient, url::Url)> {
    let creds = ctx.creds.clone();
    let profile = ctx.profile().await;
    if profile.is_exchange() && profile.dav_url.trim().is_empty() {
        return Err(AppError::Other("Для учётной записи Exchange это недоступно".into()));
    }
    let base = url::Url::parse(&profile.dav_base())
        .map_err(|e| AppError::Other(format!("Некорректный адрес сервера календаря: {e}")))?;
    Ok((dav::DavClient::new(&creds.username, &creds.password, &base)?, base))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsResult {
    pub(crate) books: Vec<dav::Collection>,
    pub(crate) contacts: Vec<dav::Contact>,
    pub(crate) errors: Vec<String>,
}

#[tauri::command]
pub async fn list_contacts(state: State<'_, AppState>) -> Result<ContactsResult> {
    let ctx = state.ctx().await?;
    if exchange(&ctx) {
        let books = crate::ews::EwsSession::books();
        *ctx.books.lock().await = Some(books.clone());
        let mut contacts = with_ctx!(ctx, c => async { ews_of(c)?.contacts().await })?;
        contacts.sort_by_key(|c| c.full_name.to_lowercase());
        return Ok(ContactsResult { books, contacts, errors: Vec::new() });
    }
    let (mut client, base) = dav_client(&ctx).await?;
    // Кэш читаем отдельным оператором: guard в выражении `match` жил бы до
    // конца match, и повторный lock ниже заблокировал бы команду навсегда.
    let cached = ctx.books.lock().await.clone();
    let books = match cached {
        Some(b) => b,
        None => {
            let found = client.discover(&base, dav::Kind::Contacts).await?;
            *ctx.books.lock().await = Some(found.clone());
            found
        }
    };
    let mut contacts = Vec::new();
    let mut errors = Vec::new();
    for book in &books {
        match client.contacts(book).await {
            Ok(list) => contacts.extend(list),
            Err(e) => errors.push(format!("{}: {e}", book.name)),
        }
    }
    contacts.sort_by_key(|c| c.full_name.to_lowercase());
    Ok(ContactsResult { books, contacts, errors })
}

/// Поиск контактов на сервере во всех адресных книгах (в т.ч. корпоративной).
/// Сначала CardDAV; если корпоративная (только для чтения) книга ничего не
/// вернула, а сервер — SOGo, дополнительно ищем через веб-API SOGo.
#[tauri::command]
pub async fn search_contacts(state: State<'_, AppState>, query: String) -> Result<Vec<dav::Contact>> {
    let ctx = state.ctx().await?;
    find_contacts(&ctx, &query).await
}

pub(crate) async fn find_contacts(ctx: &AccountCtx, query: &str) -> Result<Vec<dav::Contact>> {
    let query = query.trim().to_string();
    if exchange(ctx) {
        if query.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let q = query.to_lowercase();
        let (mine, gal) = with_ctx!(ctx, c => async {
            let e = ews_of(c)?;
            let mine = e.contacts().await.unwrap_or_default();
            let gal = e.resolve(&query).await.unwrap_or_default();
            Ok((mine, gal))
        })?;
        let mut out: Vec<dav::Contact> = mine
            .into_iter()
            .filter(|c| c.full_name.to_lowercase().contains(&q) || c.emails.iter().any(|e| e.to_lowercase().contains(&q)) || c.phones.iter().any(|p| p.contains(&q)) || c.org.as_deref().is_some_and(|o| o.to_lowercase().contains(&q)))
            .collect();
        for g in gal {
            if !out.iter().any(|o| o.emails.iter().any(|e| g.emails.iter().any(|x| x.eq_ignore_ascii_case(e)))) {
                out.push(g);
            }
        }
        out.truncate(50);
        return Ok(out);
    }
    if query.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let (mut client, base) = dav_client(ctx).await?;
    let cached = ctx.books.lock().await.clone();
    let books = match cached {
        Some(b) => b,
        None => {
            let found = client.discover(&base, dav::Kind::Contacts).await?;
            *ctx.books.lock().await = Some(found.clone());
            found
        }
    };
    let mut out: Vec<dav::Contact> = Vec::new();
    let mut last_error = None;
    let mut gal_empty = books.iter().all(|b| b.writable);
    for book in &books {
        match client.search_contacts(book, &query, 25).await {
            Ok(list) => {
                if !book.writable && list.is_empty() {
                    gal_empty = true;
                }
                for c in list {
                    if !out.iter().any(|o| o.href == c.href) {
                        out.push(c);
                    }
                }
            }
            Err(e) => {
                log::warn!("поиск в «{}»: {e}", book.name);
                gal_empty = gal_empty || !book.writable;
                last_error = Some(e);
            }
        }
    }
    let is_sogo = books.iter().any(|b| b.url.contains("/SOGo/"));
    if gal_empty && is_sogo {
        match sogo_search(ctx, &base, &query).await {
            Ok(list) => {
                for mut c in list {
                    let dup = out.iter().any(|o| o.emails.iter().any(|e| c.emails.iter().any(|x| x.eq_ignore_ascii_case(e))));
                    if dup {
                        continue;
                    }
                    // «sogo:<книга>:<id>» → адрес vCard в найденной книге (gal/<id>.vcf),
                    // чтобы подробности загрузить по CardDAV.
                    if let Some((source, id)) = c.id.strip_prefix("sogo:").and_then(|r| r.split_once(':')) {
                        if let Some(b) = books.iter().find(|b| b.url.trim_end_matches('/').rsplit('/').next() == Some(source)) {
                            let file = if id.ends_with(".vcf") { id.to_string() } else { format!("{id}.vcf") };
                            if let Ok(u) = url::Url::parse(&b.url).and_then(|u| u.join(&urlencoding_segment(&file))) {
                                c.href = u.to_string();
                            }
                            c.book = b.name.clone();
                        }
                    }
                    out.push(c);
                }
            }
            Err(e) => log::warn!("поиск через веб-API SOGo: {e}"),
        }
    }
    if out.is_empty() {
        if let Some(e) = last_error {
            return Err(e);
        }
    }
    Ok(out)
}

/// Кодирует имя файла для пути URL (пробелы, «@», кириллица).
pub(crate) fn urlencoding_segment(name: &str) -> String {
    let mut u = url::Url::parse("https://x/").expect("static");
    u.path_segments_mut().expect("base").push(name);
    u.path().trim_start_matches('/').to_string()
}

/// Корпоративная (или другая только для чтения) книга целиком — через веб-API
/// SOGo, как в веб-интерфейсе: CardDAV для LDAP-книг отдаёт пустой список.
#[tauri::command]
pub async fn browse_contacts(state: State<'_, AppState>, book_url: String) -> Result<Vec<dav::Contact>> {
    let ctx = state.ctx().await?;
    // Общая адресная книга Exchange целиком не выдаётся — только поиском.
    if exchange(&ctx) {
        let _ = book_url;
        return Ok(Vec::new());
    }
    let (_, base) = dav_client(&ctx).await?;
    let books = ctx.books.lock().await.clone().unwrap_or_default();
    let book = books.iter().find(|b| b.url == book_url).cloned().ok_or_else(|| AppError::Other("Адресная книга не найдена".into()))?;
    let source = book.url.trim_end_matches('/').rsplit('/').next().unwrap_or("gal").to_string();
    let mut log = Vec::new();
    let mut list = match sogo_session(&ctx, &base, false).await?.browse(&source, &mut log).await {
        Ok(l) => l,
        Err(_) => sogo_session(&ctx, &base, true).await?.browse(&source, &mut log).await?,
    };
    for l in &log {
        log::debug!("browse {source}: {l}");
    }
    for c in &mut list {
        c.book = book.name.clone();
    }
    list.sort_by_key(|c| c.full_name.to_lowercase());
    Ok(list)
}

/// Подробная карточка контакта: для корпоративной книги поиск отдаёт только
/// имя и почту — дозапрашиваем vCard по CardDAV, а если не вышло, карточку из
/// веб-API SOGo.
#[tauri::command]
pub async fn contact_details(state: State<'_, AppState>, href: String, id: String) -> Result<dav::Contact> {
    let ctx = state.ctx().await?;
    if exchange(&ctx) {
        let mut c = with_ctx!(ctx, c => async { ews_of(c)?.contact(&href).await })?;
        c.id = id;
        return Ok(c);
    }
    let (mut client, _) = dav_client(&ctx).await?;
    let books = ctx.books.lock().await.clone().unwrap_or_default();
    let mut found: Option<dav::Contact> = None;
    if let Ok(book) = owning_collection(&books, &href) {
        if let Ok((data, etag)) = client.get(&href).await {
            let origin = dav::Origin { href: href.clone(), etag, editable: book.writable };
            found = dav::parse_vcards(&data, &book.name, &origin).into_iter().next();
        }
    }
    let rich = |c: &dav::Contact| !c.phones.is_empty() || c.title.is_some() || c.address.is_some();
    if found.as_ref().is_some_and(rich) {
        let mut c = found.expect("checked");
        c.id = id;
        return Ok(c);
    }
    if let Some((source, sid)) = id.strip_prefix("sogo:").and_then(|r| r.split_once(':')) {
        let (_, base) = dav_client(&ctx).await?;
        if let Ok(session) = sogo_session(&ctx, &base, false).await {
            if let Ok((st, text)) = session.view_raw(source, sid).await {
                if st.is_success() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        let base_contact = found.clone().unwrap_or_else(|| dav::Contact {
                            id: id.clone(), book: "Корпоративная адресная книга".into(), href: href.clone(), etag: None, editable: false,
                            full_name: String::new(), emails: vec![], phones: vec![], org: None, title: None, note: None, address: None,
                        });
                        let mut c = sogo::parse_view(&v, base_contact);
                        c.id = id;
                        return Ok(c);
                    }
                }
            }
        }
    }
    found.map(|mut c| { c.id = id; c }).ok_or_else(|| AppError::Other("Сервер не отдал подробности контакта".into()))
}

/// Сессия веб-API SOGo из кэша или новым входом (`fresh` — войти заново).
pub(crate) async fn sogo_session(ctx: &AccountCtx, base: &url::Url, fresh: bool) -> Result<sogo::SogoSession> {
    let creds = ctx.creds.clone();
    let cached = ctx.sogo.lock().await.clone();
    match cached {
        Some(Ok(s)) if !fresh => Ok(s),
        Some(Err(e)) if !fresh => Err(AppError::Other(e)),
        _ => {
            let mut log = Vec::new();
            match sogo::login(base, &creds.username, &creds.password, &mut log).await {
                Ok(s) => {
                    *ctx.sogo.lock().await = Some(Ok(s.clone()));
                    Ok(s)
                }
                Err(e) => {
                    *ctx.sogo.lock().await = Some(Err(e.to_string()));
                    Err(e)
                }
            }
        }
    }
}

pub(crate) async fn sogo_search(ctx: &AccountCtx, base: &url::Url, query: &str) -> Result<Vec<dav::Contact>> {
    match sogo_session(ctx, base, false).await?.search(query).await {
        Ok(list) => Ok(list),
        // Сессия истекла — войти заново один раз.
        Err(_) => sogo_session(ctx, base, true).await?.search(query).await,
    }
}

/// Проверяет, что адрес записи лежит внутри найденной коллекции: frontend не
/// может заставить клиент писать или удалять по произвольному URL с паролем пользователя.
pub(crate) fn owning_collection(cols: &[dav::Collection], href: &str) -> Result<dav::Collection> {
    let target = url::Url::parse(href).map_err(|_| AppError::Other("Некорректный адрес записи".into()))?;
    cols.iter()
        .find(|c| {
            url::Url::parse(&c.url).is_ok_and(|u| {
                u.origin() == target.origin() && target.path().starts_with(u.path()) && target.path() != u.path()
            })
        })
        .cloned()
        .ok_or_else(|| AppError::Other("Запись не принадлежит ни одному известному календарю/адресной книге".into()))
}

pub(crate) fn require_writable(c: &dav::Collection) -> Result<()> {
    if c.writable {
        Ok(())
    } else {
        Err(AppError::Other(format!("«{}» доступен только для чтения", c.name)))
    }
}

pub(crate) async fn known(ctx: &AccountCtx, kind: dav::Kind) -> Result<Vec<dav::Collection>> {
    let cached = match kind {
        dav::Kind::Calendar => ctx.calendars.lock().await.clone(),
        dav::Kind::Contacts => ctx.books.lock().await.clone(),
    };
    cached.ok_or_else(|| AppError::Other("Сначала откройте раздел, чтобы загрузить список".into()))
}

pub(crate) fn collection_child(collection_url: &str, uid: &str, ext: &str) -> Result<String> {
    let base = url::Url::parse(collection_url).map_err(|e| AppError::Other(e.to_string()))?;
    let base = if base.path().ends_with('/') { base } else { url::Url::parse(&format!("{collection_url}/")).map_err(|e| AppError::Other(e.to_string()))? };
    base.join(&format!("{uid}.{ext}"))
        .map(|u| u.to_string())
        .map_err(|e| AppError::Other(e.to_string()))
}

#[tauri::command]
pub async fn create_event(state: State<'_, AppState>, calendar_url: String, input: dav::EventInput) -> Result<()> {
    let ctx = state.ctx().await?;
    if exchange(&ctx) {
        let cal = ews_calendars(&ctx).await?.into_iter().find(|c| c.url == calendar_url).ok_or_else(|| AppError::Other("Календарь не найден".into()))?;
        return with_ctx!(ctx, c => async { ews_of(c)?.create_event(&cal, &input).await });
    }
    let cols = known(&ctx, dav::Kind::Calendar).await?;
    let cal = cols
        .iter()
        .find(|c| c.url == calendar_url)
        .ok_or_else(|| AppError::Other("Календарь не найден".into()))?;
    require_writable(cal)?;
    let uid = dav::new_uid();
    let ics = dav::build_event_ics(&uid, &input)?;
    let (mut client, _) = dav_client(&ctx).await?;
    client.put(&collection_child(&cal.url, &uid, "ics")?, "text/calendar; charset=utf-8", &ics, None).await
}

#[tauri::command]
pub async fn update_event(
    state: State<'_, AppState>,
    href: String,
    input: dav::EventInput,
    change_time: bool,
) -> Result<()> {
    let ctx = state.ctx().await?;
    if exchange(&ctx) {
        return with_ctx!(ctx, c => async { ews_of(c)?.update_event(&href, &input, change_time).await });
    }
    let cal = owning_collection(&known(&ctx, dav::Kind::Calendar).await?, &href)?;
    require_writable(&cal)?;
    let (mut client, _) = dav_client(&ctx).await?;
    // Берём свежую версию с сервера: правим её, а не устаревшую копию из списка.
    let (current, etag) = client.get(&href).await?;
    let ics = dav::update_event_ics(&current, &input, change_time)?;
    client.put(&href, "text/calendar; charset=utf-8", &ics, etag.as_deref().or(Some("*"))).await
}

#[tauri::command]
pub async fn delete_event(state: State<'_, AppState>, href: String, etag: Option<String>) -> Result<()> {
    let ctx = state.ctx().await?;
    if exchange(&ctx) {
        let _ = etag;
        return with_ctx!(ctx, c => async { ews_of(c)?.delete_event(&href).await });
    }
    let cal = owning_collection(&known(&ctx, dav::Kind::Calendar).await?, &href)?;
    require_writable(&cal)?;
    let (mut client, _) = dav_client(&ctx).await?;
    client.delete(&href, etag.as_deref()).await
}

#[tauri::command]
pub async fn create_contact(state: State<'_, AppState>, book_url: String, input: dav::ContactInput) -> Result<()> {
    let ctx = state.ctx().await?;
    if exchange(&ctx) {
        if book_url != crate::ews::CONTACTS_URL {
            return Err(AppError::Other("Общая адресная книга доступна только для чтения".into()));
        }
        return with_ctx!(ctx, c => async { ews_of(c)?.create_contact(&input).await });
    }
    let cols = known(&ctx, dav::Kind::Contacts).await?;
    let book = cols
        .iter()
        .find(|c| c.url == book_url)
        .ok_or_else(|| AppError::Other("Адресная книга не найдена".into()))?;
    require_writable(book)?;
    let uid = dav::new_uid();
    let card = dav::build_vcard(&uid, &input)?;
    let (mut client, _) = dav_client(&ctx).await?;
    client.put(&collection_child(&book.url, &uid, "vcf")?, "text/vcard; charset=utf-8", &card, None).await
}

#[tauri::command]
pub async fn update_contact(state: State<'_, AppState>, href: String, input: dav::ContactInput) -> Result<()> {
    let ctx = state.ctx().await?;
    if exchange(&ctx) {
        return with_ctx!(ctx, c => async { ews_of(c)?.update_contact(&href, &input).await });
    }
    let book = owning_collection(&known(&ctx, dav::Kind::Contacts).await?, &href)?;
    require_writable(&book)?;
    let (mut client, _) = dav_client(&ctx).await?;
    let (current, etag) = client.get(&href).await?;
    let card = dav::update_vcard(&current, &input)?;
    client.put(&href, "text/vcard; charset=utf-8", &card, etag.as_deref().or(Some("*"))).await
}

#[tauri::command]
pub async fn delete_contact(state: State<'_, AppState>, href: String, etag: Option<String>) -> Result<()> {
    let ctx = state.ctx().await?;
    if exchange(&ctx) {
        let _ = etag;
        return with_ctx!(ctx, c => async { ews_of(c)?.delete_contact(&href).await });
    }
    let book = owning_collection(&known(&ctx, dav::Kind::Contacts).await?, &href)?;
    require_writable(&book)?;
    let (mut client, _) = dav_client(&ctx).await?;
    client.delete(&href, etag.as_deref()).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsResult {
    pub(crate) calendars: Vec<dav::Collection>,
    pub(crate) events: Vec<dav::CalEvent>,
    pub(crate) errors: Vec<String>,
}

#[tauri::command]
pub async fn list_events(state: State<'_, AppState>, start: String, end: String) -> Result<EventsResult> {
    let ctx = state.ctx().await?;
    let parse = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&chrono::Utc))
            .map_err(|e| AppError::Other(format!("Некорректная дата {s}: {e}")))
    };
    let (start, end) = (parse(&start)?, parse(&end)?);
    fetch_events(&ctx, start, end).await
}

/// Календари учётной записи (обнаруживаются один раз за сеанс).
pub(crate) async fn calendars_of(ctx: &AccountCtx, client: &mut dav::DavClient, base: &url::Url) -> Result<Vec<dav::Collection>> {
    let cached = ctx.calendars.lock().await.clone();
    match cached {
        Some(c) => Ok(c),
        None => {
            let found = client.discover(base, dav::Kind::Calendar).await?;
            *ctx.calendars.lock().await = Some(found.clone());
            Ok(found)
        }
    }
}

/// Файл календаря (.ics) из письма: события и календари, куда их можно добавить.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarFile {
    pub(crate) events: Vec<dav::IcsEvent>,
    /// Только те, куда можно записывать.
    pub(crate) calendars: Vec<dav::Collection>,
}

/// Вложение — файл календаря: не больше 5 МБ, текст UTF-8.
pub(crate) async fn ics_attachment(ctx: &AccountCtx, folder: &str, uid: u32, index: usize) -> Result<Vec<dav::IcsEvent>> {
    let (_, data) = attachment(ctx, folder, uid, index).await?;
    if data.len() > 5 * 1024 * 1024 {
        return Err(AppError::Other("Файл календаря больше 5 МБ".into()));
    }
    let text = String::from_utf8_lossy(&data).to_string();
    tokio::task::spawn_blocking(move || dav::split_ics(&text)).await.map_err(|e| AppError::Other(e.to_string()))?
}

#[tauri::command]
pub async fn calendar_file_preview(state: State<'_, AppState>, folder: String, uid: u32, index: usize) -> Result<CalendarFile> {
    let ctx = state.ctx().await?;
    let events = ics_attachment(&ctx, &folder, uid, index).await?;
    if exchange(&ctx) {
        return Ok(CalendarFile { events, calendars: ews_calendars(&ctx).await? });
    }
    let (mut client, base) = dav_client(&ctx).await?;
    let calendars = calendars_of(&ctx, &mut client, &base).await?.into_iter().filter(|c| c.writable).collect();
    Ok(CalendarFile { events, calendars })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarImport {
    pub(crate) added: u32,
    pub(crate) updated: u32,
    pub(crate) errors: Vec<String>,
}

/// Добавить события файла календаря из письма в календарь. Событие, которое уже
/// есть в этом календаре (тот же UID — например, обновлённое расписание), заменяется.
#[tauri::command]
pub async fn calendar_file_import(state: State<'_, AppState>, folder: String, uid: u32, index: usize, calendar_url: String, uids: Option<Vec<String>>) -> Result<CalendarImport> {
    let ctx = state.ctx().await?;
    let events = ics_attachment(&ctx, &folder, uid, index).await?;
    if exchange(&ctx) {
        let cal = ews_calendars(&ctx).await?.into_iter().find(|c| c.url == calendar_url).ok_or_else(|| AppError::Other("Календарь не найден".into()))?;
        let mut result = CalendarImport { added: 0, updated: 0, errors: Vec::new() };
        for e in events.iter().filter(|e| uids.as_ref().is_none_or(|u| u.contains(&e.uid))) {
            match with_ctx!(ctx, c => async { ews_of(c)?.import_ics(&cal, e).await }) {
                Ok(true) => result.updated += 1,
                Ok(false) => result.added += 1,
                Err(err) => result.errors.push(format!("«{}»: {err}", e.title)),
            }
        }
        if result.added + result.updated == 0 && !result.errors.is_empty() {
            return Err(AppError::Other(result.errors.join("; ")));
        }
        return Ok(result);
    }
    let (mut client, base) = dav_client(&ctx).await?;
    let cal = calendars_of(&ctx, &mut client, &base)
        .await?
        .into_iter()
        .find(|c| c.url == calendar_url)
        .ok_or_else(|| AppError::Other("Календарь не найден".into()))?;
    require_writable(&cal)?;
    let mut result = CalendarImport { added: 0, updated: 0, errors: Vec::new() };
    for e in events.iter().filter(|e| uids.as_ref().is_none_or(|u| u.contains(&e.uid))) {
        let done = match client.find_uid(&cal, &e.uid).await {
            Ok(Some((href, etag))) => client.put(&href, "text/calendar; charset=utf-8", &e.ics, Some(etag.as_deref().unwrap_or("*"))).await.map(|_| false),
            Ok(None) => match collection_child(&cal.url, &dav::resource_name(&e.uid), "ics") {
                Ok(href) => client.put(&href, "text/calendar; charset=utf-8", &e.ics, None).await.map(|_| true),
                Err(err) => Err(err),
            },
            Err(err) => Err(err),
        };
        match done {
            Ok(true) => result.added += 1,
            Ok(false) => result.updated += 1,
            Err(err) => result.errors.push(format!("«{}»: {err}", e.title)),
        }
    }
    if result.added + result.updated == 0 && !result.errors.is_empty() {
        return Err(AppError::Other(result.errors.join("; ")));
    }
    Ok(result)
}

pub(crate) async fn fetch_events(ctx: &AccountCtx, start: chrono::DateTime<chrono::Utc>, end: chrono::DateTime<chrono::Utc>) -> Result<EventsResult> {
    if exchange(ctx) {
        let calendars = ews_calendars(ctx).await?;
        let mut events = Vec::new();
        let mut errors = Vec::new();
        for cal in &calendars {
            match with_ctx!(ctx, c => async { ews_of(c)?.events(cal, start, end).await }) {
                Ok(list) => events.extend(list),
                Err(e) => errors.push(format!("{}: {e}", cal.name)),
            }
        }
        events.sort_by(|a, b| a.start.cmp(&b.start));
        return Ok(EventsResult { calendars, events, errors });
    }
    let (mut client, base) = dav_client(ctx).await?;
    let cached = ctx.calendars.lock().await.clone();
    let calendars = match cached {
        Some(c) => c,
        None => {
            let found = client.discover(&base, dav::Kind::Calendar).await?;
            *ctx.calendars.lock().await = Some(found.clone());
            found
        }
    };
    let mut events = Vec::new();
    let mut errors = Vec::new();
    for cal in &calendars {
        match client.events(cal, start, end).await {
            Ok(list) => events.extend(list),
            Err(e) => errors.push(format!("{}: {e}", cal.name)),
        }
    }
    events.sort_by(|a, b| a.start.cmp(&b.start));
    Ok(EventsResult { calendars, events, errors })
}
