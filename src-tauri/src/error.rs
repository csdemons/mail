use serde::{Serialize, Serializer};

/// Ошибки, которые видит пользователь. Сериализуются в строку для frontend.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Нет подключения к почтовому серверу. Войдите заново.")]
    NotLoggedIn,
    #[error("Неверный логин или пароль")]
    AuthFailed,
    #[error("Сервер {0} недоступен: {1}")]
    Connect(String, String),
    #[error("Ошибка защищённого соединения (TLS): {0}")]
    Tls(String),
    #[error("Сервер не ответил вовремя")]
    Timeout,
    #[error("Соединение с сервером потеряно")]
    ConnectionLost,
    #[error("Письмо или папка не найдены")]
    NotFound,
    #[error("Письмо слишком большое ({0} МБ), лимит {1} МБ")]
    TooLarge(u32, u32),
    #[error("Ошибка сервера IMAP: {0}")]
    Imap(String),
    /// Связь оборвалась после начала передачи письма: принял ли его SMTP-сервер, неизвестно.
    #[error("Неизвестно, принял ли почтовый сервер письмо (обрыв связи во время отправки): {0}")]
    DeliveryUnknown(String),
    /// Сервер недоступен (не удалось подключиться) — запрос точно не выполнен.
    #[error("Сервер {0} недоступен: {1}")]
    Unreachable(String, String),
    #[error("{0}")]
    Other(String),
}

impl AppError {
    /// Ошибки, после которых имеет смысл переподключиться и повторить запрос.
    pub fn is_connection(&self) -> bool {
        matches!(self, AppError::ConnectionLost | AppError::Timeout)
    }

    /// Нет связи с сервером (а не отказ сервера или ошибка TLS): можно работать
    /// с письмами на компьютере и выполнить действие позже.
    pub fn is_offline(&self) -> bool {
        matches!(self, AppError::ConnectionLost | AppError::Timeout | AppError::Connect(..) | AppError::Unreachable(..))
    }
}

impl From<async_imap::error::Error> for AppError {
    fn from(err: async_imap::error::Error) -> Self {
        use async_imap::error::Error as E;
        match err {
            // Неразобранный/неожиданный ответ означает, что поток ответов
            // рассинхронизирован, — соединение дальше использовать нельзя.
            E::Io(_) | E::ConnectionLost | E::Parse(_) => AppError::ConnectionLost,
            E::No(msg) | E::Bad(msg) => AppError::Imap(msg.trim().to_string()),
            other => AppError::Imap(other.to_string()),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Other(err.to_string())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;
