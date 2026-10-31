//! Почтовые файлы на диске: чтение и запись mbox (Thunderbird, «Экспорт» других
//! программ, резервные копии), письма .eml, поиск профилей Thunderbird и сборка
//! письма MIME из частей (для импорта из Outlook).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use base64::Engine;

/// Письмо для импорта: исходник (строки с CRLF) и флаги.
#[derive(Debug, Clone)]
pub struct Imported {
    pub raw: Vec<u8>,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    /// Дата получения (секунды Unix) — станет INTERNALDATE на сервере.
    pub date: Option<i64>,
}

const MAX_MESSAGE: usize = crate::imap::MAX_MESSAGE_MB as usize * 1024 * 1024;

// ---------------------------------------------------------------- чтение mbox

/// Результат чтения одного письма mbox.
pub enum Next {
    Mail(Imported),
    /// Письмо пропущено (слишком большое или удалено в Thunderbird).
    Skipped(&'static str),
}

/// Потоковое чтение mbox: память — одно письмо, файл может быть любого размера.
pub struct MboxReader<R: BufRead> {
    input: R,
    /// Строка «From …» следующего письма (уже прочитана).
    pending_from: Option<Vec<u8>>,
    started: bool,
    /// Прочитано байт (для хода импорта).
    pub position: u64,
}

impl<R: BufRead> MboxReader<R> {
    pub fn new(input: R) -> Self {
        MboxReader { input, pending_from: None, started: false, position: 0 }
    }

    fn line(&mut self, buf: &mut Vec<u8>) -> std::io::Result<bool> {
        buf.clear();
        let n = self.input.read_until(b'\n', buf)?;
        self.position += n as u64;
        Ok(n > 0)
    }

    pub fn next_mail(&mut self) -> std::io::Result<Option<Next>> {
        let mut line = Vec::new();
        let from_line = match self.pending_from.take() {
            Some(l) => l,
            None => {
                // Начало файла: пропускаем всё до первой строки «From ».
                loop {
                    if !self.line(&mut line)? {
                        return Ok(None);
                    }
                    if line.starts_with(b"From ") {
                        break line.clone();
                    }
                    if !self.started && !line.iter().all(u8::is_ascii_whitespace) {
                        // Не mbox, а одно письмо без строки «From » — читаем как есть.
                        self.started = true;
                        let mut raw = line.clone();
                        while self.line(&mut line)? {
                            if raw.len() <= MAX_MESSAGE {
                                raw.extend_from_slice(&line);
                            }
                        }
                        return Ok(Some(finish(raw, None)));
                    }
                }
            }
        };
        self.started = true;
        let mut raw: Vec<u8> = Vec::new();
        let mut oversized = false;
        let mut previous_blank = true;
        while self.line(&mut line)? {
            let blank = line == b"\n" || line == b"\r\n";
            if previous_blank && line.starts_with(b"From ") && looks_like_from_line(&line) {
                self.pending_from = Some(line.clone());
                break;
            }
            previous_blank = blank;
            if oversized {
                continue;
            }
            // mboxrd: «>From » в теле — экранированная строка «From ».
            let unescaped: &[u8] = if line.starts_with(b">") && line.iter().skip_while(|&&b| b == b'>').take(5).eq(b"From ".iter()) { &line[1..] } else { &line };
            push_crlf(&mut raw, unescaped);
            if raw.len() > MAX_MESSAGE {
                oversized = true;
                raw = Vec::new();
            }
        }
        if oversized {
            return Ok(Some(Next::Skipped("письмо больше 50 МБ")));
        }
        // Пустая строка-разделитель перед следующим «From » — не часть письма.
        if raw.ends_with(b"\r\n\r\n") {
            raw.truncate(raw.len() - 2);
        }
        Ok(Some(finish(raw, Some(&from_line))))
    }
}

/// «From адрес дата» — строка-разделитель (а не «From » в начале строки тела).
fn looks_like_from_line(line: &[u8]) -> bool {
    let s = String::from_utf8_lossy(line);
    let mut it = s.split_whitespace();
    it.next() == Some("From") && it.next().is_some() && s.chars().any(|c| c.is_ascii_digit())
}

fn push_crlf(out: &mut Vec<u8>, line: &[u8]) {
    if let Some(body) = line.strip_suffix(b"\r\n") {
        out.extend_from_slice(body);
    } else if let Some(body) = line.strip_suffix(b"\n") {
        out.extend_from_slice(body);
    } else {
        out.extend_from_slice(line);
    }
    out.extend_from_slice(b"\r\n");
}

/// Письмо из файла .eml (строки приводятся к CRLF).
pub fn read_eml(data: &[u8]) -> Next {
    if data.len() > MAX_MESSAGE {
        return Next::Skipped("письмо больше 50 МБ");
    }
    let mut raw = Vec::with_capacity(data.len() + data.len() / 40);
    for line in data.split_inclusive(|&b| b == b'\n') {
        push_crlf(&mut raw, line);
    }
    finish(raw, None)
}

/// Служебные заголовки Thunderbird и Gmail: флаги письма. Сами заголовки убираются.
fn finish(mut raw: Vec<u8>, from_line: Option<&[u8]>) -> Next {
    let header_end = find(&raw, b"\r\n\r\n").map(|i| i + 2).unwrap_or(raw.len());
    let mut seen = true;
    let mut flagged = false;
    let mut answered = false;
    let mut deleted = false;
    let mut remove: Vec<(usize, usize)> = Vec::new();
    let mut pos = 0;
    let mut starts = Vec::new();
    while pos < header_end {
        let end = find(&raw[pos..header_end], b"\r\n").map(|i| pos + i + 2).unwrap_or(header_end);
        starts.push((pos, end));
        pos = end;
    }
    let mut i = 0;
    while i < starts.len() {
        let (s, mut e) = starts[i];
        let mut j = i + 1;
        while j < starts.len() && matches!(raw.get(starts[j].0), Some(b' ' | b'\t')) {
            e = starts[j].1;
            j += 1;
        }
        let field = String::from_utf8_lossy(&raw[s..e]).to_string();
        if let Some((name, value)) = field.split_once(':') {
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "x-mozilla-status" => {
                    if let Ok(bits) = u32::from_str_radix(value, 16) {
                        seen = bits & 0x0001 != 0;
                        answered = bits & 0x0002 != 0;
                        flagged = bits & 0x0004 != 0;
                        deleted = bits & 0x0008 != 0;
                    }
                    remove.push((s, e));
                }
                "x-mozilla-status2" | "x-mozilla-keys" | "x-account-key" => remove.push((s, e)),
                "x-gmail-labels" => {
                    let labels: Vec<String> = value.split(',').map(|l| l.trim().trim_matches('"').to_lowercase()).collect();
                    if labels.iter().any(|l| l == "unread" || l == "непрочитанные") {
                        seen = false;
                    }
                    if labels.iter().any(|l| l == "starred" || l == "помеченные") {
                        flagged = true;
                    }
                }
                "status" if value.contains('R') || value.contains('O') => seen = value.contains('R'),
                _ => {}
            }
        }
        i = j;
    }
    if deleted {
        return Next::Skipped("письмо удалено в Thunderbird");
    }
    for (s, e) in remove.into_iter().rev() {
        raw.drain(s..e);
    }
    let date = mail_parser::MessageParser::default()
        .parse_headers(&raw)
        .and_then(|m| m.date().map(|d| d.to_timestamp()))
        .or_else(|| from_line.and_then(from_line_date));
    Next::Mail(Imported { raw, seen, flagged, answered, date })
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Дата из строки «From - Mon Jan  1 10:00:00 2024».
fn from_line_date(line: &[u8]) -> Option<i64> {
    let s = String::from_utf8_lossy(line);
    let words: Vec<&str> = s.split_whitespace().collect();
    let tail = words.get(words.len().checked_sub(5)?..)?.join(" ");
    chrono::NaiveDateTime::parse_from_str(&tail, "%a %b %d %H:%M:%S %Y").ok().map(|d| d.and_utc().timestamp())
}

// ---------------------------------------------------------------- запись mbox

/// Дописывает письмо в mbox (формат mboxrd, флаги — в заголовке Thunderbird, чтобы
/// копию можно было открыть в Thunderbird или вернуть импортом с теми же флагами).
pub fn write_mbox_entry(out: &mut impl Write, raw: &[u8], date: Option<i64>, seen: bool, answered: bool, flagged: bool) -> std::io::Result<()> {
    let when = date.and_then(|d| chrono::DateTime::from_timestamp(d, 0)).unwrap_or_else(chrono::Utc::now);
    writeln!(out, "From - {}", when.format("%a %b %e %H:%M:%S %Y"))?;
    let status = u32::from(seen) | (u32::from(answered) << 1) | (u32::from(flagged) << 2);
    writeln!(out, "X-Mozilla-Status: {status:04x}")?;
    writeln!(out, "X-Mozilla-Status2: 00000000")?;
    for line in raw.split_inclusive(|&b| b == b'\n') {
        let body = line.strip_suffix(b"\n").map(|l| l.strip_suffix(b"\r").unwrap_or(l)).unwrap_or(line);
        if body.iter().skip_while(|&&b| b == b'>').take(5).eq(b"From ".iter()) {
            out.write_all(b">")?;
        }
        out.write_all(body)?;
        out.write_all(b"\n")?;
    }
    out.write_all(b"\n")
}

// ---------------------------------------------------------------- Thunderbird

/// Учётная запись (каталог) в профиле Thunderbird.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TbAccount {
    pub name: String,
    /// Имя учётной записи без профиля — папка, в которую идёт импорт.
    pub account: String,
    pub path: String,
    /// Папок и общий размер (для подсказки).
    pub folders: usize,
    pub bytes: u64,
}

fn profile_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(appdata).join("Thunderbird").join("Profiles"));
    }
    #[cfg(not(windows))]
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join("Library/Thunderbird/Profiles"));
        roots.push(home.join(".thunderbird"));
        roots.push(home.join("snap/thunderbird/common/.thunderbird"));
        roots.push(home.join(".var/app/org.mozilla.Thunderbird/.thunderbird"));
        roots.push(home.join(".icedove"));
    }
    // Автотесты: каталог профилей задаётся явно (в релизе этого нет).
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("RRCKI_E2E_TB_PROFILES") {
        roots = vec![PathBuf::from(dir)];
    }
    roots
}

/// Учётные записи всех профилей Thunderbird на компьютере.
pub fn thunderbird_accounts() -> Vec<TbAccount> {
    let mut out = Vec::new();
    for root in profile_roots() {
        let Ok(profiles) = std::fs::read_dir(&root) else { continue };
        for profile in profiles.flatten().filter(|e| e.path().is_dir()) {
            let profile_name = profile.file_name().to_string_lossy().split_once('.').map(|(_, n)| n.to_string()).unwrap_or_else(|| profile.file_name().to_string_lossy().to_string());
            for kind in ["Mail", "ImapMail"] {
                let Ok(accounts) = std::fs::read_dir(profile.path().join(kind)) else { continue };
                for acc in accounts.flatten().filter(|e| e.path().is_dir()) {
                    let files = scan_folders(&acc.path());
                    if files.is_empty() {
                        continue;
                    }
                    let bytes = files.iter().filter_map(|(_, p)| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
                    let acc_name = acc.file_name().to_string_lossy().to_string();
                    let name = if acc_name == "Local Folders" { "Локальные папки".to_string() } else { acc_name };
                    out.push(TbAccount { account: name.clone(), name: format!("{name} ({profile_name})"), path: acc.path().display().to_string(), folders: files.len(), bytes });
                }
            }
        }
    }
    out
}

/// Почтовые папки (файлы mbox) каталога Thunderbird: путь папки и файл.
/// Вложенные папки Thunderbird хранит в каталоге «Имя.sbd».
pub fn scan_folders(root: &Path) -> Vec<(Vec<String>, PathBuf)> {
    let mut out = Vec::new();
    scan(root, &mut Vec::new(), &mut out, 0);
    out.sort();
    out
}

fn scan(dir: &Path, prefix: &mut Vec<String>, out: &mut Vec<(Vec<String>, PathBuf)>, depth: usize) {
    if depth > 12 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.is_dir() {
            if let Some(folder) = name.strip_suffix(".sbd") {
                prefix.push(folder.to_string());
                scan(&path, prefix, out, depth + 1);
                prefix.pop();
            }
            continue;
        }
        if !meta.is_file() || meta.len() == 0 {
            continue;
        }
        let ext = path.extension().map(|x| x.to_string_lossy().to_lowercase());
        let candidate = matches!(ext.as_deref(), None | Some("mbox" | "mbx"));
        if !candidate {
            continue;
        }
        let has_msf = dir.join(format!("{name}.msf")).exists();
        if has_msf || starts_with_from(&path) {
            let folder = name.strip_suffix(".mbox").or_else(|| name.strip_suffix(".mbx")).unwrap_or(&name).to_string();
            let mut p = prefix.clone();
            p.push(folder);
            out.push((p, path));
        }
    }
}

fn starts_with_from(path: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 5];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut buf)).is_ok() && &buf == b"From "
}

/// Понятное имя служебной папки Thunderbird.
pub fn folder_title(name: &str) -> String {
    match name {
        "Inbox" => "Входящие".into(),
        "Sent" | "Sent Items" => "Отправленные".into(),
        "Deleted Items" => "Удалённые".into(),
        "Junk E-mail" | "Junk Email" => "Спам".into(),
        "Outbox" => "Исходящие".into(),
        "Drafts" => "Черновики".into(),
        "Trash" => "Корзина".into(),
        "Junk" => "Спам".into(),
        "Archives" => "Архив".into(),
        "Templates" => "Шаблоны".into(),
        "Unsent Messages" => "Неотправленные".into(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------- сборка MIME

/// Слово заголовка RFC 2047 (если есть не-ASCII).
pub fn encode_word(s: &str) -> String {
    let s: String = s.chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }).collect();
    if s.is_ascii() {
        return s;
    }
    // Куски не длиннее 45 байт UTF-8 (≈60 символов base64), не разрывая символы.
    let mut parts = Vec::new();
    let mut chunk = String::new();
    for c in s.chars() {
        if chunk.len() + c.len_utf8() > 45 {
            parts.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        parts.push(chunk);
    }
    parts.iter().map(|p| format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(p))).collect::<Vec<_>>().join("\r\n ")
}

/// Имя в адресе: ASCII в кавычках, иначе RFC 2047.
pub fn encode_phrase(s: &str) -> String {
    if s.is_ascii() {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        encode_word(s)
    }
}

fn b64_lines(data: &[u8]) -> String {
    let enc = base64::engine::general_purpose::STANDARD.encode(data);
    let mut out = String::with_capacity(enc.len() + enc.len() / 38 + 2);
    for chunk in enc.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push_str("\r\n");
    }
    out
}

fn boundary(n: usize) -> String {
    format!("----=_AtomImport_{n}_{}", uuid::Uuid::new_v4().simple())
}

fn text_part(kind: &str, text: &str) -> String {
    format!("Content-Type: text/{kind}; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}", b64_lines(text.as_bytes()))
}

fn filename_params(name: &str) -> (String, String) {
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    if clean.is_ascii() {
        let q = clean.replace('\\', "_").replace('"', "'");
        return (format!("name=\"{q}\""), format!("filename=\"{q}\""));
    }
    let pct = percent_encoding::utf8_percent_encode(&clean, percent_encoding::NON_ALPHANUMERIC).to_string();
    (format!("name=\"{}\"", encode_word(&clean).replace("\r\n ", "")), format!("filename*=UTF-8''{pct}"))
}

fn attachment_part(p: &crate::pst::Part, inline: bool) -> String {
    let (name, filename) = filename_params(&p.name);
    if p.message {
        let body = String::from_utf8_lossy(&p.data);
        return format!("Content-Type: message/rfc822; {name}\r\nContent-Disposition: attachment; {filename}\r\n\r\n{body}\r\n");
    }
    let mut s = format!("Content-Type: {}; {name}\r\nContent-Transfer-Encoding: base64\r\n", p.mime);
    if let Some(id) = &p.content_id {
        s.push_str(&format!("Content-ID: <{}>\r\n", id.trim_matches(['<', '>'])));
    }
    s.push_str(&format!("Content-Disposition: {}; {filename}\r\n\r\n", if inline { "inline" } else { "attachment" }));
    s.push_str(&b64_lines(&p.data));
    s
}

fn multipart(kind: &str, parts: &[String], n: usize) -> String {
    let b = boundary(n);
    let mut s = format!("Content-Type: multipart/{kind}; boundary=\"{b}\"\r\n\r\n");
    for p in parts {
        s.push_str(&format!("--{b}\r\n{p}\r\n"));
    }
    s.push_str(&format!("--{b}--\r\n"));
    s
}

/// Письмо из заголовков, текста, HTML и вложений.
pub fn compose(headers: &[(String, String)], text: Option<&str>, html: Option<&str>, parts: &[crate::pst::Part]) -> Vec<u8> {
    let mut out = String::new();
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("MIME-Version: 1.0\r\n");
    let html = html.filter(|h| !h.trim().is_empty());
    let text = text.filter(|t| !t.trim().is_empty() || html.is_none()).unwrap_or("");
    let body = match html {
        Some(h) => multipart("alternative", &[text_part("plain", text), text_part("html", h)], 1),
        None => text_part("plain", text),
    };
    let (inline, attached): (Vec<&crate::pst::Part>, Vec<&crate::pst::Part>) =
        parts.iter().partition(|p| html.is_some() && p.content_id.as_deref().is_some_and(|id| html.is_some_and(|h| h.contains(id.trim_matches(['<', '>'])))));
    let body = if inline.is_empty() {
        body
    } else {
        let mut list = vec![body];
        list.extend(inline.iter().map(|p| attachment_part(p, true)));
        multipart("related", &list, 2)
    };
    let body = if attached.is_empty() {
        body
    } else {
        let mut list = vec![body];
        list.extend(attached.iter().map(|p| attachment_part(p, false)));
        multipart("mixed", &list, 3)
    };
    out.push_str(&body);
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_parser::MimeHeaders;

    fn mails(data: &[u8]) -> Vec<Next> {
        let mut r = MboxReader::new(std::io::Cursor::new(data.to_vec()));
        let mut v = Vec::new();
        while let Some(n) = r.next_mail().unwrap() {
            v.push(n);
        }
        v
    }

    #[test]
    fn reads_thunderbird_mbox() {
        let data = b"From - Mon Jan  1 10:00:00 2024\nX-Mozilla-Status: 0005\nX-Mozilla-Status2: 00000000\nSubject: one\nDate: Mon, 01 Jan 2024 10:00:00 +0000\n\nbody\n>From here\nFrom the start of a line\n\nFrom - Tue Jan  2 10:00:00 2024\nX-Mozilla-Status: 0000\nSubject: two\n\nsecond\n\nFrom - Tue Jan  2 11:00:00 2024\nX-Mozilla-Status: 0009\nSubject: deleted\n\nx\n";
        let v = mails(data);
        assert_eq!(v.len(), 3);
        let Next::Mail(a) = &v[0] else { panic!() };
        assert!(a.seen && a.flagged && !a.answered);
        assert_eq!(a.date, Some(1_704_103_200));
        let text = String::from_utf8_lossy(&a.raw);
        assert!(!text.contains("X-Mozilla"), "{text}");
        assert!(text.starts_with("Subject: one\r\n"));
        assert!(text.ends_with("body\r\nFrom here\r\nFrom the start of a line\r\n"), "{text:?}");
        let Next::Mail(b) = &v[1] else { panic!() };
        assert!(!b.seen);
        assert_eq!(b.date, Some(1_704_189_600), "дата из строки From");
        assert!(matches!(v[2], Next::Skipped(_)));
    }

    #[test]
    fn gmail_labels_and_plain_eml() {
        let v = mails(b"From 123@xxx Mon Jan 01 10:00:00 +0000 2024\r\nX-Gmail-Labels: Inbox,Unread,Starred\r\nSubject: g\r\n\r\nhi\r\n");
        let Next::Mail(m) = &v[0] else { panic!() };
        assert!(!m.seen && m.flagged);
        let Next::Mail(e) = read_eml(b"Subject: e\n\nline\n") else { panic!() };
        assert_eq!(e.raw, b"Subject: e\r\n\r\nline\r\n");
        assert!(e.seen);
        // Файл без «From » в начале — одно письмо.
        assert_eq!(mails(b"Subject: x\n\ny\n").len(), 1);
    }

    #[test]
    fn mbox_round_trip() {
        let mut out = Vec::new();
        write_mbox_entry(&mut out, b"Subject: a\r\n\r\nFrom me\r\n>From you\r\n", Some(1_704_103_200), true, true, false).unwrap();
        write_mbox_entry(&mut out, b"Subject: b\r\n\r\nx\r\n", None, false, false, true).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.starts_with("From - Mon Jan  1 10:00:00 2024\nX-Mozilla-Status: 0003\n"), "{text}");
        assert!(text.contains("\n>From me\n>>From you\n"));
        let v = mails(&out);
        assert_eq!(v.len(), 2);
        let Next::Mail(a) = &v[0] else { panic!() };
        assert_eq!(a.raw, b"Subject: a\r\n\r\nFrom me\r\n>From you\r\n");
        assert!(a.seen && a.answered);
        let Next::Mail(b) = &v[1] else { panic!() };
        assert!(!b.seen && b.flagged);
    }

    #[test]
    fn scans_thunderbird_folders() {
        let dir = std::env::temp_dir().join(format!("rrcki-tb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Archives.sbd")).unwrap();
        std::fs::write(dir.join("Inbox"), b"From - x 1\n\n").unwrap();
        std::fs::write(dir.join("Inbox.msf"), b"").unwrap();
        std::fs::write(dir.join("Archives.sbd/2023"), b"From - Mon Jan  1 10:00:00 2024\n").unwrap();
        std::fs::write(dir.join("popstate.dat"), b"x").unwrap();
        std::fs::write(dir.join("filterlog"), b"not mail").unwrap();
        let found: Vec<Vec<String>> = scan_folders(&dir).into_iter().map(|(p, _)| p).collect();
        assert_eq!(found, vec![vec!["Archives".to_string(), "2023".into()], vec!["Inbox".into()]]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn composes_mime() {
        let parts = vec![
            crate::pst::Part { name: "отчёт.pdf".into(), mime: "application/pdf".into(), content_id: None, data: b"%PDF".to_vec(), message: false },
            crate::pst::Part { name: "img.png".into(), mime: "image/png".into(), content_id: Some("img1".into()), data: vec![1, 2, 3], message: false },
        ];
        let raw = compose(&[("Subject".into(), encode_word("Привет"))], Some("текст"), Some("<p>html <img src=\"cid:img1\"></p>"), &parts);
        let msg = mail_parser::MessageParser::default().parse(&raw).unwrap();
        assert_eq!(msg.subject(), Some("Привет"));
        assert_eq!(msg.body_text(0).as_deref(), Some("текст"));
        assert!(msg.body_html(0).unwrap().contains("html"));
        let names: Vec<_> = msg.attachments().map(|a| a.attachment_name().unwrap_or_default().to_string()).collect();
        assert!(names.contains(&"отчёт.pdf".to_string()), "{names:?}");
        assert!(String::from_utf8_lossy(&raw).contains("multipart/related"));
        assert_eq!(encode_phrase("Ivan \"I\""), "\"Ivan \\\"I\\\"\"");
    }
}
