//! Перенос почты: импорт из Thunderbird (mbox), Outlook (.pst), файлов .eml,
//! импорт контактов (.vcf, .csv) и резервная копия папок в файлы mbox.
//! Долгие операции идут в фоне; интерфейс опрашивает их ход по номеру задачи.

use std::collections::{HashMap, HashSet};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use serde::Serialize;

use crate::dav::ContactInput;
use crate::error::{AppError, Result};
use crate::imap::{self, Credentials};
use crate::mbox::{self, Imported, Next};
use crate::pst;

// ---------------------------------------------------------------- задачи

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub kind: String,
    /// Что происходит сейчас («Чтение файла», «Папка Входящие» …).
    pub phase: String,
    /// Ход: выполнено из всего (байты файлов или письма).
    pub done: u64,
    pub total: u64,
    pub imported: u64,
    pub duplicates: u64,
    pub skipped: u64,
    pub finished: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    /// Итог для пользователя.
    pub summary: Option<String>,
    /// Первые замечания (что пропущено и почему).
    pub notes: Vec<String>,
}

pub struct Job {
    cancel: AtomicBool,
    status: std::sync::Mutex<JobStatus>,
}

impl Job {
    pub fn new(kind: &str) -> Arc<Job> {
        Arc::new(Job { cancel: AtomicBool::new(false), status: std::sync::Mutex::new(JobStatus { kind: kind.into(), ..Default::default() }) })
    }

    pub fn update(&self, f: impl FnOnce(&mut JobStatus)) {
        if let Ok(mut s) = self.status.lock() {
            f(&mut s);
        }
    }

    pub fn note(&self, text: String) {
        self.update(|s| {
            if s.notes.len() < 30 {
                s.notes.push(text);
            }
        });
    }

    pub fn status(&self) -> JobStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Завершение: итог или ошибка.
    pub fn finish(&self, result: Result<String>) {
        let cancelled = self.cancelled();
        self.update(|s| {
            s.finished = true;
            s.cancelled = cancelled;
            match result {
                Ok(text) => s.summary = Some(text),
                Err(e) => s.error = Some(e.to_string()),
            }
        });
    }
}

#[derive(Default)]
pub struct Jobs {
    map: std::sync::Mutex<HashMap<u64, Arc<Job>>>,
    next: AtomicU64,
}

impl Jobs {
    pub fn add(&self, job: Arc<Job>) -> u64 {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut m) = self.map.lock() {
            // Завершённые задачи старше последних 20 не нужны.
            if m.len() > 20 {
                let mut done: Vec<u64> = m.iter().filter(|(_, j)| j.status().finished).map(|(k, _)| *k).collect();
                done.sort_unstable();
                for k in done.iter().take(m.len() - 20) {
                    m.remove(k);
                }
            }
            m.insert(id, job);
        }
        id
    }

    pub fn get(&self, id: u64) -> Option<Arc<Job>> {
        self.map.lock().ok()?.get(&id).cloned()
    }

    /// Идёт ли задача данного вида (одновременно — одна).
    pub fn running(&self, kind: &str) -> bool {
        self.map.lock().map(|m| m.values().any(|j| !j.status().finished && j.status().kind == kind)).unwrap_or(false)
    }

    pub fn cancel_all(&self) {
        if let Ok(m) = self.map.lock() {
            m.values().for_each(|j| j.cancel());
        }
    }
}

// ---------------------------------------------------------------- источники импорта

/// Что импортируется: папка назначения (внутри «Импорт») и откуда.
#[derive(Debug, Clone)]
enum Unit {
    Mbox { path: Vec<String>, file: PathBuf },
    Eml { path: Vec<String>, files: Vec<PathBuf> },
    Pst { path: Vec<String>, file: PathBuf },
}

impl Unit {
    fn bytes(&self) -> u64 {
        let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        match self {
            Unit::Mbox { file, .. } | Unit::Pst { file, .. } => size(file),
            Unit::Eml { files, .. } => files.iter().map(|f| size(f)).sum(),
        }
    }
}

/// Выбранный пользователем источник импорта (пути остаются в программе).
#[derive(Debug, Clone)]
pub struct Source {
    pub title: String,
    units: Vec<Unit>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub id: String,
    pub title: String,
    /// Папки, которые будут созданы (первые 50).
    pub folders: Vec<String>,
    pub folder_count: usize,
    pub bytes: u64,
    pub outlook: bool,
}

impl Source {
    pub fn info(&self, id: String) -> SourceInfo {
        let names: Vec<String> = self
            .units
            .iter()
            .map(|u| match u {
                Unit::Mbox { path, .. } | Unit::Eml { path, .. } => path.join(" / "),
                Unit::Pst { path, file } => format!("{} (файл Outlook {})", path.join(" / "), file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
            })
            .collect();
        SourceInfo {
            id,
            title: self.title.clone(),
            folder_count: names.len(),
            folders: names.into_iter().take(50).collect(),
            bytes: self.units.iter().map(Unit::bytes).sum(),
            outlook: self.units.iter().any(|u| matches!(u, Unit::Pst { .. })),
        }
    }
}

fn file_stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "Импорт".into())
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn tb_path(parts: Vec<String>) -> Vec<String> {
    parts.into_iter().map(|p| mbox::folder_title(&p)).collect()
}

/// Источник из выбранных файлов: .pst/.ost, .eml, mbox (с расширением или без).
pub fn source_from_files(files: Vec<PathBuf>) -> Result<Source> {
    let mut units = Vec::new();
    let mut emls = Vec::new();
    let mut unknown = Vec::new();
    for f in files {
        match ext(&f).as_str() {
            "pst" | "ost" => units.push(Unit::Pst { path: vec![file_stem(&f)], file: f }),
            "eml" => emls.push(f),
            "msg" => return Err(AppError::Other("Файлы .msg (отдельные письма Outlook) не поддерживаются: перетащите письма из Outlook в папку на компьютере как .eml или импортируйте файл .pst.".into())),
            "vcf" | "csv" => return Err(AppError::Other("Это файл контактов — импортируйте его в разделе «Контакты» этих настроек.".into())),
            _ => {
                if looks_like_mail_file(&f) {
                    units.push(Unit::Mbox { path: vec![mbox::folder_title(&file_stem(&f))], file: f });
                } else {
                    unknown.push(f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                }
            }
        }
    }
    if !emls.is_empty() {
        emls.sort();
        units.push(Unit::Eml { path: vec!["Письма".into()], files: emls });
    }
    if units.is_empty() {
        return Err(AppError::Other(format!("В выбранных файлах нет почты ({}). Поддерживаются .pst, .eml и mbox (папки Thunderbird).", unknown.join(", "))));
    }
    let title = match units.as_slice() {
        [Unit::Pst { file, .. }] | [Unit::Mbox { file, .. }] => file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        _ => format!("Выбрано файлов: {}", units.iter().map(|u| if let Unit::Eml { files, .. } = u { files.len() } else { 1 }).sum::<usize>()),
    };
    Ok(Source { title, units })
}

fn looks_like_mail_file(p: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 512];
    let Ok(mut f) = std::fs::File::open(p) else { return false };
    let n = f.read(&mut buf).unwrap_or(0);
    let head = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
    head.starts_with("from ") || ["\nfrom:", "\nsubject:", "\nreceived:", "\nmessage-id:", "\ndate:"].iter().filter(|h| format!("\n{head}").contains(*h)).count() >= 2
}

/// Источник из каталога: учётная запись или профиль Thunderbird, папка с .eml.
/// `account` — имя учётной записи Thunderbird: папки импортируются внутрь неё.
pub fn source_from_dir(dir: &Path, title: Option<String>, account: Option<String>) -> Result<Source> {
    let mut units: Vec<Unit> = mbox::scan_folders(dir)
        .into_iter()
        .map(|(path, file)| {
            let mut p: Vec<String> = account.iter().cloned().collect();
            p.extend(tb_path(path));
            Unit::Mbox { path: p, file }
        })
        .collect();
    // Профиль целиком: учётные записи в Mail/ и ImapMail/ (поиск scan_folders их не видит).
    if units.is_empty() {
        for kind in ["Mail", "ImapMail"] {
            let Ok(accounts) = std::fs::read_dir(dir.join(kind)) else { continue };
            for acc in accounts.flatten().filter(|e| e.path().is_dir()) {
                let name = acc.file_name().to_string_lossy().to_string();
                let name = if name == "Local Folders" { "Локальные папки".to_string() } else { name };
                for (path, file) in mbox::scan_folders(&acc.path()) {
                    let mut p = vec![name.clone()];
                    p.extend(tb_path(path));
                    units.push(Unit::Mbox { path: p, file });
                }
            }
        }
    }
    // Письма .eml (с подкаталогами).
    let mut by_dir: HashMap<Vec<String>, Vec<PathBuf>> = HashMap::new();
    collect_eml(dir, &mut Vec::new(), &mut by_dir, 0);
    let mut eml: Vec<_> = by_dir.into_iter().collect();
    eml.sort();
    for (path, mut files) in eml {
        files.sort();
        let path = if path.is_empty() { vec!["Письма".to_string()] } else { path };
        units.push(Unit::Eml { path, files });
    }
    // PST в каталоге (например, «Документы\Файлы Outlook»).
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut pst: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && matches!(ext(p).as_str(), "pst")).collect();
        pst.sort();
        units.extend(pst.into_iter().map(|f| Unit::Pst { path: vec![file_stem(&f)], file: f }));
    }
    if units.is_empty() {
        return Err(AppError::Other("В этой папке не найдено почты: ни папок Thunderbird, ни файлов .eml или .pst.".into()));
    }
    let title = title.unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| dir.display().to_string()));
    Ok(Source { title, units })
}

fn collect_eml(dir: &Path, prefix: &mut Vec<String>, out: &mut HashMap<Vec<String>, Vec<PathBuf>>, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let Ok(meta) = std::fs::symlink_metadata(e.path()) else { continue };
        let name = e.file_name().to_string_lossy().to_string();
        if meta.is_dir() && !name.ends_with(".sbd") && !name.starts_with('.') {
            prefix.push(name);
            collect_eml(&e.path(), prefix, out, depth + 1);
            prefix.pop();
        } else if meta.is_file() && ext(&e.path()) == "eml" {
            out.entry(prefix.clone()).or_default().push(e.path());
        }
    }
}

// ---------------------------------------------------------------- импорт почты

enum Item {
    Mail(Vec<String>, Imported),
    Skipped(Vec<String>, String),
    Progress(u64),
    Folder(Vec<String>),
}

/// Чтение источника в отдельном потоке: письма по одному уходят в канал.
fn produce(units: Vec<Unit>, tx: tokio::sync::mpsc::Sender<Item>, job: Arc<Job>) -> Result<()> {
    let mut base = 0u64;
    for unit in units {
        if job.cancelled() {
            break;
        }
        let size = unit.bytes();
        match unit {
            Unit::Mbox { path, file } => {
                let _ = tx.blocking_send(Item::Folder(path.clone()));
                let f = std::fs::File::open(&file).map_err(|e| AppError::Other(format!("{}: {e}", file.display())))?;
                let mut reader = mbox::MboxReader::new(BufReader::with_capacity(256 * 1024, f));
                loop {
                    if job.cancelled() {
                        return Ok(());
                    }
                    let item = match reader.next_mail() {
                        Ok(None) => break,
                        Ok(Some(Next::Mail(m))) => Item::Mail(path.clone(), m),
                        Ok(Some(Next::Skipped(why))) => Item::Skipped(path.clone(), why.into()),
                        Err(e) => {
                            let _ = tx.blocking_send(Item::Skipped(path.clone(), format!("ошибка чтения файла: {e}")));
                            break;
                        }
                    };
                    if tx.blocking_send(item).is_err() || tx.blocking_send(Item::Progress(base + reader.position)).is_err() {
                        return Ok(());
                    }
                }
            }
            Unit::Eml { path, files } => {
                let _ = tx.blocking_send(Item::Folder(path.clone()));
                let mut done = 0u64;
                for f in files {
                    if job.cancelled() {
                        return Ok(());
                    }
                    let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let item = match std::fs::read(&f) {
                        Ok(data) => {
                            done += data.len() as u64;
                            match mbox::read_eml(&data) {
                                Next::Mail(m) => Item::Mail(path.clone(), m),
                                Next::Skipped(why) => Item::Skipped(path.clone(), format!("{name}: {why}")),
                            }
                        }
                        Err(e) => Item::Skipped(path.clone(), format!("{name}: {e}")),
                    };
                    if tx.blocking_send(item).is_err() || tx.blocking_send(Item::Progress(base + done)).is_err() {
                        return Ok(());
                    }
                }
            }
            Unit::Pst { path, file } => {
                if ext(&file) == "ost" {
                    let _ = tx.blocking_send(Item::Skipped(path.clone(), "файлы .ost не читаются — в Outlook выберите «Файл → Открыть и экспортировать → Импорт и экспорт → Экспорт в файл → .pst» и импортируйте полученный .pst".into()));
                    base += size;
                    continue;
                }
                struct ToChannel<'a> {
                    prefix: Vec<String>,
                    tx: &'a tokio::sync::mpsc::Sender<Item>,
                    job: &'a Job,
                }
                impl ToChannel<'_> {
                    /// Стандартные папки англоязычного Outlook — под русскими названиями.
                    fn full(&self, path: &[String]) -> Vec<String> {
                        let mut p = self.prefix.clone();
                        p.extend(path.iter().map(|x| mbox::folder_title(x)));
                        p
                    }
                }
                impl pst::Sink for ToChannel<'_> {
                    fn on_folder(&mut self, path: &[String], _count: usize) {
                        let p = self.full(path);
                        let _ = self.tx.blocking_send(Item::Folder(p));
                    }
                    fn on_message(&mut self, path: &[String], mail: Imported) -> bool {
                        let p = self.full(path);
                        !self.job.cancelled() && self.tx.blocking_send(Item::Mail(p, mail)).is_ok()
                    }
                    fn on_skipped(&mut self, path: &[String], reason: &str) {
                        let p = self.full(path);
                        let _ = self.tx.blocking_send(Item::Skipped(p, reason.to_string()));
                    }
                }
                let mut sink = ToChannel { prefix: path.clone(), tx: &tx, job: &job };
                if let Err(e) = pst::read(&file, &mut sink) {
                    let _ = tx.blocking_send(Item::Skipped(path, e.to_string()));
                }
            }
        }
        base += size;
        let _ = tx.blocking_send(Item::Progress(base));
    }
    Ok(())
}

/// Импорт источника в папку `root` («Импорт») на сервере.
pub async fn import_mail(creds: Credentials, source: Source, root: String, job: Arc<Job>) {
    let result = import_inner(&creds, source, &root, &job).await;
    job.finish(result);
}

async fn import_inner(creds: &Credentials, source: Source, root: &str, job: &Arc<Job>) -> Result<String> {
    let total: u64 = source.units.iter().map(Unit::bytes).sum();
    job.update(|s| {
        s.total = total.max(1);
        s.phase = "Подключение к серверу".into();
    });
    let mut conn = imap::connect(creds).await?;
    let delimiter = imap::hierarchy_delimiter(&mut conn).await?;
    // Очередь короткая: письмо может весить до 50 МБ, в памяти — не больше двух.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Item>(2);
    let producer = {
        let job = job.clone();
        let units = source.units.clone();
        tokio::task::spawn_blocking(move || produce(units, tx, job))
    };

    let mut folders: HashMap<Vec<String>, (String, HashSet<String>)> = HashMap::new();
    let mut created: Vec<String> = Vec::new();
    let mut failures_in_row = 0u32;
    while let Some(item) = rx.recv().await {
        if job.cancelled() {
            break;
        }
        match item {
            Item::Progress(done) => job.update(|s| s.done = done.min(s.total)),
            Item::Folder(path) => job.update(|s| s.phase = format!("Папка «{}»", path.join(" / "))),
            Item::Skipped(path, why) => {
                job.update(|s| s.skipped += 1);
                job.note(format!("{}: {why}", path.join(" / ")));
            }
            Item::Mail(path, mail) => {
                if !folders.contains_key(&path) {
                    let mut full = vec![root.to_string()];
                    full.extend(path.iter().cloned());
                    let name = imap::ensure_folder_path(&mut conn, &full, &delimiter).await?;
                    let known = imap::message_ids(&mut conn, &name).await.unwrap_or_default();
                    created.push(full.join(" / "));
                    folders.insert(path.clone(), (name, known));
                }
                let (name, known) = folders.get_mut(&path).expect("папка создана выше");
                let id = mail_parser::MessageParser::default().parse_headers(&mail.raw).and_then(|m| m.message_id().map(str::to_string));
                if id.as_ref().is_some_and(|id| known.contains(id)) {
                    job.update(|s| s.duplicates += 1);
                    continue;
                }
                let mut result = imap::append_imported(&mut conn, name, mail.seen, mail.flagged, mail.answered, mail.date, &mail.raw).await;
                if matches!(result, Err(AppError::ConnectionLost | AppError::Timeout)) {
                    imap::reconnect_now(&mut conn).await?;
                    result = imap::append_imported(&mut conn, name, mail.seen, mail.flagged, mail.answered, mail.date, &mail.raw).await;
                }
                match result {
                    Ok(()) => {
                        failures_in_row = 0;
                        if let Some(id) = id {
                            known.insert(id);
                        }
                        job.update(|s| s.imported += 1);
                    }
                    Err(e) => {
                        failures_in_row += 1;
                        job.update(|s| s.skipped += 1);
                        let subject = mail_parser::MessageParser::default().parse_headers(&mail.raw).and_then(|m| m.subject().map(str::to_string)).unwrap_or_default();
                        job.note(format!("{}: «{subject}» — сервер не принял письмо: {e}", path.join(" / ")));
                        if failures_in_row >= 20 {
                            return Err(AppError::Other(format!("Сервер отклоняет письма подряд, импорт остановлен: {e}")));
                        }
                    }
                }
            }
        }
    }
    drop(rx);
    producer.await.map_err(|e| AppError::Other(e.to_string()))??;
    conn.logout().await;
    let s = job.status();
    let mut text = format!("Загружено писем: {}", s.imported);
    if s.duplicates > 0 {
        text.push_str(&format!(", уже были на сервере: {}", s.duplicates));
    }
    if s.skipped > 0 {
        text.push_str(&format!(", пропущено: {}", s.skipped));
    }
    if let Some(first) = created.first() {
        text.push_str(&format!(". Папки — в «{}»", first.split(" / ").next().unwrap_or(first)));
    }
    if job.cancelled() {
        text.insert_str(0, "Импорт остановлен. ");
    }
    Ok(text)
}

// ---------------------------------------------------------------- резервная копия

/// Папка для резервной копии: имя на сервере и путь для показа.
#[derive(Debug, Clone)]
pub struct BackupFolder {
    pub id: String,
    pub display: Vec<String>,
}

/// Каталог копии: письма — личные данные, поэтому в Linux доступ только владельцу.
fn create_backup_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    let b = {
        use std::os::unix::fs::DirBuilderExt;
        let mut b = std::fs::DirBuilder::new();
        b.mode(0o700);
        b
    };
    #[cfg(not(unix))]
    let b = std::fs::DirBuilder::new();
    b.create(dir).map_err(|e| AppError::Other(format!("Не удалось создать папку {}: {e}", dir.display())))?;
    crate::winacl::restrict(dir);
    Ok(())
}

/// Путь файла папки внутри каталога копии: «A.sbd/B.mbox» (как в Thunderbird,
/// поэтому копию можно и открыть в Thunderbird, и вернуть импортом с той же структурой).
fn backup_file(dir: &Path, display: &[String]) -> PathBuf {
    let mut p = dir.to_path_buf();
    for (i, part) in display.iter().enumerate() {
        let safe = imap::sanitize_filename(part).replace(['/', '\\'], "_");
        if i + 1 == display.len() {
            p.push(format!("{safe}.mbox"));
        } else {
            p.push(format!("{safe}.sbd"));
        }
    }
    p
}

pub async fn backup(creds: Credentials, folders: Vec<BackupFolder>, dir: PathBuf, job: Arc<Job>) {
    let result = backup_inner(&creds, folders, &dir, &job).await;
    job.finish(result);
}

async fn backup_inner(creds: &Credentials, folders: Vec<BackupFolder>, dir: &Path, job: &Arc<Job>) -> Result<String> {
    job.update(|s| s.phase = "Подключение к серверу".into());
    let mut conn = imap::connect(creds).await?;
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M");
    let target = dir.join(format!("Atom Mail — копия {stamp}"));
    create_backup_dir(&target)?;

    // Сначала считаем письма, чтобы показать ход.
    let mut plan = Vec::new();
    for f in folders {
        if job.cancelled() {
            break;
        }
        job.update(|s| s.phase = format!("Подсчёт писем: «{}»", f.display.join(" / ")));
        match imap::all_uids(&mut conn, &f.id).await {
            Ok(uids) => {
                job.update(|s| s.total += uids.len() as u64);
                plan.push((f, uids));
            }
            Err(e) => job.note(format!("{}: {e}", f.display.join(" / "))),
        }
    }
    let mut files = 0;
    let max = crate::imap::MAX_MESSAGE_MB * 1024 * 1024;
    for (folder, uids) in plan {
        if job.cancelled() {
            break;
        }
        job.update(|s| s.phase = format!("Папка «{}»", folder.display.join(" / ")));
        let path = backup_file(&target, &folder.display);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Новый файл только для владельца (0600), без перехода по ссылкам.
        let file = crate::updater::create_new_file(&path).map_err(|e| AppError::Other(format!("{}: {e}", path.display())))?;
        let mut out = Some(std::io::BufWriter::new(file));
        files += 1;
        imap::all_uids(&mut conn, &folder.id).await?;
        let too_big: Vec<u32> = uids.iter().filter(|(_, size)| *size > max).map(|(u, _)| *u).collect();
        if !too_big.is_empty() {
            job.note(format!("{}: писем больше {} МБ не сохранено: {}", folder.display.join(" / "), crate::imap::MAX_MESSAGE_MB, too_big.len()));
            job.update(|s| {
                s.skipped += too_big.len() as u64;
                s.done += too_big.len() as u64;
            });
        }
        let fitting: Vec<(u32, u32)> = uids.iter().copied().filter(|(_, size)| *size <= max).collect();
        // Порции не больше 10 писем и 16 МБ: в памяти — одна порция.
        for chunk in crate::store::body_batches(&fitting) {
            let chunk = &chunk[..];
            if job.cancelled() {
                break;
            }
            let batch = match imap::export_batch(&mut conn, chunk).await {
                Ok(b) => b,
                Err(e) => {
                    // Обрыв: переподключаемся и повторяем порцию один раз.
                    imap::reconnect_now(&mut conn).await?;
                    imap::all_uids(&mut conn, &folder.id).await?;
                    imap::export_batch(&mut conn, chunk).await.map_err(|_| e)?
                }
            };
            let missing = chunk.len().saturating_sub(batch.len());
            let mut writer = out.take().expect("файл открыт");
            let (writer, written) = tokio::task::spawn_blocking(move || {
                let mut n = 0u64;
                for m in &batch {
                    mbox::write_mbox_entry(&mut writer, &m.raw, m.date, m.seen, m.answered, m.flagged)?;
                    n += 1;
                }
                Ok::<_, std::io::Error>((writer, n))
            })
            .await
            .map_err(|e| AppError::Other(e.to_string()))??;
            out = Some(writer);
            job.update(|s| {
                s.imported += written;
                s.skipped += missing as u64;
                s.done += chunk.len() as u64;
            });
        }
        if let Some(mut w) = out {
            use std::io::Write;
            w.flush()?;
        }
        if job.cancelled() {
            // Недописанная папка — не копия: файл удаляется, готовые папки остаются.
            let _ = std::fs::remove_file(&path);
            files -= 1;
            break;
        }
    }
    conn.logout().await;
    let s = job.status();
    let mut text = format!("Сохранено писем: {} в {files} файлах — папка «{}»", s.imported, target.display());
    if s.skipped > 0 {
        text.push_str(&format!(", не загрузились: {}", s.skipped));
    }
    if job.cancelled() {
        text.insert_str(0, "Копирование остановлено. ");
    }
    Ok(text)
}

// ---------------------------------------------------------------- контакты

/// Текст файла: UTF-8 (с BOM или без), UTF-16 (Outlook «Юникод»), иначе Windows-1251.
pub fn decode_text(data: &[u8]) -> String {
    if let Some((enc, bom)) = encoding_rs::Encoding::for_bom(data) {
        return enc.decode_without_bom_handling(&data[bom..]).0.into_owned();
    }
    match std::str::from_utf8(data) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1251.decode_without_bom_handling(data).0.into_owned(),
    }
}

fn qp_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            if let (Some(h), Some(l)) = (b.get(i + 1).and_then(|c| (*c as char).to_digit(16)), b.get(i + 2).and_then(|c| (*c as char).to_digit(16))) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn vcard_unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\N", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// Карточки vCard 2.1/3.0/4.0 (Outlook, Thunderbird, телефоны).
pub fn parse_vcf(text: &str) -> Vec<ContactInput> {
    // Склейка строк: продолжение начинается с пробела; в 2.1 строка QP может кончаться «=».
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n').map(|l| l.trim_end_matches('\r')) {
        if let Some(last) = lines.last_mut() {
            if raw.starts_with([' ', '\t']) {
                last.push_str(&raw[1..]);
                continue;
            }
            if last.to_ascii_uppercase().contains("QUOTED-PRINTABLE") && last.ends_with('=') {
                last.pop();
                last.push_str(raw);
                continue;
            }
        }
        lines.push(raw.to_string());
    }
    let mut out = Vec::new();
    let mut cur: Option<(ContactInput, String)> = None;
    for line in lines {
        let Some((head, value)) = line.split_once(':') else { continue };
        let mut params = head.split(';');
        let name = params.next().unwrap_or_default().rsplit('.').next().unwrap_or_default().to_ascii_uppercase();
        let params: Vec<String> = params.map(|p| p.to_ascii_uppercase()).collect();
        if name == "BEGIN" && value.eq_ignore_ascii_case("VCARD") {
            cur = Some((ContactInput { full_name: String::new(), emails: vec![], phones: vec![], org: None, title: None, note: None }, String::new()));
            continue;
        }
        let Some((c, n_field)) = cur.as_mut() else { continue };
        if name == "END" {
            let (mut c, n_field) = cur.take().expect("карточка открыта");
            if c.full_name.trim().is_empty() {
                // Имя из N (Фамилия;Имя;Отчество) или из адреса.
                let parts: Vec<&str> = n_field.split(';').collect();
                let fio: Vec<&str> = [parts.first(), parts.get(1), parts.get(2)].into_iter().flatten().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                c.full_name = if !fio.is_empty() { fio.join(" ") } else { c.emails.first().map(|e| e.split('@').next().unwrap_or(e).to_string()).unwrap_or_default() };
            }
            if !c.full_name.trim().is_empty() && (!c.emails.is_empty() || !c.phones.is_empty()) {
                out.push(c);
            }
            continue;
        }
        let qp = params.iter().any(|p| p.contains("QUOTED-PRINTABLE"));
        let charset = params.iter().find_map(|p| p.strip_prefix("CHARSET=")).map(str::to_string);
        let value = if qp {
            let bytes = qp_decode(value);
            let enc = charset.as_deref().and_then(|c| encoding_rs::Encoding::for_label(c.as_bytes())).unwrap_or(encoding_rs::UTF_8);
            enc.decode_without_bom_handling(&bytes).0.into_owned()
        } else {
            value.to_string()
        };
        let value = value.trim();
        match name.as_str() {
            "FN" => c.full_name = vcard_unescape(value),
            "N" => *n_field = value.to_string(),
            "EMAIL" => {
                let e = vcard_unescape(value).trim().trim_start_matches("mailto:").to_string();
                if e.contains('@') && !c.emails.iter().any(|x| x.eq_ignore_ascii_case(&e)) {
                    c.emails.push(e);
                }
            }
            "TEL" => {
                let t = vcard_unescape(value).trim_start_matches("tel:").trim().to_string();
                if !t.is_empty() && !c.phones.contains(&t) && !params.iter().any(|p| p.contains("FAX")) {
                    c.phones.push(t);
                }
            }
            "ORG" => {
                let units: Vec<String> = value.split(';').map(vcard_unescape).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                if !units.is_empty() {
                    c.org = Some(units.join(", "));
                }
            }
            "TITLE" => c.title = Some(vcard_unescape(value)).filter(|s| !s.is_empty()),
            "NOTE" => c.note = Some(vcard_unescape(value)).filter(|s| !s.is_empty()),
            _ => {}
        }
    }
    out
}

fn csv_rows(text: &str, delim: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.is_empty() {
            quoted = true;
        } else if c == delim {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' {
            row.push(std::mem::take(&mut field).trim_end_matches('\r').to_string());
            if row.iter().any(|f| !f.trim().is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
        } else {
            field.push(c);
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field.trim_end_matches('\r').to_string());
        rows.push(row);
    }
    rows
}

/// Контакты из CSV: экспорт Outlook и Thunderbird (заголовки по-русски или по-английски).
pub fn parse_csv(text: &str) -> Result<Vec<ContactInput>> {
    let text = text.trim_start_matches('\u{feff}');
    let first = text.lines().next().unwrap_or_default();
    let delim = [',', ';', '\t'].into_iter().max_by_key(|d| first.matches(*d).count()).unwrap_or(',');
    let rows = csv_rows(text, delim);
    let Some((header, data)) = rows.split_first() else { return Ok(Vec::new()) };
    let h: Vec<String> = header.iter().map(|s| s.trim().to_lowercase()).collect();
    let find = |pred: &dyn Fn(&str) -> bool| h.iter().position(|x| pred(x));
    let cols = |pred: &dyn Fn(&str) -> bool| h.iter().enumerate().filter(|(_, x)| pred(x)).map(|(i, _)| i).collect::<Vec<_>>();
    let is_email = |x: &str| (x.contains("e-mail") || x.contains("email") || x.contains("почт")) && !x.contains("тип") && !x.contains("type") && !x.contains("отображ") && !x.contains("display");
    let emails = cols(&is_email);
    if emails.is_empty() {
        return Err(AppError::Other("В файле нет столбца с адресом почты (E-mail Address / Адрес эл. почты).".into()));
    }
    let full = find(&|x| x == "name" || x == "display name" || x == "full name" || x == "отображаемое имя" || x == "полное имя" || x == "имя для отображения");
    let first_name = find(&|x| x == "first name" || x == "given name" || x == "имя");
    let middle = find(&|x| x == "middle name" || x == "отчество");
    let last = find(&|x| x == "last name" || x == "family name" || x == "фамилия");
    let phones = cols(&|x| (x.contains("phone") || x.contains("телефон") || x.contains("mobile") || x.contains("мобильн")) && !x.contains("fax") && !x.contains("факс"));
    let org = find(&|x| x == "company" || x == "organization" || x == "организация" || x == "компания" || x == "организация (компания)");
    let title = find(&|x| x == "job title" || x == "title" || x == "должность");
    let note = find(&|x| x == "notes" || x == "note" || x == "заметки" || x == "примечания");
    let get = |row: &[String], i: Option<usize>| i.and_then(|i| row.get(i)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    for row in data {
        let mut addr: Vec<String> = Vec::new();
        for &i in &emails {
            if let Some(e) = row.get(i).map(|s| s.trim()).filter(|s| s.contains('@')) {
                if !addr.iter().any(|x| x.eq_ignore_ascii_case(e)) {
                    addr.push(e.to_string());
                }
            }
        }
        let phone_list: Vec<String> = phones.iter().filter_map(|&i| row.get(i)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if addr.is_empty() && phone_list.is_empty() {
            continue;
        }
        // «Фамилия Имя Отчество», как в адресной книге программы.
        let fio: Vec<String> = [get(row, last), get(row, first_name), get(row, middle)].into_iter().flatten().collect();
        let name = get(row, full).or_else(|| (!fio.is_empty()).then(|| fio.join(" "))).or_else(|| addr.first().map(|e| e.split('@').next().unwrap_or(e).to_string()));
        let Some(full_name) = name else { continue };
        out.push(ContactInput { full_name, emails: addr, phones: phone_list, org: get(row, org), title: get(row, title), note: get(row, note) });
    }
    Ok(out)
}

/// Контакты из файла по расширению (.vcf или .csv).
pub fn read_contacts(path: &Path) -> Result<Vec<ContactInput>> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > 64 * 1024 * 1024 {
        return Err(AppError::Other("Файл контактов слишком большой (больше 64 МБ).".into()));
    }
    let text = decode_text(&std::fs::read(path)?);
    match ext(path).as_str() {
        "vcf" | "vcard" => Ok(parse_vcf(&text)),
        "csv" | "txt" => parse_csv(&text),
        _ if text.contains("BEGIN:VCARD") => Ok(parse_vcf(&text)),
        _ => Err(AppError::Other("Выберите файл .vcf (vCard) или .csv (экспорт Outlook, Thunderbird).".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcard_21_quoted_printable_cp1251() {
        let text = "BEGIN:VCARD\r\nVERSION:2.1\r\nN;CHARSET=windows-1251;ENCODING=QUOTED-PRINTABLE:=C8=E2=E0=ED=EE=E2;=C8=E2=E0=ED\r\nEMAIL;PREF;INTERNET:ivanov@example.ru\r\nTEL;WORK;VOICE:+7 495 000\r\nTEL;WORK;FAX:+7 495 001\r\nORG:НИЦ;Отдел\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:Петров Пётр\r\nitem1.EMAIL;TYPE=INTERNET:petrov@ex\r\n ample.ru\r\nNOTE:строка\\nвторая\r\nEND:VCARD\r\nBEGIN:VCARD\r\nFN:Без адреса\r\nEND:VCARD\r\n";
        let v = parse_vcf(text);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].full_name, "Иванов Иван");
        assert_eq!(v[0].emails, ["ivanov@example.ru"]);
        assert_eq!(v[0].phones, ["+7 495 000"]);
        assert_eq!(v[0].org.as_deref(), Some("НИЦ, Отдел"));
        assert_eq!(v[1].emails, ["petrov@example.ru"]);
        assert_eq!(v[1].note.as_deref(), Some("строка\nвторая"));
    }

    #[test]
    fn outlook_csv_russian_and_english() {
        let ru = "\"Имя\";\"Отчество\";\"Фамилия\";\"Организация\";\"Должность\";\"Телефон раб.\";\"Адрес эл. почты\";\"Тип эл. почты\";\"Краткое имя эл. почты\"\r\n\"Иван\";\"Иванович\";\"Сидоров\";\"НИЦ\";\"инженер\";\"123\";\"sidorov@example.ru\";\"SMTP\";\"Сидоров\"\r\n\"\";\"\";\"\";\"\";\"\";\"\";\"\";\"\";\"\"\r\n";
        let v = parse_csv(ru).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].full_name, "Сидоров Иван Иванович");
        assert_eq!(v[0].emails, ["sidorov@example.ru"]);
        assert_eq!(v[0].phones, ["123"]);
        assert_eq!(v[0].title.as_deref(), Some("инженер"));
        let en = "First Name,Last Name,Display Name,Primary Email,Secondary Email,Work Phone,Notes\nAnna,Smith,\"Smith, Anna\",anna@ex.com,a2@ex.com,,\"multi\nline\"\nBob,,,,,555,\n";
        let v = parse_csv(en).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].full_name, "Smith, Anna");
        assert_eq!(v[0].emails, ["anna@ex.com", "a2@ex.com"]);
        assert_eq!(v[0].note.as_deref(), Some("multi\nline"));
        assert_eq!(v[1].full_name, "Bob");
        assert!(parse_csv("a,b\n1,2\n").is_err());
    }

    #[test]
    fn decodes_files_and_backup_paths() {
        assert_eq!(decode_text(&[0xcf, 0xf0, 0xe8]), "При");
        assert_eq!(decode_text("\u{feff}Да".as_bytes()), "Да");
        assert_eq!(decode_text(&[0xff, 0xfe, 0x14, 0x04]), "Д");
        let p = backup_file(Path::new("/b"), &["Входящие".into(), "Проекты: 2024".into()]);
        assert_eq!(p, Path::new("/b/Входящие.sbd/Проекты_ 2024.mbox"));
    }

    #[test]
    fn source_from_dir_finds_thunderbird_and_eml() {
        let dir = std::env::temp_dir().join(format!("rrcki-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Mail/Local Folders/Inbox.sbd")).unwrap();
        std::fs::write(dir.join("Mail/Local Folders/Inbox"), b"From - Mon Jan  1 10:00:00 2024\nSubject: a\n\nx\n").unwrap();
        std::fs::write(dir.join("Mail/Local Folders/Inbox.sbd/Work"), b"From - Mon Jan  1 10:00:00 2024\nSubject: b\n\ny\n").unwrap();
        let s = source_from_dir(&dir, None, None).unwrap();
        let info = s.info("1".into());
        assert_eq!(info.folders, ["Локальные папки / Входящие", "Локальные папки / Входящие / Work"]);
        std::fs::create_dir_all(dir.join("eml/sub")).unwrap();
        std::fs::write(dir.join("eml/sub/1.eml"), b"Subject: c\n\nz\n").unwrap();
        let s = source_from_dir(&dir.join("eml"), None, None).unwrap();
        assert_eq!(s.info("2".into()).folders, ["sub"]);
        assert!(source_from_dir(&dir.join("eml/sub/none"), None, None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
