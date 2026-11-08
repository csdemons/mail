//! История взаимодействия с контрагентом — организацией, которую определяет
//! домен почтового адреса (`ihep.ru`; адреса поддоменов, например `noc.ihep.ru`,
//! относятся к той же организации).
//!
//! Шкала собирается из того, что уже есть у программы: письма (индекс на
//! компьютере и, по запросу, поиск на сервере), их вложения, встречи календаря
//! с участниками из этого домена, контакты адресной книги. Своё у модуля только
//! «досье» организации: название, заметка и договорённости (что, когда, срок,
//! выполнено ли, из какого письма). Досье хранится на компьютере
//! (`counterparties.json`, доступ только у пользователя), отдельно для каждой
//! учётной записи, и уходит вместе с ней.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::imap::MessageSummary;

/// Домен из адреса: `Иванов <ivanov@IHEP.ru>` → `ihep.ru`.
pub fn domain_of(addr: &str) -> Option<String> {
    let addr = addr.trim();
    let addr = match (addr.rfind('<'), addr.rfind('>')) {
        (Some(o), Some(c)) if o < c => &addr[o + 1..c],
        _ => addr,
    };
    let (_, dom) = addr.trim().rsplit_once('@')?;
    normalize(dom)
}

/// Адрес без имени: `Иванов <ivanov@ihep.ru>` → `ivanov@ihep.ru`.
pub fn bare(addr: &str) -> String {
    let a = addr.trim();
    match (a.rfind('<'), a.rfind('>')) {
        (Some(o), Some(c)) if o < c => a[o + 1..c].trim().to_lowercase(),
        _ => a.to_lowercase(),
    }
}

/// Имя из `Иванов И. <ivanov@ihep.ru>` (пусто, если имени нет).
fn display_name(addr: &str) -> String {
    let a = addr.trim();
    match a.rfind('<') {
        Some(o) if o > 0 => a[..o].trim().trim_matches('"').to_string(),
        _ => String::new(),
    }
}

/// Домен, введённый пользователем: `@IHEP.ru ` → `ihep.ru`. `None` — не домен.
pub fn normalize(dom: &str) -> Option<String> {
    let d = dom.trim().trim_start_matches('@').trim_end_matches('.').to_lowercase();
    let ok = d.len() <= 253
        && d.contains('.')
        && d.split('.').all(|l| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-'))
        && d.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-');
    ok.then_some(d)
}

/// Адрес относится к организации `dom` (сам домен или его поддомен).
pub fn in_domain(addr: &str, dom: &str) -> bool {
    domain_of(addr).is_some_and(|d| d == dom || d.ends_with(&format!(".{dom}")))
}

/// Письмо касается организации: от неё или ей (среди получателей).
pub fn concerns(m: &MessageSummary, dom: &str) -> bool {
    in_domain(&m.from_address, dom) || m.to.iter().any(|a| in_domain(a, dom))
}

fn timestamp(m: &MessageSummary) -> i64 {
    m.date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map_or(0, |d| d.timestamp())
}

/// Организация в списке контрагентов.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Counterparty {
    pub domain: String,
    /// Название из досье, если пользователь его задал.
    pub name: Option<String>,
    pub messages: u32,
    /// Письма от организации.
    pub incoming: u32,
    /// Письма организации (она среди получателей, отправитель — не она).
    pub outgoing: u32,
    /// Последнее письмо (RFC 3339).
    pub last: Option<String>,
    /// Домен своей учётной записи — переписка с коллегами.
    pub own: bool,
    /// Открытые договорённости из досье.
    pub open_agreements: u32,
}

/// Организации из писем: для каждого письма — домены «другой стороны»
/// (отправитель; для своих писем — получатели). Сначала с самой свежей перепиской.
pub fn list(messages: &[MessageSummary], own_address: &str, dossiers: &BTreeMap<String, Dossier>) -> Vec<Counterparty> {
    let own_domain = domain_of(own_address).unwrap_or_default();
    let own_addr = bare(own_address);
    #[derive(Default)]
    struct Acc {
        messages: u32,
        incoming: u32,
        outgoing: u32,
        last: i64,
        last_s: Option<String>,
    }
    let mut acc: HashMap<String, Acc> = HashMap::new();
    for m in messages {
        let from_me = bare(&m.from_address) == own_addr;
        let mut doms: Vec<(String, bool)> = Vec::new();
        if !from_me {
            if let Some(d) = domain_of(&m.from_address) {
                doms.push((d, true));
            }
        }
        for a in &m.to {
            if bare(a) == own_addr {
                continue;
            }
            if let Some(d) = domain_of(a) {
                if !doms.iter().any(|(x, _)| *x == d) {
                    doms.push((d, false));
                }
            }
        }
        let ts = timestamp(m);
        for (d, incoming) in doms {
            let e = acc.entry(d).or_default();
            e.messages += 1;
            if incoming {
                e.incoming += 1;
            } else {
                e.outgoing += 1;
            }
            if ts >= e.last {
                e.last = ts;
                e.last_s = m.date.clone();
            }
        }
    }
    // Организации, для которых заведено досье, видны и без писем на компьютере.
    for d in dossiers.keys() {
        acc.entry(d.clone()).or_default();
    }
    let mut out: Vec<(i64, Counterparty)> = acc
        .into_iter()
        .map(|(domain, a)| {
            let dossier = dossiers.get(&domain);
            let c = Counterparty {
                own: domain == own_domain,
                name: dossier.and_then(|d| d.name.clone()).filter(|n| !n.trim().is_empty()),
                open_agreements: dossier.map_or(0, |d| d.agreements.iter().filter(|a| !a.done).count() as u32),
                domain,
                messages: a.messages,
                incoming: a.incoming,
                outgoing: a.outgoing,
                last: a.last_s,
            };
            (a.last, c)
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.domain.cmp(&b.1.domain)));
    out.into_iter().map(|(_, c)| c).collect()
}

/// Человек из организации: с кем шла переписка.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub address: String,
    pub name: String,
    pub messages: u32,
    pub last: Option<String>,
}

/// Собеседники из организации по письмам; самые активные сверху.
pub fn people(messages: &[MessageSummary], dom: &str) -> Vec<Person> {
    let mut map: HashMap<String, (Person, i64)> = HashMap::new();
    for m in messages {
        let ts = timestamp(m);
        let from = if m.from_name.trim().is_empty() {
            m.from_address.clone()
        } else {
            format!("{} <{}>", m.from_name.trim(), m.from_address.trim())
        };
        let mut seen: Vec<String> = Vec::new();
        for a in std::iter::once(&from).chain(m.to.iter()) {
            if !in_domain(a, dom) {
                continue;
            }
            let address = bare(a);
            if seen.contains(&address) {
                continue;
            }
            seen.push(address.clone());
            let name = display_name(a);
            let e = map.entry(address.clone()).or_insert_with(|| (Person { address, name: String::new(), messages: 0, last: None }, i64::MIN));
            e.0.messages += 1;
            if e.0.name.is_empty() && !name.is_empty() {
                e.0.name = name;
            }
            if ts >= e.1 {
                e.1 = ts;
                e.0.last = m.date.clone();
            }
        }
    }
    let mut out: Vec<Person> = map.into_values().map(|(p, _)| p).collect();
    out.sort_by(|a, b| b.messages.cmp(&a.messages).then_with(|| b.last.cmp(&a.last)).then_with(|| a.address.cmp(&b.address)));
    out
}

// ---------------------------------------------------------------- досье

/// Договорённость с организацией.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Agreement {
    pub id: String,
    /// Когда договорились: `ГГГГ-ММ-ДД`.
    pub date: String,
    pub text: String,
    /// Срок выполнения: `ГГГГ-ММ-ДД`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(default)]
    pub done: bool,
    /// Письмо, в котором договорились (папка, UID, тема — для перехода к нему).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

/// Досье организации.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dossier {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agreements: Vec<Agreement>,
}

impl Dossier {
    fn is_empty(&self) -> bool {
        self.name.as_deref().is_none_or(|n| n.trim().is_empty()) && self.note.trim().is_empty() && self.agreements.is_empty()
    }
}

const MAX_AGREEMENTS: usize = 500;
const MAX_TEXT: usize = 4000;

fn valid_date(d: &str) -> bool {
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok()
}

pub fn validate(d: &Dossier) -> Result<()> {
    let bad = |m: &str| Err(AppError::Other(m.to_string()));
    if d.name.as_deref().is_some_and(|n| n.chars().count() > 200) {
        return bad("Название организации длиннее 200 символов");
    }
    if d.note.chars().count() > MAX_TEXT * 4 {
        return bad("Заметка слишком длинная");
    }
    if d.agreements.len() > MAX_AGREEMENTS {
        return bad("Слишком много договорённостей (не больше 500)");
    }
    let mut ids = std::collections::HashSet::new();
    for a in &d.agreements {
        if a.id.is_empty() || a.id.len() > 64 || !ids.insert(a.id.as_str()) {
            return bad("Некорректный идентификатор договорённости");
        }
        if a.text.trim().is_empty() {
            return bad("Договорённость без текста");
        }
        if a.text.chars().count() > MAX_TEXT {
            return bad("Текст договорённости длиннее 4000 символов");
        }
        if !valid_date(&a.date) || a.due.as_deref().is_some_and(|d| !valid_date(d)) {
            return bad("Дата договорённости — в формате ГГГГ-ММ-ДД");
        }
        if a.subject.as_deref().is_some_and(|s| s.chars().count() > 1000) || a.folder.as_deref().is_some_and(|f| f.len() > 1000) {
            return bad("Слишком длинная ссылка на письмо");
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Default)]
struct DossierFile {
    #[serde(default)]
    accounts: BTreeMap<String, BTreeMap<String, Dossier>>,
}

fn file(dir: &Path) -> std::path::PathBuf {
    dir.join("counterparties.json")
}

fn read(dir: &Path) -> DossierFile {
    std::fs::read(file(dir)).ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default()
}

fn write(dir: &Path, all: &DossierFile) -> Result<()> {
    let raw = serde_json::to_vec_pretty(all).map_err(|e| AppError::Other(e.to_string()))?;
    crate::settings::write_private(&file(dir), &raw)
}

/// Все досье учётной записи.
pub fn dossiers(dir: &Path, account: &str) -> BTreeMap<String, Dossier> {
    read(dir).accounts.remove(account).unwrap_or_default()
}

pub fn dossier(dir: &Path, account: &str, domain: &str) -> Dossier {
    dossiers(dir, account).remove(domain).unwrap_or_default()
}

pub fn save(dir: &Path, account: &str, domain: &str, d: Dossier) -> Result<()> {
    validate(&d)?;
    let mut all = read(dir);
    let mine = all.accounts.entry(account.to_string()).or_default();
    if d.is_empty() {
        mine.remove(domain);
    } else {
        mine.insert(domain.to_string(), d);
    }
    if mine.is_empty() {
        all.accounts.remove(account);
    }
    write(dir, &all)
}

/// Досье учётной записи убираются вместе с ней.
pub fn forget(dir: &Path, account: &str) {
    let mut all = read(dir);
    if all.accounts.remove(account).is_some() {
        let _ = write(dir, &all);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(from: &str, to: &[&str], date: &str) -> MessageSummary {
        MessageSummary {
            folder: "INBOX".into(),
            uid: 1,
            from_name: String::new(),
            from_address: from.into(),
            to: to.iter().map(|s| s.to_string()).collect(),
            subject: "s".into(),
            date: Some(date.into()),
            unread: false,
            flagged: false,
            important: false,
            answered: false,
            keywords: vec![],
            message_id: None,
            thread: vec![],
            has_attachments: false,
            attachments: vec![],
            size: 1,
            snippet: None,
        }
    }

    #[test]
    fn domains() {
        assert_eq!(domain_of("Иванов <ivanov@IHEP.ru>").as_deref(), Some("ihep.ru"));
        assert_eq!(domain_of("noc@mail.ihep.ru").as_deref(), Some("mail.ihep.ru"));
        assert_eq!(domain_of("без адреса"), None);
        assert!(in_domain("a@noc.ihep.ru", "ihep.ru"));
        assert!(!in_domain("a@notihep.ru", "ihep.ru"));
        assert_eq!(normalize(" @IHEP.ru. ").as_deref(), Some("ihep.ru"));
        assert_eq!(normalize("ihep"), None);
        assert_eq!(normalize("ih ep.ru"), None);
        assert_eq!(normalize("../etc"), None);
        assert_eq!(bare("Иванов <Ivanov@ihep.ru>"), "ivanov@ihep.ru");
    }

    #[test]
    fn counterparties_and_people() {
        let me = "yushkov_do@inst.test";
        let msgs = vec![
            m("petrov@ihep.ru", &["Юшков <yushkov_do@inst.test>"], "2026-10-01T10:00:00+03:00"),
            m("yushkov_do@inst.test", &["Петров П. <petrov@ihep.ru>", "sidorov@jinr.ru"], "2026-10-05T10:00:00+03:00"),
            m("kolya@inst.test", &["yushkov_do@inst.test"], "2026-09-01T10:00:00+03:00"),
            m("noc@lab.ihep.ru", &["yushkov_do@inst.test"], "2026-08-01T10:00:00+03:00"),
        ];
        let mut dossiers = BTreeMap::new();
        dossiers.insert("cern.ch".to_string(), Dossier { name: Some("ЦЕРН".into()), ..Default::default() });
        let list = list(&msgs, me, &dossiers);
        let names: Vec<&str> = list.iter().map(|c| c.domain.as_str()).collect();
        assert_eq!(names, ["ihep.ru", "jinr.ru", "inst.test", "lab.ihep.ru", "cern.ch"]);
        let ihep = &list[0];
        assert_eq!((ihep.messages, ihep.incoming, ihep.outgoing), (2, 1, 1));
        assert!(list[2].own && !ihep.own);
        assert_eq!(list[4].name.as_deref(), Some("ЦЕРН"));

        let people = people(&msgs, "ihep.ru");
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].address, "petrov@ihep.ru");
        assert_eq!(people[0].messages, 2);
        assert_eq!(people[0].name, "Петров П.");
        assert_eq!(people[1].address, "noc@lab.ihep.ru");
        assert!(concerns(&msgs[1], "jinr.ru") && !concerns(&msgs[2], "ihep.ru"));
    }

    #[test]
    fn dossier_storage() {
        let dir = std::env::temp_dir().join(format!("cp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = Agreement {
            id: "a1".into(),
            date: "2026-11-01".into(),
            text: "Поставка до конца года".into(),
            due: Some("2026-12-31".into()),
            done: false,
            folder: Some("INBOX".into()),
            uid: Some(5),
            subject: Some("Договор".into()),
        };
        let d = Dossier { name: Some("ИФВЭ".into()), note: "Партнёр".into(), agreements: vec![a.clone()] };
        save(&dir, "u@h", "ihep.ru", d.clone()).unwrap();
        assert_eq!(dossier(&dir, "u@h", "ihep.ru"), d);
        assert_eq!(dossier(&dir, "other@h", "ihep.ru"), Dossier::default());
        let bad = Dossier { agreements: vec![Agreement { date: "01.11.2026".into(), ..a.clone() }], ..Default::default() };
        assert!(save(&dir, "u@h", "ihep.ru", bad).is_err());
        let dup = Dossier { agreements: vec![a.clone(), a.clone()], ..Default::default() };
        assert!(save(&dir, "u@h", "ihep.ru", dup).is_err());
        save(&dir, "u@h", "ihep.ru", Dossier::default()).unwrap();
        assert!(dossiers(&dir, "u@h").is_empty());
        save(&dir, "u@h", "ihep.ru", d).unwrap();
        forget(&dir, "u@h");
        assert!(dossiers(&dir, "u@h").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
