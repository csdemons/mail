//! Диагностика: сырые команды и отчёт о сервере для разбора проблем.
//!
//! Часть модуля `imap` — общий обзор в `mod.rs`.

use super::prelude::*;
use super::*;

/// Выполняет произвольную команду и возвращает все ответы сервера текстом.
#[doc(hidden)]
pub async fn raw_command(conn: &mut Connection, command: &str) -> Vec<String> {
    use async_imap::imap_proto::types::Response;
    let mut out = Vec::new();
    let Ok(session) = conn.imap() else { return vec!["Exchange: команды IMAP не применяются".into()] };
    let tag = match session.run_command(command).await {
        Ok(tag) => tag,
        Err(e) => return vec![format!("ошибка отправки: {e}")],
    };
    while let Ok(session) = conn.imap() {
        match session.read_response().await {
            Ok(Some(resp)) => {
                let done = matches!(resp.parsed(), Response::Done { tag: t, .. } if *t == tag);
                out.push(format!("{:?}", resp.parsed()));
                if done {
                    break;
                }
            }
            Ok(None) => {
                out.push("соединение закрыто".into());
                break;
            }
            Err(e) => {
                out.push(format!("ошибка чтения: {e}"));
                // Поток ответов сломан — дальше работаем через новое соединение.
                if let Err(e) = conn.reconnect().await {
                    out.push(format!("переподключение не удалось: {e}"));
                }
                break;
            }
        }
    }
    out
}

/// Отчёт для разбора проблем: что сервер отвечает на ключевые запросы.
/// Пароль в отчёт не попадает. Папка открывается только на чтение (EXAMINE).
pub async fn diagnostics(conn: &mut Connection, folder: Option<&str>) -> Result<String> {
    if conn.is_ews() {
        let folders = conn.ews().expect("ews").list_folders().await;
        return Ok(format!(
            "Atom Workspace {} — диагностика Exchange (EWS)\n\nАдрес EWS: {}\nВход: выполнен\nПапок: {}",
            env!("CARGO_PKG_VERSION"),
            conn.creds.ews.clone().unwrap_or_default(),
            folders.map(|f| f.len().to_string()).unwrap_or_else(|e| format!("ошибка: {e}"))
        ));
    }
    use std::fmt::Write;
    let mut r = String::new();
    let _ = writeln!(r, "Atom Workspace {} — диагностика IMAP", env!("CARGO_PKG_VERSION"));

    let caps = conn.capabilities().await.unwrap_or_else(|e| vec![format!("ошибка: {e}")]);
    let _ = writeln!(r, "\n## CAPABILITY\n{}", caps.join(" "));

    let _ = writeln!(r, "\n## LIST \"\" \"*\"");
    let mut ids = Vec::new();
    let listed: std::result::Result<Vec<_>, String> = match conn.imap()?.list(Some(""), Some("*")).await {
        Ok(stream) => stream.try_collect::<Vec<_>>().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match listed {
        Ok(names) => {
            for n in &names {
                let _ = writeln!(
                    r,
                    "{:?} | {:?} | {} | {}",
                    n.attributes(),
                    n.delimiter(),
                    n.name(),
                    utf7_imap::decode_utf7_imap(n.name().to_string())
                );
                if !n.attributes().iter().any(|a| matches!(a, NameAttribute::NoSelect)) {
                    ids.push(n.name().to_string());
                }
            }
        }
        Err(e) => {
            let _ = writeln!(r, "ошибка: {e}");
            conn.reconnect().await?;
        }
    }

    let _ = writeln!(r, "\n## STATUS (MESSAGES UNSEEN) по папкам");
    for id in &ids {
        match conn.imap()?.status(id, "(MESSAGES UNSEEN)").await {
            Ok(mb) => {
                let _ = writeln!(r, "ok  {id}: {} / {:?}", mb.exists, mb.unseen);
            }
            Err(e) => {
                let fatal = !matches!(e, async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_));
                let _ = writeln!(r, "ERR {id}: {e}{}", if fatal { " — соединение пересоздано" } else { "" });
                if fatal {
                    conn.reconnect().await?;
                }
            }
        }
    }

    if let Some(folder) = folder {
        conn.selected = None;
        let _ = writeln!(r, "\n## EXAMINE {folder} ({})", utf7_imap::decode_utf7_imap(folder.to_string()));
        match conn.imap()?.examine(folder).await {
            Ok(mb) => {
                let _ = writeln!(
                    r,
                    "exists={} recent={} unseen={:?} uidvalidity={:?} uidnext={:?}",
                    mb.exists, mb.recent, mb.unseen, mb.uid_validity, mb.uid_next
                );
                match conn.imap()?.uid_search("ALL").await {
                    Ok(uids) => {
                        let mut uids: Vec<u32> = uids.into_iter().collect();
                        uids.sort_unstable();
                        let _ = writeln!(
                            r,
                            "UID SEARCH ALL: {} шт., первые {:?}, последние {:?}",
                            uids.len(),
                            &uids[..uids.len().min(5)],
                            &uids[uids.len().saturating_sub(5)..]
                        );
                        if let Some(last) = uids.last() {
                            let _ = writeln!(r, "UID FETCH {last} (FLAGS RFC822.SIZE BODYSTRUCTURE):");
                            for line in raw_command(
                                conn,
                                &format!("UID FETCH {last} (UID FLAGS RFC822.SIZE BODYSTRUCTURE)"),
                            )
                            .await
                            {
                                let _ = writeln!(r, "{}", line.chars().take(1500).collect::<String>());
                            }
                        }
                    }
                    Err(e) => {
                        let _ = writeln!(r, "UID SEARCH ALL: ошибка: {e}");
                    }
                }
                if mb.exists > 0 {
                    let _ = writeln!(r, "FETCH {}:{} (UID FLAGS):", mb.exists.saturating_sub(2).max(1), mb.exists);
                    for line in raw_command(
                        conn,
                        &format!("FETCH {}:{} (UID FLAGS)", mb.exists.saturating_sub(2).max(1), mb.exists),
                    )
                    .await
                    {
                        let _ = writeln!(r, "{line}");
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(r, "ошибка: {e}");
            }
        }
        conn.selected = None;
    }
    Ok(r)
}

