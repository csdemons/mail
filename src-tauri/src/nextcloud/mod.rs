//! Nextcloud: вход, Deck, Talk, файлы, поиск, профиль и редакторы документов.
//!
//! Разделён по задачам: см. подмодули ниже. Ветки бесед и инструменты чата —
//! в talk_more.rs и talk_tools.rs (подключены через #[path]).

mod prelude;

use prelude::*;
mod core;
mod login;
mod deck;
mod talk;
mod search;
mod profile;
mod files;

pub use core::*;
pub use login::*;
pub use deck::*;
pub use talk::*;
pub use search::*;
pub use profile::*;
pub use files::*;
pub(crate) use core::{http, net_error, seg, status_error};
pub(crate) use deck::{int, text};
pub(crate) use talk::{parse_chat};
pub(crate) use talk::{OrInt, TALK_CHAT, TALK_ROOM};
pub(crate) use search::{dav_search_body};


#[path = "../nextcloud_talk.rs"]
mod talk_more;
pub use talk_more::*;
// Календарь и беседы, напоминания, сводка и перевод (ИИ), модерация.
#[path = "../nextcloud_talk_tools.rs"]
mod talk_tools;
pub use talk_tools::*;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub(crate) fn editor_window_stays_on_own_servers() {
        let base = url::Url::parse("https://cloud.inst.test/index.php/apps/onlyoffice/direct/x").unwrap();
        let ok = |s: &str| editor_url_allowed(&base, &url::Url::parse(s).unwrap());
        assert!(ok("https://cloud.inst.test/index.php/f/5"));
        assert!(ok("https://eurooffice.inst.test/web-apps/x"));
        assert!(ok("about:blank"));
        assert!(!ok("https://evil.example/login"));
        assert!(!ok("https://inst.test.evil.example/"));
        assert!(!ok("http://eurooffice.inst.test/"));
        let local = url::Url::parse("https://localhost:8443/x").unwrap();
        assert!(!editor_url_allowed(&local, &url::Url::parse("https://example.org/").unwrap()));
        // Служебные адреса: только about:blank и blob: своих серверов.
        assert!(!ok("data:text/html,<form>login</form>"));
        assert!(!ok("javascript:alert(1)"));
        assert!(!ok("about:config"));
        assert!(ok("blob:https://eurooffice.inst.test/1234-5678"));
        assert!(!ok("blob:https://evil.example/1234"));
        // Порт, логин в адресе, похожие имена.
        assert!(!ok("https://eurooffice.inst.test:8443/"));
        assert!(!ok("https://cloud.inst.test@evil.example/"));
        assert!(!ok("https://evilinst.test/"));
        assert!(!ok("https://cloud.inst.test.evil.example/"));
        assert!(ok("https://INST.test/"));
        // IP-адрес и публичный домен второго уровня — только сам сервер.
        let ip = url::Url::parse("https://10.0.0.5/x").unwrap();
        assert!(editor_url_allowed(&ip, &url::Url::parse("https://10.0.0.5/y").unwrap()));
        assert!(!editor_url_allowed(&ip, &url::Url::parse("https://1.0.0.5/").unwrap()));
        let uk = url::Url::parse("https://nc.example.co.uk/").unwrap();
        assert!(editor_url_allowed(&uk, &url::Url::parse("https://docs.example.co.uk/").unwrap()));
        let suffix = url::Url::parse("https://firm.co.uk/").unwrap();
        assert!(!editor_url_allowed(&suffix, &url::Url::parse("https://other.co.uk/").unwrap()));
    }

    #[test]
    pub(crate) fn encodes_path_segment() {
        assert_eq!(seg("ivanov@inst.test"), "ivanov@inst.test");
        assert_eq!(seg("Иван Петров/../x"), "%D0%98%D0%B2%D0%B0%D0%BD%20%D0%9F%D0%B5%D1%82%D1%80%D0%BE%D0%B2%2F..%2Fx");
    }

    #[test]
    pub(crate) fn normalizes_base_url() {
        assert_eq!(normalize_base("next.inst.test").unwrap().as_str(), "https://next.inst.test/");
        assert_eq!(normalize_base("https://next.inst.test/nc").unwrap().as_str(), "https://next.inst.test/nc/");
        assert!(normalize_base("http://next.inst.test").is_err());
    }

    #[test]
    pub(crate) fn renders_talk_parameters() {
        let v = json!({
            "id": 7, "actorType": "users", "actorId": "yushkov_do@inst.test", "actorDisplayName": "Юшков Дмитрий",
            "timestamp": 1793000000, "message": "{mention-user1}, посмотри {file}", "messageType": "comment", "systemMessage": "",
            "messageParameters": {
                "mention-user1": { "type": "user", "id": "ivanov", "name": "Иванов Иван" },
                "file": { "type": "file", "id": "42", "name": "Отчёт.docx" }
            },
            "parent": { "id": 5, "actorDisplayName": "Иванов Иван", "message": "Где отчёт?", "messageParameters": [] }
        });
        let m = parse_message(&v, "yushkov_do@inst.test");
        assert_eq!(m.text, "@Иванов Иван, посмотри");
        assert_eq!(m.files, vec![TalkFile { id: "42".into(), name: "Отчёт.docx".into(), size: 0, mimetype: String::new(), path: None, preview: false }]);
        // Сообщение «только файл»: текста нет, ID числом.
        let (t, f) = render_message(&json!({"message": "{file}", "messageParameters": {"file": {"type": "file", "id": 9, "name": "a.png", "size": 5, "mimetype": "image/png", "path": "/Talk/a.png", "preview-available": "yes"}}}));
        assert!(t.is_empty() && f[0].id == "9" && f[0].size == 5);
        assert!(f[0].preview && f[0].path.as_deref() == Some("Talk/a.png"));
        assert!(m.own && !m.system);
        assert_eq!(m.parent_text.as_deref(), Some("Где отчёт?"));
        // Пустые параметры приходят как [] — не должно падать.
        assert_eq!(render_message(&json!({"message": "Привет", "messageParameters": []})).0, "Привет");
    }

    #[test]
    pub(crate) fn talk_updates_reactions_and_edits() {
        let me = "test";
        let chat = json!([
            {"id": 10, "actorType": "users", "actorId": "test", "actorDisplayName": "Я", "timestamp": 1, "message": "Привет, {mention-user1}",
             "messageType": "comment", "systemMessage": "", "lastEditTime": 0,
             "messageParameters": {"mention-user1": {"type": "user", "id": "ivanov@inst.test", "name": "Иванов", "mention-id": "ivanov@inst.test"}},
             "reactions": {"👍": 2, "🎉": 1}, "reactionsSelf": ["👍"]},
            {"id": 11, "actorType": "users", "actorId": "ivanov", "actorDisplayName": "Иванов", "timestamp": 2, "message": "{actor} reacted",
             "messageType": "system", "systemMessage": "reaction",
             "parent": {"id": 10, "actorType": "users", "actorId": "test", "actorDisplayName": "Я", "timestamp": 1, "message": "Привет", "messageType": "comment",
                        "systemMessage": "", "lastEditTime": 5, "messageParameters": [], "reactions": {"👍": 3}, "reactionsSelf": ["👍"]}},
            {"id": 12, "actorType": "users", "actorId": "test", "actorDisplayName": "Я", "timestamp": 3, "message": "x", "messageType": "system", "systemMessage": "message_deleted",
             "parent": {"id": 9, "actorType": "users", "actorId": "test", "actorDisplayName": "Я", "timestamp": 0, "message": "Message deleted by you", "messageType": "comment_deleted", "systemMessage": "", "messageParameters": []}},
            {"id": 13, "actorType": "users", "actorId": "x", "message": "{actor} joined", "messageType": "system", "systemMessage": "user_added", "messageParameters": []}
        ]);
        let m = parse_chat(&chat, me);
        assert_eq!(m.iter().map(|x| (x.id, x.update)).collect::<Vec<_>>(), vec![(10, false), (13, false), (10, true), (9, true)]);
        assert_eq!(m[0].reactions, vec![TalkReaction { emoji: "👍".into(), count: 2, own: true }, TalkReaction { emoji: "🎉".into(), count: 1, own: false }]);
        assert_eq!(m[0].edit_text.as_deref(), Some("Привет, @\"ivanov@inst.test\""));
        assert!(m[2].edited && m[2].reactions[0].count == 3);
        assert!(m[3].deleted && m[3].text == "Сообщение удалено" && m[3].edit_text.is_none());
        assert!(m[1].system);
    }

    #[test]
    pub(crate) fn parses_board_details() {
        let d = parse_board_details(&json!({
            "owner": {"uid": "admin", "displayname": "Администратор"},
            "labels": [{"id": 1, "title": "Срочно", "color": "e9322d"}],
            "users": [{"uid": "ivanov", "displayname": "Иванов"}],
            "acl": [{"type": 0, "participant": {"uid": "petrova", "displayname": "Петрова"}},
                    {"type": 1, "participant": {"uid": "staff", "displayname": "Сотрудники"}},
                    {"type": 0, "participant": {"uid": "ivanov", "displayname": "Иванов"}}]
        }));
        assert_eq!(d.labels[0].title, "Срочно");
        assert_eq!(d.users.iter().map(|u| u.uid.as_str()).collect::<Vec<_>>(), vec!["admin", "ivanov", "petrova"]);
    }

    #[test]
    pub(crate) fn parses_webdav_listing_and_paths() {
        let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">
          <d:response><d:href>/remote.php/dav/files/test%40corp.test/Docs/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><oc:size>300</oc:size><oc:fileid>5</oc:fileid></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          <d:response><d:href>/remote.php/dav/files/test%40corp.test/Docs/%D0%9E%D1%82%D1%87%D1%91%D1%82%20(1).pdf</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>120</d:getcontentlength><d:getcontenttype>application/pdf</d:getcontenttype><oc:fileid>7</oc:fileid><oc:permissions>RGDNVW</oc:permissions><oc:share-types><oc:share-type>3</oc:share-type></oc:share-types></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat><d:propstat><d:prop><oc:favorite/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>
          <d:response><d:href>/remote.php/dav/files/test%40corp.test/Docs/Sub/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><oc:permissions>RGD</oc:permissions></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
        </d:multistatus>"#;
        let f = parse_file_list(xml, "/remote.php/dav/files/test@corp.test/").unwrap();
        assert_eq!(f.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(), vec!["/Docs", "/Docs/Отчёт (1).pdf", "/Docs/Sub"]);
        assert!(f[0].is_dir && f[0].size == 300);
        assert_eq!((f[1].name.as_str(), f[1].size, f[1].file_id, f[1].shared, f[1].can_write), ("Отчёт (1).pdf", 120, Some(7), true, true));
        assert!(!f[2].can_write);
        assert_eq!(clean_path("Docs//a/").unwrap(), "/Docs/a");
        assert!(clean_path("/Docs/../etc").is_err());
        assert!(valid_name("a/b").is_err() && valid_name("  Отчёт.pdf ").unwrap() == "Отчёт.pdf");
    }

    #[test]
    pub(crate) fn numbers_duplicate_names() {
        assert_eq!(numbered_name("отчёт.pdf", 0), "отчёт.pdf");
        assert_eq!(numbered_name("отчёт.pdf", 2), "отчёт (2).pdf");
        assert_eq!(numbered_name(".bashrc", 1), ".bashrc (1)");
        assert_eq!(numbered_name("README", 1), "README (1)");
    }

    #[test]
    pub(crate) fn parses_deck_card_variants() {
        let c = parse_card(&json!({
            "id": "12", "stackId": 3, "title": "Обновить RuPost", "description": "", "order": 1,
            "duedate": "2026-10-30T00:00:00+00:00", "done": null, "archived": false,
            "labels": [{"id": 1, "title": "Срочно", "color": "ff0000", "boardId": "1"}],
            "assignedUsers": [{"participant": {"uid": "ivanov", "displayname": "Иванов"}}],
            "owner": {"uid": "yushkov_do", "displayname": "Юшков"}, "commentsUnread": 2
        }));
        assert_eq!((c.id, c.stack_id, c.done), (12, 3, false));
        assert_eq!(c.assigned, vec![DeckUser { uid: "ivanov".into(), name: "Иванов".into() }]);
        assert_eq!(c.labels[0].title, "Срочно");
        let done = parse_card(&json!({"id": 1, "done": "2026-10-27T10:00:00+00:00", "owner": "u"}));
        assert!(done.done && done.owner == "u");
    }
}

