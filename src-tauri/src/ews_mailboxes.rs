// Подключённые ящики Exchange: какие есть (Autodiscover, как у Outlook, и добавленные вручную)
// и что с ними (найдены ли, есть ли доступ) — для списка папок и настроек.
use super::*;

/// Сколько ящиков подключаем самое большее (и найденных Autodiscover, и вручную).
pub const MAX_MAILBOXES: usize = 30;

/// Повторить Autodiscover, если в прошлый раз сервер не ответил.
const DISCOVER_RETRY: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
pub(super) struct Mailboxes {
    /// Свой адрес почты: для Autodiscover и чтобы не подключить свой ящик второй раз.
    pub(super) email: String,
    /// Добавленные пользователем.
    manual: Vec<String>,
    /// Найденные Autodiscover (ящики с полным доступом — Outlook подключает их сам):
    /// (адрес, имя). `None` — ещё не спрашивали.
    discovered: Option<Vec<(String, String)>>,
    discovered_at: Option<std::time::Instant>,
    /// Ответ Autodiscover был (а не ошибка сети) — тогда не повторяем.
    discovered_ok: bool,
    /// Итог последнего автопоиска понятными словами — для настроек.
    note: String,
    /// Ящики, которые не открылись при последней загрузке папок: (адрес, причина).
    unavailable: Vec<(String, String)>,
}

pub(super) fn mailbox_registry() -> &'static Mutex<HashMap<String, Mailboxes>> {
    static R: OnceLock<Mutex<HashMap<String, Mailboxes>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Ящик учётной записи и подключённые вручную ящики (при входе и после изменения в настройках).
pub fn set_mailboxes(key: &str, email: &str, manual: &[String]) {
    let mut reg = lock(mailbox_registry());
    let m = reg.entry(key.to_string()).or_default();
    if !m.email.eq_ignore_ascii_case(email) {
        m.discovered = None;
        m.discovered_at = None;
        m.discovered_ok = false;
    }
    m.email = email.trim().to_lowercase();
    m.manual = manual.iter().map(|a| a.trim().to_lowercase()).filter(|a| valid_mailbox(a)).collect();
}

/// Найденные Autodiscover ящики — для настроек.
pub fn discovered_mailboxes(key: &str) -> Vec<(String, String)> {
    lock(mailbox_registry()).get(key).and_then(|m| m.discovered.clone()).unwrap_or_default()
}

/// Что известно о подключённых ящиках: (автопоиск удался, итог автопоиска, ящики без доступа).
pub fn mailbox_status(key: &str) -> (bool, String, Vec<(String, String)>) {
    lock(mailbox_registry()).get(key).map(|m| (m.discovered_ok, m.note.clone(), m.unavailable.clone())).unwrap_or_default()
}

/// Спросить сервер о ящиках заново при следующей загрузке папок.
pub fn forget_discovered(key: &str) {
    if let Some(m) = lock(mailbox_registry()).get_mut(key) {
        m.discovered = None;
        m.discovered_at = None;
        m.discovered_ok = false;
    }
}

pub(super) fn set_unavailable(key: &str, list: Vec<(String, String)>) {
    if let Some(m) = lock(mailbox_registry()).get_mut(key) {
        m.unavailable = list;
    }
}

/// Адрес ящика: имя@домен, без пробелов и угловых скобок.
pub fn valid_mailbox(a: &str) -> bool {
    let a = a.trim();
    a.len() <= 254
        && a.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.'))
        && !a.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"' | ',' | ';' | '/' | '\\'))
}

/// Ящик из ответа Autodiscover. Exchange 2010–2016 в своей сети часто не сообщает адрес
/// (только LegacyDN и имя) — тогда адрес ищется в адресной книге.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AltMailbox {
    pub(super) smtp: Option<String>,
    pub(super) name: String,
    pub(super) legacy_dn: Option<String>,
}

/// Ящики из ответа Autodiscover (`AlternativeMailbox`): делегированные и общие
/// (Delegate, TeamMailbox); архив (Archive) — это часть своего ящика, не подключаем.
pub(super) fn alternative_mailboxes(text: &str, own: &str) -> Vec<AltMailbox> {
    let Ok(d) = roxmltree::Document::parse(text) else { return Vec::new() };
    let mut out: Vec<AltMailbox> = Vec::new();
    for n in d.descendants().filter(|n| n.is_element() && n.tag_name().name() == "AlternativeMailbox") {
        let ty = text_of(n, "Type").unwrap_or_default();
        if !matches!(ty.as_str(), "Delegate" | "TeamMailbox") {
            continue;
        }
        let smtp = text_of(n, "SmtpAddress").or_else(|| text_of(n, "OwnerSmtpAddress")).map(|a| a.trim().to_lowercase()).filter(|a| valid_mailbox(a));
        let legacy_dn = text_of(n, "LegacyDN").map(|s| s.trim().to_string()).filter(|s| s.starts_with('/'));
        let name = text_of(n, "DisplayName").map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if smtp.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(own)) {
            continue;
        }
        let dup = out.iter().any(|m| match (&smtp, &legacy_dn) {
            (Some(a), _) if m.smtp.as_ref() == Some(a) => true,
            (_, Some(l)) => m.legacy_dn.as_ref().is_some_and(|x| x.eq_ignore_ascii_case(l)),
            _ => false,
        });
        if dup || (smtp.is_none() && legacy_dn.is_none() && name.is_none()) {
            continue;
        }
        let name = name.or_else(|| smtp.clone()).unwrap_or_default();
        out.push(AltMailbox { smtp, name, legacy_dn });
    }
    out.truncate(MAX_MAILBOXES);
    out
}

/// Перенаправление в ответе Autodiscover: на другой адрес почты или на другой сервер.
#[derive(Debug, PartialEq)]
pub(super) enum Redirect {
    Addr(String),
    Url(String),
}

pub(super) fn autodiscover_redirect(text: &str) -> Option<Redirect> {
    let d = roxmltree::Document::parse(text).ok()?;
    let acc = d.descendants().find(|n| n.is_element() && n.tag_name().name() == "Account")?;
    match text_of(acc, "Action")?.trim() {
        "redirectAddr" => text_of(acc, "RedirectAddr").map(|a| Redirect::Addr(a.trim().to_lowercase())),
        "redirectUrl" => text_of(acc, "RedirectUrl").map(|u| Redirect::Url(u.trim().to_string())),
        _ => None,
    }
}

/// Где спрашивать Autodiscover о ящиках: сначала сервер EWS, на который уже уходит пароль
/// (в сети организации autodiscover.<домен> часто не настроен), затем адреса в домене почты.
pub(super) fn discovery_candidates(domain: &str, ews: &Url) -> Vec<String> {
    let mut out = Vec::new();
    if ews.scheme() == "https" {
        if let Ok(u) = ews.join("/autodiscover/autodiscover.xml") {
            out.push(u.to_string());
        }
    }
    for c in [format!("https://autodiscover.{domain}/autodiscover/autodiscover.xml"), format!("https://{domain}/autodiscover/autodiscover.xml")] {
        if !out.iter().any(|x| x.eq_ignore_ascii_case(&c)) {
            out.push(c);
        }
    }
    out
}

/// Ящики с полным доступом по Autodiscover и сервер, который ответил; ошибка — понятной строкой.
async fn discover_alternative(email: &str, ews: &Url, login: &str, password: &str) -> std::result::Result<(Vec<AltMailbox>, String), String> {
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_lowercase()).filter(|d| d.contains('.')).ok_or_else(|| "не указан адрес почты (имя@домен)".to_string())?;
    #[allow(unused_mut)]
    let mut allowed = crate::policy::load().exchange_hosts;
    #[cfg(debug_assertions)]
    if let Ok(h) = std::env::var("RRCKI_E2E_EXCHANGE_HOSTS") {
        allowed.push(h);
    }
    let ews_host = ews.host_str().unwrap_or_default().to_lowercase();
    let mut queue = discovery_candidates(&domain, ews);
    let mut address = email.to_string();
    let mut hops = 0;
    let mut errors: Vec<String> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut i = 0;
    while i < queue.len() {
        let c = queue[i].clone();
        i += 1;
        let Ok(url) = Url::parse(&c) else { continue };
        let host = url.host_str().unwrap_or_default().to_string();
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            errors.push("время ожидания истекло".into());
            break;
        }
        let mut s = probe_session(url, login, password);
        match tokio::time::timeout(left.min(Duration::from_secs(15)), s.post_raw(&autodiscover_request(&address), crate::netio::SMALL_LIMIT)).await {
            Ok(Ok(text)) => match autodiscover_redirect(&text) {
                Some(Redirect::Addr(a)) if hops < 3 && valid_mailbox(&a) && !a.eq_ignore_ascii_case(&address) => {
                    hops += 1;
                    address = a;
                    i -= 1; // тот же сервер, другой адрес
                }
                Some(Redirect::Url(u)) if hops < 3 => {
                    hops += 1;
                    let h = Url::parse(&u).ok().filter(|x| x.scheme() == "https").and_then(|x| x.host_str().map(str::to_lowercase)).unwrap_or_default();
                    // Пароль уходит только на сервер EWS, в домен почты или на разрешённый политикой сервер.
                    if h == ews_host || trusted_ews_host(&h, &domain, &allowed) {
                        queue.insert(i, u);
                    } else {
                        errors.push(format!("{host}: перенаправляет на {h} — сервер не в домене почты"));
                    }
                }
                _ if ews_url_of(&text).is_some() || text.contains("AlternativeMailbox") => return Ok((alternative_mailboxes(&text, email), host)),
                _ => errors.push(format!("{host}: в ответе нет настроек Exchange")),
            },
            Ok(Err(AppError::AuthFailed)) => errors.push(format!("{host}: сервер не принял пароль")),
            Ok(Err(e)) => errors.push(format!("{host}: {e}")),
            Err(_) => errors.push(format!("{host}: нет ответа")),
        }
    }
    Err(errors.first().cloned().unwrap_or_else(|| "сервер Autodiscover не найден".into()))
}

impl EwsSession {
    /// Адрес ящика, о котором Autodiscover сообщил только LegacyDN и имя: поиск в адресной книге.
    async fn alt_address(&mut self, m: &AltMailbox) -> Option<String> {
        if let Some(dn) = &m.legacy_dn {
            if let Ok(found) = self.resolve(dn).await {
                if let [one] = found.as_slice() {
                    if let Some(a) = one.emails.first().filter(|a| valid_mailbox(a)) {
                        return Some(a.to_lowercase());
                    }
                }
            }
        }
        if m.name.chars().count() < 2 {
            return None;
        }
        let name = m.name.to_lowercase();
        let found = self.resolve(&m.name).await.ok()?;
        found.iter().find(|c| c.full_name.to_lowercase() == name).and_then(|c| c.emails.first()).filter(|a| valid_mailbox(a)).map(|a| a.to_lowercase())
    }

    /// Спросить Autodiscover о ящиках с полным доступом и запомнить ответ.
    async fn discover_mailboxes(&mut self, email: &str) {
        let url = self.url.clone();
        let (found, ok, note) = match discover_alternative(email, &url, &self.login, &self.password).await {
            Ok((list, host)) => {
                let total = list.len();
                let mut out: Vec<(String, String)> = Vec::new();
                let mut lost: Vec<String> = Vec::new();
                for m in list {
                    let a = match &m.smtp {
                        Some(a) => Some(a.clone()),
                        None => self.alt_address(&m).await,
                    };
                    match a {
                        Some(a) if !a.eq_ignore_ascii_case(email) && !out.iter().any(|(x, _)| *x == a) => out.push((a, m.name)),
                        Some(_) => {}
                        None => lost.push(m.name),
                    }
                }
                let mut note = if total == 0 {
                    format!("Сервер {host} не сообщил ни одного ящика с полным доступом")
                } else {
                    format!("Сервер {host} сообщил ящиков с полным доступом: {total}")
                };
                if !lost.is_empty() {
                    note.push_str(&format!("; адрес не найден в адресной книге: {}", lost.join(", ")));
                }
                (Some(out), true, note)
            }
            Err(e) => {
                log::warn!("autodiscover mailboxes: {e}");
                (None, false, format!("Не удалось спросить сервер о ящиках с полным доступом: {e}"))
            }
        };
        let mut reg = lock(mailbox_registry());
        if let Some(m) = reg.get_mut(&self.key) {
            m.discovered_ok = ok;
            m.discovered = Some(found.unwrap_or_default());
            m.discovered_at = Some(std::time::Instant::now());
            m.note = note;
        }
    }

    /// Подключённые ящики: найденные Autodiscover (как у Outlook) и добавленные вручную.
    pub(super) async fn mailbox_list(&mut self) -> Vec<(String, String)> {
        let (email, manual, need) = {
            let reg = lock(mailbox_registry());
            match reg.get(&self.key) {
                Some(m) => (
                    m.email.clone(),
                    m.manual.clone(),
                    m.discovered.is_none() || (!m.discovered_ok && m.discovered_at.is_none_or(|t| t.elapsed() > DISCOVER_RETRY)),
                ),
                None => return Vec::new(),
            }
        };
        if need {
            self.discover_mailboxes(&email).await;
        }
        let discovered = discovered_mailboxes(&self.key);
        let mut out: Vec<(String, String)> = Vec::new();
        for (a, n) in discovered.into_iter().chain(manual.into_iter().map(|a| (a.clone(), a))) {
            if !a.eq_ignore_ascii_case(&email) && !out.iter().any(|(x, _)| x.eq_ignore_ascii_case(&a)) {
                out.push((a, n));
            }
        }
        out.truncate(MAX_MAILBOXES);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alt(smtp: Option<&str>, name: &str, dn: Option<&str>) -> AltMailbox {
        AltMailbox { smtp: smtp.map(str::to_string), name: name.to_string(), legacy_dn: dn.map(str::to_string) }
    }

    #[test]
    fn alternative_mailboxes_from_autodiscover() {
        let ad = r#"<?xml version="1.0"?><Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/responseschema/2006"><Response xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a"><Account><Protocol><Type>EXCH</Type><EwsUrl>https://mail.corp.ru/EWS/Exchange.asmx</EwsUrl></Protocol><AlternativeMailbox><Type>Archive</Type><DisplayName>Архив</DisplayName><SmtpAddress>archive@corp.ru</SmtpAddress></AlternativeMailbox><AlternativeMailbox><Type>Delegate</Type><DisplayName>Канцелярия</DisplayName><LegacyDN>/o=corp/ou=x</LegacyDN><Server>mbx1</Server><SmtpAddress>Office@Corp.ru</SmtpAddress></AlternativeMailbox><AlternativeMailbox><Type>Delegate</Type><DisplayName>Я</DisplayName><SmtpAddress>me@corp.ru</SmtpAddress></AlternativeMailbox><AlternativeMailbox><Type>TeamMailbox</Type><OwnerSmtpAddress>team@corp.ru</OwnerSmtpAddress></AlternativeMailbox><AlternativeMailbox><Type>Delegate</Type><DisplayName>Дубль</DisplayName><SmtpAddress>office@corp.ru</SmtpAddress></AlternativeMailbox><AlternativeMailbox><Type>Delegate</Type><DisplayName>Бухгалтерия</DisplayName><LegacyDN>/o=corp/ou=Exchange/cn=Recipients/cn=buh</LegacyDN><Server>mbx1</Server></AlternativeMailbox><AlternativeMailbox><Type>Delegate</Type><DisplayName>Бухгалтерия 2</DisplayName><LegacyDN>/O=CORP/ou=Exchange/cn=Recipients/cn=BUH</LegacyDN></AlternativeMailbox></Account></Response></Autodiscover>"#;
        assert_eq!(
            alternative_mailboxes(ad, "me@corp.ru"),
            vec![
                alt(Some("office@corp.ru"), "Канцелярия", Some("/o=corp/ou=x")),
                alt(Some("team@corp.ru"), "team@corp.ru", None),
                // Exchange в своей сети: без адреса, только LegacyDN — адрес найдётся в адресной книге.
                alt(None, "Бухгалтерия", Some("/o=corp/ou=Exchange/cn=Recipients/cn=buh")),
            ]
        );
        assert!(alternative_mailboxes("not xml", "me@corp.ru").is_empty());
    }

    #[test]
    fn autodiscover_redirects() {
        let r = |action: &str, extra: &str| format!(r#"<?xml version="1.0"?><Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/responseschema/2006"><Response xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a"><Account><AccountType>email</AccountType><Action>{action}</Action>{extra}</Account></Response></Autodiscover>"#);
        assert_eq!(autodiscover_redirect(&r("redirectAddr", "<RedirectAddr>Ivanov@Corp.ru</RedirectAddr>")), Some(Redirect::Addr("ivanov@corp.ru".into())));
        assert_eq!(autodiscover_redirect(&r("redirectUrl", "<RedirectUrl>https://mbx.corp.ru/autodiscover/autodiscover.xml</RedirectUrl>")), Some(Redirect::Url("https://mbx.corp.ru/autodiscover/autodiscover.xml".into())));
        assert_eq!(autodiscover_redirect(&r("settings", "")), None);
        assert_eq!(autodiscover_redirect("oops"), None);
    }

    #[test]
    fn discovery_asks_the_ews_server_first() {
        let ews = Url::parse("https://mail.corp.ru:8443/EWS/Exchange.asmx").unwrap();
        assert_eq!(
            discovery_candidates("corp.ru", &ews),
            vec!["https://mail.corp.ru:8443/autodiscover/autodiscover.xml", "https://autodiscover.corp.ru/autodiscover/autodiscover.xml", "https://corp.ru/autodiscover/autodiscover.xml"]
        );
        let same = Url::parse("https://corp.ru/EWS/Exchange.asmx").unwrap();
        assert_eq!(discovery_candidates("corp.ru", &same).len(), 2);
        let plain = Url::parse("http://mail.corp.ru/EWS/Exchange.asmx").unwrap();
        assert_eq!(discovery_candidates("corp.ru", &plain).len(), 2);
    }
}
