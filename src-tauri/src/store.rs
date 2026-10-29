//! Локальное хранилище почты: индекс заголовков всех писем (для мгновенного
//! списка и поиска по всей почте) и исходники писем за последние N дней (чтение
//! без сервера, вложения без повторной загрузки).
//!
//! Всё на диске зашифровано (XChaCha20-Poly1305, ключ 256 бит — в системном
//! хранилище паролей рядом с паролем почты): копия профиля, резервная копия или
//! другой пользователь терминального сервера переписку не прочитают. Каталог —
//! свой для каждой учётной записи, права 0700. «Выйти и забыть пароль» удаляет
//! и каталог, и ключ.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};
use crate::imap::{self, Credentials, Folder, MessagePage, MessageSummary};

const KEYRING_SERVICE: &str = "ru.rrcki.mail.store";
/// Письма больше этого размера на диск не кладём (их всё равно открывают редко).
pub const MAX_RAW_BYTES: u32 = 15 * 1024 * 1024;
/// Сколько текста письма держать в индексе для поиска.
const TEXT_CHARS: usize = 20_000;
pub const DEFAULT_DAYS: u32 = 7;

#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    s: MessageSummary,
    /// Текст письма (только для писем, исходник которых хранится).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default)]
    cached: bool,
    /// Строка для поиска (в нижнем регистре), строится при загрузке.
    #[serde(skip)]
    hay: String,
}

impl Entry {
    fn new(s: MessageSummary) -> Self {
        let mut e = Entry { s, text: None, cached: false, hay: String::new() };
        e.rebuild();
        e
    }

    fn rebuild(&mut self) {
        let s = &self.s;
        self.hay = format!("{} {} {} {} {}", s.from_name, s.from_address, s.subject, s.to.join(" "), self.text.as_deref().unwrap_or("")).to_lowercase();
    }

    fn timestamp(&self) -> i64 {
        self.s.date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map_or(0, |d| d.timestamp())
    }
}

#[derive(Serialize, Deserialize, Default)]
struct FolderIndex {
    folder: String,
    uid_validity: u32,
    total: u32,
    items: BTreeMap<u32, Entry>,
}

#[derive(Default)]
struct Inner {
    folders: Option<Vec<Folder>>,
    index: HashMap<String, FolderIndex>,
    /// Все индексы папок прочитаны с диска (для поиска).
    all_loaded: bool,
    dirty: HashSet<String>,
}

pub struct Store {
    dir: PathBuf,
    cipher: XChaCha20Poly1305,
    days: u32,
    inner: Mutex<Inner>,
    closed: AtomicBool,
    syncing: AtomicBool,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct StoreStatus {
    pub enabled: bool,
    pub days: u32,
    pub messages: usize,
    pub cached: usize,
    pub syncing: bool,
    pub bytes: u64,
}

fn hash(s: &str) -> String {
    Sha256::digest(s.as_bytes()).iter().take(10).map(|b| format!("{b:02x}")).collect()
}

/// Каталог хранилища учётной записи.
pub fn account_dir(root: &Path, account: &str) -> PathBuf {
    root.join("mailstore").join(hash(account))
}

enum Key {
    Found([u8; 32]),
    /// Хранилище паролей отвечает, но ключа для этой учётной записи нет.
    Missing,
}

fn key_entry(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| AppError::Other(format!("системное хранилище паролей недоступно: {e}")))
}

/// Ключ архива. «Нет ключа» и «хранилище недоступно/заблокировано» — разные
/// случаи: во втором архив не трогаем, работаем без него до следующего запуска.
fn load_key(account: &str) -> Result<Key> {
    match key_entry(account)?.get_password() {
        Ok(k) => base64::engine::general_purpose::STANDARD
            .decode(k.trim())
            .ok()
            .and_then(|raw| <[u8; 32]>::try_from(raw.as_slice()).ok())
            .map(Key::Found)
            .ok_or_else(|| AppError::Other("ключ локального хранилища повреждён".into())),
        Err(keyring::Error::NoEntry) => Ok(Key::Missing),
        Err(e) => Err(AppError::Other(format!("системное хранилище паролей недоступно: {e}"))),
    }
}

fn create_key(account: &str) -> Result<[u8; 32]> {
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).map_err(|e| AppError::Other(e.to_string()))?;
    key_entry(account)?
        .set_password(&base64::engine::general_purpose::STANDARD.encode(key))
        .map_err(|e| AppError::Other(format!("системное хранилище паролей недоступно: {e}")))?;
    Ok(key)
}

/// Удалить локальную почту учётной записи и её ключ. Ошибка — если что-то осталось.
pub fn wipe(root: &Path, account: &str) -> Result<()> {
    let dir = account_dir(root, account);
    let files = match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::Other(format!("не удалось удалить {}: {e}", dir.display()))),
    };
    let key = key_entry(account).and_then(|e| match e.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::Other(format!("ключ хранилища не удалён: {e}"))),
    });
    files.and(key)
}

/// Каталог только для владельца: создаётся с правами 0700, затем проверяется, что
/// это настоящий каталог (не ссылка), он принадлежит текущему пользователю и
/// закрыт для группы и остальных. Не удалось — хранилище не открывается.
fn create_private_dir(dir: &Path) -> Result<()> {
    let fail = |what: String| AppError::Other(format!("каталог локального хранилища {}: {what}", dir.display()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        if let Err(e) = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir) {
            return Err(fail(e.to_string()));
        }
        let meta = std::fs::symlink_metadata(dir).map_err(|e| fail(e.to_string()))?;
        if !meta.file_type().is_dir() {
            return Err(fail("не является каталогом (ссылка?)".into()));
        }
        // SAFETY: geteuid не имеет предусловий.
        if meta.uid() != unsafe { libc::geteuid() } {
            return Err(fail("принадлежит другому пользователю".into()));
        }
        if meta.mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| fail(format!("права 0700 не установлены: {e}")))?;
            let mode = std::fs::symlink_metadata(dir).map_err(|e| fail(e.to_string()))?.mode();
            if mode & 0o077 != 0 {
                return Err(fail(format!("доступен другим пользователям (права {:o})", mode & 0o777)));
            }
        }
    }
    #[cfg(not(unix))]
    {
        // Windows: каталог в %LOCALAPPDATA% профиля пользователя, ACL профиля закрывает
        // его от других пользователей; проверяем, что это не ссылка/точка соединения.
        std::fs::create_dir_all(dir).map_err(|e| fail(e.to_string()))?;
        let meta = std::fs::symlink_metadata(dir).map_err(|e| fail(e.to_string()))?;
        if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
            return Err(fail("не является каталогом (ссылка?)".into()));
        }
    }
    Ok(())
}

/// Каталоги архивов, отложенные из-за отсутствия ключа, удаляются через неделю.
fn sweep_orphans(parent: &Path) {
    let Ok(rd) = std::fs::read_dir(parent) else { return };
    let now = std::time::SystemTime::now();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|d| d.as_secs() > 7 * 86_400);
        if name.contains(".nokey-") && old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

impl Store {
    /// Открыть (или создать) хранилище учётной записи `user@host`.
    pub fn open(root: &Path, account: &str, days: u32) -> Result<Store> {
        let dir = account_dir(root, account);
        let parent = dir.parent().expect("account dir has parent").to_path_buf();
        create_private_dir(&parent)?;
        // Хранилище паролей недоступно — ошибка, архив не трогаем.
        let key = match load_key(account)? {
            Key::Found(k) => k,
            Key::Missing => {
                // Ключа точно нет (удалён или профиль перенесён): старый архив без него
                // не расшифровать. Не удаляем сразу — откладываем, через неделю он уйдёт сам.
                if dir.exists() {
                    let aside = dir.with_extension(format!("nokey-{}", chrono::Utc::now().timestamp()));
                    std::fs::rename(&dir, &aside).map_err(|e| AppError::Other(format!("{}: {e}", dir.display())))?;
                    log::warn!("local store: key missing, old archive set aside");
                }
                create_key(account)?
            }
        };
        sweep_orphans(&parent);
        create_private_dir(&dir)?;
        create_private_dir(&dir.join("raw"))?;
        Ok(Store {
            dir,
            cipher: XChaCha20Poly1305::new((&key).into()),
            days,
            inner: Mutex::new(Inner::default()),
            closed: AtomicBool::new(false),
            syncing: AtomicBool::new(false),
        })
    }

    /// Сессия закончилась (блокировка, выход): фоновая синхронизация останавливается.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.flush();
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    // ------------------------------------------------------------ шифрованные файлы

    fn write_file(&self, name: &str, data: &[u8]) -> Result<()> {
        let mut nonce = [0u8; 24];
        getrandom::getrandom(&mut nonce).map_err(|e| AppError::Other(e.to_string()))?;
        let ct = self
            .cipher
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: data, aad: name.as_bytes() })
            .map_err(|_| AppError::Other("шифрование".into()))?;
        let path = self.dir.join(name);
        let tmp = path.with_extension("tmp");
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        std::fs::write(&tmp, out).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| AppError::Other(e.to_string()))
    }

    fn read_file(&self, name: &str) -> Option<Vec<u8>> {
        let raw = std::fs::read(self.dir.join(name)).ok()?;
        if raw.len() < 24 {
            return None;
        }
        let (nonce, ct) = raw.split_at(24);
        self.cipher.decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: name.as_bytes() }).ok()
    }

    fn index_name(folder: &str) -> String {
        format!("idx-{}.bin", hash(folder))
    }

    fn raw_name(folder: &str, uid: u32) -> String {
        format!("raw/{}-{uid}.bin", hash(folder))
    }

    fn ensure_loaded<'a>(&self, inner: &'a mut Inner, folder: &str) -> &'a mut FolderIndex {
        if !inner.index.contains_key(folder) {
            let mut idx: FolderIndex = self
                .read_file(&Self::index_name(folder))
                .and_then(|d| serde_json::from_slice(&d).ok())
                .unwrap_or_default();
            idx.folder = folder.to_string();
            for e in idx.items.values_mut() {
                e.rebuild();
            }
            inner.index.insert(folder.to_string(), idx);
        }
        inner.index.get_mut(folder).expect("index loaded")
    }

    fn load_all(&self, inner: &mut Inner) {
        if inner.all_loaded {
            return;
        }
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !(name.starts_with("idx-") && name.ends_with(".bin")) {
                    continue;
                }
                let Some(idx) = self.read_file(&name).and_then(|d| serde_json::from_slice::<FolderIndex>(&d).ok()) else { continue };
                if idx.folder.is_empty() || inner.index.contains_key(&idx.folder) {
                    continue;
                }
                let mut idx = idx;
                for e in idx.items.values_mut() {
                    e.rebuild();
                }
                inner.index.insert(idx.folder.clone(), idx);
            }
        }
        inner.all_loaded = true;
    }

    /// Записать изменённые индексы на диск.
    pub fn flush(&self) {
        let pending: Vec<(String, Vec<u8>)> = {
            let mut inner = self.lock();
            let dirty: Vec<String> = inner.dirty.drain().collect();
            dirty
                .into_iter()
                .filter_map(|f| {
                    if f == "\u{0}folders" {
                        return inner.folders.as_ref().and_then(|l| serde_json::to_vec(l).ok()).map(|d| ("folders.bin".to_string(), d));
                    }
                    inner.index.get(&f).and_then(|i| serde_json::to_vec(i).ok()).map(|d| (Self::index_name(&f), d))
                })
                .collect()
        };
        for (name, data) in pending {
            if let Err(e) = self.write_file(&name, &data) {
                log::warn!("local store: {name} not written: {e}");
            }
        }
    }

    // ------------------------------------------------------------ папки и списки

    pub fn folders(&self) -> Option<Vec<Folder>> {
        let mut inner = self.lock();
        if inner.folders.is_none() {
            inner.folders = self.read_file("folders.bin").and_then(|d| serde_json::from_slice(&d).ok());
        }
        inner.folders.clone()
    }

    pub fn set_folders(&self, folders: &[Folder]) {
        let mut inner = self.lock();
        inner.folders = Some(folders.to_vec());
        inner.dirty.insert("\u{0}folders".into());
    }

    /// Страница из индекса (новые сверху) — показывается сразу, пока идёт запрос к серверу.
    pub fn page(&self, folder: &str, before_uid: Option<u32>, limit: usize) -> Option<MessagePage> {
        let mut inner = self.lock();
        let idx = self.ensure_loaded(&mut inner, folder);
        if idx.items.is_empty() {
            return None;
        }
        let upper = before_uid.unwrap_or(u32::MAX);
        let mut iter = idx.items.range(..upper).rev();
        let items: Vec<MessageSummary> = iter.by_ref().take(limit).map(|(_, e)| e.s.clone()).collect();
        let has_more = iter.next().is_some();
        Some(MessagePage { items, total: idx.total.max(idx.items.len() as u32), has_more })
    }

    /// Свежая страница с сервера: обновить записи, убрать удалённые в этом диапазоне UID.
    pub fn merge_page(&self, folder: &str, before_uid: Option<u32>, page: &MessagePage) {
        let mut inner = self.lock();
        let idx = self.ensure_loaded(&mut inner, folder);
        let lowest = if page.has_more { page.items.iter().map(|m| m.uid).min().unwrap_or(u32::MAX) } else { 0 };
        let upper = before_uid.unwrap_or(u32::MAX);
        let seen: HashSet<u32> = page.items.iter().map(|m| m.uid).collect();
        let gone: Vec<u32> = idx.items.range(lowest..upper).map(|(u, _)| *u).filter(|u| !seen.contains(u)).collect();
        for u in &gone {
            idx.items.remove(u);
            let _ = std::fs::remove_file(self.dir.join(Self::raw_name(folder, *u)));
        }
        for m in &page.items {
            match idx.items.get_mut(&m.uid) {
                Some(e) => {
                    let has_att = e.s.has_attachments;
                    e.s = m.clone();
                    e.s.has_attachments = e.s.has_attachments || has_att;
                    e.rebuild();
                }
                None => {
                    idx.items.insert(m.uid, Entry::new(m.clone()));
                }
            }
        }
        idx.total = page.total;
        inner.dirty.insert(folder.to_string());
    }

    /// Флаг изменён в программе — индекс сразу знает об этом.
    pub fn patch(&self, folder: &str, uid: u32, f: impl FnOnce(&mut MessageSummary)) {
        let mut inner = self.lock();
        let idx = self.ensure_loaded(&mut inner, folder);
        let Some(e) = idx.items.get_mut(&uid) else { return };
        f(&mut e.s);
        e.rebuild();
        inner.dirty.insert(folder.to_string());
    }

    /// Письма перемещены или удалены.
    pub fn remove(&self, folder: &str, uids: &[u32]) {
        let mut inner = self.lock();
        let idx = self.ensure_loaded(&mut inner, folder);
        for u in uids {
            idx.items.remove(u);
            let _ = std::fs::remove_file(self.dir.join(Self::raw_name(folder, *u)));
        }
        idx.total = idx.total.saturating_sub(uids.len() as u32);
        inner.dirty.insert(folder.to_string());
    }

    pub fn summary(&self, folder: &str, uid: u32) -> Option<MessageSummary> {
        let mut inner = self.lock();
        self.ensure_loaded(&mut inner, folder).items.get(&uid).map(|e| e.s.clone())
    }

    // ------------------------------------------------------------ исходники писем

    fn wanted(&self, s: &MessageSummary, now: i64) -> bool {
        if self.days == 0 || s.size > MAX_RAW_BYTES {
            return false;
        }
        let ts = s.date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map_or(0, |d| d.timestamp());
        ts >= now - i64::from(self.days) * 86_400
    }

    pub fn raw(&self, folder: &str, uid: u32) -> Option<Vec<u8>> {
        {
            let mut inner = self.lock();
            if !self.ensure_loaded(&mut inner, folder).items.get(&uid).is_some_and(|e| e.cached) {
                return None;
            }
        }
        self.read_file(&Self::raw_name(folder, uid))
    }

    /// Сохранить исходник, если письмо в пределах срока хранения.
    pub fn put_raw(&self, folder: &str, uid: u32, raw: &[u8]) {
        let now = chrono::Utc::now().timestamp();
        let ok = {
            let mut inner = self.lock();
            let idx = self.ensure_loaded(&mut inner, folder);
            idx.items.get(&uid).is_some_and(|e| !e.cached && self.wanted(&e.s, now))
        };
        if !ok || self.write_file(&Self::raw_name(folder, uid), raw).is_err() {
            return;
        }
        let text = imap::search_text(raw, TEXT_CHARS);
        let mut inner = self.lock();
        let idx = self.ensure_loaded(&mut inner, folder);
        if let Some(e) = idx.items.get_mut(&uid) {
            e.cached = true;
            e.text = Some(text);
            e.rebuild();
        }
        inner.dirty.insert(folder.to_string());
    }

    /// Убрать исходники старше срока хранения.
    fn evict(&self, folder: &str) {
        let now = chrono::Utc::now().timestamp();
        let old: Vec<u32> = {
            let mut inner = self.lock();
            let idx = self.ensure_loaded(&mut inner, folder);
            let old: Vec<u32> = idx.items.iter().filter(|(_, e)| e.cached && !self.wanted(&e.s, now)).map(|(u, _)| *u).collect();
            for u in &old {
                if let Some(e) = idx.items.get_mut(u) {
                    e.cached = false;
                    e.text = None;
                    e.rebuild();
                }
            }
            if !old.is_empty() {
                inner.dirty.insert(folder.to_string());
            }
            old
        };
        for u in old {
            let _ = std::fs::remove_file(self.dir.join(Self::raw_name(folder, u)));
        }
    }

    // ------------------------------------------------------------ поиск

    /// Поиск по всем папкам: отправитель, получатели, тема и текст хранимых писем.
    /// Все слова запроса должны встретиться; новые письма сверху.
    pub fn search(&self, query: &str, limit: usize) -> Vec<MessageSummary> {
        let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let mut inner = self.lock();
        self.load_all(&mut inner);
        let mut hits: Vec<(i64, &Entry)> = inner
            .index
            .values()
            .flat_map(|i| i.items.values())
            .filter(|e| words.iter().all(|w| e.hay.contains(w.as_str())))
            .map(|e| (e.timestamp(), e))
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        hits.into_iter().take(limit).map(|(_, e)| e.s.clone()).collect()
    }

    pub fn status(&self) -> StoreStatus {
        let mut inner = self.lock();
        self.load_all(&mut inner);
        let (messages, cached) = inner
            .index
            .values()
            .flat_map(|i| i.items.values())
            .fold((0, 0), |(m, c), e| (m + 1, c + usize::from(e.cached)));
        drop(inner);
        let bytes = dir_size(&self.dir);
        StoreStatus { enabled: true, days: self.days, messages, cached, syncing: self.syncing.load(Ordering::SeqCst), bytes }
    }
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------- фоновая синхронизация

/// Синхронизация в отдельном соединении: заголовки всех писем своих папок и
/// исходники за срок хранения. Повторяется, пока хранилище открыто.
pub async fn run_sync(store: Arc<Store>, creds: Credentials) {
    let mut delay = std::time::Duration::from_secs(2);
    loop {
        tokio::time::sleep(delay).await;
        if store.is_closed() {
            return;
        }
        store.syncing.store(true, Ordering::SeqCst);
        let result = sync_once(&store, &creds).await;
        store.syncing.store(false, Ordering::SeqCst);
        store.flush();
        if let Err(e) = result {
            log::warn!("local store sync: {e}");
        }
        delay = std::time::Duration::from_secs(if cfg!(debug_assertions) && std::env::var_os("RRCKI_E2E_POLL_SECS").is_some() { 20 } else { 600 });
    }
}

async fn sync_once(store: &Store, creds: &Credentials) -> Result<()> {
    let mut conn = imap::connect(creds).await?;
    let folders = imap::list_folders(&mut conn).await?;
    store.set_folders(&folders);
    // Сначала «Входящие», затем остальные свои папки; общие ящики не индексируем.
    let mut order: Vec<&Folder> = folders.iter().filter(|f| f.selectable && !imap::is_shared(f)).collect();
    order.sort_by_key(|f| f.role != imap::FolderRole::Inbox);
    for f in order {
        if store.is_closed() {
            break;
        }
        if let Err(e) = sync_folder(store, &mut conn, &f.id).await {
            log::warn!("local store: folder sync failed: {e}");
            if e.is_connection() {
                return Err(e);
            }
        }
        store.flush();
    }
    conn.logout().await;
    Ok(())
}

async fn sync_folder(store: &Store, conn: &mut imap::Connection, folder: &str) -> Result<()> {
    let (validity, flags) = imap::folder_flags(conn, folder).await?;
    let now = chrono::Utc::now().timestamp();
    let missing: Vec<u32> = {
        let mut inner = store.lock();
        let idx = store.ensure_loaded(&mut inner, folder);
        if idx.uid_validity != validity {
            // Папку пересоздали на сервере — старые UID ничего не значат.
            for u in idx.items.keys() {
                let _ = std::fs::remove_file(store.dir.join(Store::raw_name(folder, *u)));
            }
            idx.items.clear();
            idx.uid_validity = validity;
        }
        let live: HashMap<u32, imap::FlagState> = flags.iter().cloned().collect();
        let gone: Vec<u32> = idx.items.keys().copied().filter(|u| !live.contains_key(u)).collect();
        for u in &gone {
            idx.items.remove(u);
            let _ = std::fs::remove_file(store.dir.join(Store::raw_name(folder, *u)));
        }
        for (u, e) in idx.items.iter_mut() {
            if let Some(f) = live.get(u) {
                e.s.set_flags(f.clone());
            }
        }
        idx.total = flags.len() as u32;
        let mut missing: Vec<u32> = live.keys().copied().filter(|u| !idx.items.contains_key(u)).collect();
        missing.sort_unstable_by(|a, b| b.cmp(a));
        inner.dirty.insert(folder.to_string());
        missing
    };
    for chunk in missing.chunks(300) {
        if store.is_closed() {
            return Ok(());
        }
        let items = imap::index_summaries(conn, folder, chunk).await?;
        let mut inner = store.lock();
        let idx = store.ensure_loaded(&mut inner, folder);
        for s in items {
            idx.items.entry(s.uid).or_insert_with(|| Entry::new(s));
        }
        inner.dirty.insert(folder.to_string());
    }
    store.evict(folder);
    // Исходники за срок хранения, новые сначала.
    let want: Vec<u32> = {
        let mut inner = store.lock();
        let idx = store.ensure_loaded(&mut inner, folder);
        let mut w: Vec<(i64, u32)> = idx.items.values().filter(|e| !e.cached && store.wanted(&e.s, now)).map(|e| (e.timestamp(), e.s.uid)).collect();
        w.sort_by_key(|h| std::cmp::Reverse(h.0));
        w.into_iter().map(|(_, u)| u).take(2000).collect()
    };
    for chunk in want.chunks(10) {
        if store.is_closed() {
            return Ok(());
        }
        for (uid, raw) in imap::raw_bodies(conn, chunk).await? {
            store.put_raw(folder, uid, &raw);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(folder: &str, uid: u32, subject: &str, days_ago: i64) -> MessageSummary {
        MessageSummary {
            folder: folder.into(),
            uid,
            from_name: "Иванов Иван".into(),
            from_address: "ivanov@nrcki.ru".into(),
            to: vec!["test@nrcki.ru".into()],
            subject: subject.into(),
            date: Some((chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339()),
            unread: true,
            flagged: false,
            important: false,
            answered: false,
            keywords: vec![],
            message_id: None,
            thread: vec![],
            has_attachments: false,
            size: 100,
        }
    }

    fn store_in(dir: &Path, days: u32) -> Store {
        let key = [7u8; 32];
        create_private_dir(&dir.join("raw")).unwrap();
        Store {
            dir: dir.to_path_buf(),
            cipher: XChaCha20Poly1305::new((&key).into()),
            days,
            inner: Mutex::new(Inner::default()),
            closed: AtomicBool::new(false),
            syncing: AtomicBool::new(false),
        }
    }

    #[test]
    fn page_merge_search_and_encryption() {
        let tmp = std::env::temp_dir().join(format!("rrcki-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let st = store_in(&tmp, 7);
        let page = MessagePage {
            items: vec![summary("INBOX", 12, "Отчёт за квартал", 1), summary("INBOX", 11, "Совещание", 2), summary("INBOX", 10, "Старое", 30)],
            total: 3,
            has_more: false,
        };
        st.merge_page("INBOX", None, &page);
        let raw = "From: a@b\r\nSubject: x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nсекретный бюджет\r\n".as_bytes();
        st.put_raw("INBOX", 12, raw);
        st.put_raw("INBOX", 10, raw); // старше срока — не сохраняется
        st.flush();

        // На диске — только шифротекст.
        for e in walk(&tmp) {
            let data = std::fs::read(&e).unwrap();
            assert!(!String::from_utf8_lossy(&data).contains("бюджет"), "{e:?}");
            assert!(!String::from_utf8_lossy(&data).contains("ivanov"), "{e:?}");
        }

        let st2 = store_in(&tmp, 7);
        let p = st2.page("INBOX", None, 2).unwrap();
        assert_eq!(p.items.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![12, 11]);
        assert!(p.has_more);
        assert_eq!(st2.raw("INBOX", 12).as_deref(), Some(raw));
        assert!(st2.raw("INBOX", 10).is_none());
        assert_eq!(st2.search("бюджет", 10).iter().map(|m| m.uid).collect::<Vec<_>>(), vec![12]);
        assert_eq!(st2.search("иванов совещание", 10).len(), 1);
        assert_eq!(st2.search("ivanov", 10).len(), 3);

        // Письмо 11 удалено на сервере: свежая страница без него.
        st2.merge_page("INBOX", None, &MessagePage { items: vec![summary("INBOX", 12, "Отчёт за квартал", 1), summary("INBOX", 10, "Старое", 30)], total: 2, has_more: false });
        assert!(st2.summary("INBOX", 11).is_none());
        st2.remove("INBOX", &[12]);
        assert!(st2.raw("INBOX", 12).is_none());

        // Чужой ключ — ничего не расшифровать.
        let mut other = store_in(&tmp, 7);
        other.cipher = XChaCha20Poly1305::new((&[9u8; 32]).into());
        assert!(other.page("INBOX", None, 10).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.path().is_dir() {
                out.extend(walk(&e.path()));
            } else {
                out.push(e.path());
            }
        }
        out
    }
}
