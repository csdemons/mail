//! IMAP: подключение, папки, список писем, чтение, флаги, вложения.
//!
//! Разделён по задачам: `connect` — TLS и вход, `folders` — папки и роли, `content` —
//! разбор письма и вложения, `messages` — списки и идентификаторы, `sending` — отправка,
//! черновики, отложенные и шаблоны, `diagnostics` — отчёт о сервере.

mod prelude;
mod connect;
mod folders;
mod content;
mod messages;
mod sending;
mod diagnostics;

pub use connect::*;
pub use folders::*;
pub use content::*;
pub use messages::*;
pub use sending::*;
pub use diagnostics::*;
pub(crate) use connect::{has_capability};
pub(crate) use content::{addresses, attachment_names, has_attachment_part, repair_display_address};
pub(crate) use messages::{summary_from};
pub(crate) use sending::{uid_set};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_filenames() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\Users\\x\\отчёт.docx"), "отчёт.docx");
        assert_eq!(sanitize_filename("a<b>:c?.txt"), "a_b__c_.txt");
        assert_eq!(sanitize_filename(".."), "attachment");
        assert_eq!(sanitize_filename("con.txt"), "_con.txt");
        assert_eq!(sanitize_filename("COM1"), "_COM1");
        assert_eq!(sanitize_filename("report.pdf. . "), "report.pdf");
        assert_eq!(sanitize_filename("console.log"), "console.log");
        let long = format!("{}.docx", "я".repeat(300));
        let cut = sanitize_filename(&long);
        assert!(cut.ends_with(".docx") && cut.chars().count() <= 180);
        assert_eq!(sanitize_filename(""), "attachment");
    }

    #[test]
    fn repairs_encoded_from() {
        assert_eq!(
            repair_display_address("Робот <robot@corp.test>".into(), String::new()),
            ("Робот".into(), "robot@corp.test".into())
        );
        assert_eq!(
            repair_display_address("\"Иванов И.\" <ivanov@inst.test>".into(), String::new()),
            ("Иванов И.".into(), "ivanov@inst.test".into())
        );
        assert_eq!(
            repair_display_address("Имя".into(), "a@b.ru".into()),
            ("Имя".into(), "a@b.ru".into())
        );
    }

    #[test]
    fn detects_roles_by_name() {
        assert_eq!(role_by_name("Отправленные"), FolderRole::Sent);
        assert_eq!(role_by_name("Sent Items"), FolderRole::Sent);
        assert_eq!(role_by_name("Проекты"), FolderRole::Other);
    }

    fn raw(id: &str, special: Option<FolderRole>, selectable: bool) -> RawFolder {
        RawFolder {
            id: utf7_imap::encode_utf7_imap(id.to_string()),
            delimiter: Some("/".into()),
            special_use: special,
            selectable,
        }
    }

    #[test]
    fn labels_rupost_shared_mailboxes() {
        let list = vec![
            raw("INBOX", None, true),
            raw("shared", None, false),
            raw("shared/noc@inst.test", None, false),
            raw("shared/noc@inst.test/INBOX", None, true),
            raw("shared/noc@inst.test/Sent", None, true),
            raw("shared/noc@inst.test/imap", None, true),
            raw("shared/_soc@inst.test/INBOX/MaxPatrol", None, true),
            raw("Archives", None, false),
            raw("Archives/Archives", Some(FolderRole::Archive), true),
        ];
        let tree = build_folder_tree(list);
        let view: Vec<(usize, &str, FolderRole, bool)> =
            tree.iter().map(|f| (f.depth, f.name.as_str(), f.role, f.system)).collect();
        assert_eq!(
            view,
            vec![
                (0, "INBOX", FolderRole::Inbox, true),
                (0, "Archives", FolderRole::Archive, false),
                (1, "Archives", FolderRole::Archive, false),
                (0, "Подключённые ящики", FolderRole::Other, false),
                (1, "_soc@inst.test", FolderRole::Other, false),
                (2, "INBOX", FolderRole::Inbox, false),
                (3, "MaxPatrol", FolderRole::Other, false),
                (1, "noc@inst.test", FolderRole::Other, false),
                (2, "INBOX", FolderRole::Inbox, true),
                (2, "Sent", FolderRole::Sent, true),
                (2, "imap", FolderRole::Other, false),
            ]
        );
    }

    #[test]
    fn builds_tree_with_shared_mailboxes() {
        let list = vec![
            raw("Other Users", None, false),
            raw("Other Users/uex2016-06", None, false),
            raw("Other Users/uex2016-06/INBOX", None, true),
            raw("Other Users/uex2016-06/Sent", Some(FolderRole::Sent), true),
            raw("Корзина/2024/Astra", None, true),
            raw("Корзина", Some(FolderRole::Trash), true),
            raw("Проекты", None, true),
            raw("INBOX/Входящие отдела", None, true),
            raw("Отправленные", Some(FolderRole::Sent), true),
            raw("INBOX", None, true),
            raw("Архив", None, true),
        ];
        let tree = build_folder_tree(list);
        let view: Vec<(usize, &str, bool, bool)> = tree
            .iter()
            .map(|f| (f.depth, f.name.as_str(), f.system, f.selectable))
            .collect();
        assert_eq!(
            view,
            vec![
                (0, "INBOX", true, true),
                (1, "Входящие отдела", false, true),
                (0, "Отправленные", true, true),
                (0, "Архив", true, true),
                (0, "Корзина", true, true),
                (1, "2024", false, false),
                (2, "Astra", false, true),
                (0, "Проекты", false, true),
                (0, "Подключённые ящики", false, false),
                (1, "uex2016-06", false, false),
                (2, "INBOX", true, true),
                (2, "Sent", true, true),
            ]
        );
        // Папки подключённого ящика подписываются стандартно («Входящие», «Отправленные»).
        assert_eq!(tree[11].role, FolderRole::Sent);
        assert!(tree[0].has_children && tree[4].has_children && !tree[2].has_children);
        assert_eq!(tree[1].parent.as_deref(), Some("INBOX"));
    }

    #[test]
    fn scheduled_send_status_never_resends_unclear_mail() {
        let now = 1_800_000_000;
        let kw = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(send_status(&kw(&[]), now), "pending");
        assert_eq!(send_status(&kw(&["$label1", "important"]), now), "pending");
        // Свежий захват — отправляется прямо сейчас (другой копией программы).
        assert_eq!(send_status(&kw(&[&format!("atomclaim{}auto1a2b3c4d", now - 30)]), now), "sending");
        // Захват старше 10 минут: отправка прервалась — результат неизвестен, автоповтора нет.
        assert_eq!(send_status(&kw(&[&format!("atomclaim{}auto1a2b3c4d", now - 3600)]), now), "unknown");
        assert_eq!(send_status(&kw(&["atomclaimмусор"]), now), "unknown");
        assert_eq!(send_status(&kw(&[UNKNOWN_MARK]), now), "unknown");
        // «Отправлено» важнее всего остального: письмо больше не уходит.
        assert_eq!(send_status(&kw(&[UNKNOWN_MARK, SENT_MARK, &format!("atomclaim{now}x")]), now), "sent");
        assert!(valid_keyword(SENT_MARK) && valid_keyword(UNKNOWN_MARK));
    }

    #[test]
    fn decodes_utf7_folder_names() {
        let encoded = utf7_imap::encode_utf7_imap("Отправленные".to_string());
        assert_eq!(utf7_imap::decode_utf7_imap(encoded), "Отправленные");
    }
}

#[cfg(test)]
mod draft_key_tests {
    use super::valid_draft_key;

    #[test]
    fn only_safe_keys_reach_imap_search() {
        assert!(valid_draft_key("d-lx3k9a-0123456789abcdef01234567"));
        assert!(!valid_draft_key("short"));
        assert!(!valid_draft_key("d-1234567\" OR ALL"));
        assert!(!valid_draft_key("d-12345678\r\nA1 LOGOUT"));
        assert!(!valid_draft_key(&"a".repeat(65)));
    }
}

