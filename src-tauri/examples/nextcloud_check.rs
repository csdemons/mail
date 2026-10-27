//! Проверка Nextcloud (Login Flow v2, Deck, Talk) без интерфейса.
//! NC_URL=https://next.nrcki.ru cargo run --example nextcloud_check
use rrcki_mail_lib::nextcloud as nc;

#[tokio::main]
async fn main() {
    let base = nc::normalize_base(&std::env::var("NC_URL").unwrap_or_else(|_| "https://next.nrcki.ru/".into())).unwrap();
    let flow = nc::login_flow_start(&base).await.expect("login flow");
    println!("Откройте в браузере и войдите: {}", flow.login_url);
    let creds = loop {
        match nc::login_flow_poll(&base, &flow).await.expect("poll") {
            Some(c) => break c,
            None => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
        }
    };
    let probe = nc::Nc::new(creds.clone(), &creds.login).unwrap();
    let (uid, name) = probe.user_info().await.expect("user");
    println!("вход: {} (uid {uid}, {name})", creds.login);
    let client = nc::Nc::new(creds.clone(), &uid).unwrap();

    let boards = client.deck_boards().await.expect("boards");
    for b in &boards {
        let stacks = client.deck_stacks(b.id).await.expect("stacks");
        println!("доска «{}» edit={} : {}", b.title, b.can_edit,
            stacks.iter().map(|s| format!("{}({})", s.title, s.cards.len())).collect::<Vec<_>>().join(", "));
    }
    if std::env::var("NC_WRITE").is_ok() {
        let b = boards.iter().find(|b| b.can_edit).expect("editable board");
        let stacks = client.deck_stacks(b.id).await.unwrap();
        let (s0, s1) = (stacks[0].id, stacks[1].id);
        client.deck_create_card(b.id, s0, "Проверка RRCKI Mail", "создано из клиента", None).await.expect("create");
        let card = client.deck_stacks(b.id).await.unwrap()[0].cards.iter().find(|c| c.title == "Проверка RRCKI Mail").unwrap().id;
        client.deck_update_card(b.id, s0, card, "Проверка RRCKI Mail ✓", "обновлено", Some("2026-11-01T00:00:00+00:00"), true).await.expect("update");
        client.deck_move_card(b.id, s0, card, s1, 0).await.expect("move");
        let moved = client.deck_stacks(b.id).await.unwrap()[1].cards.iter().find(|c| c.id == card).map(|c| (c.title.clone(), c.done, c.duedate.clone()));
        println!("карточка после правки и переноса: {moved:?}");
        client.deck_delete_card(b.id, s1, card).await.expect("delete");
        println!("карточка удалена");
        if let Some(ro) = boards.iter().find(|b| !b.can_edit) {
            let s = client.deck_stacks(ro.id).await.unwrap()[0].id;
            println!("запись в доску только для чтения: {:?}", client.deck_create_card(ro.id, s, "x", "", None).await.err().map(|e| e.to_string()));
        }
    }

    let rooms = client.talk_rooms().await.expect("rooms");
    for r in &rooms {
        println!("беседа «{}» [{}] непрочитано {} — {:?}", r.name, r.token, r.unread, r.last_message);
    }
    if let Some(r) = rooms.first() {
        let hist = client.talk_history(&r.token, None).await.expect("history");
        for m in &hist {
            println!("  #{} {}{}: {}", m.id, m.author, if m.own { " (я)" } else { "" }, m.text);
        }
        let last = hist.last().map(|m| m.id).unwrap_or(0);
        if std::env::var("NC_WRITE").is_ok() {
            client.talk_send(&r.token, "Привет из RRCKI Mail", hist.first().map(|m| m.id)).await.expect("send");
            let poller = nc::Nc::long_poll(creds.clone(), &uid).unwrap();
            let t = std::time::Instant::now();
            let new = poller.talk_poll(&r.token, last).await.expect("poll");
            println!("  новое ({:?}): {:?}", t.elapsed(), new.iter().map(|m| (m.own, m.text.clone(), m.parent_text.clone())).collect::<Vec<_>>());
        }
    }
}
