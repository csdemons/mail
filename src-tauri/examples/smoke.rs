//! Смоук-тест почтового ядра против реального IMAP-сервера.
//! IMAP_HOST=... IMAP_PORT=993 IMAP_USERNAME=... IMAP_PASSWORD=... cargo run --example smoke
use rrcki_mail_lib::imap;

#[tokio::main]
async fn main() {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let creds = imap::Credentials {
        host: env("IMAP_HOST", "rp.nrcki.ru"),
        port: env("IMAP_PORT", "993").parse().expect("IMAP_PORT"),
        username: std::env::var("IMAP_USERNAME").expect("IMAP_USERNAME"),
        password: std::env::var("IMAP_PASSWORD").expect("IMAP_PASSWORD"),
    };

    let t = std::time::Instant::now();
    let mut conn = imap::connect(&creds).await.expect("connect");
    println!("login ok ({:?})", t.elapsed());

    let caps = conn.capabilities().await.expect("caps");
    println!("capabilities: {}", caps.join(" "));

    let folders = imap::list_folders(&mut conn).await.expect("folders");
    for f in &folders {
        println!(
            "  {:indent$}{} [{:?}{}] {} / {} unseen",
            "", f.name, f.role, if f.selectable { "" } else { ", noselect" }, f.total, f.unseen,
            indent = f.depth * 2
        );
    }

    let t = std::time::Instant::now();
    let page = imap::list_messages(&mut conn, "INBOX", None, 50).await.expect("page");
    println!("INBOX page: {} of {} (more: {}) in {:?}", page.items.len(), page.total, page.has_more, t.elapsed());
    for m in page.items.iter().take(5) {
        println!("  #{} {} | {} <{}> | att={} unread={} flagged={}", m.uid, m.subject, m.from_name, m.from_address, m.has_attachments, m.unread, m.flagged);
    }
    let mut cursor = page.items.last().map(|m| m.uid);
    let mut seen = page.items.len();
    while let Some(before) = cursor {
        let next = imap::list_messages(&mut conn, "INBOX", Some(before), 50).await.expect("next page");
        seen += next.items.len();
        cursor = if next.has_more { next.items.last().map(|m| m.uid) } else { None };
    }
    println!("paged through {seen} of {} messages", page.total);
    println!("{}", imap::diagnostics(&mut conn, Some("INBOX")).await.expect("diag").lines().take(12).collect::<Vec<_>>().join("\n"));

    for m in page.items.iter().take(5) {
        let d = imap::get_message(&mut conn, "INBOX", m.uid).await.expect("message");
        println!("--- #{} «{}» cc={:?} attachments={:?}", m.uid, d.summary.subject, d.cc,
            d.attachments.iter().map(|a| (&a.name, &a.content_type, a.size)).collect::<Vec<_>>());
        println!("{}", d.body_text.lines().take(4).collect::<Vec<_>>().join(" ⏎ "));
        if !d.attachments.is_empty() {
            let (name, data) = imap::get_attachment(&mut conn, "INBOX", m.uid, 0).await.expect("attachment");
            println!("attachment[0] {name}: {:?}", String::from_utf8_lossy(&data));
        }
    }

    if let Some(m) = page.items.iter().find(|m| m.unread) {
        imap::set_flag(&mut conn, "INBOX", m.uid, imap::FlagKind::Seen, true).await.expect("flag");
        let d = imap::get_message(&mut conn, "INBOX", m.uid).await.expect("message");
        println!("mark seen #{} -> unread={}", m.uid, d.summary.unread);
        imap::set_flag(&mut conn, "INBOX", m.uid, imap::FlagKind::Seen, false).await.expect("flag");
    }
    conn.logout().await;
}
