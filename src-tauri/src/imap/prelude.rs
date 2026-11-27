//! Общие импорты модулей IMAP: чтобы каждый подмодуль не повторял шапку.

pub(crate) use std::time::Duration;

pub(crate) use async_imap::imap_proto::types::NameAttribute;
pub(crate) use async_imap::types::{Fetch, Flag};
pub(crate) use futures::TryStreamExt;
pub(crate) use mail_parser::{MessageParser, MimeHeaders};
pub(crate) use serde::{Deserialize, Serialize};
pub(crate) use tokio::net::TcpStream;
pub(crate) use tokio_native_tls::TlsStream;

pub(crate) use crate::error::{AppError, Result};
