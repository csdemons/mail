//! Проверка поиска адресатов через веб-API SOGo без интерфейса:
//! `SOGO_URL=https://rp.example.ru/ cargo run --example sogo_check -- user@example.ru Иванов`
//! (пароль — из переменной `MAIL_PASSWORD` или спросит).
use rrcki_mail_lib::sogo;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let user = args.next().expect("логин");
    let query = args.next().unwrap_or_else(|| "иван".into());
    let base = url::Url::parse(&std::env::var("SOGO_URL").expect("SOGO_URL")).expect("URL");
    let pass = std::env::var("MAIL_PASSWORD").expect("MAIL_PASSWORD");
    let mut log = Vec::new();
    let res = sogo::login(&base, &user, &pass, &mut log).await;
    for l in &log {
        println!("{l}");
    }
    let session = res.expect("вход");
    let (status, raw) = session.search_raw(&query).await.expect("поиск");
    println!("allContactSearch → {status}: {}", raw.chars().take(300).collect::<String>());
    for c in session.search(&query).await.expect("разбор") {
        println!("{} <{}> {:?} {:?}", c.full_name, c.emails.join(", "), c.title, c.org);
    }
    session.logout().await;
}
