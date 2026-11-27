//! Общие импорты модулей Nextcloud: чтобы каждый подфайл не повторял шапку.

pub(crate) use std::time::Duration;
pub(crate) use reqwest::{Client, Method, StatusCode};
pub(crate) use serde::Serialize;
pub(crate) use serde_json::{json, Value};
pub(crate) use url::Url;
pub(crate) use crate::error::{AppError, Result};
