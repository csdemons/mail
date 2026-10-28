//! Проверка SMTP и CalDAV/CardDAV без интерфейса.
//! SMTP_HOST=… SMTP_PORT=465 DAV_URL=https://… IMAP_USERNAME=… IMAP_PASSWORD=… cargo run --example groupware_check -- [получатель]
use rrcki_mail_lib::{dav, smtp};

#[tokio::main]
async fn main() {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let user = std::env::var("IMAP_USERNAME").expect("IMAP_USERNAME");
    let pass = std::env::var("IMAP_PASSWORD").expect("IMAP_PASSWORD");

    if let Some(rcpt) = std::env::args().nth(1) {
        let (host, port) = (env("SMTP_HOST", "rp.rrcki.ru"), env("SMTP_PORT", "465").parse().unwrap());
        let msg = smtp::build(smtp::Outgoing {
            from_name: Some("Проверка RRCKI Mail".into()),
            from_address: user.clone(),
            to: vec![rcpt],
            cc: vec![],
            subject: "Проверка отправки RRCKI Mail".into(),
            body: "Тестовое письмо.\nЕсли вы его видите — SMTP работает.".into(),
            html: None,
            in_reply_to: None,
            references: None,
            attachments: vec![],
        })
        .expect("build");
        match smtp::send(&host, port, &user, &pass, &msg).await {
            Ok(()) => println!("SMTP {host}:{port}: отправлено"),
            Err(e) => println!("SMTP {host}:{port}: ОШИБКА {e}"),
        }
    }

    let base = url::Url::parse(&env("DAV_URL", "https://rp.nrcki.ru/")).unwrap();
    let mut client = dav::DavClient::new(&user, &pass, &base).unwrap();
    match client.discover(&base, dav::Kind::Contacts).await {
        Ok(books) => {
            for b in &books {
                let list = client.contacts(b).await.unwrap_or_default();
                println!("книга «{}»: {} контактов", b.name, list.len());
                for c in list.iter().take(5) {
                    println!("  {} {:?} {:?}", c.full_name, c.emails, c.org);
                }
            }
        }
        Err(e) => println!("CardDAV: ОШИБКА {e}"),
    }
    let now = chrono::Utc::now();
    match client.discover(&base, dav::Kind::Calendar).await {
        Ok(cals) => {
            for c in &cals {
                let ev = client
                    .events(c, now - chrono::Duration::days(7), now + chrono::Duration::days(14))
                    .await
                    .unwrap_or_default();
                println!("календарь «{}» {:?}: {} событий за 3 недели", c.name, c.color, ev.len());
                for e in ev.iter().take(8) {
                    println!("  {} — {} {}", e.start, e.title, if e.recurring { "(повтор)" } else { "" });
                }
            }
        }
        Err(e) => println!("CalDAV: ОШИБКА {e}"),
    }
    println!("--- журнал запросов ---\n{}", client.log.join("\n"));
}
