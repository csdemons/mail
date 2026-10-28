//! Показ HTML-писем: очистка (ammonia), встроенные картинки `cid:` → `data:`,
//! картинки из интернета — только по запросу пользователя (загружает бэкенд,
//! в письмо они попадают как `data:`, страница сама в сеть не ходит).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use mail_parser::{Message, MimeHeaders, PartType};

/// Сколько встроенных картинок (суммарно) превращается в `data:`.
const MAX_INLINE_TOTAL: usize = 25 * 1024 * 1024;
/// Внешние картинки: не больше стольких и такого размера каждая.
pub const MAX_REMOTE_IMAGES: usize = 60;
const MAX_REMOTE_BYTES: usize = 5 * 1024 * 1024;

pub struct Rendered {
    pub html: String,
    /// Номера вложений, показанных внутри письма (их нет смысла дублировать списком).
    pub inline: HashSet<usize>,
    /// Адреса внешних картинок, которые не показаны.
    pub remote: Vec<String>,
}

/// HTML-часть письма, если она есть (не текст, превращённый парсером в HTML).
pub fn html_part<'a>(msg: &'a Message<'a>) -> Option<&'a str> {
    msg.html_body.iter().filter_map(|&i| msg.part(i)).find_map(|p| match &p.body {
        PartType::Html(h) => Some(h.as_ref()),
        _ => None,
    })
}

fn content_type(part: &mail_parser::MessagePart<'_>) -> String {
    part.content_type()
        .map(|ct| match ct.subtype() {
            Some(sub) => format!("{}/{}", ct.ctype(), sub),
            None => ct.ctype().to_string(),
        })
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_remote(url: &str) -> bool {
    let u = url.trim_start().to_ascii_lowercase();
    u.starts_with("http://") || u.starts_with("https://") || u.starts_with("//")
}

fn normalize_remote(url: &str) -> String {
    let u = url.trim();
    if u.starts_with("//") { format!("https:{u}") } else { u.to_string() }
}

/// Очищенный HTML письма. `remote` — уже загруженные внешние картинки (адрес → data:).
pub fn render(msg: &Message<'_>, html: &str, remote: Option<&HashMap<String, String>>) -> Rendered {
    // cid → (номер вложения, data:)
    let mut cids: HashMap<String, (usize, String)> = HashMap::new();
    let mut total = 0usize;
    for (index, part) in msg.attachments().enumerate() {
        let Some(id) = part.content_id() else { continue };
        let ct = content_type(part);
        if !ct.starts_with("image/") || total + part.contents().len() > MAX_INLINE_TOTAL {
            continue;
        }
        total += part.contents().len();
        let data = base64::engine::general_purpose::STANDARD.encode(part.contents());
        cids.insert(id.trim_matches(['<', '>']).to_ascii_lowercase(), (index, format!("data:{ct};base64,{data}")));
    }

    let used: Arc<Mutex<HashSet<usize>>> = Arc::default();
    let skipped: Arc<Mutex<Vec<String>>> = Arc::default();
    let cids = Arc::new(cids);
    let remote: Option<Arc<HashMap<String, String>>> = remote.map(|m| Arc::new(m.clone()));
    let cleaned = {
        let (used, skipped) = (used.clone(), skipped.clone());
        let mut b = ammonia::Builder::default();
        b.add_tags(["font", "center", "style", "u", "s", "strike", "small", "big", "hr", "caption", "colgroup", "col", "span", "div"])
            .rm_clean_content_tags(["style"])
            .add_generic_attributes([
                "style", "align", "valign", "bgcolor", "width", "height", "border", "dir", "class", "color", "face", "size",
                "cellpadding", "cellspacing", "background", "colspan", "rowspan", "lang",
            ])
            .add_tag_attributes("img", ["src", "alt", "width", "height"])
            .add_url_schemes(["data", "cid", "tel"])
            .url_relative(ammonia::UrlRelative::Deny)
            .link_rel(Some("noopener noreferrer"))
            .attribute_filter(move |element, attribute, value| {
                let is_image_src = (element == "img" && attribute == "src") || attribute == "background";
                if is_image_src {
                    let v = value.trim();
                    if let Some(id) = v.strip_prefix("cid:").or_else(|| v.strip_prefix("CID:")) {
                        return match cids.get(&id.trim_matches(['<', '>']).to_ascii_lowercase()) {
                            Some((index, data)) => {
                                used.lock().expect("lock").insert(*index);
                                Some(data.clone().into())
                            }
                            None => None,
                        };
                    }
                    if is_remote(v) {
                        let url = normalize_remote(v);
                        if let Some(data) = remote.as_ref().and_then(|m| m.get(&url)) {
                            return Some(data.clone().into());
                        }
                        let mut s = skipped.lock().expect("lock");
                        if !s.contains(&url) {
                            s.push(url);
                        }
                        return None;
                    }
                    // data:image/... оставляем, всё остальное — нет.
                    return v.to_ascii_lowercase().starts_with("data:image/").then(|| value.into());
                }
                if attribute == "href" {
                    let v = value.trim().to_ascii_lowercase();
                    let ok = ["http://", "https://", "mailto:", "tel:"].iter().any(|p| v.starts_with(p));
                    return ok.then(|| value.into());
                }
                Some(value.into())
            });
        b.clean(html).to_string()
    };
    Rendered {
        html: cleaned,
        inline: used.lock().map(|g| g.clone()).unwrap_or_default(),
        remote: skipped.lock().map(|g| g.clone()).unwrap_or_default(),
    }
}

/// Загружает внешние картинки письма (только http/https, только image/*, с ограничением размера).
pub async fn fetch_remote(urls: &[String]) -> HashMap<String, String> {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
    else {
        return HashMap::new();
    };
    let jobs = urls.iter().take(MAX_REMOTE_IMAGES).map(|url| {
        let client = client.clone();
        let url = url.clone();
        async move {
            let parsed = url::Url::parse(&url).ok().filter(|u| matches!(u.scheme(), "http" | "https"))?;
            let resp = client.get(parsed).send().await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let ct = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
                .unwrap_or_default();
            if !ct.starts_with("image/") || resp.content_length().is_some_and(|n| n as usize > MAX_REMOTE_BYTES) {
                return None;
            }
            let bytes = resp.bytes().await.ok()?;
            if bytes.len() > MAX_REMOTE_BYTES {
                return None;
            }
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Some((url, format!("data:{ct};base64,{data}")))
        }
    });
    futures::future::join_all(jobs).await.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_parser::MessageParser;

    const RAW: &[u8] = b"From: a@b.ru\r\nTo: c@d.ru\r\nSubject: t\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"B\"\r\n\r\n--B\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><head><style>p { color: red } a > b {}</style></head><body onload=\"x()\"><p style=\"color:blue\">Hi <img src=\"cid:logo@x\"> <img src=\"https://ex.com/a.png\"> <a href=\"javascript:alert(1)\">bad</a> <a href=\"https://ok.ru\">ok</a></p><script>alert(1)</script><table bgcolor=\"#fff\" width=\"600\"><tr><td>c</td></tr></table></body></html>\r\n--B\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--B--\r\n";

    #[test]
    fn renders_html_safely_with_inline_images() {
        let msg = MessageParser::default().parse(RAW).expect("parse");
        let html = html_part(&msg).expect("html part");
        let r = render(&msg, html, None);
        assert!(r.html.contains("data:image/png;base64,iVBORw0KGgo="), "{}", r.html);
        assert!(r.inline.contains(&0));
        assert_eq!(r.remote, vec!["https://ex.com/a.png".to_string()]);
        assert!(!r.html.contains("script") && !r.html.contains("onload") && !r.html.contains("javascript:"), "{}", r.html);
        assert!(r.html.contains("style=\"color:blue\"") && r.html.contains("bgcolor") && r.html.contains("a > b"), "{}", r.html);
        assert!(r.html.contains("href=\"https://ok.ru\""));
        let mut remote = HashMap::new();
        remote.insert("https://ex.com/a.png".to_string(), "data:image/png;base64,AAAA".to_string());
        let r = render(&msg, html, Some(&remote));
        assert!(r.remote.is_empty() && r.html.contains("data:image/png;base64,AAAA"));
    }

    #[test]
    fn plain_text_message_has_no_html_part() {
        let msg = MessageParser::default().parse(b"From: a@b.ru\r\nSubject: t\r\n\r\nhello\r\n".as_slice()).expect("parse");
        assert!(html_part(&msg).is_none());
    }
}
