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

/// Можно ли обращаться к адресу из письма: только публичный интернет. Внутренние,
/// локальные и служебные адреса запрещены — письмо не должно заставить программу
/// обратиться к сервисам на этом компьютере или во внутренней сети (SSRF).
pub fn is_public_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || o[0] == 0                                   // 0.0.0.0/8
                || (o[0] == 100 && (o[1] & 0xc0) == 64)        // 100.64.0.0/10 CGNAT
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)     // 192.0.0.0/24
                || (o[0] == 198 && (o[1] & 0xfe) == 18)        // 198.18.0.0/15
                || o[0] >= 240)                                // 240.0.0.0/4 и 255.255.255.255
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let s = v6.segments();
            // 64:ff9b::/96 (NAT64) и 2002::/16 (6to4) — проверяем вложенный IPv4.
            if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
                let [a, b] = s[6].to_be_bytes();
                let [c, d] = s[7].to_be_bytes();
                return is_public_ip(IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d)));
            }
            if s[0] == 0x2002 {
                let [a, b] = s[1].to_be_bytes();
                let [c, d] = s[2].to_be_bytes();
                return is_public_ip(IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d)));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00   // fc00::/7 уникальные локальные
                || (s[0] & 0xffc0) == 0xfe80   // fe80::/10 link-local
                || (s[0] & 0xffc0) == 0xfec0   // fec0::/10 site-local
                || (s[0] == 0x2001 && s[1] == 0x0db8) // 2001:db8::/32 документация
                || (s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0)) // ::/96 IPv4-compatible
        }
    }
}

/// Суммарный объём внешних картинок одного письма и число одновременных загрузок.
const MAX_REMOTE_TOTAL: usize = 25 * 1024 * 1024;
const REMOTE_PARALLEL: usize = 6;
const MAX_REDIRECTS: usize = 3;

/// Проверяет адрес и разрешает имя: все IP должны быть публичными. Возвращает
/// адрес, к которому подключаться (имя закрепляется за ним — повторное
/// DNS-разрешение не подменит его на внутренний, DNS rebinding).
async fn resolve_public(url: &url::Url, allow_local: bool) -> Option<std::net::SocketAddr> {
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?;
    let port = url.port_or_known_default()?;
    if !matches!(port, 80 | 443 | 8080 | 8443) && !allow_local {
        return None; // нестандартные порты — частый признак обращения к служебным сервисам
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<std::net::SocketAddr> = tokio::time::timeout(std::time::Duration::from_secs(5), tokio::net::lookup_host((host, port)))
        .await
        .ok()?
        .ok()?
        .collect();
    if addrs.is_empty() || (!allow_local && addrs.iter().any(|a| !is_public_ip(a.ip()))) {
        return None;
    }
    addrs.into_iter().next()
}

async fn fetch_one(url: String, allow_local: bool, budget: std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Option<(String, String)> {
    use std::sync::atomic::Ordering;
    let mut current = url::Url::parse(&url).ok()?;
    for _ in 0..=MAX_REDIRECTS {
        let addr = resolve_public(&current, allow_local).await?;
        let host = current.host_str()?.trim_start_matches('[').trim_end_matches(']').to_string();
        // Свой клиент на каждый шаг: имя → проверенный адрес, без прокси (прокси сам
        // разрешал бы имена), без автоматических перенаправлений, без cookie.
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .connect_timeout(std::time::Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve(&host, addr)
            .build()
            .ok()?;
        let mut resp = client.get(current.clone()).send().await.ok()?;
        if resp.status().is_redirection() {
            let loc = resp.headers().get(reqwest::header::LOCATION)?.to_str().ok()?;
            current = current.join(loc).ok()?;
            continue;
        }
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
        let mut bytes = Vec::new();
        while let Some(chunk) = resp.chunk().await.ok()? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > MAX_REMOTE_BYTES {
                return None;
            }
        }
        // Общий лимит на письмо: превысили — дальше картинки не берём.
        if budget.fetch_sub(bytes.len().min(budget.load(Ordering::Relaxed)), Ordering::Relaxed) < bytes.len() {
            return None;
        }
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Some((url, format!("data:{ct};base64,{data}")));
    }
    None
}

/// Загружает внешние картинки письма — только по кнопке пользователя, только из
/// публичного интернета, с проверкой каждого перенаправления и ограничением объёма.
/// `allow_local` — только для автотестов (мок-сервер на localhost).
pub async fn fetch_remote(urls: &[String], allow_local: bool) -> HashMap<String, String> {
    use futures::StreamExt;
    let budget = Arc::new(std::sync::atomic::AtomicUsize::new(MAX_REMOTE_TOTAL));
    futures::stream::iter(urls.iter().take(MAX_REMOTE_IMAGES).cloned())
        .map(|u| fetch_one(u, allow_local, budget.clone()))
        .buffer_unordered(REMOTE_PARALLEL)
        .filter_map(|r| async move { r })
        .collect()
        .await
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
    fn only_public_addresses_are_allowed() {
        use std::net::IpAddr;
        for bad in [
            "127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "0.0.0.0", "100.64.0.1",
            "224.0.0.1", "255.255.255.255", "198.18.0.1", "::1", "::", "fe80::1", "fc00::1", "fd12::1",
            "::ffff:127.0.0.1", "::ffff:10.0.0.1", "64:ff9b::a00:1", "2002:7f00:1::", "ff02::1", "2001:db8::1",
        ] {
            assert!(!is_public_ip(bad.parse::<IpAddr>().unwrap()), "{bad}");
        }
        for ok in ["8.8.8.8", "77.88.55.60", "2a02:6b8::2:242", "::ffff:8.8.8.8"] {
            assert!(is_public_ip(ok.parse::<IpAddr>().unwrap()), "{ok}");
        }
    }

    #[tokio::test]
    async fn remote_images_never_reach_internal_hosts() {
        let urls: Vec<String> = [
            "http://127.0.0.1/a.png", "http://localhost/a.png", "http://[::1]/a.png", "http://169.254.169.254/latest/meta-data",
            "http://10.0.0.1/a.png", "https://8.8.8.8:22/a.png", "ftp://example.com/a.png", "http://user:pw@example.com/a.png",
            "file:///etc/passwd",
        ].iter().map(|s| s.to_string()).collect();
        let started = std::time::Instant::now();
        assert!(fetch_remote(&urls, false).await.is_empty());
        // Отказ до любого подключения — быстро (localhost разрешается без сети).
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
    }

    #[test]
    fn plain_text_message_has_no_html_part() {
        let msg = MessageParser::default().parse(b"From: a@b.ru\r\nSubject: t\r\n\r\nhello\r\n".as_slice()).expect("parse");
        assert!(html_part(&msg).is_none());
    }
}
