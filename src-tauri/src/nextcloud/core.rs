//! Основа: учётные данные, HTTP-клиент, базовый адрес, общие запросы OCS.
//!
//! Часть модуля `nextcloud` — общий обзор в `mod.rs`.

use super::prelude::*;


#[derive(Clone)]
pub struct NcCreds {
    pub base: Url,
    pub login: String,
    pub app_password: String,
}

impl std::fmt::Debug for NcCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NcCreds").field("base", &self.base.as_str()).field("login", &self.login).finish_non_exhaustive()
    }
}

pub fn normalize_base(raw: &str) -> Result<Url> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let mut url = Url::parse(&with_scheme).map_err(|_| AppError::Other(format!("Некорректный адрес Nextcloud: {raw}")))?;
    // http — только для отладочной сборки на этом компьютере (автотесты); в релизе только HTTPS.
    if url.scheme() != "https" && !(cfg!(debug_assertions) && url.host_str() == Some("localhost")) {
        return Err(AppError::Other("Nextcloud подключается только по HTTPS".into()));
    }
    if !url.path().ends_with('/') {
        let p = format!("{}/", url.path());
        url.set_path(&p);
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

pub(crate) fn http(timeout: Duration) -> Result<Client> {
    crate::tls::client_builder()?
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(15))
        .user_agent(concat!("RRCKI Mail/", env!("CARGO_PKG_VERSION"), " (Desktop)"))
        // Перенаправления — только в пределах того же сервера (схема, имя, порт):
        // пароль приложения и токены входа не уходят на чужой адрес или по http.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let same = attempt.previous().first().is_some_and(|first| first.origin() == attempt.url().origin());
            if same && attempt.previous().len() < 5 { attempt.follow() } else { attempt.stop() }
        }))
        .build()
        .map_err(|e| AppError::Other(format!("HTTP-клиент: {e}")))
}

/// Один сегмент пути URL (ID пользователя может содержать пробелы, @, кириллицу).
pub(crate) fn seg(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'@' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub(crate) fn net_error(e: reqwest::Error) -> AppError {
    let mut detail = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(cause) = src {
        detail.push_str(&format!(": {cause}"));
        src = cause.source();
    }
    AppError::Other(format!("Nextcloud недоступен: {detail}"))
}

pub(crate) fn status_error(status: StatusCode, what: &str) -> AppError {
    match status.as_u16() {
        401 => AppError::Other("Nextcloud: пароль приложения недействителен — подключите Nextcloud заново".into()),
        403 => AppError::Other(format!("{what}: нет прав")),
        404 => AppError::Other(format!("{what}: не найдено (возможно, приложение не установлено на сервере)")),
        412 => AppError::Other(format!("{what}: включено лобби, доступ только модераторам")),
        413 => AppError::Other("Сообщение слишком длинное".into()),
        _ => AppError::Other(format!("{what}: Nextcloud ответил HTTP {status}")),
    }
}

// ---------------------------------------------------------------- Login Flow v2

pub struct Nc {
    pub(crate) client: Client,
    pub creds: NcCreds,
    /// ID пользователя Nextcloud (actorId в Talk) — может отличаться от логина.
    pub me: String,
}

impl Nc {
    pub fn new(creds: NcCreds, me: &str) -> Result<Self> {
        Ok(Self { client: http(Duration::from_secs(45))?, creds, me: me.to_string() })
    }

    /// Загрузка файлов: большие файлы по медленному каналу идут долго.
    pub fn upload(creds: NcCreds, me: &str) -> Result<Self> {
        Ok(Self { client: http(Duration::from_secs(30 * 60))?, creds, me: me.to_string() })
    }

    /// Long-poll Talk ждёт до 30 с — нужен клиент с запасом по времени.
    pub fn long_poll(creds: NcCreds, me: &str) -> Result<Self> {
        Ok(Self { client: http(Duration::from_secs(75))?, creds, me: me.to_string() })
    }

    pub(crate) async fn call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
        what: &str,
    ) -> Result<(StatusCode, reqwest::header::HeaderMap, Value)> {
        let url = self.creds.base.join(path).map_err(|e| AppError::Other(e.to_string()))?;
        let mut req = self
            .client
            .request(method, url)
            .basic_auth(&self.creds.login, Some(&self.creds.app_password))
            .header("OCS-APIRequest", "true")
            .header("Accept", "application/json")
            .query(query);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(net_error)?;
        let status = resp.status();
        let headers = resp.headers().clone();
        if status == StatusCode::NOT_MODIFIED {
            return Ok((status, headers, Value::Null));
        }
        if !status.is_success() {
            return Err(status_error(status, what));
        }
        let text = crate::netio::text_limited(resp, crate::netio::API_LIMIT).await?;
        let v = if text.trim().is_empty() { Value::Null } else {
            serde_json::from_str(&text).map_err(|_| AppError::Other(format!("{what}: Nextcloud вернул не JSON")))?
        };
        Ok((status, headers, v))
    }

    /// OCS-ответ: `{ ocs: { meta, data } }` → data.
    pub(crate) async fn ocs(&self, method: Method, path: &str, query: &[(&str, String)], body: Option<Value>, what: &str) -> Result<(StatusCode, reqwest::header::HeaderMap, Value)> {
        let (status, headers, v) = self.call(method, path, query, body, what).await?;
        let data = v.pointer("/ocs/data").cloned().unwrap_or(v);
        Ok((status, headers, data))
    }

    /// (ID пользователя, отображаемое имя). Заодно проверяет пароль приложения.
    pub async fn user_info(&self) -> Result<(String, String)> {
        let (_, _, d) = self.ocs(Method::GET, "ocs/v1.php/cloud/user", &[("format", "json".into())], None, "Профиль Nextcloud").await?;
        let id = d["id"].as_str().unwrap_or(&self.creds.login).to_string();
        let name = d["displayname"].as_str().or(d["display-name"].as_str()).unwrap_or(&id).to_string();
        Ok((id, name))
    }

    /// Картинка (аватар) по пути Nextcloud → `data:`-URL. `None` — картинки нет.
    pub async fn image(&self, path: &str) -> Result<Option<String>> {
        use base64::Engine as _;
        let url = self.creds.base.join(path).map_err(|e| AppError::Other(e.to_string()))?;
        let resp = self
            .client
            .get(url)
            .basic_auth(&self.creds.login, Some(&self.creds.app_password))
            .header("OCS-APIRequest", "true")
            .send()
            .await
            .map_err(net_error)?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
            .unwrap_or_default();
        if !ct.starts_with("image/") {
            return Ok(None);
        }
        // Аватар больше 2 МБ не нужен: загрузка прерывается на пределе.
        let Ok(bytes) = crate::netio::read_limited(resp, 2 * 1024 * 1024).await else {
            return Ok(None);
        };
        Ok(Some(format!("data:{ct};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes))))
    }

    /// Аватар пользователя (тот же, что в веб-интерфейсе; без своего — цветная буква Nextcloud).
    pub async fn user_avatar(&self, uid: &str, size: u32) -> Result<Option<String>> {
        let path = format!("index.php/avatar/{}/{size}", seg(uid));
        self.image(&path).await
    }

    /// Аватар беседы Talk (группы; у личной беседы — аватар собеседника).
    pub async fn room_avatar(&self, token: &str) -> Result<Option<String>> {
        let path = format!("ocs/v2.php/apps/spreed/api/v1/room/{}/avatar", seg(token));
        self.image(&path).await
    }

    /// Уменьшенная копия файла (фото в чате) — `core/preview` по ID файла.
    pub async fn file_preview(&self, file_id: &str, size: u32) -> Result<Option<String>> {
        if file_id.is_empty() || !file_id.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(None);
        }
        self.image(&format!("index.php/core/preview?fileId={file_id}&x={size}&y={size}&a=1&forceIcon=0")).await
    }

    pub async fn revoke(&self) {
        let _ = self.ocs(Method::DELETE, "ocs/v2.php/core/apppassword", &[], None, "Отзыв пароля").await;
    }
}

// ---------------------------------------------------------------- Deck

