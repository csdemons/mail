//! Общие импорты модулей: чтобы каждый подфайл не повторял шапку.

pub(crate) use std::collections::HashMap;
pub(crate) use std::io::BufReader;
pub(crate) use std::time::Duration;
pub(crate) use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
pub(crate) use reqwest::{Client, Method, StatusCode};
pub(crate) use serde::Serialize;
pub(crate) use url::Url;
pub(crate) use crate::error::{AppError, Result};
