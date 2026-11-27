//! Поиск по Nextcloud: карточки Deck, беседы Talk и файлы.
//!
//! Часть модуля `nextcloud` — общий обзор в `mod.rs`.

use super::*;

/// Найденная карточка Deck — с доской и колонкой, чтобы открыть её на месте.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckHit {
    pub board_id: i64,
    pub board: String,
    pub stack_id: i64,
    pub stack: String,
    pub card: DeckCard,
}

/// Найденное в чате: сообщение (`message_id`) или сама беседа по названию (без него).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHit {
    pub token: String,
    pub room: String,
    pub message_id: Option<i64>,
    pub author: String,
    pub text: String,
    pub timestamp: i64,
}

/// Общий поиск по Nextcloud: файлы, карточки Deck, чат. Источник, который не
/// ответил, не мешает остальным — его ошибка в `errors`.
#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NcSearch {
    pub files: Vec<NcFile>,
    pub cards: Vec<DeckHit>,
    pub chat: Vec<ChatHit>,
    pub errors: Vec<String>,
}

/// Совпадение без учёта регистра и «ё».
pub fn fold(s: &str) -> String {
    s.to_lowercase().replace('ё', "е")
}

/// Запрос WebDAV SEARCH: файлы, в имени которых есть `term`, новые сверху.
pub(crate) fn dav_search_body(user: &str, term: &str, limit: usize) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    // В LIKE «%» и «_» — подстановочные; экранировать их DAV не умеет, поэтому убираем.
    let t: String = term.chars().filter(|c| *c != '%' && *c != '_').collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><d:searchrequest xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns"><d:basicsearch><d:select><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/><d:getcontenttype/><oc:fileid/><oc:size/><oc:favorite/><oc:share-types/><oc:permissions/></d:prop></d:select><d:from><d:scope><d:href>/files/{}</d:href><d:depth>infinity</d:depth></d:scope></d:from><d:where><d:like><d:prop><d:displayname/></d:prop><d:literal>%{}%</d:literal></d:like></d:where><d:orderby><d:order><d:prop><d:getlastmodified/></d:prop><d:descending/></d:order></d:orderby><d:limit><d:nresults>{limit}</d:nresults></d:limit></d:basicsearch></d:searchrequest>"#,
        esc(user),
        esc(&t)
    )
}

