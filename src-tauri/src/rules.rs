//! Правила обработки почты, которые выполняет программа.
//!
//! Правила, которые умеет сервер (Sieve в SOGo: отправитель, тема, текст, размер,
//! заголовок → переместить, пометить, перенаправить…), хранятся на сервере и
//! работают всегда. Программа хранит у себя (`rules.json`, отдельно для каждой
//! учётной записи):
//! - **правила программы** — с условиями, которых нет в Sieve (имя вложения, есть
//!   вложение); выполняются, пока Atom Mail запущен;
//! - **дополнения к серверным правилам** — действия, которых нет в Sieve
//!   (уведомить, создать задачу в Deck, своя категория): сервер, например,
//!   перемещает письмо, а программа по тому же правилу показывает уведомление.
//!
//! Формат правила — как у фильтров SOGo (`name`, `match`, `rules`, `actions`).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::imap::MessageSummary;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Cond {
    pub field: String,
    pub operator: String,
    #[serde(default)]
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_header: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Act {
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    #[serde(default = "yes")]
    pub active: serde_json::Value,
    #[serde(rename = "match")]
    pub match_: String,
    #[serde(default)]
    pub rules: Vec<Cond>,
    #[serde(default)]
    pub actions: Vec<Act>,
}

fn yes() -> serde_json::Value {
    serde_json::Value::from(1)
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RuleSet {
    /// Правила, которые целиком выполняет программа.
    #[serde(default)]
    pub program: Vec<Rule>,
    /// Действия программы к серверным правилам: имя фильтра SOGo → действия.
    #[serde(default)]
    pub ext: BTreeMap<String, Vec<Act>>,
}

impl Rule {
    pub fn is_active(&self) -> bool {
        match &self.active {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::Number(n) => n.as_i64() != Some(0),
            serde_json::Value::String(s) => s != "0" && !s.is_empty(),
            _ => true,
        }
    }

    /// Нужен текст письма (условие по тексту).
    pub fn needs_text(&self) -> bool {
        self.match_ != "allmessages" && self.rules.iter().any(|c| c.field == "body")
    }
}

/// Поля условий, которые проверяет программа.
const FIELDS: &[&str] = &["from", "to", "cc", "to_or_cc", "subject", "body", "size", "attachment", "has_attachment", "domain"];
const TEXT_OPS: &[&str] = &["contains", "not_contains", "is", "is_not", "matches", "not_matches"];
/// Действия правил программы.
const PROGRAM_ACTIONS: &[&str] = &["fileinto", "addflag", "category", "notify", "task", "stop"];
/// Действия-дополнения к серверным правилам.
const EXT_ACTIONS: &[&str] = &["category", "notify", "task"];
/// Пометки, которые ставит программа.
pub const FLAGS: &[&str] = &["seen", "flagged", "junk", "not_junk", "label1", "label2", "label3", "label4", "label5"];

fn short(s: &str, max: usize) -> bool {
    s.chars().count() <= max && !s.chars().any(|c| c.is_control() && c != '\t')
}

fn check_action(a: &Act, allowed: &[&str], rule: &str) -> Result<()> {
    let bad = || AppError::Other(format!("Правило «{rule}»: некорректное действие «{}»", a.method));
    if !allowed.contains(&a.method.as_str()) {
        return Err(bad());
    }
    let arg = a.argument.as_deref().unwrap_or("");
    let ok = match a.method.as_str() {
        "fileinto" => !arg.trim().is_empty() && short(arg, 500),
        "addflag" => FLAGS.contains(&arg),
        "category" => crate::imap::valid_keyword(arg),
        "task" => arg.split_once(':').is_some_and(|(b, s)| b.parse::<i64>().is_ok() && s.parse::<i64>().is_ok()),
        _ => true,
    };
    if ok { Ok(()) } else { Err(bad()) }
}

pub fn validate(set: &RuleSet) -> Result<()> {
    if set.program.len() > 100 || set.ext.len() > 200 {
        return Err(AppError::Other("Слишком много правил".into()));
    }
    for r in &set.program {
        if r.name.trim().is_empty() || !short(&r.name, 200) {
            return Err(AppError::Other("У правила должно быть название".into()));
        }
        if !matches!(r.match_.as_str(), "all" | "any" | "allmessages") {
            return Err(AppError::Other(format!("Правило «{}»: условие «все/любое/все письма»", r.name)));
        }
        if r.match_ != "allmessages" && r.rules.is_empty() {
            return Err(AppError::Other(format!("Правило «{}»: нет условий", r.name)));
        }
        for c in &r.rules {
            let op_ok = match c.field.as_str() {
                "size" => matches!(c.operator.as_str(), "over" | "under") && c.value.trim().parse::<u64>().is_ok(),
                "has_attachment" => true,
                f if FIELDS.contains(&f) => TEXT_OPS.contains(&c.operator.as_str()) && !c.value.trim().is_empty(),
                _ => false,
            };
            if !op_ok || !short(&c.value, 500) {
                return Err(AppError::Other(format!("Правило «{}»: условие «{}» не поддерживается программой", r.name, c.field)));
            }
        }
        if r.actions.is_empty() || r.actions.len() > 10 {
            return Err(AppError::Other(format!("Правило «{}»: нужно от одного до десяти действий", r.name)));
        }
        for a in &r.actions {
            check_action(a, PROGRAM_ACTIONS, &r.name)?;
        }
    }
    for (name, acts) in &set.ext {
        if name.trim().is_empty() || !short(name, 200) || acts.is_empty() || acts.len() > 10 {
            return Err(AppError::Other("Некорректное дополнение к правилу".into()));
        }
        for a in acts {
            check_action(a, EXT_ACTIONS, name)?;
        }
    }
    Ok(())
}

/// Сравнение по шаблону с `*` и `?` (без учёта регистра).
fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

fn text_op(op: &str, hay: &str, value: &str) -> bool {
    let hay = hay.to_lowercase();
    let v = value.trim().to_lowercase();
    match op {
        "contains" => hay.contains(&v),
        "not_contains" => !hay.contains(&v),
        "is" => hay.trim() == v,
        "is_not" => hay.trim() != v,
        "matches" => glob(&v, hay.trim()),
        "not_matches" => !glob(&v, hay.trim()),
        _ => false,
    }
}

fn domain_of(addr: &str) -> String {
    let a = addr.trim().trim_end_matches('>');
    let a = a.rsplit('<').next().unwrap_or(a);
    a.rsplit_once('@').map(|(_, d)| d.to_lowercase()).unwrap_or_default()
}

/// Подходит ли письмо под условие. `text` — текст письма (для условия по тексту).
pub fn cond_matches(c: &Cond, s: &MessageSummary, text: Option<&str>) -> bool {
    let from = format!("{} <{}>", s.from_name, s.from_address);
    match c.field.as_str() {
        "from" => text_op(&c.operator, &from, &c.value),
        "to" | "cc" | "to_or_cc" => {
            let all = s.to.join(", ");
            // «не содержит» — для всего списка; остальные — хотя бы один адресат.
            if c.operator.starts_with("not_") || c.operator == "is_not" { text_op(&c.operator, &all, &c.value) } else { s.to.iter().any(|a| text_op(&c.operator, a, &c.value)) || text_op(&c.operator, &all, &c.value) }
        }
        "subject" => text_op(&c.operator, &s.subject, &c.value),
        "body" => text.is_some_and(|t| text_op(&c.operator, t, &c.value)),
        "attachment" => {
            if c.operator.starts_with("not_") || c.operator == "is_not" {
                s.attachments.iter().all(|a| text_op(&c.operator, a, &c.value))
            } else {
                s.attachments.iter().any(|a| text_op(&c.operator, a, &c.value))
            }
        }
        "has_attachment" => s.has_attachments,
        "domain" => {
            let d = domain_of(&s.from_address);
            let v = c.value.trim().trim_start_matches('@');
            match c.operator.as_str() {
                "is" | "contains" => d == v.to_lowercase() || d.ends_with(&format!(".{}", v.to_lowercase())),
                "is_not" | "not_contains" => d != v.to_lowercase() && !d.ends_with(&format!(".{}", v.to_lowercase())),
                op => text_op(op, &d, v),
            }
        }
        "size" => {
            let kb = u64::from(s.size) / 1024;
            let v = c.value.trim().parse::<u64>().unwrap_or(0);
            match c.operator.as_str() {
                "over" => kb > v,
                "under" => kb < v,
                _ => false,
            }
        }
        _ => false,
    }
}

pub fn rule_matches(r: &Rule, s: &MessageSummary, text: Option<&str>) -> bool {
    match r.match_.as_str() {
        "allmessages" => true,
        "any" => r.rules.iter().any(|c| cond_matches(c, s, text)),
        _ => !r.rules.is_empty() && r.rules.iter().all(|c| cond_matches(c, s, text)),
    }
}

// ---------------------------------------------------------------- хранение

#[derive(Serialize, Deserialize, Default)]
struct RulesFile {
    #[serde(default)]
    accounts: BTreeMap<String, RuleSet>,
}

fn file(dir: &Path) -> std::path::PathBuf {
    dir.join("rules.json")
}

fn read(dir: &Path) -> RulesFile {
    std::fs::read(file(dir)).ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default()
}

pub fn load(dir: &Path, account: &str) -> RuleSet {
    read(dir).accounts.remove(account).unwrap_or_default()
}

pub fn save(dir: &Path, account: &str, set: RuleSet) -> Result<()> {
    validate(&set)?;
    let mut all = read(dir);
    if set == RuleSet::default() {
        all.accounts.remove(account);
    } else {
        all.accounts.insert(account.to_string(), set);
    }
    let raw = serde_json::to_vec_pretty(&all).map_err(|e| AppError::Other(e.to_string()))?;
    crate::settings::write_private(&file(dir), &raw)
}

/// Правила учётной записи убираются вместе с ней.
pub fn forget(dir: &Path, account: &str) {
    let mut all = read(dir);
    if all.accounts.remove(account).is_some() {
        if let Ok(raw) = serde_json::to_vec_pretty(&all) {
            let _ = crate::settings::write_private(&file(dir), &raw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg() -> MessageSummary {
        MessageSummary {
            folder: "INBOX".into(),
            uid: 7,
            from_name: "Мониторинг".into(),
            from_address: "zabbix@noc.nrcki.ru".into(),
            to: vec!["Юшков Дмитрий <yushkov_do@nrcki.ru>".into()],
            subject: "PROBLEM: диск заполнен на srv-12".into(),
            date: None,
            unread: true,
            flagged: false,
            important: false,
            answered: false,
            keywords: vec![],
            message_id: None,
            thread: vec![],
            has_attachments: true,
            attachments: vec!["отчёт за октябрь.xlsx".into()],
            size: 300 * 1024,
        }
    }

    fn c(field: &str, op: &str, v: &str) -> Cond {
        Cond { field: field.into(), operator: op.into(), value: v.into(), custom_header: None }
    }

    #[test]
    fn conditions() {
        let m = msg();
        assert!(cond_matches(&c("from", "contains", "ZABBIX"), &m, None));
        assert!(cond_matches(&c("subject", "matches", "problem:*srv-??"), &m, None));
        assert!(!cond_matches(&c("subject", "not_contains", "диск"), &m, None));
        assert!(cond_matches(&c("attachment", "contains", ".xlsx"), &m, None));
        assert!(!cond_matches(&c("attachment", "contains", ".pdf"), &m, None));
        assert!(cond_matches(&c("has_attachment", "is", ""), &m, None));
        assert!(cond_matches(&c("domain", "is", "nrcki.ru"), &m, None), "subdomain counts");
        assert!(cond_matches(&c("domain", "is", "@noc.nrcki.ru"), &m, None));
        assert!(!cond_matches(&c("domain", "is", "ihep.ru"), &m, None));
        assert!(cond_matches(&c("to", "contains", "yushkov"), &m, None));
        assert!(cond_matches(&c("size", "over", "100"), &m, None) && !cond_matches(&c("size", "under", "100"), &m, None));
        assert!(cond_matches(&c("body", "contains", "срочно"), &m, Some("Это срочно!")));
        assert!(!cond_matches(&c("body", "contains", "срочно"), &m, None), "no text — no match");
        let r = Rule { name: "Мониторинг".into(), active: yes(), match_: "all".into(), rules: vec![c("from", "contains", "zabbix"), c("subject", "contains", "PROBLEM")], actions: vec![] };
        assert!(rule_matches(&r, &m, None));
        let any = Rule { match_: "any".into(), rules: vec![c("from", "is", "x"), c("has_attachment", "is", "")], ..r.clone() };
        assert!(rule_matches(&any, &m, None));
        assert!(!Rule { active: serde_json::Value::from(0), ..r }.is_active());
    }

    #[test]
    fn validation() {
        let ok = Rule {
            name: "Отчёты".into(),
            active: yes(),
            match_: "all".into(),
            rules: vec![c("attachment", "contains", "отчёт")],
            actions: vec![
                Act { method: "fileinto".into(), argument: Some("INBOX/Отчёты".into()) },
                Act { method: "notify".into(), argument: None },
                Act { method: "task".into(), argument: Some("1:10".into()) },
                Act { method: "category".into(), argument: Some("$label4".into()) },
            ],
        };
        let mut set = RuleSet { program: vec![ok.clone()], ext: BTreeMap::new() };
        assert!(validate(&set).is_ok());
        set.ext.insert("Рассылка".into(), vec![Act { method: "notify".into(), argument: None }]);
        assert!(validate(&set).is_ok());
        // Пересылку и удаление программа не выполняет — это только для сервера.
        let redirect = Rule { actions: vec![Act { method: "redirect".into(), argument: Some("a@b.ru".into()) }], ..ok.clone() };
        assert!(validate(&RuleSet { program: vec![redirect], ..Default::default() }).is_err());
        let bad_task = Rule { actions: vec![Act { method: "task".into(), argument: Some("доска".into()) }], ..ok.clone() };
        assert!(validate(&RuleSet { program: vec![bad_task], ..Default::default() }).is_err());
        let bad_kw = Rule { actions: vec![Act { method: "category".into(), argument: Some("a b".into()) }], ..ok.clone() };
        assert!(validate(&RuleSet { program: vec![bad_kw], ..Default::default() }).is_err());
        let ext_move = RuleSet { ext: BTreeMap::from([("x".to_string(), vec![Act { method: "fileinto".into(), argument: Some("Spam".into()) }])]), ..Default::default() };
        assert!(validate(&ext_move).is_err(), "server moves; extensions only notify/task/category");
        let header = Rule { rules: vec![Cond { field: "header".into(), operator: "contains".into(), value: "x".into(), custom_header: Some("List-Id".into()) }], ..ok };
        assert!(validate(&RuleSet { program: vec![header], ..Default::default() }).is_err());
    }

    #[test]
    fn storage_per_account() {
        let dir = std::env::temp_dir().join(format!("rules-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let set = RuleSet { program: vec![], ext: BTreeMap::from([("Рассылка".to_string(), vec![Act { method: "notify".into(), argument: None }])]) };
        save(&dir, "a@x", set.clone()).unwrap();
        save(&dir, "b@x", RuleSet::default()).unwrap();
        assert_eq!(load(&dir, "a@x"), set);
        assert_eq!(load(&dir, "b@x"), RuleSet::default());
        forget(&dir, "a@x");
        assert_eq!(load(&dir, "a@x"), RuleSet::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
