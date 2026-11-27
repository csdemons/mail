//! Соединение: учётные данные, TLS, вход, повторное подключение, возможности сервера.
//!
//! Часть модуля `imap` — общий обзор в `mod.rs`.

use super::prelude::*;

pub type ImapSession = async_imap::Session<TlsStream<TcpStream>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Максимальный размер письма, которое загружаем целиком.
pub const MAX_MESSAGE_MB: u32 = 50;

#[derive(Clone)]
pub struct Credentials {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// Адрес EWS — учётная запись Exchange (почта по EWS, а не по IMAP).
    pub ews: Option<String>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

pub(crate) enum Backend {
    Imap(Box<ImapSession>),
    Ews(Box<crate::ews::EwsSession>),
}

pub struct Connection {
    pub(crate) backend: Backend,
    pub(crate) selected: Option<String>,
    pub(crate) creds: Credentials,
    /// Последнее открытое письмо целиком (папка, UID, исходник): вложения и их
    /// предпросмотр берутся отсюда, без повторной загрузки с сервера.
    pub(crate) raw_cache: Option<(String, u32, Vec<u8>)>,
}

/// TLS-подключение (IMAPS) и вход. Сертификат проверяется через системное
/// хранилище: SChannel в Windows, OpenSSL + системные CA в Linux, поэтому
/// корпоративные корневые сертификаты подхватываются автоматически.
pub async fn connect(creds: &Credentials) -> Result<Connection> {
    if let Some(url) = &creds.ews {
        let key = format!("{}@{}", creds.username.to_lowercase(), creds.host.to_lowercase());
        let ews = crate::ews::connect(url, &creds.username, &creds.password, &key).await?;
        return Ok(Connection { backend: Backend::Ews(Box::new(ews)), selected: None, creds: creds.clone(), raw_cache: None });
    }
    let tcp = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((creds.host.as_str(), creds.port)),
    )
    .await
    .map_err(|_| AppError::Timeout)?
    .map_err(|e| AppError::Connect(format!("{}:{}", creds.host, creds.port), e.to_string()))?;

    // Системные корни плюс вшитые (внутренний УЦ, например Минцифры) — см. tls.rs.
    let connector = crate::tls::connector()?;
    let connector = tokio_native_tls::TlsConnector::from(connector);

    let tls = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(&creds.host, tcp))
        .await
        .map_err(|_| AppError::Timeout)?
        .map_err(|e| AppError::Tls(e.to_string()))?;

    let mut client = async_imap::Client::new(tls);
    // Приветствие сервера (* OK ...).
    tokio::time::timeout(CONNECT_TIMEOUT, client.read_response())
        .await
        .map_err(|_| AppError::Timeout)?
        .map_err(|_| AppError::ConnectionLost)?
        .ok_or(AppError::ConnectionLost)?;

    let session = tokio::time::timeout(
        CONNECT_TIMEOUT,
        client.login(&creds.username, &creds.password),
    )
    .await
    .map_err(|_| AppError::Timeout)?
    .map_err(|(err, _client)| match err {
        async_imap::error::Error::No(_) => AppError::AuthFailed,
        other => other.into(),
    })?;

    Ok(Connection {
        backend: Backend::Imap(Box::new(session)),
        selected: None,
        creds: creds.clone(),
        raw_cache: None,
    })
}

impl Connection {
    /// Сеанс IMAP; для Exchange — ошибка «недоступно» (функция не поддерживается).
    pub(crate) fn imap(&mut self) -> Result<&mut ImapSession> {
        match &mut self.backend {
            Backend::Imap(s) => Ok(s),
            Backend::Ews(_) => Err(AppError::Other("Для учётной записи Exchange это пока недоступно".into())),
        }
    }

    /// Сеанс Exchange (EWS), если это ящик Exchange.
    pub fn ews(&mut self) -> Option<&mut crate::ews::EwsSession> {
        match &mut self.backend {
            Backend::Ews(e) => Some(e),
            Backend::Imap(_) => None,
        }
    }

    pub fn is_ews(&self) -> bool {
        matches!(self.backend, Backend::Ews(_))
    }

    /// Соединение открыто для этой учётной записи (тот же ящик на том же сервере).
    pub fn is_account(&self, creds: &Credentials) -> bool {
        self.creds.username.eq_ignore_ascii_case(&creds.username) && self.creds.host.eq_ignore_ascii_case(&creds.host) && self.creds.port == creds.port
    }

    /// Список возможностей сервера (CAPABILITY) — для диагностики.
    pub async fn capabilities(&mut self) -> Result<Vec<String>> {
        if self.is_ews() {
            return Ok(vec!["EWS".into()]);
        }
        let caps = self.imap()?.capabilities().await?;
        let mut list: Vec<String> = caps.iter().map(|c| format!("{c:?}")).collect();
        list.sort();
        Ok(list)
    }

    pub async fn logout(mut self) {
        match &mut self.backend {
            Backend::Imap(s) => {
                let _ = s.logout().await;
            }
            Backend::Ews(e) => e.close(),
        }
    }

    pub(crate) async fn ensure_selected(&mut self, folder: &str) -> Result<()> {
        if self.is_ews() {
            self.selected = Some(folder.to_string());
            return Ok(());
        }
        if self.selected.as_deref() != Some(folder) {
            self.select(folder).await?;
        }
        Ok(())
    }

    /// Новое соединение вместо текущего (после рассинхронизации протокола).
    pub(crate) async fn reconnect(&mut self) -> Result<()> {
        log::warn!("IMAP: reconnecting after protocol desync");
        let fresh = connect(&self.creds).await?;
        let old = std::mem::replace(self, fresh);
        drop(old); // старое соединение в неизвестном состоянии — просто закрываем сокет
        Ok(())
    }

    /// SELECT с проверкой: ответ без обязательного UIDVALIDITY означает, что
    /// поток ответов рассинхронизирован (сервер прислал то, что библиотека не
    /// разобрала). В этом случае переподключаемся и повторяем один раз,
    /// иначе папка выглядела бы «пустой» без всякой ошибки.
    pub(crate) async fn select(&mut self, folder: &str) -> Result<async_imap::types::Mailbox> {
        for attempt in 0..2 {
            self.selected = None;
            let mailbox = self.imap()?.select(folder).await.map_err(|e| match e {
                async_imap::error::Error::No(_) => AppError::NotFound,
                other => other.into(),
            })?;
            if mailbox.uid_validity.is_some() {
                self.selected = Some(folder.to_string());
                return Ok(mailbox);
            }
            if attempt == 0 {
                self.reconnect().await?;
            }
        }
        Err(AppError::Imap(format!(
            "сервер не вернул UIDVALIDITY для папки «{}»",
            utf7_imap::decode_utf7_imap(folder.to_string())
        )))
    }
}

// ---------------------------------------------------------------- папки

impl Connection {
    /// Исходник последнего загруженного с сервера письма (для локального хранилища).
    pub fn last_raw(&self, folder: &str, uid: u32) -> Option<&[u8]> {
        self.raw_cache.as_ref().filter(|(f, u, _)| f == folder && *u == uid).map(|(_, _, r)| r.as_slice())
    }

    /// Письмо открыто из локального хранилища: вложения и картинки — без повторной загрузки.
    pub fn prime_raw(&mut self, folder: &str, uid: u32, raw: Vec<u8>) {
        self.raw_cache = Some((folder.to_string(), uid, raw));
    }
}

// ---------------------------------------------------------------- синхронизация локального хранилища

/// Переподключение после сбоя посреди долгой операции.
pub async fn reconnect_now(conn: &mut Connection) -> Result<()> {
    conn.reconnect().await
}

pub(crate) async fn has_capability(conn: &mut Connection, name: &str) -> bool {
    conn.capabilities().await.map(|c| c.iter().any(|x| x.eq_ignore_ascii_case(name))).unwrap_or(false)
}

