//! Файлы и общие ссылки: список, загрузка, переименование, публикация.
//!
//! Часть модуля `nextcloud` — общий обзор в `mod.rs`.

use super::*;

impl Nc {
    /// Отправляет файл в беседу так же, как веб-интерфейс Talk: загружает его
    /// в папку вложений пользователя и публикует в беседе (shareType 10).
    /// Имя не перезаписывает существующие файлы: «файл (1).pdf», «файл (2).pdf»…
    /// `voice` — голосовое сообщение (Talk показывает проигрыватель), `silent` — без звука уведомления.
    pub async fn talk_send_file(&self, token: &str, name: &str, src: UploadSource, caption: &str, opts: TalkFileOpts) -> Result<String> {
        let TalkFileOpts { reply_to, voice, silent, thread_id } = opts;
        let folder = self.talk_attachment_folder().await;
        let path = self.upload_unique_src(&folder, name, src).await?;
        let mut meta = json!({ "messageType": if voice { "voice-message" } else { "comment" } });
        if silent { meta["silent"] = json!(true); }
        if !caption.trim().is_empty() { meta["caption"] = json!(caption.trim()); }
        if let Some(r) = reply_to { meta["replyTo"] = json!(r); }
        if let Some(t) = thread_id.filter(|t| *t > 0) { meta["threadId"] = json!(t); }
        let body = json!({ "shareType": 10, "shareWith": token, "path": path, "talkMetaData": meta.to_string() });
        self.ocs(Method::POST, "ocs/v2.php/apps/files_sharing/api/v1/shares", &[("format", "json".into())], Some(body), "Публикация файла в беседе").await?;
        Ok(path)
    }
}

// ---------------------------------------------------------------- Файлы

/// Файл или папка в Nextcloud. `path` — от корня файлов пользователя, с «/».
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NcFile {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: i64,
    pub modified: String,
    pub mime: String,
    pub file_id: Option<i64>,
    pub shared: bool,
    pub favorite: bool,
    pub can_write: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NcShare {
    pub id: String,
    /// 0 — пользователь, 1 — группа, 3 — публичная ссылка, 4 — e-mail, 10 — Talk.
    pub share_type: i64,
    pub share_with: String,
    pub share_with_name: String,
    pub url: String,
    pub expiration: String,
    pub has_password: bool,
    pub can_edit: bool,
}

/// Путь внутри файлов пользователя: только «/a/b», без «..» и пустых частей.
pub fn clean_path(path: &str) -> Result<String> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.iter().any(|p| *p == ".." || *p == "." || p.contains('\\') || p.chars().any(char::is_control)) {
        return Err(AppError::Other("Некорректный путь".into()));
    }
    Ok(format!("/{}", parts.join("/")))
}

pub(crate) fn valid_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() || n == "." || n == ".." || n.contains(['/', '\\']) || n.chars().any(char::is_control) {
        return Err(AppError::Other("Недопустимое имя".into()));
    }
    Ok(n.to_string())
}

const PROPFIND_FILES: &str = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/><d:getcontenttype/><oc:fileid/><oc:size/><oc:favorite/><oc:share-types/><oc:permissions/></d:prop></d:propfind>"#;

/// Разбор ответа PROPFIND по файлам: `root` — путь URL каталога files/<uid>/.
pub fn parse_file_list(xml: &str, root: &str) -> Result<Vec<NcFile>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| AppError::Other(format!("Ответ WebDAV: {e}")))?;
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.has_tag_name(("DAV:", "response"))) {
        let href = resp.descendants().find(|n| n.has_tag_name(("DAV:", "href"))).and_then(|n| n.text()).unwrap_or_default();
        let decoded = percent_encoding::percent_decode_str(href).decode_utf8_lossy().to_string();
        let Some(rel) = decoded.strip_prefix(root) else { continue };
        let path = format!("/{}", rel.trim_matches('/'));
        // Только свойства со статусом 200.
        let ok = resp.children().filter(|n| n.has_tag_name(("DAV:", "propstat"))).find(|ps| {
            ps.descendants().find(|n| n.has_tag_name(("DAV:", "status"))).and_then(|n| n.text()).is_some_and(|t| t.contains(" 200"))
        });
        let Some(ps) = ok else { continue };
        let prop = |ns: &str, name: &str| ps.descendants().find(|n| n.has_tag_name((ns, name))).and_then(|n| n.text()).unwrap_or_default().trim().to_string();
        let is_dir = ps.descendants().any(|n| n.has_tag_name(("DAV:", "collection")));
        let size_str = if is_dir { prop("http://owncloud.org/ns", "size") } else { prop("DAV:", "getcontentlength") };
        let perms = prop("http://owncloud.org/ns", "permissions");
        let shared = ps.descendants().find(|n| n.has_tag_name(("http://owncloud.org/ns", "share-types"))).is_some_and(|n| n.children().any(|c| c.is_element()));
        out.push(NcFile {
            name: path.rsplit('/').next().unwrap_or_default().to_string(),
            path,
            is_dir,
            size: size_str.parse().unwrap_or(0),
            modified: prop("DAV:", "getlastmodified"),
            mime: if is_dir { String::new() } else { prop("DAV:", "getcontenttype") },
            file_id: prop("http://owncloud.org/ns", "fileid").parse().ok(),
            shared,
            favorite: prop("http://owncloud.org/ns", "favorite") == "1",
            can_write: perms.is_empty() || perms.contains('W') || perms.contains('C') || perms.contains('K'),
        });
    }
    Ok(out)
}

fn parse_share(v: &Value) -> NcShare {
    NcShare {
        id: text(&v["id"]).or_int(&v["id"]),
        share_type: int(&v["share_type"]),
        share_with: text(&v["share_with"]),
        share_with_name: v["share_with_displayname"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| text(&v["share_with"])),
        url: text(&v["url"]),
        expiration: text(&v["expiration"]),
        has_password: v["password"].as_str().is_some_and(|p| !p.is_empty()) || v["share_with"].as_str().is_some_and(|p| !p.is_empty() && int(&v["share_type"]) == 3),
        can_edit: int(&v["permissions"]) & 2 != 0,
    }
}

impl Nc {
    pub(crate) fn files_root_path(&self) -> Result<String> {
        Ok(self.files_url("/")?.path().trim_end_matches('/').to_string() + "/")
    }

    pub async fn files_list(&self, path: &str) -> Result<Vec<NcFile>> {
        let path = clean_path(path)?;
        let resp = self
            .dav(Method::from_bytes(b"PROPFIND").expect("PROPFIND"), self.files_url(&path)?)
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_FILES)
            .send()
            .await
            .map_err(net_error)?;
        if resp.status().as_u16() != 207 {
            return Err(status_error(resp.status(), "Файлы"));
        }
        let xml = crate::netio::text_limited(resp, crate::netio::API_LIMIT).await?;
        let root = self.files_root_path()?;
        let decoded_root = percent_encoding::percent_decode_str(&root).decode_utf8_lossy().to_string();
        let mut list: Vec<NcFile> = parse_file_list(&xml, &decoded_root)?.into_iter().filter(|f| f.path != path).collect();
        list.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(list)
    }

    /// Создаёт папку и все недостающие родительские.
    pub async fn ensure_dir(&self, path: &str) -> Result<String> {
        let path = clean_path(path)?;
        let mut acc = String::new();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            acc.push('/');
            acc.push_str(part);
            let resp = self.dav(Method::from_bytes(b"MKCOL").expect("MKCOL"), self.files_url(&acc)?).send().await.map_err(net_error)?;
            // 201 — создана, 405 — уже существует.
            if !(resp.status().is_success() || resp.status() == StatusCode::METHOD_NOT_ALLOWED) {
                return Err(status_error(resp.status(), "Папка"));
            }
        }
        Ok(path)
    }

    pub async fn files_mkdir(&self, parent: &str, name: &str) -> Result<String> {
        let path = format!("{}/{}", clean_path(parent)?.trim_end_matches('/'), valid_name(name)?);
        let resp = self.dav(Method::from_bytes(b"MKCOL").expect("MKCOL"), self.files_url(&path)?).send().await.map_err(net_error)?;
        match resp.status() {
            s if s.is_success() => Ok(path),
            StatusCode::METHOD_NOT_ALLOWED => Err(AppError::Other("Папка с таким именем уже есть".into())),
            s => Err(status_error(s, "Новая папка")),
        }
    }

    /// Загружает файл в папку, не перезаписывая существующие: «файл (1).pdf»…
    pub async fn upload_unique(&self, dir: &str, name: &str, data: Vec<u8>) -> Result<String> {
        self.upload_unique_src(dir, name, UploadSource::Bytes(bytes::Bytes::from(data))).await
    }

    /// Загрузка без перезаписи: «имя (1).ext» и т.д. Файл с диска передаётся потоком
    /// (не читается в память целиком — важно на терминальных серверах).
    pub async fn upload_unique_src(&self, dir: &str, name: &str, src: UploadSource) -> Result<String> {
        let dir = self.ensure_dir(dir).await?;
        let name = valid_name(name)?;
        for n in 0..100 {
            let path = format!("{}/{}", dir.trim_end_matches('/'), numbered_name(&name, n));
            let (body, len) = match &src {
                UploadSource::Bytes(b) => (reqwest::Body::from(b.clone()), b.len() as u64),
                UploadSource::File(p) => {
                    let f = tokio::fs::File::open(p).await?;
                    let len = f.metadata().await?.len();
                    (reqwest::Body::from(f), len)
                }
            };
            let resp = self
                .dav(Method::PUT, self.files_url(&path)?)
                .header("If-None-Match", "*")
                .header("Content-Type", mime_guess::from_path(&name).first_or_octet_stream().essence_str())
                .header(reqwest::header::CONTENT_LENGTH, len)
                .body(body)
                .send()
                .await
                .map_err(net_error)?;
            match resp.status() {
                s if s.is_success() => return Ok(path),
                StatusCode::PRECONDITION_FAILED => continue,
                StatusCode::INSUFFICIENT_STORAGE => return Err(AppError::Other("В Nextcloud закончилось место (квота)".into())),
                StatusCode::PAYLOAD_TOO_LARGE => return Err(AppError::Other("Файл слишком большой для сервера Nextcloud".into())),
                s => return Err(status_error(s, "Загрузка файла")),
            }
        }
        Err(AppError::Other("Не удалось подобрать свободное имя файла".into()))
    }

    /// Скачивает файл потоком прямо на диск (без копии целиком в памяти).
    pub async fn files_download_to(&self, path: &str, target: &std::path::Path) -> Result<u64> {
        use tokio::io::AsyncWriteExt as _;
        let mut resp = self.dav(Method::GET, self.files_url(&clean_path(path)?)?).send().await.map_err(net_error)?;
        if !resp.status().is_success() {
            return Err(status_error(resp.status(), "Скачивание"));
        }
        let mut file = tokio::fs::File::create(target).await?;
        let mut total = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(net_error)? {
            file.write_all(&chunk).await?;
            total += chunk.len() as u64;
        }
        file.flush().await?;
        Ok(total)
    }

    /// Содержимое файла для просмотра; больше `max` байт — ошибка (загрузка прерывается).
    pub async fn files_read(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        let resp = self.dav(Method::GET, self.files_url(&clean_path(path)?)?).send().await.map_err(net_error)?;
        if !resp.status().is_success() {
            return Err(status_error(resp.status(), "Открытие файла"));
        }
        if resp.content_length().is_some_and(|n| n > max as u64) {
            return Err(AppError::Other(format!("Файл больше {} МБ — сохраните его на компьютер", max / (1024 * 1024))));
        }
        crate::netio::read_limited(resp, max)
            .await
            .map_err(|_| AppError::Other(format!("Файл больше {} МБ — сохраните его на компьютер", max / (1024 * 1024))))
    }

    /// Удаление (в Nextcloud — в корзину).
    pub async fn files_delete(&self, path: &str) -> Result<()> {
        let path = clean_path(path)?;
        if path == "/" { return Err(AppError::Other("Нельзя удалить корень".into())); }
        let resp = self.dav(Method::DELETE, self.files_url(&path)?).send().await.map_err(net_error)?;
        if !resp.status().is_success() {
            return Err(status_error(resp.status(), "Удаление"));
        }
        Ok(())
    }

    pub async fn files_rename(&self, path: &str, new_name: &str) -> Result<String> {
        let path = clean_path(path)?;
        let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        let target = format!("{parent}/{}", valid_name(new_name)?);
        let resp = self
            .dav(Method::from_bytes(b"MOVE").expect("MOVE"), self.files_url(&path)?)
            .header("Destination", self.files_url(&target)?.as_str())
            .header("Overwrite", "F")
            .send()
            .await
            .map_err(net_error)?;
        match resp.status() {
            s if s.is_success() => Ok(target),
            StatusCode::PRECONDITION_FAILED => Err(AppError::Other("Файл с таким именем уже есть".into())),
            s => Err(status_error(s, "Переименование")),
        }
    }

    pub async fn shares_list(&self, path: &str) -> Result<Vec<NcShare>> {
        let q = [("path", clean_path(path)?), ("reshares", "true".into()), ("format", "json".into())];
        let (_, _, d) = self.ocs(Method::GET, "ocs/v2.php/apps/files_sharing/api/v1/shares", &q, None, "Общий доступ").await?;
        Ok(d.as_array().map(|a| a.iter().map(parse_share).collect()).unwrap_or_default())
    }

    /// Публичная ссылка (только чтение), с паролем и сроком при необходимости.
    pub async fn share_link(&self, path: &str, password: Option<&str>, expire: Option<&str>) -> Result<NcShare> {
        let mut body = json!({ "path": clean_path(path)?, "shareType": 3, "permissions": 1 });
        if let Some(p) = password.filter(|p| !p.trim().is_empty()) { body["password"] = json!(p); }
        if let Some(e) = expire.filter(|e| !e.trim().is_empty()) { body["expireDate"] = json!(e); }
        let (_, _, d) = self.ocs(Method::POST, "ocs/v2.php/apps/files_sharing/api/v1/shares", &[("format", "json".into())], Some(body), "Публичная ссылка").await?;
        Ok(parse_share(&d))
    }

    /// Редакторы документов, доступные через Direct Editing (ONLYOFFICE / Euro-Office,
    /// Nextcloud Text). Нет приложения или API — пустой список, не ошибка.
    pub async fn editors(&self) -> NcEditors {
        let Ok((_, _, d)) = self.ocs(Method::GET, "ocs/v2.php/apps/files/api/v1/directEditing", &[("format", "json".into())], None, "Редакторы документов").await else {
            return NcEditors::default();
        };
        let strings = |v: &Value| -> Vec<String> { v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        let objects = |v: &Value| -> Vec<Value> {
            match v {
                Value::Object(m) => m.values().cloned().collect(),
                Value::Array(a) => a.clone(),
                _ => Vec::new(),
            }
        };
        NcEditors {
            editors: objects(&d["editors"]).iter().map(|e| NcEditor {
                id: text(&e["id"]),
                name: text(&e["name"]),
                mimetypes: strings(&e["mimetypes"]),
                optional_mimetypes: strings(&e["optionalMimetypes"]),
            }).filter(|e| !e.id.is_empty()).collect(),
            creators: objects(&d["creators"]).iter().map(|c| NcCreator {
                id: text(&c["id"]),
                editor: text(&c["editor"]),
                name: text(&c["name"]),
                extension: text(&c["extension"]).trim_start_matches('.').to_ascii_lowercase(),
            }).filter(|c| !c.id.is_empty()).collect(),
        }
    }

    /// Одноразовая ссылка на редактор для файла (вход не нужен — как в мобильных приложениях).
    pub async fn direct_open(&self, path: &str, editor_id: &str) -> Result<String> {
        let body = json!({ "path": clean_path(path)?, "editorId": editor_id });
        let (_, _, d) = self.ocs(Method::POST, "ocs/v2.php/apps/files/api/v1/directEditing/open", &[("format", "json".into())], Some(body), "Открытие в редакторе").await?;
        self.own_url(&text(&d["url"]))
    }

    /// Новый документ через редактор (шаблон сервера) — ссылка на редактор.
    pub async fn direct_create(&self, path: &str, editor_id: &str, creator_id: &str) -> Result<String> {
        let body = json!({ "path": clean_path(path)?, "editorId": editor_id, "creatorId": creator_id });
        let (_, _, d) = self.ocs(Method::POST, "ocs/v2.php/apps/files/api/v1/directEditing/create", &[("format", "json".into())], Some(body), "Новый документ").await?;
        self.own_url(&text(&d["url"]))
    }

    /// Ссылка должна вести на этот же Nextcloud — чужой адрес в окно программы не открываем.
    pub(crate) fn own_url(&self, url: &str) -> Result<String> {
        let u = url::Url::parse(url).map_err(|_| AppError::Other("Редактор вернул некорректную ссылку".into()))?;
        let b = &self.creds.base;
        if u.scheme() != b.scheme() || u.host_str() != b.host_str() || u.port_or_known_default() != b.port_or_known_default() {
            return Err(AppError::Other("Редактор вернул ссылку на другой сервер".into()));
        }
        Ok(u.to_string())
    }

    /// ID файла (для ссылки `index.php/f/<id>`).
    pub async fn file_id(&self, path: &str) -> Result<i64> {
        let path = clean_path(path)?;
        let resp = self
            .dav(Method::from_bytes(b"PROPFIND").expect("PROPFIND"), self.files_url(&path)?)
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_FILES)
            .send()
            .await
            .map_err(net_error)?;
        if resp.status().as_u16() != 207 {
            return Err(status_error(resp.status(), "Файл"));
        }
        let xml = crate::netio::text_limited(resp, crate::netio::API_LIMIT).await?;
        let root = self.files_root_path()?;
        let decoded_root = percent_encoding::percent_decode_str(&root).decode_utf8_lossy().to_string();
        parse_file_list(&xml, &decoded_root)?
            .into_iter()
            .find_map(|f| f.file_id)
            .ok_or_else(|| AppError::Other("Nextcloud не сообщил ID файла".into()))
    }

    /// Публичная ссылка для письма: существующая без пароля и со сроком не меньше
    /// `days`/2 дней — или новая со сроком `days` дней (0 — без срока, по политике).
    pub async fn link_for(&self, path: &str, days: u32) -> Result<String> {
        let today = chrono::Local::now().date_naive();
        let min_left = chrono::Duration::days(i64::from(days / 2).max(1));
        let usable = |s: &NcShare| {
            s.share_type == 3 && !s.has_password && !s.url.is_empty() && {
                let exp = chrono::NaiveDate::parse_from_str(s.expiration.get(..10).unwrap_or(""), "%Y-%m-%d").ok();
                match (days, exp) {
                    (0, _) => true,
                    (_, Some(e)) => e - today >= min_left,
                    (_, None) => false, // ссылка без срока — для письма не используем
                }
            }
        };
        if let Some(s) = self.shares_list(path).await?.into_iter().find(usable) {
            return Ok(s.url);
        }
        let expire = (days > 0).then(|| (today + chrono::Duration::days(i64::from(days))).format("%Y-%m-%d").to_string());
        Ok(self.share_link(path, None, expire.as_deref()).await?.url)
    }

    pub async fn share_user(&self, path: &str, uid: &str, can_edit: bool) -> Result<NcShare> {
        let body = json!({ "path": clean_path(path)?, "shareType": 0, "shareWith": uid, "permissions": if can_edit { 31 } else { 1 } });
        let (_, _, d) = self.ocs(Method::POST, "ocs/v2.php/apps/files_sharing/api/v1/shares", &[("format", "json".into())], Some(body), "Доступ сотруднику").await?;
        Ok(parse_share(&d))
    }

    pub async fn share_delete(&self, id: &str) -> Result<()> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '_') {
            return Err(AppError::Other("Некорректный идентификатор доступа".into()));
        }
        self.ocs(Method::DELETE, &format!("ocs/v2.php/apps/files_sharing/api/v1/shares/{id}"), &[("format", "json".into())], None, "Закрытие доступа").await?;
        Ok(())
    }
}

