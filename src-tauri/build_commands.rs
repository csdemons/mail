// Общая часть build.rs обеих сборок (src-tauri и legacy): единый список команд.
//
// `commands.list` — единственное место, где перечислены команды. Отсюда:
//   * `$OUT_DIR/app_commands.rs` — `tauri::generate_handler![...]` для `app_commands!()`;
//   * (сборка Tauri 2) разрешения `allow-<команда>` и capabilities окон.
// Сборка останавливается, если список и функции с `#[tauri::command]` расходятся.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Команда из списка: имя и доступна ли окнам писем (`msg`).
#[allow(dead_code)]
pub struct Command {
    pub name: String,
    pub message_window: bool,
    /// Путь к функции: `crate::commands::<модуль>::<имя>`.
    pub path: String,
}

/// Прочитать `commands.list` и сверить его с исходниками в `src_dir`.
pub fn read_commands(list: &Path, src_dir: &Path) -> Vec<Command> {
    println!("cargo:rerun-if-changed={}", list.display());
    let text = fs::read_to_string(list).unwrap_or_else(|e| panic!("{}: {e}", list.display()));
    let mut out: Vec<Command> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let name = parts.next().unwrap_or("").to_string();
        let tags: Vec<&str> = parts.collect();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            panic!("{}:{}: неверное имя команды «{name}»", list.display(), n + 1);
        }
        if let Some(t) = tags.iter().find(|t| **t != "msg") {
            panic!("{}:{}: неизвестная метка «{t}» (допустима только msg)", list.display(), n + 1);
        }
        if out.iter().any(|c| c.name == name) {
            panic!("{}:{}: команда «{name}» указана дважды", list.display(), n + 1);
        }
        out.push(Command { name, message_window: tags.contains(&"msg"), path: String::new() });
    }

    // Все #[tauri::command] в исходниках — и только они.
    let mut defined = BTreeSet::new();
    let mut module_of = std::collections::BTreeMap::new();
    for file in rust_files(src_dir) {
        // src/commands/mail.rs → crate::commands::mail; src/commands/mod.rs → crate::commands
        let rel = file.strip_prefix(src_dir).unwrap_or(&file).with_extension("");
        let mut segs: Vec<String> = rel.iter().map(|s| s.to_string_lossy().to_string()).collect();
        if segs.last().is_some_and(|s| s == "mod") {
            segs.pop();
        }
        let module = std::iter::once("crate".to_string()).chain(segs).collect::<Vec<_>>().join("::");
        println!("cargo:rerun-if-changed={}", file.display());
        let src = fs::read_to_string(&file).unwrap_or_default();
        let mut pending = false;
        for line in src.lines() {
            let t = line.trim();
            if t.starts_with("#[tauri::command") {
                pending = true;
                continue;
            }
            if pending {
                if t.starts_with("#[") || t.starts_with("//") || t.is_empty() {
                    continue;
                }
                if let Some(rest) = t.split(" fn ").nth(1).or_else(|| t.strip_prefix("fn ")) {
                    let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    module_of.insert(name.clone(), module.clone());
                    defined.insert(name);
                }
                pending = false;
            }
        }
    }
    let listed: BTreeSet<String> = out.iter().map(|c| c.name.clone()).collect();
    let missing: Vec<_> = defined.difference(&listed).collect();
    let extra: Vec<_> = listed.difference(&defined).collect();
    if !missing.is_empty() || !extra.is_empty() {
        panic!(
            "commands.list расходится с исходниками.\n  Есть #[tauri::command], но нет в списке: {missing:?}\n  В списке, но функции нет: {extra:?}\nДобавьте или уберите строки в src-tauri/commands.list."
        );
    }
    for c in &mut out {
        c.path = format!("{}::{}", module_of[&c.name], c.name);
    }
    out
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else { return out };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            out.extend(rust_files(&p));
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
    out
}

/// `$OUT_DIR/app_commands.rs`: обработчик со всеми командами списка.
pub fn write_handler(commands: &[Command]) {
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let body: String = commands.iter().map(|c| format!("    {},\n", c.path)).collect();
    write_if_changed(&out.join("app_commands.rs"), &format!("tauri::generate_handler![\n{body}]\n"));
}

/// Capabilities окон: главное — все команды, окна писем — помеченные `msg`.
#[allow(dead_code)]
pub fn write_capabilities(commands: &[Command], dir: &Path) {
    let perms = |pick: &dyn Fn(&Command) -> bool| -> String {
        std::iter::once("    \"core:default\"".to_string())
            .chain(commands.iter().filter(|c| pick(c)).map(|c| format!("    \"allow-{}\"", c.name.replace('_', "-"))))
            .collect::<Vec<_>>()
            .join(",\n")
    };
    let cap = |id: &str, desc: &str, window: &str, list: String| {
        format!(
            "{{\n  \"$schema\": \"../gen/schemas/desktop-schema.json\",\n  \"identifier\": \"{id}\",\n  \"description\": \"{desc}\",\n  \"windows\": [\n    \"{window}\"\n  ],\n  \"permissions\": [\n{list}\n  ]\n}}\n"
        )
    };
    write_if_changed(
        &dir.join("default.json"),
        &cap("default", "Главное окно: базовые API Tauri и все команды приложения. Файл создаётся из commands.list — не правьте вручную.", "main", perms(&|_| true)),
    );
    write_if_changed(
        &dir.join("message-window.json"),
        &cap(
            "message-window",
            "Окна писем (msg-*): только команды, нужные для чтения письма, ответа и действий с ним (метка msg в commands.list; файл создаётся из него — не правьте вручную). Окна редактора документов (doc-*) не получают команд вообще.",
            "msg-*",
            perms(&|c| c.message_window),
        ),
    );
}

fn write_if_changed(path: &Path, text: &str) {
    if fs::read_to_string(path).ok().as_deref() != Some(text) {
        fs::write(path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}
