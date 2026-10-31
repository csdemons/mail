//! Политика администратора: файл `policy.json` в `/etc/rrcki-mail/` (Linux) или
//! `%ProgramData%\RRCKI Mail\` (Windows), а в Windows ещё и групповая политика —
//! `HKLM\SOFTWARE\Policies\RRCKI Mail` (её значения важнее файла).
//!
//! ```json
//! {
//!   "remoteImages": "direct",            // "direct" | "proxy" | "off"
//!   "internalNetworks": ["10.0.0.0/8", "144.206.0.0/16"],
//!   "rememberPassword": true,            // состояние галочки «Запомнить пароль» по умолчанию
//!   "allowRememberPassword": true,       // false — запоминать пароль нельзя
//!   "shareLinkDays": 30,                 // срок публичных ссылок «Отправить ссылкой»; 0 — без срока
//!   "notificationContent": "full"        // "full" | "sender" | "none" — что видно в уведомлениях ОС
//! }
//! ```
//! Реестр (HKLM\SOFTWARE\Policies\RRCKI Mail): RemoteImages (REG_SZ), InternalNetworks
//! (REG_SZ, через запятую), RememberPassword, AllowRememberPassword, ShareLinkDays,
//! UpdateDisabled (REG_DWORD), UpdateUrl (REG_SZ), NotificationContent (REG_SZ).

use std::net::IpAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteImages {
    /// Напрямую, только публичные адреса (кроме внутренних сетей администратора).
    #[default]
    Direct,
    /// Через системный/корпоративный прокси (его фильтр и журнал).
    Proxy,
    /// Внешние картинки в письмах не загружаются.
    Off,
}

/// Что показывать во всплывающих уведомлениях ОС (они видны на экране блокировки и в
/// центре уведомлений, в том числе посторонним).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationContent {
    /// Отправитель и (если пользователь не отключил) тема или текст.
    #[default]
    Full,
    /// Только отправитель/заголовок, без темы и текста.
    Sender,
    /// Только сам факт: «Atom Mail — новое уведомление».
    None,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Policy {
    pub remote_images: RemoteImages,
    pub internal_networks: Vec<String>,
    pub remember_password: Option<bool>,
    pub allow_remember_password: Option<bool>,
    pub share_link_days: Option<u32>,
    /// Локальное (зашифрованное) хранилище писем и индекс поиска; false — ничего на диск.
    pub local_store: Option<bool>,
    /// Наибольший срок хранения писем на компьютере, дней (0 — только индекс заголовков).
    pub offline_days: Option<u32>,
    pub notification_content: NotificationContent,
}

/// То, что нужно интерфейсу.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPolicy {
    pub remote_images: RemoteImages,
    pub remember_password: bool,
    pub allow_remember_password: bool,
    pub share_link_days: u32,
    pub local_store: bool,
    pub max_offline_days: u32,
    pub notification_content: NotificationContent,
}

impl Policy {
    pub fn ui(&self) -> UiPolicy {
        let allow = self.allow_remember_password.unwrap_or(true);
        UiPolicy {
            remote_images: self.remote_images,
            remember_password: allow && self.remember_password.unwrap_or(true),
            allow_remember_password: allow,
            share_link_days: self.share_link_days(),
            local_store: self.local_store(),
            max_offline_days: self.max_offline_days(),
            notification_content: self.notification_content,
        }
    }

    /// Текст уведомления по политике администратора.
    pub fn redact_notification(&self, title: &str, body: &str) -> (String, String) {
        match self.notification_content {
            NotificationContent::Full => (title.to_string(), body.to_string()),
            NotificationContent::Sender => (title.to_string(), "Откройте Atom Mail, чтобы прочитать".to_string()),
            NotificationContent::None => ("Atom Mail".to_string(), "Новое уведомление — откройте программу".to_string()),
        }
    }

    pub fn local_store(&self) -> bool {
        self.local_store.unwrap_or(true)
    }

    pub fn max_offline_days(&self) -> u32 {
        if self.local_store() { self.offline_days.unwrap_or(365).min(3650) } else { 0 }
    }

    pub fn share_link_days(&self) -> u32 {
        self.share_link_days.unwrap_or(30)
    }

    /// Адрес во внутренней сети по списку администратора.
    pub fn is_internal(&self, ip: IpAddr) -> bool {
        self.internal_networks.iter().any(|n| cidr_contains(n, ip))
    }
}

/// Каталог политик администратора (тот же, что у `update.json`).
pub fn admin_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var("ProgramData").ok().map(|pd| PathBuf::from(pd).join("RRCKI Mail"))
    } else {
        Some(PathBuf::from("/etc/rrcki-mail"))
    }
}

pub fn load() -> Policy {
    #[allow(unused_mut)]
    let mut p: Policy = admin_dir()
        .and_then(|d| std::fs::read(d.join("policy.json")).ok())
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default();
    #[cfg(windows)]
    {
        if let Some(v) = registry::string("RemoteImages") {
            match v.trim().to_ascii_lowercase().as_str() {
                "direct" => p.remote_images = RemoteImages::Direct,
                "proxy" => p.remote_images = RemoteImages::Proxy,
                "off" => p.remote_images = RemoteImages::Off,
                _ => {}
            }
        }
        if let Some(v) = registry::string("InternalNetworks") {
            p.internal_networks = v.split([',', ';', ' ']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
        }
        if let Some(v) = registry::dword("RememberPassword") {
            p.remember_password = Some(v != 0);
        }
        if let Some(v) = registry::dword("AllowRememberPassword") {
            p.allow_remember_password = Some(v != 0);
        }
        if let Some(v) = registry::dword("ShareLinkDays") {
            p.share_link_days = Some(v);
        }
        if let Some(v) = registry::dword("LocalStore") {
            p.local_store = Some(v != 0);
        }
        if let Some(v) = registry::dword("OfflineDays") {
            p.offline_days = Some(v);
        }
        if let Some(v) = registry::string("NotificationContent") {
            match v.trim().to_ascii_lowercase().as_str() {
                "full" => p.notification_content = NotificationContent::Full,
                "sender" => p.notification_content = NotificationContent::Sender,
                "none" => p.notification_content = NotificationContent::None,
                _ => {}
            }
        }
    }
    p
}

/// `10.0.0.0/8`, `2001:db8::/32` или отдельный адрес.
pub fn cidr_contains(net: &str, ip: IpAddr) -> bool {
    let (addr, bits) = match net.trim().split_once('/') {
        Some((a, b)) => (a.trim(), b.trim().parse::<u32>().ok()),
        None => (net.trim(), None),
    };
    let Ok(base) = addr.parse::<IpAddr>() else { return false };
    let ip = match (base, ip) {
        // IPv4-адрес, пришедший как IPv4-mapped IPv6.
        (IpAddr::V4(_), IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => return false,
        },
        _ => ip,
    };
    match (base, ip) {
        (IpAddr::V4(b), IpAddr::V4(i)) => {
            let bits = bits.unwrap_or(32).min(32);
            let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
            (u32::from(b) & mask) == (u32::from(i) & mask)
        }
        (IpAddr::V6(b), IpAddr::V6(i)) => {
            let bits = bits.unwrap_or(128).min(128);
            let mask = if bits == 0 { 0 } else { u128::MAX << (128 - bits) };
            (u128::from(b) & mask) == (u128::from(i) & mask)
        }
        _ => false,
    }
}

/// Значения групповой политики Windows (HKLM — пишет только администратор).
#[cfg(windows)]
pub mod registry {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};

    const KEY: &str = "SOFTWARE\\Policies\\RRCKI Mail";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn dword(name: &str) -> Option<u32> {
        let (key, value) = (wide(KEY), wide(name));
        let mut data: u32 = 0;
        let mut size: u32 = 4;
        // SAFETY: строки нуль-терминированы, буфер — 4 байта под REG_DWORD.
        let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut data as *mut u32).cast(), &mut size) };
        (r == 0).then_some(data)
    }

    pub fn string(name: &str) -> Option<String> {
        let (key, value) = (wide(KEY), wide(name));
        let mut buf = vec![0u16; 2048];
        let mut size: u32 = (buf.len() * 2) as u32;
        // SAFETY: буфер `buf` размером `size` байт, строка ключа и имени нуль-терминированы.
        let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
        if r != 0 {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_content_policy() {
        let mut p: Policy = serde_json::from_str(r#"{"notificationContent": "sender"}"#).unwrap();
        assert_eq!(p.redact_notification("Новое письмо — Иванов", "Тема: зарплата").1, "Откройте Atom Mail, чтобы прочитать");
        p.notification_content = NotificationContent::None;
        let (t, b) = p.redact_notification("Новое письмо — Иванов", "Тема");
        assert!(!t.contains("Иванов") && !b.contains("Тема"));
        assert_eq!(Policy::default().redact_notification("a", "b"), ("a".to_string(), "b".to_string()));
    }

    #[test]
    fn networks() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(cidr_contains("144.206.0.0/16", ip("144.206.12.3")));
        assert!(!cidr_contains("144.206.0.0/16", ip("144.207.0.1")));
        assert!(cidr_contains("144.206.0.0/16", ip("::ffff:144.206.1.1")));
        assert!(cidr_contains("10.1.2.3", ip("10.1.2.3")));
        assert!(cidr_contains("2001:db8::/32", ip("2001:db8:1::5")));
        assert!(!cidr_contains("garbage", ip("10.0.0.1")));
        let p: Policy = serde_json::from_str(r#"{"remoteImages":"off","internalNetworks":["192.0.2.0/24"],"allowRememberPassword":false,"rememberPassword":true}"#).unwrap();
        assert_eq!(p.remote_images, RemoteImages::Off);
        assert!(p.is_internal(ip("192.0.2.7")));
        let ui = p.ui();
        assert!(!ui.allow_remember_password && !ui.remember_password);
        assert_eq!(ui.share_link_days, 30);
    }
}
