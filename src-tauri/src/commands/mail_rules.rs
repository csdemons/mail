//! Правила обработки почты, категории и сроки, фильтры на сервере (SOGo / Sieve).

use super::*;

/// Правила изменились (в программе или фильтры на сервере) — обработчик перечитывает их.
pub(crate) static RULES_VERSION: AtomicU64 = AtomicU64::new(0);

/// Что обработчик правил уже видел: UIDNEXT папок, серверные правила, папки.
#[derive(Default)]
pub(crate) struct RuleTracker {
    pub(crate) next: HashMap<String, u32>,
    pub(crate) version: u64,
    pub(crate) loaded_at: Option<std::time::Instant>,
    pub(crate) server: Vec<crate::rules::Rule>,
    pub(crate) folders: HashMap<String, String>,
    pub(crate) folders_at: Option<std::time::Instant>,
}

/// Правила программы и дополнения к серверным правилам — для новых писем, пришедших
/// после запуска (письма, которые уже были, не трогаются). Новые письма видны по
/// UIDNEXT «Входящих» и папок, куда серверные правила с дополнениями их перемещают.
pub(crate) async fn process_rules(app: &AppHandle, ctx: &AccountCtx, t: &mut RuleTracker) -> Result<()> {
    use crate::rules;
    let set = rules::load(&host::config_dir(app)?, &ctx.key);
    let program: Vec<rules::Rule> = set.program.into_iter().filter(rules::Rule::is_active).collect();
    if program.is_empty() && set.ext.is_empty() {
        t.next.clear();
        return Ok(());
    }
    let version = RULES_VERSION.load(Ordering::SeqCst);
    let stale = t.loaded_at.is_none_or(|at| at.elapsed() > Duration::from_secs(300)) || t.version != version;
    if stale {
        t.server = if set.ext.is_empty() {
            Vec::new()
        } else {
            let filters = async {
                let (_, base) = dav_client(ctx).await?;
                let (_, d) = sogo_defaults(ctx, &base).await?;
                Ok::<_, AppError>(d.get("SOGoSieveFilters").cloned().unwrap_or_default())
            }
            .await
            .unwrap_or_default();
            serde_json::from_value::<Vec<serde_json::Value>>(filters)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|v| serde_json::from_value::<rules::Rule>(v).ok())
                .filter(|r| r.is_active() && set.ext.contains_key(&r.name))
                .collect()
        };
        t.folders = with_ctx!(ctx, c => imap::list_folders(c))?.into_iter().map(|f| (f.path, f.id)).collect();
        t.folders_at = Some(std::time::Instant::now());
        t.loaded_at = Some(std::time::Instant::now());
        t.version = version;
    }
    let target_of = |r: &rules::Rule| r.actions.iter().find(|a| a.method == "fileinto").and_then(|a| a.argument.clone());
    // Папку из правила создали после загрузки списка — перечитать его (не чаще раза в 10 с).
    let missing = t.server.iter().chain(program.iter()).filter_map(target_of).any(|p| !t.folders.contains_key(&p));
    if missing && t.folders_at.is_none_or(|at| at.elapsed() > Duration::from_secs(10)) {
        t.folders = with_ctx!(ctx, c => imap::list_folders(c))?.into_iter().map(|f| (f.path, f.id)).collect();
        t.folders_at = Some(std::time::Instant::now());
    }
    let folder_id = |path: &str| t.folders.get(path).cloned().unwrap_or_else(|| path.to_string());
    let mut watched = vec!["INBOX".to_string()];
    for r in &t.server {
        if let Some(p) = target_of(r) {
            let id = folder_id(&p);
            if !watched.contains(&id) {
                watched.push(id);
            }
        }
    }
    let needs_text = program.iter().any(rules::Rule::needs_text) || t.server.iter().any(rules::Rule::needs_text);
    for folder in watched {
        if ctx.is_closed() {
            return Ok(());
        }
        let next = match with_ctx!(ctx, c => imap::uid_next(c, &folder)) {
            Ok(n) => n,
            Err(e) => {
                log::warn!("rules: {folder}: {e}");
                continue;
            }
        };
        // Первый взгляд на папку — только запомнить, где она сейчас.
        let Some(prev) = t.next.insert(folder.clone(), next) else { continue };
        if next <= prev {
            continue;
        }
        let (items, texts) = with_ctx!(ctx, c => async {
            let uids: Vec<u32> = imap::uids_from(c, &folder, prev).await?.into_iter().take(200).collect();
            let items = imap::summaries_of(c, &folder, &uids).await?;
            let texts: HashMap<u32, String> = if needs_text {
                imap::raw_bodies(c, &uids, 2 * 1024 * 1024).await?.into_iter().map(|(u, raw)| (u, imap::search_text(&raw, 20_000))).collect()
            } else {
                HashMap::new()
            };
            Ok((items, texts))
        })?;
        for m in items {
            let text = texts.get(&m.uid).map(String::as_str);
            if folder == "INBOX" {
                for r in &program {
                    if rules::rule_matches(r, &m, text) {
                        let moved = run_rule_actions(app, ctx, &r.name, &r.actions, &m, &folder_id).await;
                        if moved || r.actions.iter().any(|a| a.method == "stop") {
                            break;
                        }
                    }
                }
            }
            // Дополнения к серверным правилам: письмо там, куда его положил сервер
            // (или во «Входящих», если сервер его не перемещал).
            for r in &t.server {
                let place = target_of(r).map(|p| folder_id(&p)).unwrap_or_else(|| "INBOX".into());
                if place == folder && rules::rule_matches(r, &m, text) {
                    if let Some(acts) = set.ext.get(&r.name) {
                        run_rule_actions(app, ctx, &r.name, acts, &m, &folder_id).await;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Действия правила для одного письма. Возвращает true, если письмо перемещено
/// (дальше его уже нет в этой папке). Ошибка одного действия не мешает остальным.
pub(crate) async fn run_rule_actions(app: &AppHandle, ctx: &AccountCtx, rule: &str, acts: &[crate::rules::Act], m: &imap::MessageSummary, folder_id: &(dyn Fn(&str) -> String + Sync)) -> bool {
    // Сначала пометки, уведомления и задачи, перемещение — последним.
    for a in acts.iter().filter(|a| a.method != "fileinto") {
        if let Err(e) = rule_action(app, ctx, rule, a, m).await {
            log::warn!("rule «{rule}», action {}: {e}", a.method);
        }
    }
    if let Some(target) = acts.iter().find(|a| a.method == "fileinto").and_then(|a| a.argument.clone()) {
        let target = folder_id(&target);
        if target != m.folder {
            match rule_move(ctx, &m.folder, m.uid, &target).await {
                Ok(()) => return true,
                Err(e) => log::warn!("rule «{rule}», move: {e}"),
            }
        }
    }
    false
}

pub(crate) async fn rule_move(ctx: &AccountCtx, folder: &str, uid: u32, target: &str) -> Result<()> {
    with_ctx!(ctx, c => imap::move_messages(c, folder, &[uid], target))
}

pub(crate) async fn rule_action(app: &AppHandle, ctx: &AccountCtx, rule: &str, a: &crate::rules::Act, m: &imap::MessageSummary) -> Result<()> {
    let arg = a.argument.clone().unwrap_or_default();
    let (folder, uid) = (m.folder.as_str(), m.uid);
    match a.method.as_str() {
        "addflag" => match arg.as_str() {
            "seen" => with_ctx!(ctx, c => imap::set_flag(c, folder, uid, FlagKind::Seen, true)),
            "flagged" => with_ctx!(ctx, c => imap::set_flag(c, folder, uid, FlagKind::Flagged, true)),
            "junk" => with_ctx!(ctx, c => imap::set_keyword(c, folder, &[uid], "$Junk", true)),
            "not_junk" => with_ctx!(ctx, c => imap::set_keyword(c, folder, &[uid], "$NotJunk", true)),
            l if l.starts_with("label") => {
                let kw = format!("${l}");
                with_ctx!(ctx, c => imap::set_keyword(c, folder, &[uid], &kw, true))
            }
            _ => Ok(()),
        },
        "category" => with_ctx!(ctx, c => imap::set_keyword(c, folder, &[uid], &arg, true)),
        "notify" => {
            let who = if m.from_name.is_empty() { m.from_address.clone() } else { m.from_name.clone() };
            let subject = if m.subject.is_empty() { "(без темы)" } else { &m.subject };
            host::notify(app, &format!("Правило «{rule}»"), &format!("{who}: {subject}"))
        }
        "task" => {
            let (board, stack) = arg.split_once(':').and_then(|(b, s)| Some((b.parse::<i64>().ok()?, s.parse::<i64>().ok()?))).ok_or_else(|| AppError::Other("доска не указана".into()))?;
            let title = if m.subject.trim().is_empty() { format!("Письмо от {}", m.from_address) } else { m.subject.clone() };
            let when = m.date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map(|d| d.with_timezone(&chrono::Local).format("%d.%m.%Y %H:%M").to_string()).unwrap_or_default();
            let mut description = format!("Письмо от {} <{}>\nДата: {when}\nТема: {}\n\nСоздано правилом «{rule}» в Atom Workspace.", m.from_name, m.from_address, m.subject);
            // Связь с организацией отправителя — карточка появится в её истории.
            if let Some(d) = crate::counterparty::domain_of(&m.from_address) {
                description.push_str(&format!("\n\n---\n🏢 Контрагент: {d}"));
            }
            nc(ctx).await?.deck_create_card(board, stack, &title, &description, None).await.map(|_| ())
        }
        _ => Ok(()),
    }
}

/// Папки, письма которых не входят в историю: корзина, спам, черновики.
pub(crate) fn history_skip(st: &store::Store) -> std::collections::HashSet<String> {
    st.folders()
        .unwrap_or_default()
        .into_iter()
        .filter(|f| matches!(f.role, imap::FolderRole::Trash | imap::FolderRole::Junk | imap::FolderRole::Drafts))
        .map(|f| f.id)
        .collect()
}

/// Правила программы и дополнения к серверным правилам (активная учётная запись).
#[tauri::command]
pub async fn rules_get(app: AppHandle, state: State<'_, AppState>) -> Result<crate::rules::RuleSet> {
    let ctx = state.ctx().await?;
    Ok(crate::rules::load(&host::config_dir(&app)?, &ctx.key))
}

#[tauri::command]
pub async fn rules_save(app: AppHandle, state: State<'_, AppState>, set: crate::rules::RuleSet) -> Result<()> {
    let ctx = state.ctx().await?;
    crate::rules::save(&host::config_dir(&app)?, &ctx.key, set)?;
    RULES_VERSION.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    /// Ключевое слово IMAP ($label2… или своё).
    pub(crate) keyword: String,
    pub(crate) name: String,
    pub(crate) color: String,
}

/// Метки веб-почты SOGo по умолчанию — с русскими названиями.
pub(crate) fn default_categories() -> Vec<Category> {
    [("$label1", "Важное", "#f00"), ("$label2", "Работа", "#ff9a00"), ("$label3", "Личное", "#009900"), ("$label4", "К исполнению", "#3333ff"), ("$label5", "Позже", "#993399")]
        .iter()
        .map(|(k, n, c)| Category { keyword: (*k).into(), name: (*n).into(), color: (*c).into() })
        .collect()
}

/// Английские названия меток SOGo по умолчанию показываем по-русски.
pub(crate) fn localize_label(name: &str) -> String {
    match name {
        "Important" => "Важное",
        "Work" => "Работа",
        "Personal" => "Личное",
        "To Do" => "К исполнению",
        "Later" => "Позже",
        other => other,
    }
    .to_string()
}

pub(crate) fn valid_color(c: &str) -> bool {
    let h = c.strip_prefix('#').unwrap_or("");
    (h.len() == 3 || h.len() == 6) && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Категории писем — общие с веб-почтой (SOGo: SOGoMailLabelsColors).
#[tauri::command]
pub async fn categories_get(state: State<'_, AppState>) -> Result<Vec<Category>> {
    let ctx = state.ctx().await?;
    if let Ok((_, base)) = dav_client(&ctx).await {
        if let Ok((_, d)) = sogo_defaults(&ctx, &base).await {
            if let Some(map) = d.get("SOGoMailLabelsColors").and_then(|v| v.as_object()) {
                let mut list: Vec<Category> = map
                    .iter()
                    .filter_map(|(k, v)| {
                        let arr = v.as_array()?;
                        Some(Category { keyword: k.clone(), name: localize_label(arr.first()?.as_str()?), color: arr.get(1)?.as_str()?.to_string() })
                    })
                    .filter(|c| imap::valid_keyword(&c.keyword))
                    .collect();
                list.sort_by(|a, b| a.keyword.cmp(&b.keyword));
                if !list.is_empty() {
                    return Ok(list);
                }
            }
            return Ok(default_categories());
        }
    }
    let profile = ctx.profile().await;
    Ok(profile
        .categories
        .map(|l| l.into_iter().map(|(keyword, name, color)| Category { keyword, name, color }).collect())
        .unwrap_or_else(default_categories))
}

/// Сохранить категории (в веб-почте SOGo; если она недоступна — в программе).
#[tauri::command]
pub async fn categories_save(app: AppHandle, state: State<'_, AppState>, categories: Vec<Category>) -> Result<Option<String>> {
    let ctx = state.ctx().await?;
    for c in &categories {
        if !imap::valid_keyword(&c.keyword) || c.name.trim().is_empty() || c.name.len() > 60 || !valid_color(&c.color) {
            return Err(AppError::Other(format!("Некорректная категория «{}»", c.name)));
        }
    }
    let profile = {
        let mut p = ctx.profile.lock().await;
        p.categories = Some(categories.iter().map(|c| (c.keyword.clone(), c.name.trim().to_string(), c.color.clone())).collect());
        p.clone()
    };
    settings::save(&app, &profile)?;
    let synced = async {
        let (_, base) = dav_client(&ctx).await?;
        let (session, mut defaults) = sogo_defaults(&ctx, &base).await?;
        let map: serde_json::Map<String, serde_json::Value> =
            categories.iter().map(|c| (c.keyword.clone(), serde_json::json!([c.name.trim(), c.color]))).collect();
        defaults["SOGoMailLabelsColors"] = serde_json::Value::Object(map);
        session.save_defaults(&defaults).await
    }
    .await;
    Ok(synced.err().map(|e| format!("Категории сохранены в программе, но не в веб-почте: {e}")))
}

/// Поставить или снять категорию (ключевое слово) у писем.
#[tauri::command]
pub async fn set_keywords(state: State<'_, AppState>, folder: String, uids: Vec<u32>, keyword: String, value: bool) -> Result<()> {
    let ctx = state.ctx().await?;
    if !imap::valid_keyword(&keyword) {
        return Err(AppError::Other("Недопустимое имя метки".into()));
    }
    let queued = queue_first(&ctx).await || match with_ctx!(ctx, c => imap::set_keyword(c, &folder, &uids, &keyword, value)) {
        Ok(()) => false,
        Err(e) if queue_on(&ctx, &e).await => true,
        Err(e) => return Err(e),
    };
    if queued {
        let targets = targets(&ctx, &folder, &uids).await;
        enqueue(&ctx, Op::Keyword { folder: folder.clone(), targets, keyword: keyword.clone(), value }).await?;
    }
    if let Some(st) = local(&ctx).await {
        for uid in &uids {
            st.patch(&folder, *uid, |m| {
                m.keywords.retain(|k| !k.eq_ignore_ascii_case(&keyword));
                if value {
                    m.keywords.push(keyword.clone());
                }
            });
        }
    }
    Ok(())
}

/// Флажок со сроком (RFC 3339) или без срока (`None`). Срок хранится на сервере
/// ключевым словом — виден на всех компьютерах пользователя.
#[tauri::command]
pub async fn set_due(state: State<'_, AppState>, folder: String, uid: u32, due: Option<String>) -> Result<()> {
    let ctx = state.ctx().await?;
    let due = match due {
        Some(d) => Some(chrono::DateTime::parse_from_rfc3339(&d).map_err(|_| AppError::Other("Некорректная дата".into()))?.with_timezone(&chrono::Utc)),
        None => None,
    };
    let queued = queue_first(&ctx).await || match with_ctx!(ctx, c => imap::set_due(c, &folder, uid, due)) {
        Ok(()) => false,
        Err(e) if queue_on(&ctx, &e).await => true,
        Err(e) => return Err(e),
    };
    if queued {
        let target = targets(&ctx, &folder, &[uid]).await.remove(0);
        enqueue(&ctx, Op::Due { folder: folder.clone(), target, due: due.map(|d| d.to_rfc3339()) }).await?;
    }
    if let Some(st) = local(&ctx).await {
        st.patch(&folder, uid, |m| {
            m.flagged = true;
            m.keywords.retain(|k| !k.starts_with(imap::DUE_PREFIX));
            if let Some(at) = due {
                m.keywords.push(imap::due_keyword(&at));
            }
        });
    }
    Ok(())
}

/// Письма с флажком и сроком во всех своих папках (для напоминаний).
#[tauri::command]
pub async fn due_list(state: State<'_, AppState>) -> Result<Vec<imap::MessageSummary>> {
    let ctx = state.ctx().await?;
    let creds = ctx.creds.clone();
    let mut conn = imap::connect(&creds).await?;
    let folders = imap::list_folders(&mut conn).await?;
    let mut out = Vec::new();
    for f in folders.iter().filter(|f| f.selectable && !imap::is_shared(f) && !matches!(f.role, imap::FolderRole::Trash | imap::FolderRole::Junk)) {
        match imap::due_in(&mut conn, &f.id).await {
            Ok(items) => out.extend(items),
            Err(e) if e.is_connection() => return Err(e),
            Err(e) => log::warn!("due scan failed in a folder: {e}"),
        }
    }
    conn.logout().await;
    out.sort_by_key(|m| imap::due_of(&m.keywords));
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailFilters {
    pub(crate) filters: serde_json::Value,
}

/// Фильтры почты пользователя — те же, что в веб-почте SOGo (Настройки → Фильтры).
#[tauri::command]
pub async fn mail_filters_get(state: State<'_, AppState>) -> Result<MailFilters> {
    let ctx = state.ctx().await?;
    let (_, base) = dav_client(&ctx).await?;
    let (_, defaults) = sogo_defaults(&ctx, &base).await?;
    let filters = defaults.get("SOGoSieveFilters").cloned().filter(|v| v.is_array()).unwrap_or_else(|| serde_json::json!([]));
    Ok(MailFilters { filters })
}

/// Сессия SOGo и настройки пользователя; при ошибке (сессия истекла, прошлый вход
/// не удался) — один повтор с новым входом.
pub(crate) async fn sogo_defaults(ctx: &AccountCtx, base: &url::Url) -> Result<(sogo::SogoSession, serde_json::Value)> {
    if let Ok(s) = sogo_session(ctx, base, false).await {
        if let Ok(d) = s.defaults().await {
            return Ok((s, d));
        }
    }
    let s = sogo_session(ctx, base, true).await?;
    let d = s.defaults().await?;
    Ok((s, d))
}

/// Сохраняет фильтры: читает свежие настройки, меняет только `SOGoSieveFilters`
/// и отправляет обратно целиком (SOGo пересобирает Sieve-скрипт).
#[tauri::command]
pub async fn mail_filters_save(state: State<'_, AppState>, filters: serde_json::Value) -> Result<()> {
    let ctx = state.ctx().await?;
    sogo::validate_filters(&filters)?;
    let (_, base) = dav_client(&ctx).await?;
    let (session, mut defaults) = sogo_defaults(&ctx, &base).await?;
    defaults["SOGoSieveFilters"] = filters;
    session.save_defaults(&defaults).await?;
    RULES_VERSION.fetch_add(1, Ordering::SeqCst);
    Ok(())
}
