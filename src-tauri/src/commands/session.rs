//! Вход и выход, учётные записи, профиль, политика, второй фактор и блокировка, диагностика.

use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub(crate) username: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    /// Программа заблокирована: нужен код TOTP (почта ещё не подключена).
    pub(crate) locked: bool,
    /// Сервер недоступен — сеанс открыт без сети, с письмами на компьютере.
    #[serde(default)]
    pub(crate) offline: bool,
    /// Адрес почты (у Exchange логин может быть DOMAIN\user).
    pub(crate) email: String,
    pub(crate) exchange: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    pub(crate) account: Account,
    /// Предупреждение, если пароль не удалось сохранить в системное хранилище.
    pub(crate) warning: Option<String>,
}

/// Пошаговая проверка входа в Exchange (до входа в программу): отчёт без пароля.
#[tauri::command]
pub async fn exchange_diagnose(url: String, address: String, username: String, password: String) -> Result<String> {
    let login = if username.trim().is_empty() { address.trim().to_string() } else { username.trim().to_string() };
    Ok(tokio::time::timeout(Duration::from_secs(180), crate::ews::diagnose(&url, address.trim(), &login, &password))
        .await
        .unwrap_or_else(|_| "Диагностика не уложилась в 3 минуты".into()))
}

/// Серверы почты по адресу (вход «адрес + пароль»).
#[tauri::command]
pub async fn discover_servers(email: String) -> Result<crate::discover::Discovered> {
    crate::discover::discover(&email).await
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Option<Settings> {
    settings::load(&app)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn login(
    app: AppHandle,
    state: State<'_, AppState>,
    host: String,
    port: u16,
    username: String,
    password: String,
    remember: bool,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    dav_url: Option<String>,
    ews_url: Option<String>,
    address: Option<String>,
) -> Result<LoginResult> {
    if let Some(u) = dav_url.as_deref() {
        settings::validate_dav_url(u)?;
    }
    let username = username.trim().to_string();
    let address = address.map(|a| a.trim().to_string()).unwrap_or_default();
    let exchange = match ews_url {
        Some(u) => Some(resolve_exchange(&username, &password, &address, &u).await?),
        None => None,
    };
    let (host, port) = match &exchange {
        Some(url) => (exchange_host(url), 443),
        None => (host.trim().to_string(), port),
    };
    if host.is_empty() || username.is_empty() || password.is_empty() {
        return Err(AppError::Other("Укажите сервер, логин и пароль".into()));
    }

    let previous = settings::load(&app);
    // Другой сотрудник на том же компьютере не должен унаследовать чужой Nextcloud
    // (Deck, Talk, Файлы) и имя отправителя: эти поля берём, только если учётка та же.
    let same_user = previous
        .as_ref()
        .is_some_and(|p| p.username.eq_ignore_ascii_case(&username) && p.host.eq_ignore_ascii_case(&host));
    let mine = previous.as_ref().filter(|_| same_user);
    if let Some(p) = previous.as_ref().filter(|_| !same_user) {
        if !p.nextcloud_login.is_empty() {
            if let Ok(base) = nextcloud::normalize_base(&p.nextcloud_url) {
                settings::nc_forget_password(base.as_str(), &p.nextcloud_login);
            }
        }
    }
    let s = Settings {
        host: host.clone(),
        port,
        username: username.clone(),
        smtp_host: smtp_host
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty())
            .or_else(|| previous.as_ref().map(|p| p.smtp_host.clone()))
            .unwrap_or_else(|| settings::DEFAULT_SMTP_HOST.to_string()),
        smtp_port: smtp_port
            .or_else(|| previous.as_ref().map(|p| p.smtp_port))
            .unwrap_or(settings::DEFAULT_SMTP_PORT),
        dav_url: dav_url
            .map(|u| u.trim().to_string())
            .or_else(|| previous.as_ref().map(|p| p.dav_url.clone()))
            .unwrap_or_default(),
        display_name: mine.map(|p| p.display_name.clone()).unwrap_or_default(),
        nextcloud_url: previous
            .as_ref()
            .map(|p| p.nextcloud_url.clone())
            .unwrap_or_else(|| settings::DEFAULT_NEXTCLOUD_URL.to_string()),
        nextcloud_login: mine.map(|p| p.nextcloud_login.clone()).unwrap_or_default(),
        nextcloud_uid: mine.map(|p| p.nextcloud_uid.clone()).unwrap_or_default(),
        signature: mine.and_then(|p| p.signature.clone()),
        signatures: mine.map(|p| p.signatures.clone()).unwrap_or_default(),
        signature_new: mine.and_then(|p| p.signature_new.clone()),
        signature_reply: mine.and_then(|p| p.signature_reply.clone()),
        offline_days: mine.and_then(|p| p.offline_days),
        categories: mine.and_then(|p| p.categories.clone()),
        ews_url: exchange.clone().unwrap_or_default(),
        address: if exchange.is_some() { address.clone() } else { String::new() },
        mailboxes: mine.map(|p| p.mailboxes.clone()).unwrap_or_default(),
    };

    let creds = creds_of(&s, password.clone());
    if totp::load(&account_key(&s.username, &s.host)).is_some() {
        // Пароль проверяем сразу, а почту подключаем только после кода TOTP.
        imap::connect(&creds).await?.logout().await;
        if let Err(e) = settings::save(&app, &s) {
            log::warn!("settings not saved: {e}");
        }
        *state.pending.lock().await = Some(Pending { creds, profile: s.clone(), remember: Some(remember) });
        return Ok(LoginResult { account: Account { locked: true, ..account_of(&s) }, warning: None });
    }
    start_session(&app, &state, creds, s.clone(), false).await?;

    // Вход уже выполнен: ошибки сохранения настроек/пароля не отменяют его,
    // а показываются пользователю предупреждением.
    let mut warnings = Vec::new();
    if let Err(e) = settings::save(&app, &s) {
        log::warn!("settings not saved: {e}");
        warnings.push(format!("Не удалось сохранить настройки подключения: {e}."));
    }
    // Политика администратора может запретить запоминать пароль.
    let remember = remember && crate::policy::load().ui().allow_remember_password;
    if remember {
        if let Err(e) = settings::store_password(&s, &password) {
            log::warn!("keyring unavailable: {e}");
            warnings.push("Не удалось сохранить пароль в системном хранилище — при следующем запуске его нужно будет ввести снова.".to_string());
        }
    } else {
        settings::forget_password(&s);
    }
    let warning = (!warnings.is_empty()).then(|| warnings.join(" "));

    Ok(LoginResult {
        account: account_of(&s),
        warning,
    })
}

/// Вход с сохранённым паролем. `None` — пароль не сохранён, нужен экран входа.
#[tauri::command]
pub async fn auto_login(app: AppHandle, state: State<'_, AppState>) -> Result<Option<Account>> {
    let Some(s) = settings::load(&app) else {
        return Ok(None);
    };
    let Some(password) = settings::load_password(&s) else {
        return Ok(None);
    };
    let creds = creds_of(&s, password);
    if totp::load(&account_key(&s.username, &s.host)).is_some() {
        // Сохранённый пароль используется только после кода из приложения.
        *state.pending.lock().await = Some(Pending { creds, profile: s.clone(), remember: None });
        return Ok(Some(Account { locked: true, ..account_of(&s) }));
    }
    match start_session(&app, &state, creds, s.clone(), true).await {
        Ok(online) => Ok(Some(Account { offline: !online, ..account_of(&s) })),
        Err(AppError::AuthFailed) => {
            // Пароль сменили — сохранённый больше не годится.
            settings::forget_password(&s);
            Err(AppError::AuthFailed)
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn logout(app: AppHandle, state: State<'_, AppState>, reset_totp: Option<bool>) -> Result<()> {
    let _one_at_a_time = state.switching.lock().await;
    // Окна писем показывают данные этой учётной записи.
    host::close_account_windows(&app);
    // Телефон потерян: с экрана блокировки можно выйти со сбросом защиты кодом.
    // Сохранённый пароль при этом тоже забывается — снова войти можно только зная
    // пароль почты (с ним переписку и так можно прочитать в веб-почте).
    if reset_totp == Some(true) {
        if let Some(p) = state.pending.lock().await.as_ref() {
            totp::remove(&account_key(&p.creds.username, &p.creds.host));
        }
    }
    // Письма из «окна отмены» уходят, фоновые задачи сессии останавливаются.
    state.outbox.flush().await;
    let active = state.ctx_opt().await;
    let account = match (active.as_ref(), state.pending.lock().await.as_ref()) {
        (Some(c), _) => Some(c.key.clone()),
        (None, Some(p)) => Some(account_key(&p.creds.username, &p.creds.host)),
        _ => settings::load(&app).map(|s| account_key(&s.username, &s.host)),
    };
    // Nextcloud сеанса — только если он этой же учётной записи (контекст её).
    let current_nc = match &active {
        Some(c) => c.nc.lock().await.clone(),
        None => None,
    };
    end_session(&state).await;
    // Письма на компьютере удаляются вместе с паролем.
    if let (Some(account), Ok(root)) = (account.clone(), host::data_dir(&app)) {
        let acc = account.clone();
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || store::wipe(&root, &account)).await {
            log::warn!("local store not removed on logout: {e}");
        }
        crate::ews::forget(&acc);
        // Куки входа в Nextcloud окон редактора и звонков.
        forget_web_profile(&app, &acc);
    }
    // «Выйти и забыть пароль» забывает и подключение к Nextcloud: пароль приложения
    // отзывается на сервере и удаляется из системного хранилища — именно этой
    // учётной записи (из её настроек и её контекста).
    let leaving = account
        .as_deref()
        .and_then(|k| settings::load_accounts(&app).into_iter().find(|a| settings::account_key(a) == k))
        .or_else(|| settings::load(&app).filter(|s| account.as_deref().is_none_or(|k| settings::account_key(s) == k)));
    if let Some(s) = &leaving {
        let same = |c: &nextcloud::NcCreds| nextcloud::normalize_base(&s.nextcloud_url).is_ok_and(|b| b == c.base) && c.login == s.nextcloud_login;
        if let Some((creds, uid)) = load_nc(s).or_else(|| current_nc.filter(|(c, _)| same(c))) {
            if let Ok(client) = nextcloud::Nc::new(creds.clone(), &uid) {
                client.revoke().await;
            }
            settings::nc_forget_password(creds.base.as_str(), &creds.login);
        }
    }
    if let Some(mut s) = leaving {
        settings::forget_password(&s);
        if !s.nextcloud_login.is_empty() {
            if let Ok(base) = nextcloud::normalize_base(&s.nextcloud_url) {
                settings::nc_forget_password(base.as_str(), &s.nextcloud_login);
            }
            s.nextcloud_login.clear();
            s.nextcloud_uid.clear();
            let _ = settings::save(&app, &s);
        }
    }
    *state.pending.lock().await = None;
    *state.totp_setup.lock().await = None;
    // Учётная запись уходит из списка; остальные остаются подключёнными.
    if let Some(key) = account {
        if let Ok(dir) = host::config_dir(&app) {
            crate::rules::forget(&dir, &key);
            crate::counterparty::forget(&dir, &key);
        }
        state.session_passwords.lock().await.remove(&key);
        if let Some(c) = state.others.lock().await.remove(&key) {
            c.logout().await;
        }
        if let Err(e) = settings::remove_account(&app, &key) {
            log::warn!("account list not updated: {e}");
        }
    }
    // Последняя учётная запись — в памяти не остаётся ни паролей, ни соединений.
    if settings::load_accounts(&app).is_empty() {
        state.session_passwords.lock().await.clear();
        state.others_last.lock().await.clear();
        drop_others(&state).await;
    }
    Ok(())
}

/// Текстовый отчёт о том, что отвечает сервер (без пароля) — для разбора проблем.
#[tauri::command]
pub async fn diagnostics(state: State<'_, AppState>, folder: Option<String>) -> Result<String> {
    let ctx = state.ctx().await?;
    let header = format!("Сервер: {}:{}, пользователь: {}\n", ctx.creds.host, ctx.creds.port, ctx.creds.username);
    // Отдельное соединение: диагностика не должна повлиять на рабочую сессию.
    let creds = ctx.creds.clone();
    let mut conn = imap::connect(&creds).await?;
    let report = tokio::time::timeout(OP_TIMEOUT * 2, imap::diagnostics(&mut conn, folder.as_deref()))
        .await
        .map_err(|_| AppError::Timeout)??;
    conn.logout().await;

    let mut extra = String::new();
    if let Some(p) = Some(ctx.profile().await) {
        extra.push_str(&format!("\n## SMTP {}:{}\n", p.smtp_host, p.smtp_port));
        match smtp::check(&p.smtp_host, p.smtp_port, &creds.username, &creds.password).await {
            Ok(true) => extra.push_str("подключение и EHLO: ok (вход проверяется при отправке)\n"),
            Ok(false) => extra.push_str("сервер не ответил на EHLO\n"),
            Err(e) => extra.push_str(&format!("ошибка: {e}\n")),
        }
        for (title, kind) in [("CalDAV", dav::Kind::Calendar), ("CardDAV", dav::Kind::Contacts)] {
            extra.push_str(&format!("\n## {title} ({})\n", p.dav_base()));
            let base = url::Url::parse(&p.dav_base()).map_err(|e| AppError::Other(e.to_string()))?;
            match dav::DavClient::new(&creds.username, &creds.password, &base) {
                Ok(mut client) => {
                    let res = client.discover(&base, kind).await;
                    for line in &client.log {
                        extra.push_str(line);
                        extra.push('\n');
                    }
                    match res {
                        Ok(cols) => {
                            // Пробный поиск: корпоративная книга (LDAP) отвечает только на поиск.
                            let probe: String = creds.username.split('@').next().unwrap_or_default().split(['_', '.']).next().unwrap_or_default().chars().take(6).collect();
                            for c in cols {
                                extra.push_str(&format!("коллекция: {} — {}\n", c.name, c.url));
                                if matches!(kind, dav::Kind::Contacts) && probe.chars().count() >= 2 {
                                    match client.contacts(&c).await {
                                        Ok(all) => extra.push_str(&format!("  весь список: {} записей\n", all.len())),
                                        Err(e) => extra.push_str(&format!("  весь список: ошибка: {e}\n")),
                                    }
                                    match client.search_contacts(&c, &probe, 25).await {
                                        Ok(found) => extra.push_str(&format!("  поиск «{probe}»: {} записей\n", found.len())),
                                        Err(e) => extra.push_str(&format!("  поиск «{probe}»: ошибка: {e}\n")),
                                    }
                                    if !c.writable {
                                        // Подробно для корпоративной книги: что именно отвечает сервер.
                                        for (label, props) in [("FN+EMAIL+N+ORG", &["FN", "EMAIL", "N", "ORG"][..]), ("только FN", &["FN"][..]), ("только EMAIL", &["EMAIL"][..])] {
                                            let body = dav::DavClient::search_body(&probe, props, 5);
                                            match client.report_raw(&c.url, &body).await {
                                                Ok((st, text)) => {
                                                    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                                                    extra.push_str(&format!("  REPORT {label} → {st}, {} байт: {}\n", text.len(), one.chars().take(400).collect::<String>()));
                                                }
                                                Err(e) => extra.push_str(&format!("  REPORT {label} → ошибка: {e}\n")),
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => extra.push_str(&format!("ошибка: {e}\n")),
                    }
                }
                Err(e) => extra.push_str(&format!("ошибка: {e}\n")),
            }
        }
    }
    if let Some(p) = Some(ctx.profile().await) {
        let probe: String = creds.username.split('@').next().unwrap_or_default().split(['_', '.']).next().unwrap_or_default().chars().take(6).collect();
        extra.push_str("\n## Веб-API SOGo (поиск адресатов)\n");
        match url::Url::parse(&p.dav_base()) {
            Ok(base) => {
                let mut log = Vec::new();
                let res = sogo::login(&base, &creds.username, &creds.password, &mut log).await;
                for l in &log {
                    extra.push_str(l);
                    extra.push('\n');
                }
                match res {
                    Ok(session) => {
                        match session.search_raw(&probe).await {
                            Ok((st, text)) => {
                                let parsed = serde_json::from_str::<serde_json::Value>(&text).map(|v| sogo::parse_search(&v).len());
                                let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                                extra.push_str(&format!("allContactSearch «{probe}» → {st}, найдено: {}, ответ: {}\n",
                                    parsed.map(|n| n.to_string()).unwrap_or_else(|_| "не JSON".into()), one.chars().take(400).collect::<String>()));
                            }
                            Err(e) => extra.push_str(&format!("allContactSearch: ошибка: {e}\n")),
                        }
                        match session.defaults().await {
                            Ok(d) => {
                                let f = d.get("SOGoSieveFilters").cloned().unwrap_or(serde_json::Value::Null);
                                let n = f.as_array().map(Vec::len).unwrap_or(0);
                                let sample: String = f.to_string().chars().take(400).collect();
                                extra.push_str(&format!("фильтры (SOGoSieveFilters): {n}, пример: {sample}\n"));
                                extra.push_str(&format!("настройки: {} ключей, Sieve: {}\n", d.as_object().map(|o| o.len()).unwrap_or(0),
                                    d.get("SOGoSieveScriptsEnabled").or(d.get("SOGoSieveFiltersEnabled")).map(|v| v.to_string()).unwrap_or_else(|| "—".into())));
                            }
                            Err(e) => extra.push_str(&format!("фильтры: {e}\n")),
                        }
                        let mut blog = Vec::new();
                        let _ = session.browse("gal", &mut blog).await;
                        for l in blog {
                            extra.push_str(&format!("список gal: {l}\n"));
                        }
                        // Подробности первого найденного: веб-API SOGo и vCard по CardDAV.
                        if let Ok(hit) = session.search(&probe).await.map(|l| l.into_iter().next()) {
                            if let Some((source, sid)) = hit.as_ref().and_then(|h| h.id.strip_prefix("sogo:")).and_then(|r| r.split_once(':')) {
                                match session.view_raw(source, sid).await {
                                    Ok((st, text)) => {
                                        let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                                        extra.push_str(&format!("view {source}/{sid} → {st}: {}\n", one.chars().take(500).collect::<String>()));
                                    }
                                    Err(e) => extra.push_str(&format!("view: ошибка: {e}\n")),
                                }
                                let dav_base = url::Url::parse(&p.dav_base()).map_err(|e| AppError::Other(e.to_string()))?;
                                if let Ok(mut dc) = dav::DavClient::new(&creds.username, &creds.password, &dav_base) {
                                    let vcf = format!("{}Contacts/{source}/{}", p.dav_base().trim_end_matches('/').to_string() + "/SOGo/dav/" + &creds.username + "/", urlencoding_segment(&format!("{sid}.vcf")));
                                    match dc.get(&vcf).await {
                                        Ok((text, _)) => extra.push_str(&format!("GET {vcf} → ok, {} байт, поля: {}\n", text.len(),
                                            text.lines().filter_map(|l| l.split([':', ';']).next()).filter(|k| !k.is_empty() && !k.starts_with(' ')).collect::<Vec<_>>().join(","))),
                                        Err(e) => extra.push_str(&format!("GET {vcf} → {e}\n")),
                                    }
                                }
                            }
                        }
                        session.logout().await;
                    }
                    Err(e) => extra.push_str(&format!("вход: {e}\n")),
                }
            }
            Err(e) => extra.push_str(&format!("адрес: {e}\n")),
        }
    }
    Ok(format!("{header}{report}{extra}"))
}

#[tauri::command]
pub async fn get_profile(state: State<'_, AppState>) -> Result<Option<Settings>> {
    Ok(match state.ctx_opt().await {
        Some(ctx) => Some(ctx.profile().await),
        None => None,
    })
}

#[tauri::command]
pub async fn update_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    display_name: String,
    smtp_host: String,
    smtp_port: u16,
    dav_url: String,
    password: Option<String>,
) -> Result<Settings> {
    let ctx = state.ctx().await?;
    settings::validate_dav_url(&dav_url)?;
    let mut guard = ctx.profile.lock().await;
    let mut p = guard.clone();
    // Смена серверов (куда уходит пароль почты) — только с подтверждением паролем:
    // даже если в интерфейс попадёт чужой код, увести пароль на свой сервер он не сможет.
    let servers_changed = p.smtp_host != smtp_host.trim() || p.smtp_port != smtp_port || p.dav_url != dav_url.trim();
    if servers_changed {
        let creds = ctx.creds.clone();
        if password.as_deref() != Some(creds.password.as_str()) {
            return Err(AppError::Other("Чтобы изменить адреса серверов, введите пароль почты".into()));
        }
    }
    p.display_name = display_name.trim().to_string();
    p.smtp_host = smtp_host.trim().to_string();
    p.smtp_port = smtp_port;
    if p.dav_url != dav_url.trim() {
        p.dav_url = dav_url.trim().to_string();
        *ctx.calendars.lock().await = None;
        *ctx.books.lock().await = None;
        *ctx.sogo.lock().await = None;
    }
    settings::save(&app, &p)?;
    *guard = p.clone();
    Ok(p)
}

/// Политика администратора для интерфейса (галочка пароля, внешние картинки, срок ссылок).
#[tauri::command]
pub fn get_policy() -> crate::policy::UiPolicy {
    crate::policy::load().ui()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpStatus {
    pub(crate) enabled: bool,
    pub(crate) keyring_available: bool,
    pub(crate) recovery_left: usize,
}

/// Проверка кода TOTP или резервного (резервный удаляется). Состояние — последний
/// принятый шаг и счётчик ошибок — хранится вместе с секретом и переживает перезапуск.
pub(crate) async fn check_totp(state: &AppState, account: &str, code: &str) -> Result<()> {
    // Проверки по очереди: состояние читается и записывается целиком.
    let _gate = state.totp_gate.lock().await;
    let (account, code) = (account.to_string(), code.to_string());
    tokio::task::spawn_blocking(move || totp::check_and_record(&account, &code)).await.map_err(|e| AppError::Other(e.to_string()))?
}

#[tauri::command]
pub async fn totp_status(state: State<'_, AppState>) -> Result<TotpStatus> {
    let account = current_account(&state).await?;
    let stored = totp::load(&account);
    Ok(TotpStatus {
        enabled: stored.is_some(),
        keyring_available: stored.is_some() || totp::keyring_available(&account),
        recovery_left: stored.map_or(0, |s| s.recovery.len()),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpSetup {
    pub(crate) secret: String,
    pub(crate) uri: String,
    pub(crate) qr_svg: String,
}

/// Начать настройку: новый секрет и QR-код для приложения-аутентификатора.
#[tauri::command]
pub async fn totp_setup_begin(state: State<'_, AppState>) -> Result<TotpSetup> {
    let ctx = state.ctx().await?;
    let creds = ctx.creds.clone();
    let account = account_key(&creds.username, &creds.host);
    let secret = totp::new_secret()?;
    let uri = totp::otpauth_uri(&secret, &creds.username);
    let qr_svg = totp::qr_svg(&uri)?;
    *state.totp_setup.lock().await = Some((account, secret.clone()));
    Ok(TotpSetup { secret, uri, qr_svg })
}

/// Подтвердить настройку кодом из приложения; возвращает резервные коды (показываются один раз).
#[tauri::command]
pub async fn totp_setup_confirm(state: State<'_, AppState>, code: String) -> Result<Vec<String>> {
    state.totp_throttle.lock().await.check(totp::unix_now())?;
    let (account, secret) = state.totp_setup.lock().await.clone().ok_or_else(|| AppError::Other("Сначала начните настройку".into()))?;
    let Some(step) = totp::verify(&secret, &code, totp::unix_now() / totp::STEP, None) else {
        state.totp_throttle.lock().await.fail(totp::unix_now());
        return Err(AppError::Other("Неверный код — проверьте время на телефоне и введите новый код".into()));
    };
    let (codes, hashes) = totp::new_recovery_codes()?;
    totp::store(&account, &totp::Stored { secret, recovery: hashes, last_step: Some(step), throttle: totp::Throttle::default() })?;
    state.totp_throttle.lock().await.ok();
    *state.totp_setup.lock().await = None;
    Ok(codes)
}

/// Отключить защиту — нужен текущий код (или резервный).
#[tauri::command]
pub async fn totp_disable(state: State<'_, AppState>, code: String) -> Result<()> {
    let account = current_account(&state).await?;
    check_totp(&state, &account, &code).await?;
    totp::remove(&account);
    Ok(())
}

/// Заблокировать программу: почта отключается, для продолжения нужен код.
#[tauri::command]
pub async fn lock_app(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let ctx = state.ctx().await?;
    if totp::load(&ctx.key).is_none() {
        return Err(AppError::Other("Сначала включите защиту кодом в настройках".into()));
    }
    let creds = ctx.creds.clone();
    let profile = ctx.profile().await;
    host::close_account_windows(&app);
    end_session(&state).await;
    drop_others(&state).await;
    *state.pending.lock().await = Some(Pending { creds, profile, remember: None });
    Ok(())
}

/// Разблокировать кодом TOTP (или резервным) и подключить почту.
#[tauri::command]
pub async fn totp_unlock(app: AppHandle, state: State<'_, AppState>, code: String) -> Result<LoginResult> {
    let account = {
        let p = state.pending.lock().await;
        let p = p.as_ref().ok_or_else(|| AppError::Other("Программа не заблокирована".into()))?;
        account_key(&p.creds.username, &p.creds.host)
    };
    check_totp(&state, &account, &code).await?;
    let p = state.pending.lock().await.take().ok_or(AppError::NotLoggedIn)?;
    let mut warning = None;
    let online = match start_session(&app, &state, p.creds.clone(), p.profile.clone(), true).await {
        Ok(online) => online,
        Err(e) => {
            if matches!(e, AppError::AuthFailed) && p.remember.is_none() {
                settings::forget_password(&p.profile);
            }
            return Err(e);
        }
    };
    match p.remember {
        Some(true) => {
            if settings::store_password(&p.profile, &p.creds.password).is_err() {
                warning = Some("Не удалось сохранить пароль в системном хранилище — при следующем запуске его нужно будет ввести снова.".to_string());
            }
        }
        Some(false) => settings::forget_password(&p.profile),
        None => {}
    }
    Ok(LoginResult { account: Account { offline: !online, ..account_of(&p.profile) }, warning })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountEntry {
    pub(crate) key: String,
    pub(crate) username: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) display_name: String,
    pub(crate) active: bool,
    /// Пароль известен (сохранён или введён в этой сессии) — можно переключиться.
    pub(crate) can_switch: bool,
    pub(crate) email: String,
    pub(crate) exchange: bool,
}

pub(crate) async fn active_key(state: &AppState) -> Option<String> {
    if let Some(c) = state.ctx_opt().await {
        return Some(c.key.clone());
    }
    state.pending.lock().await.as_ref().map(|p| account_key(&p.creds.username, &p.creds.host))
}

pub(crate) async fn password_of(state: &AppState, s: &Settings) -> Option<String> {
    let key = settings::account_key(s);
    if let Some(p) = state.session_passwords.lock().await.get(&key) {
        return Some(p.clone());
    }
    let s = s.clone();
    tokio::task::spawn_blocking(move || settings::load_password(&s)).await.ok().flatten()
}

/// Адрес EWS для входа в Exchange: введённый вручную или найденный автоматически
/// (Autodiscover) по адресу почты.
pub(crate) async fn resolve_exchange(username: &str, password: &str, address: &str, input: &str) -> Result<String> {
    let input = input.trim();
    if !input.is_empty() && input != "auto" {
        return Ok(crate::ews::normalize_url(input)?.to_string());
    }
    let email = if address.contains('@') { address } else { username };
    if !email.contains('@') {
        return Err(AppError::Other("Укажите адрес почты или адрес сервера Exchange".into()));
    }
    crate::ews::autodiscover(email, username, password).await
}

pub(crate) fn exchange_host(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_lowercase)).unwrap_or_default()
}

pub(crate) fn creds_of(s: &Settings, password: String) -> Credentials {
    if s.is_exchange() {
        crate::ews::set_mailboxes(&settings::account_key(s), &s.email(), &s.mailboxes);
    }
    Credentials { host: s.host.clone(), port: s.port, username: s.username.clone(), password, ews: s.is_exchange().then(|| s.ews_url.clone()) }
}

/// Подключённые учётные записи (активная — первой).
#[tauri::command]
pub async fn accounts_list(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<AccountEntry>> {
    let active = active_key(&state).await;
    let mut out = Vec::new();
    for s in settings::load_accounts(&app) {
        let key = settings::account_key(&s);
        let is_active = active.as_deref() == Some(key.as_str());
        let can_switch = is_active || password_of(&state, &s).await.is_some();
        out.push(AccountEntry { key, username: s.username.clone(), host: s.host.clone(), port: s.port, display_name: s.display_name.clone(), active: is_active, can_switch, email: s.email(), exchange: s.is_exchange() });
    }
    out.sort_by_key(|a| !a.active);
    Ok(out)
}

/// Добавить учётную запись (без переключения на неё): пароль проверяется входом на сервер.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn account_add(
    app: AppHandle,
    state: State<'_, AppState>,
    host: String,
    port: u16,
    username: String,
    password: String,
    remember: bool,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    dav_url: Option<String>,
    ews_url: Option<String>,
    address: Option<String>,
) -> Result<AccountEntry> {
    if let Some(u) = dav_url.as_deref() {
        settings::validate_dav_url(u)?;
    }
    let username = username.trim().to_string();
    let address = address.map(|a| a.trim().to_string()).unwrap_or_default();
    let exchange = match ews_url {
        Some(u) => Some(resolve_exchange(&username, &password, &address, &u).await?),
        None => None,
    };
    let (host, port) = match &exchange {
        Some(url) => (exchange_host(url), 443),
        None => (host.trim().to_string(), port),
    };
    if host.is_empty() || username.is_empty() || password.is_empty() {
        return Err(AppError::Other("Укажите сервер, логин и пароль".into()));
    }
    let key = account_key(&username, &host);
    let mut list = settings::load_accounts(&app);
    if list.iter().any(|a| settings::account_key(a) == key) {
        return Err(AppError::Other("Эта учётная запись уже подключена".into()));
    }
    if list.len() >= 10 {
        return Err(AppError::Other("Можно подключить до 10 учётных записей".into()));
    }
    let s = Settings {
        host: host.clone(),
        port,
        username: username.clone(),
        smtp_host: smtp_host.map(|h| h.trim().to_string()).filter(|h| !h.is_empty()).unwrap_or_else(|| host.clone()),
        smtp_port: smtp_port.unwrap_or(settings::DEFAULT_SMTP_PORT),
        dav_url: dav_url.map(|u| u.trim().to_string()).unwrap_or_default(),
        display_name: String::new(),
        nextcloud_url: settings::DEFAULT_NEXTCLOUD_URL.to_string(),
        nextcloud_login: String::new(),
        nextcloud_uid: String::new(),
        signature: None,
        signatures: Vec::new(),
        signature_new: None,
        signature_reply: None,
        offline_days: None,
        categories: None,
        ews_url: exchange.clone().unwrap_or_default(),
        address: if exchange.is_some() { address.clone() } else { String::new() },
        mailboxes: Vec::new(),
    };
    let creds = creds_of(&s, password.clone());
    let conn = imap::connect(&creds).await?;
    let remember = remember && crate::policy::load().ui().allow_remember_password;
    if !(remember && settings::store_password(&s, &password).is_ok()) {
        state.session_passwords.lock().await.insert(key.clone(), password);
    }
    state.others.lock().await.insert(key.clone(), conn);
    list.push(s.clone());
    settings::save_accounts(&app, &list)?;
    Ok(AccountEntry { key, username, host, port, display_name: String::new(), active: false, can_switch: true, email: s.email(), exchange: s.is_exchange() })
}

/// Переключиться на другую учётную запись: сессия текущей закрывается, её пароль
/// остаётся в памяти, чтобы вернуться без ввода.
#[tauri::command]
pub async fn account_switch(app: AppHandle, state: State<'_, AppState>, key: String) -> Result<LoginResult> {
    let _one_at_a_time = state.switching.lock().await;
    let s = settings::load_accounts(&app)
        .into_iter()
        .find(|a| settings::account_key(a) == key)
        .ok_or_else(|| AppError::Other("Учётная запись не найдена".into()))?;
    if state.ctx_opt().await.is_some_and(|c| c.key == key) {
        return Ok(LoginResult { account: account_of(&s), warning: None });
    }
    let password = password_of(&state, &s).await.ok_or_else(|| AppError::Other("Пароль этой учётной записи не сохранён — удалите её и добавьте снова".into()))?;
    let creds = creds_of(&s, password);
    // Сначала проверяем вход: при ошибке остаёмся в текущей учётной записи.
    let check = match state.others.lock().await.remove(&key) {
        Some(c) => c,
        None => imap::connect(&creds).await?,
    };
    check.logout().await;

    host::close_account_windows(&app);
    state.outbox.flush().await;
    state.jobs.cancel_all();
    state.import_sources.lock().await.clear();
    if let Some(c) = state.ctx_opt().await {
        state.session_passwords.lock().await.insert(c.key.clone(), c.creds.password.clone());
    }
    if let Some(p) = state.pending.lock().await.take() {
        state.session_passwords.lock().await.insert(account_key(&p.creds.username, &p.creds.host), p.creds.password);
    }
    *state.totp_setup.lock().await = None;
    if totp::load(&key).is_some() {
        // Учётная запись с защитой кодом: прежний сеанс закрывается целиком, новый
        // откроется только после кода — до этого в памяти нет ничего ни от одной.
        end_session(&state).await;
        settings::save(&app, &s)?;
        *state.pending.lock().await = Some(Pending { creds, profile: s.clone(), remember: None });
        return Ok(LoginResult { account: Account { locked: true, ..account_of(&s) }, warning: None });
    }
    start_session(&app, &state, creds, s.clone(), false).await?;
    let warning = settings::save(&app, &s).err().map(|e| format!("Не удалось сохранить настройки: {e}"));
    Ok(LoginResult { account: account_of(&s), warning })
}

/// Отключить неактивную учётную запись: забыть пароль и письма на компьютере.
#[tauri::command]
pub async fn account_remove(app: AppHandle, state: State<'_, AppState>, key: String) -> Result<()> {
    let _one_at_a_time = state.switching.lock().await;
    if active_key(&state).await.as_deref() == Some(key.as_str()) {
        return Err(AppError::Other("Это текущая учётная запись — для неё «Выйти»".into()));
    }
    let list = settings::load_accounts(&app);
    let Some(s) = list.iter().find(|a| settings::account_key(a) == key).cloned() else { return Ok(()) };
    settings::forget_password(&s);
    if !s.nextcloud_login.is_empty() {
        if let Ok(base) = nextcloud::normalize_base(&s.nextcloud_url) {
            settings::nc_forget_password(base.as_str(), &s.nextcloud_login);
        }
    }
    state.session_passwords.lock().await.remove(&key);
    state.others_last.lock().await.remove(&key);
    if let Ok(dir) = host::config_dir(&app) {
        crate::rules::forget(&dir, &key);
        crate::counterparty::forget(&dir, &key);
    }
    forget_web_profile(&app, &key);
    if let Some(c) = state.others.lock().await.remove(&key) {
        c.logout().await;
    }
    if let Ok(root) = host::data_dir(&app) {
        let k = key.clone();
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || store::wipe(&root, &k)).await {
            log::warn!("local store of removed account not wiped: {e}");
        }
    }
    crate::ews::forget(&key);
    settings::remove_account(&app, &key)
}

pub(crate) async fn drop_others(state: &AppState) {
    let conns: Vec<Connection> = state.others.lock().await.drain().map(|(_, c)| c).collect();
    for c in conns {
        c.logout().await;
    }
}

/// Операция на соединении с неактивной учётной записью (с одним переподключением).
pub(crate) async fn on_other<T, F>(state: &AppState, s: &Settings, op: F) -> Result<T>
where
    F: for<'c> Fn(&'c mut Connection) -> futures::future::BoxFuture<'c, Result<T>>,
{
    let key = settings::account_key(s);
    let password = password_of(state, s).await.ok_or_else(|| AppError::Other("пароль не сохранён".into()))?;
    let creds = creds_of(s, password);
    let mut pool = state.others.lock().await;
    for attempt in 0..2 {
        if !pool.contains_key(&key) {
            let c = tokio::time::timeout(Duration::from_secs(20), imap::connect(&creds)).await.map_err(|_| AppError::Timeout)??;
            pool.insert(key.clone(), c);
        }
        let conn = pool.get_mut(&key).expect("соединение есть");
        let result = match tokio::time::timeout(OP_TIMEOUT, op(conn)).await {
            Ok(r) => r,
            Err(_) => Err(AppError::Timeout),
        };
        match result {
            Err(e) if e.is_connection() && attempt == 0 => {
                pool.remove(&key);
            }
            other => return other,
        }
    }
    Err(AppError::ConnectionLost)
}

pub(crate) async fn other_accounts(app: &AppHandle, state: &AppState) -> Vec<Settings> {
    let active = active_key(state).await;
    settings::load_accounts(app).into_iter().filter(|a| Some(settings::account_key(a)) != active).collect()
}

pub(crate) fn account_label(s: &Settings) -> String {
    if s.display_name.trim().is_empty() { s.email() } else { format!("{} ({})", s.display_name.trim(), s.email()) }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUnread {
    pub(crate) key: String,
    pub(crate) unread: Option<u32>,
    pub(crate) error: Option<String>,
}

/// Непрочитанные во «Входящих» других учётных записей.
#[tauri::command]
pub async fn accounts_unread(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<AccountUnread>> {
    state.ctx().await?;
    let mut out = Vec::new();
    for s in other_accounts(&app, &state).await {
        let key = settings::account_key(&s);
        match on_other(&state, &s, |c| Box::pin(imap::inbox_unseen(c))).await {
            Ok(n) => out.push(AccountUnread { key, unread: Some(n), error: None }),
            Err(e) => out.push(AccountUnread { key, unread: None, error: Some(e.to_string()) }),
        }
    }
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherNewMail {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) items: Vec<imap::MessageSummary>,
}

/// Новые непрочитанные письма других учётных записей (для уведомлений).
/// Первая проверка только запоминает последний UID.
#[tauri::command]
pub async fn accounts_poll(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<OtherNewMail>> {
    state.ctx().await?;
    let mut out = Vec::new();
    for s in other_accounts(&app, &state).await {
        let key = settings::account_key(&s);
        let known = state.others_last.lock().await.get(&key).copied();
        let after = known.unwrap_or(0);
        let limit = if known.is_some() { 5 } else { 0 };
        match on_other(&state, &s, move |c| Box::pin(imap::new_unseen(c, "INBOX", after, limit))).await {
            Ok((items, last)) => {
                state.others_last.lock().await.insert(key.clone(), last.max(after));
                if !items.is_empty() {
                    out.push(OtherNewMail { key, label: account_label(&s), items });
                }
            }
            Err(e) => log::warn!("new mail check for {key}: {e}"),
        }
    }
    Ok(out)
}
