//! Контрагенты: карточки и лента переписки, файлов, задач и чата.

use super::*;

/// Организации, с которыми шла переписка (по письмам на компьютере), и те,
/// для которых заведено досье.
#[tauri::command]
pub async fn counterparties(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<crate::counterparty::Counterparty>> {
    let ctx = state.ctx().await?;
    let own = ctx.profile.lock().await.email();
    let dossiers = crate::counterparty::dossiers(&host::config_dir(&app)?, &ctx.key);
    let Some(st) = local(&ctx).await else {
        return Ok(crate::counterparty::list(&[], &own, &dossiers));
    };
    tokio::task::spawn_blocking(move || {
        let skip = history_skip(&st);
        let msgs = st.select(&skip, |_| true, usize::MAX);
        crate::counterparty::list(&msgs, &own, &dossiers)
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CounterpartyTimeline {
    pub(crate) domain: String,
    pub(crate) own: bool,
    pub(crate) messages: Vec<imap::MessageSummary>,
    pub(crate) events: Vec<dav::CalEvent>,
    pub(crate) contacts: Vec<dav::Contact>,
    pub(crate) people: Vec<crate::counterparty::Person>,
    pub(crate) dossier: crate::counterparty::Dossier,
    /// Письма искались и на сервере (в том числе старше хранимых на компьютере).
    pub(crate) server: bool,
    /// С какого дня искались встречи (ГГГГ-ММ-ДД).
    pub(crate) events_from: String,
    pub(crate) errors: Vec<String>,
}

/// История взаимодействия с организацией: письма (на компьютере; с `server` —
/// и на сервере), встречи с её участниками, контакты из адресных книг и досье.
/// Источник, который не ответил, не мешает остальным — его ошибка в `errors`.
#[tauri::command]
pub async fn counterparty_timeline(app: AppHandle, state: State<'_, AppState>, domain: String, server: bool, years: Option<u32>) -> Result<CounterpartyTimeline> {
    use crate::counterparty as cp;
    let ctx = state.ctx().await?;
    let dom = cp::normalize(&domain).ok_or_else(|| AppError::Other(format!("«{domain}» — не домен почтового адреса")))?;
    let own = cp::domain_of(&ctx.profile.lock().await.email()).is_some_and(|d| d == dom);
    let dossier = cp::dossier(&host::config_dir(&app)?, &ctx.key, &dom);
    let mut errors = Vec::new();

    let st = local(&ctx).await;
    let mut messages = match st.clone() {
        Some(st) => {
            let d = dom.clone();
            tokio::task::spawn_blocking(move || {
                let skip = history_skip(&st);
                st.select(&skip, |m| cp::concerns(m, &d), 2000)
            })
            .await
            .map_err(|e| AppError::Other(e.to_string()))?
        }
        None => Vec::new(),
    };
    // Без хранилища на компьютере письма ищутся только на сервере.
    let server = server || st.is_none();
    if server {
        let q = crate::search::Query { domain: Some(dom.clone()), ..Default::default() };
        match server_search(&ctx.creds, &q).await {
            Ok(found) => {
                let skip = st.as_ref().map(|s| history_skip(s)).unwrap_or_default();
                for m in found {
                    if skip.contains(&m.folder) || !cp::concerns(&m, &dom) {
                        continue;
                    }
                    if !messages.iter().any(|x| x.folder == m.folder && x.uid == m.uid) {
                        messages.push(m);
                    }
                }
            }
            Err(e) => errors.push(format!("Поиск писем на сервере: {e}")),
        }
    }
    let ts = |m: &imap::MessageSummary| m.date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map_or(0, |d| d.timestamp());
    messages.sort_by_key(|m| std::cmp::Reverse(ts(m)));

    // Встречи: за выбранное число лет назад и на 6 месяцев вперёд.
    let now = chrono::Utc::now();
    let from = now - chrono::Duration::days(365 * i64::from(years.unwrap_or(1).clamp(1, 10)));
    let to = now + chrono::Duration::days(183);
    let events = match tokio::time::timeout(Duration::from_secs(60), fetch_events(&ctx, from, to)).await {
        Ok(Ok(r)) => {
            errors.extend(r.errors.into_iter().map(|e| format!("Календарь {e}")));
            let mut seen = std::collections::HashSet::new();
            r.events
                .into_iter()
                // С участниками из организации или со ссылкой на неё (срок задачи: «Контрагент: домен»).
                .filter(|e| e.participants.iter().any(|p| cp::in_domain(p, &dom)) || e.description.as_deref().is_some_and(|d| mentions_domain(d, &dom)))
                .filter(|e| seen.insert(e.id.clone()))
                .collect()
        }
        Ok(Err(e)) => {
            errors.push(format!("Календарь: {e}"));
            Vec::new()
        }
        Err(_) => {
            errors.push("Календарь: сервер не ответил за минуту".into());
            Vec::new()
        }
    };

    let contacts = match tokio::time::timeout(Duration::from_secs(30), find_contacts(&ctx, &format!("@{dom}"))).await {
        Ok(Ok(list)) => list.into_iter().filter(|c| c.emails.iter().any(|e| cp::in_domain(e, &dom))).collect(),
        Ok(Err(e)) => {
            errors.push(format!("Контакты: {e}"));
            Vec::new()
        }
        Err(_) => {
            errors.push("Контакты: сервер не ответил за 30 секунд".into());
            Vec::new()
        }
    };

    let people = cp::people(&messages, &dom);
    Ok(CounterpartyTimeline {
        domain: dom,
        own,
        messages,
        events,
        contacts,
        people,
        dossier,
        server,
        events_from: from.date_naive().to_string(),
        errors,
    })
}

/// В тексте упомянут домен целиком (не как часть другого: «niiar.ru» не найдётся в «xniiar.ru»).
pub(crate) fn mentions_domain(text: &str, dom: &str) -> bool {
    let t = text.to_lowercase();
    // Слева допускается точка (поддомен — та же организация), справа — конец слова.
    let edge = |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '-'));
    t.match_indices(dom).any(|(i, _)| {
        let rest = &t[i + dom.len()..];
        let longer = rest.starts_with('.') && rest[1..].starts_with(char::is_alphanumeric);
        edge(t[..i].chars().next_back()) && edge(rest.chars().next()) && !longer
    })
}

#[cfg(test)]
mod mention_tests {
    use super::mentions_domain;

    #[test]
    fn whole_domain_only() {
        assert!(mentions_domain("Контрагент: niiar.ru", "niiar.ru"));
        assert!(mentions_domain("mail.NIIAR.ru, срок", "niiar.ru"));
        assert!(mentions_domain("Контрагент: niiar.ru.", "niiar.ru"));
        assert!(!mentions_domain("xniiar.ru", "niiar.ru"));
        assert!(!mentions_domain("niiar.ru-test", "niiar.ru"));
        assert!(!mentions_domain("niiar.ruble", "niiar.ru"));
        assert!(!mentions_domain("niiar.ru.example.com", "niiar.ru"));
    }
}

/// Досье организации: название, заметка, договорённости.
#[tauri::command]
pub async fn counterparty_save(app: AppHandle, state: State<'_, AppState>, domain: String, dossier: crate::counterparty::Dossier) -> Result<()> {
    let ctx = state.ctx().await?;
    let dom = crate::counterparty::normalize(&domain).ok_or_else(|| AppError::Other(format!("«{domain}» — не домен почтового адреса")))?;
    crate::counterparty::save(&host::config_dir(&app)?, &ctx.key, &dom, dossier)
}

/// Папка документов организации в Nextcloud: «/Контрагенты/<домен>».
pub const COUNTERPARTY_DIR: &str = "/Контрагенты";
