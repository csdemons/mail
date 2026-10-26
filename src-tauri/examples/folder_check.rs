//! Проверка одной папки: дерево папок и первая страница писем.
//! IMAP_HOST=... IMAP_PORT=... IMAP_USERNAME=... IMAP_PASSWORD=... cargo run --example folder_check -- "<id папки>"
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
    let folder = std::env::args().nth(1).unwrap_or_else(|| "INBOX".into());
    let mut conn = imap::connect(&creds).await.expect("connect");
    for f in imap::list_folders(&mut conn).await.expect("folders") {
        println!("{:indent$}{} [id={} role={:?} system={} select={}] {}/{}", "", f.name, f.id, f.role, f.system, f.selectable, f.total, f.unseen, indent = f.depth * 2);
    }
    match imap::list_messages(&mut conn, &folder, None, 50).await {
        Ok(p) => {
            println!("{folder}: {} items, total {}, more {}", p.items.len(), p.total, p.has_more);
            for m in p.items.iter().take(3) {
                println!("  #{} {} | {}", m.uid, m.subject, m.from_address);
            }
        }
        Err(e) => println!("{folder}: ERROR {e}"),
    }
    // Рассинхронизация: imap-proto не разбирает ответ NAMESPACE.
    println!("desync: {:?}", imap::raw_command(&mut conn, "NAMESPACE").await.iter().map(|l| l.chars().take(40).collect::<String>()).collect::<Vec<_>>());
    match imap::list_messages(&mut conn, &folder, None, 50).await {
        Ok(p) => println!("after desync {folder}: {} items, total {}", p.items.len(), p.total),
        Err(e) => println!("after desync {folder}: ERROR {e}"),
    }
    imap::raw_command(&mut conn, "NAMESPACE").await;
    let folders = imap::list_folders(&mut conn).await.expect("folders after desync");
    println!("folders after desync: {} (INBOX unseen {})", folders.len(), folders.first().map(|f| f.unseen).unwrap_or(0));
    println!("{}", imap::diagnostics(&mut conn, Some(&folder)).await.expect("diag"));
}
