//! Работа без связи с почтовым сервером.
//!
//! Пока сервер недоступен, программа показывает папки и письма из хранилища на
//! компьютере (`store`), а то, что пользователь делает, — отметки, категории,
//! перемещение, удаление, отправку писем — выполняет у себя и ставит в очередь.
//! Когда связь появляется, очередь выполняется на сервере по порядку.
//!
//! Конфликты (сервер изменился, пока связи не было) решаются так:
//! - отметки и категории — применяется сделанное без сети: это более позднее
//!   решение пользователя;
//! - письмо, к которому относится действие, ищется на сервере по Message-ID:
//!   если его переместили (здесь же или в веб-почте), действие выполняется там,
//!   где оно теперь; если его уже нет — действие не выполняется и попадает в
//!   список конфликтов с объяснением;
//! - перемещение, которое уже сделано на сервере (письмо в нужной папке), не
//!   повторяется;
//! - письмо из очереди исходящих, если связь оборвалась во время его передачи, не
//!   отправляется повторно само (результат неизвестен) — решает пользователь.
//!
//! Очередь хранится зашифрованной в хранилище писем на компьютере (ключ — в
//! системном хранилище паролей), отдельно для каждой учётной записи.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

/// Письмо, к которому относится действие: UID в папке и Message-ID (для поиска,
/// если письмо успели переместить).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub uid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
    /// UIDVALIDITY папки, когда действие ставилось в очередь: сменилась (папку
    /// пересоздали) — старый UID может принадлежать другому письму.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Op {
    /// `flag`: seen | flagged | important.
    Flag { folder: String, targets: Vec<Target>, flag: String, value: bool },
    Keyword { folder: String, targets: Vec<Target>, keyword: String, value: bool },
    /// Флажок со сроком (RFC 3339) или без срока.
    Due { folder: String, target: Target, due: Option<String> },
    Move { folder: String, targets: Vec<Target>, target: String },
    Delete { folder: String, targets: Vec<Target>, trash: Option<String> },
    /// Письмо в очереди исходящих: исходник (base64), тема и адресаты — для списка.
    Send { raw: String, subject: String, to: Vec<String> },
    /// Отложенное письмо, которое нужно положить в «Черновики».
    Schedule { raw: String, subject: String, send_at: String },
}

impl Op {
    pub fn is_send(&self) -> bool {
        matches!(self, Op::Send { .. } | Op::Schedule { .. })
    }

    /// Описание для списка очереди.
    pub fn describe(&self) -> String {
        let n = |t: &[Target]| if t.len() == 1 { "письмо".to_string() } else { format!("писем: {}", t.len()) };
        let folder = |f: &str| utf7_imap::decode_utf7_imap(f.to_string());
        match self {
            Op::Flag { targets, flag, value, folder: f } => {
                let what = match (flag.as_str(), value) {
                    ("seen", true) => "отметить прочитанным",
                    ("seen", false) => "отметить непрочитанным",
                    ("flagged", true) => "поставить флажок",
                    ("flagged", false) => "снять флажок",
                    ("important", true) => "отметить важным",
                    _ => "снять отметку «Важное»",
                };
                format!("{} — {} («{}»)", capitalize(what), n(targets), folder(f))
            }
            Op::Keyword { targets, keyword, value, folder: f } => {
                format!("{} {keyword} — {} («{}»)", if *value { "Категория" } else { "Снять категорию" }, n(targets), folder(f))
            }
            Op::Due { due, folder: f, .. } => format!("{} — письмо («{}»)", if due.is_some() { "Флажок со сроком" } else { "Снять срок" }, folder(f)),
            Op::Move { targets, target, folder: f } => format!("Переместить {} из «{}» в «{}»", n(targets), folder(f), folder(target)),
            Op::Delete { targets, folder: f, .. } => format!("Удалить {} («{}»)", n(targets), folder(f)),
            Op::Send { subject, to, .. } => format!("Отправить «{}» — {}", if subject.is_empty() { "(без темы)" } else { subject }, to.join(", ")),
            Op::Schedule { subject, .. } => format!("Запланировать «{}»", if subject.is_empty() { "(без темы)" } else { subject }),
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: u64,
    /// Когда поставлено в очередь (RFC 3339).
    pub created: String,
    pub op: Op,
    /// Последняя ошибка при попытке выполнить (сервер ответил, но не выполнил).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub attempts: u32,
}

/// Действие, которое не удалось выполнить после подключения.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub id: u64,
    pub when: String,
    pub what: String,
    pub reason: String,
    /// Письмо, которое можно отправить ещё раз (результат прошлой попытки неизвестен).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<Op>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    #[serde(default)]
    pub next: u64,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub conflicts: Vec<Conflict>,
}

/// Сколько действий хранится (защита от бесконечного роста без связи).
pub const MAX_ITEMS: usize = 5000;
const MAX_CONFLICTS: usize = 200;
/// Сервер отвечает ошибкой на действие — после стольких попыток оно уходит в конфликты.
pub const MAX_ATTEMPTS: u32 = 3;
pub const BLOB: &str = "queue.bin";

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl Queue {
    pub fn from_bytes(raw: Option<Vec<u8>>) -> Queue {
        raw.and_then(|r| serde_json::from_slice(&r).ok()).unwrap_or_default()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn push(&mut self, op: Op) -> Result<u64> {
        if self.items.len() >= MAX_ITEMS {
            return Err(AppError::Other("Очередь действий без сети переполнена — подключитесь к серверу".into()));
        }
        self.next += 1;
        self.items.push(Item { id: self.next, created: now(), op, error: None, attempts: 0 });
        Ok(self.next)
    }

    pub fn conflict(&mut self, what: String, reason: String, retry: Option<Op>) {
        self.next += 1;
        self.conflicts.push(Conflict { id: self.next, when: now(), what, reason, retry });
        if self.conflicts.len() > MAX_CONFLICTS {
            let extra = self.conflicts.len() - MAX_CONFLICTS;
            self.conflicts.drain(..extra);
        }
    }

}

/// Строка для списка очереди (без исходников писем).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    pub id: u64,
    pub created: String,
    pub what: String,
    pub send: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConflictView {
    pub id: u64,
    pub when: String,
    pub what: String,
    pub reason: String,
    pub can_retry: bool,
}

/// Состояние связи и очереди — для строки состояния и списка «Очередь».
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub online: bool,
    /// Идёт выполнение очереди после подключения.
    pub syncing: bool,
    /// Работа без сети возможна (есть хранилище писем на компьютере).
    pub available: bool,
    pub offline_since: Option<String>,
    pub last_error: Option<String>,
    pub items: Vec<ItemView>,
    pub conflicts: Vec<ConflictView>,
}

impl Queue {
    pub fn views(&self) -> (Vec<ItemView>, Vec<ConflictView>) {
        (
            self.items.iter().map(|i| ItemView { id: i.id, created: i.created.clone(), what: i.op.describe(), send: i.op.is_send(), error: i.error.clone() }).collect(),
            self.conflicts.iter().rev().map(|c| ConflictView { id: c.id, when: c.when.clone(), what: c.what.clone(), reason: c.reason.clone(), can_retry: c.retry.is_some() }).collect(),
        )
    }
}

pub fn encode(raw: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(raw)
}

pub fn decode(s: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).map_err(|_| AppError::Other("повреждённое письмо в очереди".into()))
}

/// Где письмо теперь: UID в папке `folder` по сохранённому UID и Message-ID.
/// `found` — UID и Message-ID писем папки, которые вернул сервер для исходных UID;
/// `search` — UID найденного по Message-ID (если искали).
/// `validity` — текущий UIDVALIDITY папки (если известен).
///
/// По UID письмо принимается, только если UID ещё значит то же самое (UIDVALIDITY не
/// сменился) и Message-ID совпадает. У письма в очереди был Message-ID, а у письма на
/// сервере его нет — это другое письмо. Без Message-ID и при сменившемся UIDVALIDITY
/// письмо не находится: действие уходит в конфликты, а не применяется наугад.
pub fn resolve(t: &Target, found: &[(u32, Option<String>)], search: Option<u32>, validity: Option<u32>) -> Option<u32> {
    let uid_means_same = match (t.validity, validity) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    let same = |server: &Option<String>| match (&t.msg_id, server) {
        (Some(x), Some(y)) => norm_id(x) == norm_id(y),
        (Some(_), None) => false,
        (None, _) => true,
    };
    if uid_means_same && found.iter().any(|(u, id)| *u == t.uid && same(id)) {
        return Some(t.uid);
    }
    search
}

pub fn norm_id(id: &str) -> String {
    id.trim().trim_start_matches('<').trim_end_matches('>').to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_round_trip_and_limits() {
        let mut q = Queue::default();
        let id = q.push(Op::Flag { folder: "INBOX".into(), targets: vec![Target { uid: 5, msg_id: Some("<a@b>".into()), validity: None }], flag: "seen".into(), value: true }).unwrap();
        q.push(Op::Send { raw: encode(b"Subject: x\r\n\r\nbody"), subject: "Отчёт".into(), to: vec!["a@b.ru".into()] }).unwrap();
        q.conflict("Переместить".into(), "письма нет".into(), None);
        let back = Queue::from_bytes(Some(q.to_bytes()));
        assert_eq!(back, q);
        assert_eq!(back.items[0].id, id);
        assert_eq!(back.items.iter().filter(|i| i.op.is_send()).count(), 1);
        let (items, conflicts) = back.views();
        assert_eq!(items[0].what, "Отметить прочитанным — письмо («INBOX»)");
        assert_eq!(items[1].what, "Отправить «Отчёт» — a@b.ru");
        assert!(items[1].send && !items[0].send);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(decode(&encode(b"\x00\xff")).unwrap(), b"\x00\xff");
        assert_eq!(Queue::from_bytes(Some(b"garbage".to_vec())), Queue::default());
        assert_eq!(Queue::from_bytes(None), Queue::default());

        let mut full = Queue::default();
        for _ in 0..MAX_ITEMS {
            full.push(Op::Delete { folder: "INBOX".into(), targets: vec![], trash: None }).unwrap();
        }
        assert!(full.push(Op::Delete { folder: "INBOX".into(), targets: vec![], trash: None }).is_err());
        for i in 0..(MAX_CONFLICTS + 5) {
            full.conflict(format!("{i}"), String::new(), None);
        }
        assert_eq!(full.conflicts.len(), MAX_CONFLICTS);
        assert_eq!(full.conflicts[0].what, "5");
    }

    #[test]
    fn describes_folders_decoded() {
        let op = Op::Move { folder: "INBOX".into(), targets: vec![Target { uid: 1, msg_id: None, validity: None }, Target { uid: 2, msg_id: None, validity: None }], target: "&BBAEQARFBDgEMg-".into() };
        assert_eq!(op.describe(), "Переместить писем: 2 из «INBOX» в «Архив»");
    }

    #[test]
    fn uid_reused_after_folder_recreated_is_not_trusted() {
        let t = Target { uid: 7, msg_id: Some("<a@b>".into()), validity: Some(100) };
        // UIDVALIDITY тот же, Message-ID совпадает — то самое письмо.
        assert_eq!(resolve(&t, &[(7, Some("a@b".into()))], None, Some(100)), Some(7));
        // Папку пересоздали: тот же UID ничего не значит, даже при совпавшем Message-ID берём найденное поиском.
        assert_eq!(resolve(&t, &[(7, Some("a@b".into()))], None, Some(200)), None);
        assert_eq!(resolve(&t, &[(7, Some("a@b".into()))], Some(3), Some(200)), Some(3));
        // У письма в очереди был Message-ID, у письма с этим UID на сервере — нет: другое письмо.
        assert_eq!(resolve(&t, &[(7, None)], None, Some(100)), None);
        // Письмо без Message-ID: по UID — только пока UIDVALIDITY не сменился.
        let bare = Target { uid: 4, msg_id: None, validity: Some(100) };
        assert_eq!(resolve(&bare, &[(4, None)], None, Some(100)), Some(4));
        assert_eq!(resolve(&bare, &[(4, None)], None, Some(101)), None);
    }

    #[test]
    fn resolves_moved_messages() {
        let t = Target { uid: 7, msg_id: Some("<X@host>".into()), validity: None };
        // на месте
        assert_eq!(resolve(&t, &[(7, Some("x@host".into()))], None, None), Some(7));
        // UID занят другим письмом (папку пересоздали) — берём найденное по Message-ID
        assert_eq!(resolve(&t, &[(7, Some("other@host".into()))], Some(12), None), Some(12));
        // письма нет
        assert_eq!(resolve(&t, &[], None, None), None);
        // без Message-ID — только по UID
        let bare = Target { uid: 3, msg_id: None, validity: None };
        assert_eq!(resolve(&bare, &[(3, Some("q@w".into()))], None, None), Some(3));
    }
}
