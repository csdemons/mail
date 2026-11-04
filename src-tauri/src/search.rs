//! Язык поисковых запросов: свободный текст и операторы. Один разбор — для
//! индекса на компьютере и для поиска на сервере (IMAP SEARCH).
//!
//! Операторы (русские и английские имена равноправны):
//! - `от:` / `from:` — отправитель (имя или адрес);
//! - `кому:` / `to:` — получатели (Кому и Копия);
//! - `тема:` / `subject:`;
//! - `вложение:` / `attachment:` — имя файла во вложениях;
//! - `есть:вложение` / `has:attachment`;
//! - `непрочитанные`, `важные`, `флажок`, `отвеченные` (или `is:unread`, `is:important`,
//!   `is:flagged`, `is:answered`);
//! - `после:` / `after:` и `до:` / `before:` — дата `ГГГГ-ММ-ДД` или `ДД.ММ.ГГГГ`;
//! - `папка:` / `in:` — часть имени папки;
//! - `ящик:` / `account:` — часть адреса учётной записи;
//! - `подразделение:` / `domain:` — домен адреса собеседника (отправителя или получателя).
//!
//! Значение в кавычках может содержать пробелы: `тема:"план работ"`.

use chrono::NaiveDate;

use crate::imap::MessageSummary;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Query {
    pub words: Vec<String>,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub subject: Vec<String>,
    pub attachment: Vec<String>,
    pub has_attachment: bool,
    pub unread: bool,
    pub important: bool,
    pub flagged: bool,
    pub answered: bool,
    pub after: Option<NaiveDate>,
    pub before: Option<NaiveDate>,
    pub folder: Option<String>,
    pub account: Option<String>,
    pub domain: Option<String>,
}

/// Слова запроса с учётом кавычек: `тема:"план работ" отчёт` → [`тема:план работ`, `отчёт`].
fn tokens(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' | '«' | '»' => quoted = !quoted && c != '»',
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c if c.is_control() => {}
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Дата: `2026-11-01`, `01.11.2026` или относительная — `7д`/`7d` (7 дней назад), `сегодня`, `вчера`.
fn date(v: &str) -> Option<NaiveDate> {
    let today = chrono::Local::now().date_naive();
    match v {
        "сегодня" | "today" => return Some(today),
        "вчера" | "yesterday" => return today.pred_opt(),
        _ => {}
    }
    if let Some(n) = v.strip_suffix('д').or_else(|| v.strip_suffix('d')).and_then(|n| n.parse::<i64>().ok()) {
        return today.checked_sub_signed(chrono::Duration::days(n.clamp(0, 36500)));
    }
    NaiveDate::parse_from_str(v, "%Y-%m-%d").or_else(|_| NaiveDate::parse_from_str(v, "%d.%m.%Y")).ok()
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut q = Query::default();
        for t in tokens(input).into_iter().take(20) {
            let lower = t.to_lowercase();
            let flag = match lower.as_str() {
                "непрочитанные" | "непрочитанное" | "is:unread" => Some(&mut q.unread),
                "важные" | "важное" | "is:important" => Some(&mut q.important),
                "флажок" | "с флажком" | "is:flagged" => Some(&mut q.flagged),
                "отвеченные" | "is:answered" => Some(&mut q.answered),
                "есть:вложение" | "есть:вложения" | "has:attachment" | "has:attachments" => Some(&mut q.has_attachment),
                _ => None,
            };
            if let Some(f) = flag {
                *f = true;
                continue;
            }
            let Some((key, value)) = lower.split_once(':').filter(|(k, v)| !k.is_empty() && !v.is_empty()) else {
                q.words.push(lower);
                continue;
            };
            let value = value.trim().to_string();
            match key {
                "от" | "from" => q.from.push(value),
                "кому" | "to" => q.to.push(value),
                "тема" | "subject" => q.subject.push(value),
                "вложение" | "файл" | "attachment" | "file" => {
                    q.attachment.push(value);
                    q.has_attachment = true;
                }
                "после" | "after" | "с" => q.after = date(&value).or(q.after),
                "до" | "before" | "по" => q.before = date(&value).or(q.before),
                "папка" | "in" => q.folder = Some(value),
                "ящик" | "account" => q.account = Some(value),
                "подразделение" | "домен" | "domain" => q.domain = Some(value.trim_start_matches('@').to_string()),
                // Неизвестный оператор (или адрес с двоеточием) — обычный текст.
                _ => q.words.push(lower),
            }
        }
        q
    }

    /// Нужен ли текст запроса длиной хотя бы 2 символа (защита от поиска по «а»).
    pub fn is_meaningful(&self) -> bool {
        let text_ok = self.words.iter().chain(&self.from).chain(&self.to).chain(&self.subject).chain(&self.attachment).any(|w| w.chars().count() >= 2);
        text_ok
            || self.has_attachment
            || self.unread
            || self.important
            || self.flagged
            || self.answered
            || self.after.is_some()
            || self.before.is_some()
            || self.domain.is_some()
    }

    /// Подходит ли папка под `папка:`.
    pub fn folder_ok(&self, path: &str) -> bool {
        let Some(f) = self.folder.as_deref() else { return true };
        let p = path.to_lowercase();
        // Русские названия стандартных папок, которые на сервере могут называться по-английски.
        let alias: &[&str] = match f {
            "входящие" => &["inbox"],
            "отправленные" => &["sent"],
            "черновики" => &["drafts"],
            "корзина" | "удалённые" | "удаленные" => &["trash", "deleted"],
            "спам" => &["spam", "junk"],
            "архив" => &["archive"],
            _ => &[],
        };
        p.contains(f) || alias.iter().any(|a| p.contains(a))
    }

    /// Подходит ли учётная запись под `ящик:`.
    pub fn account_ok(&self, key: &str) -> bool {
        self.account.as_deref().is_none_or(|a| key.to_lowercase().contains(a))
    }

    /// Проверка письма из индекса. `hay` — всё, что ищется свободным текстом
    /// (отправитель, получатели, тема, имена вложений, текст письма) в нижнем регистре.
    pub fn matches(&self, s: &MessageSummary, hay: &str) -> bool {
        let lower = |x: &str| x.to_lowercase();
        let from = lower(&format!("{} {}", s.from_name, s.from_address));
        let to = lower(&s.to.join(" "));
        let subject = lower(&s.subject);
        let files = lower(&s.attachments.join(" "));
        let ts = s.date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map(|d| d.date_naive());
        self.words.iter().all(|w| hay.contains(w.as_str()))
            && self.from.iter().all(|w| from.contains(w.as_str()))
            && self.to.iter().all(|w| to.contains(w.as_str()))
            && self.subject.iter().all(|w| subject.contains(w.as_str()))
            && self.attachment.iter().all(|w| files.contains(w.as_str()))
            && (!self.has_attachment || s.has_attachments)
            && (!self.unread || s.unread)
            && (!self.important || s.important)
            && (!self.flagged || s.flagged)
            && (!self.answered || s.answered)
            && self.after.is_none_or(|a| ts.is_some_and(|d| d >= a))
            && self.before.is_none_or(|b| ts.is_some_and(|d| d < b))
            && self.domain.as_deref().is_none_or(|dom| {
                let ends = |addr: &str| addr.trim().trim_end_matches('>').to_lowercase().ends_with(&format!("@{dom}"));
                // Собеседник из подразделения: отправитель или кто-то из получателей.
                ends(&s.from_address) || s.to.iter().any(|a| ends(a))
            })
    }

    /// Уточнение писем, найденных сервером: то, чего IMAP SEARCH не умеет
    /// (имена вложений, «есть вложение»). Текст сервер уже проверил сам.
    pub fn matches_structure(&self, s: &MessageSummary) -> bool {
        let files = s.attachments.join(" ").to_lowercase();
        (!self.has_attachment || s.has_attachments) && self.attachment.iter().all(|w| files.contains(w.as_str()))
    }

    /// Условия IMAP SEARCH (без CHARSET). `None` — искать на сервере нечего.
    /// Имя вложения ищется как TEXT (заголовки частей письма) и уточняется по
    /// BODYSTRUCTURE уже найденных писем.
    pub fn imap_criteria(&self) -> Option<String> {
        let quote = |w: &str| format!("\"{}\"", w.replace('\\', "\\\\").replace('"', "\\\""));
        let mut c: Vec<String> = Vec::new();
        for w in self.words.iter().take(6) {
            c.push(format!("TEXT {}", quote(w)));
        }
        for w in &self.from {
            c.push(format!("FROM {}", quote(w)));
        }
        for w in &self.to {
            c.push(format!("OR TO {} CC {}", quote(w), quote(w)));
        }
        for w in &self.subject {
            c.push(format!("SUBJECT {}", quote(w)));
        }
        for w in &self.attachment {
            c.push(format!("TEXT {}", quote(w)));
        }
        if let Some(d) = &self.domain {
            c.push(format!("OR FROM {} TO {}", quote(&format!("@{d}")), quote(&format!("@{d}"))));
        }
        if self.unread {
            c.push("UNSEEN".into());
        }
        if self.flagged {
            c.push("FLAGGED".into());
        }
        if self.answered {
            c.push("ANSWERED".into());
        }
        if self.important {
            c.push("OR KEYWORD $label1 KEYWORD $Important".into());
        }
        let imap_date = |d: NaiveDate| d.format("%d-%b-%Y").to_string();
        if let Some(a) = self.after {
            c.push(format!("SINCE {}", imap_date(a)));
        }
        if let Some(b) = self.before {
            c.push(format!("BEFORE {}", imap_date(b)));
        }
        if c.is_empty() {
            if self.has_attachment {
                // Только «есть:вложение» — сервер не умеет; ищем среди всех писем.
                return Some("ALL".into());
            }
            return None;
        }
        Some(c.join(" "))
    }

    /// Нужен CHARSET UTF-8 (есть не-ASCII текст).
    pub fn needs_utf8(&self) -> bool {
        self.words.iter().chain(&self.from).chain(&self.to).chain(&self.subject).chain(&self.attachment).chain(self.domain.iter()).any(|w| !w.is_ascii())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg() -> MessageSummary {
        MessageSummary {
            folder: "INBOX".into(),
            uid: 1,
            from_name: "Петрова Анна".into(),
            from_address: "petrova@ihep.ru".into(),
            to: vec!["Юшков Дмитрий <yushkov_do@inst.test>".into()],
            subject: "План работ на ноябрь".into(),
            date: Some("2026-11-01T10:00:00+03:00".into()),
            unread: true,
            flagged: false,
            important: true,
            answered: false,
            keywords: vec![],
            message_id: None,
            thread: vec![],
            has_attachments: true,
            attachments: vec!["План работ.xlsx".into()],
            size: 100,
        }
    }

    #[test]
    fn parses_operators_in_russian_and_english() {
        let q = Query::parse(r#"от:петрова тема:"план работ" вложение:xlsx важные после:2026-10-01 до:01.12.2026 отчёт"#);
        assert_eq!(q.from, vec!["петрова"]);
        assert_eq!(q.subject, vec!["план работ"]);
        assert_eq!(q.attachment, vec!["xlsx"]);
        assert!(q.has_attachment && q.important && !q.unread);
        assert_eq!(q.after, NaiveDate::from_ymd_opt(2026, 10, 1));
        assert_eq!(q.before, NaiveDate::from_ymd_opt(2026, 12, 1));
        assert_eq!(q.words, vec!["отчёт"]);
        let e = Query::parse("from:Petrova is:unread has:attachment domain:@ihep.ru account:second");
        assert_eq!(e.from, vec!["petrova"]);
        assert!(e.unread && e.has_attachment);
        assert_eq!(e.domain.as_deref(), Some("ihep.ru"));
        assert!(e.account_ok("second@corp.test@localhost") && !e.account_ok("test@corp.test@localhost"));
        let rel = Query::parse("после:7д папка:входящие");
        assert_eq!(rel.after, chrono::Local::now().date_naive().checked_sub_signed(chrono::Duration::days(7)));
        assert!(rel.folder_ok("INBOX") && !rel.folder_ok("Sent"));
        assert!(Query::parse("папка:спам").folder_ok("Junk"));
        // Двоеточие в обычном тексте (время, ссылка) — просто текст.
        assert_eq!(Query::parse("10:30").words, vec!["10:30"]);
    }

    #[test]
    fn matches_index_entries() {
        let m = msg();
        let hay = "петрова анна petrova@ihep.ru план работ на ноябрь план работ.xlsx";
        assert!(Query::parse("от:петрова тема:план вложение:xlsx").matches(&m, hay));
        assert!(Query::parse("подразделение:ihep.ru важные непрочитанные").matches(&m, hay));
        assert!(Query::parse("кому:yushkov").matches(&m, hay));
        assert!(Query::parse("после:2026-11-01 до:2026-11-02").matches(&m, hay));
        assert!(!Query::parse("после:2026-11-02").matches(&m, hay));
        assert!(!Query::parse("вложение:docx").matches(&m, hay));
        assert!(Query::parse("подразделение:inst.test").matches(&m, hay), "получатель из inst.test");
        assert!(!Query::parse("подразделение:mail.ru").matches(&m, hay));
        assert!(!Query::parse("флажок").matches(&m, hay));
    }

    #[test]
    fn builds_imap_search() {
        let q = Query::parse(r#"от:petrova тема:"plan" непрочитанные после:2026-10-01 подразделение:ihep.ru"#);
        assert_eq!(
            q.imap_criteria().unwrap(),
            r#"FROM "petrova" SUBJECT "plan" OR FROM "@ihep.ru" TO "@ihep.ru" UNSEEN SINCE 01-Oct-2026"#
        );
        assert!(!q.needs_utf8() && Query::parse("отчёт").needs_utf8());
        assert_eq!(Query::parse("есть:вложение").imap_criteria().as_deref(), Some("ALL"));
        assert_eq!(Query::parse("").imap_criteria(), None);
        // Кавычки и обратная косая черта экранируются.
        assert_eq!(Query::parse(r#"тема:a\b"#).imap_criteria().unwrap(), r#"SUBJECT "a\\b""#);
    }

    #[test]
    fn decodes_attachment_names() {
        assert_eq!(crate::imap::decode_param("filename*", "utf-8''%D0%9F%D0%BB%D0%B0%D0%BD.xlsx"), "План.xlsx");
        assert_eq!(crate::imap::decode_param("filename", "=?utf-8?B?0J/Qu9Cw0L0ueGxzeA==?="), "План.xlsx");
        assert_eq!(crate::imap::decode_param("name", "report.pdf"), "report.pdf");
    }
}
