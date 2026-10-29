//! Блокировка программы кодом TOTP (RFC 6238: 6 цифр, шаг 30 с, HMAC-SHA1 — как в
//! Яндекс Ключе, Google Authenticator, FreeOTP).
//!
//! Это замок на программе, а не второй фактор почтового сервера: без кода Atom Mail
//! не подключается к почте (сохранённый пароль не используется), но пароль почты
//! по-прежнему работает в других клиентах. Секрет и хэши резервных кодов — в
//! системном хранилище (рядом с паролем почты), не в файлах настроек.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};

const KEYRING_SERVICE: &str = "ru.rrcki.mail.totp";
pub const STEP: u64 = 30;
const DIGITS: u32 = 6;
pub const RECOVERY_CODES: usize = 8;

/// Хранится в системном хранилище для учётной записи `user@host`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stored {
    /// Секрет в base32 (как в QR-коде).
    pub secret: String,
    /// SHA-256 неиспользованных резервных кодов.
    pub recovery: Vec<String>,
    /// Последний принятый шаг времени: тот же код не принимается и после перезапуска.
    #[serde(default)]
    pub last_step: Option<u64>,
    /// Счётчик неверных кодов и пауза — сохраняются, перезапуск их не сбрасывает.
    #[serde(default)]
    pub throttle: Throttle,
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Проверить код (TOTP или резервный) и обновить состояние: последний шаг,
/// резервные коды, счётчик ошибок. Возвращает, подошёл ли код; `Err` — пауза.
pub fn apply(stored: &mut Stored, code: &str, now: u64) -> Result<bool> {
    stored.throttle.check(now)?;
    let code = code.trim();
    let ok = if code.contains('-') || code.len() > 7 {
        use_recovery(stored, code)
    } else {
        match verify(&stored.secret, code, now / STEP, stored.last_step) {
            Some(step) => {
                stored.last_step = Some(step);
                true
            }
            None => false,
        }
    };
    if ok {
        stored.throttle.ok();
    } else {
        stored.throttle.fail(now);
    }
    Ok(ok)
}

/// Проверка кода с сохранением состояния в системном хранилище. Если сохранить
/// не удалось — отказ (иначе счётчик ошибок и защита от повтора терялись бы).
pub fn check_and_record(account: &str, code: &str) -> Result<()> {
    let mut stored = load(account).ok_or_else(|| AppError::Other("Защита кодом не включена или хранилище паролей недоступно".into()))?;
    let ok = apply(&mut stored, code, unix_now())?;
    store(account, &stored)?;
    if ok {
        Ok(())
    } else {
        Err(AppError::Other("Неверный код".into()))
    }
}

fn entry(account: &str) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, account)
}

pub fn load(account: &str) -> Option<Stored> {
    let raw = entry(account).ok()?.get_password().ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn store(account: &str, s: &Stored) -> Result<()> {
    let raw = serde_json::to_string(s).map_err(|e| AppError::Other(e.to_string()))?;
    entry(account)
        .and_then(|e| e.set_password(&raw))
        .map_err(|e| AppError::Other(format!("Системное хранилище паролей недоступно — защиту кодом включить нельзя: {e}")))
}

pub fn remove(account: &str) {
    if let Ok(e) = entry(account) {
        let _ = e.delete_credential();
    }
}

/// Хранилище вообще работает (на терминальном сервере без keyring защиту не включить).
pub fn keyring_available(account: &str) -> bool {
    let probe = format!("{account}#probe");
    let ok = entry(&probe).and_then(|e| e.set_password("1")).is_ok();
    if ok {
        remove(&probe);
    }
    ok
}

const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in data {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            out.push(B32[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(B32[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.chars().filter(|c| !c.is_whitespace() && *c != '=' && *c != '-') {
        let v = B32.iter().position(|&x| x as char == c.to_ascii_uppercase())? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push((buf >> (bits - 8)) as u8);
            bits -= 8;
        }
    }
    Some(out)
}

/// Код для шага времени `counter`.
pub fn code_at(secret: &[u8], counter: u64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC принимает ключ любой длины");
    mac.update(&counter.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let off = (h[19] & 0x0f) as usize;
    let bin = (u32::from(h[off] & 0x7f) << 24) | (u32::from(h[off + 1]) << 16) | (u32::from(h[off + 2]) << 8) | u32::from(h[off + 3]);
    bin % 10u32.pow(DIGITS)
}

/// Проверяет код (допуск ±1 шаг — часы телефона могут отставать). Возвращает шаг,
/// которым подошёл код, — повторно этот же код не принимается.
pub fn verify(secret_b32: &str, code: &str, step: u64, last_used: Option<u64>) -> Option<u64> {
    let code = code.trim().replace(' ', "");
    if code.len() != DIGITS as usize || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let want: u32 = code.parse().ok()?;
    let secret = base32_decode(secret_b32)?;
    [step.saturating_sub(1), step, step + 1]
        .into_iter()
        .filter(|s| last_used.map_or(true, |u| *s > u))
        .find(|s| code_at(&secret, *s) == want)
}

fn random_bytes(n: usize) -> Result<Vec<u8>> {
    let mut b = vec![0u8; n];
    getrandom::getrandom(&mut b).map_err(|e| AppError::Other(format!("Генератор случайных чисел: {e}")))?;
    Ok(b)
}

pub fn new_secret() -> Result<String> {
    Ok(base32_encode(&random_bytes(20)?))
}

fn hash_code(code: &str) -> String {
    let norm: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    Sha256::digest(norm.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Резервные коды «xxxx-xxxx» (на случай потери телефона) и их хэши для хранения.
pub fn new_recovery_codes() -> Result<(Vec<String>, Vec<String>)> {
    const ALPHA: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut codes = Vec::new();
    for _ in 0..RECOVERY_CODES {
        let raw = random_bytes(8)?;
        let s: String = raw.iter().map(|b| ALPHA[*b as usize % ALPHA.len()] as char).collect();
        codes.push(format!("{}-{}", &s[..4], &s[4..]));
    }
    let hashes = codes.iter().map(|c| hash_code(c)).collect();
    Ok((codes, hashes))
}

/// Резервный код подходит — он удаляется из списка (одноразовый).
pub fn use_recovery(stored: &mut Stored, code: &str) -> bool {
    let h = hash_code(code);
    if let Some(i) = stored.recovery.iter().position(|x| *x == h) {
        stored.recovery.remove(i);
        true
    } else {
        false
    }
}

pub fn otpauth_uri(secret: &str, account: &str) -> String {
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~@".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    format!("otpauth://totp/{}:{}?secret={secret}&issuer={}&algorithm=SHA1&digits=6&period=30", enc("Atom Mail"), enc(account), enc("Atom Mail"))
}

/// QR-код ссылки для приложения-аутентификатора (SVG).
pub fn qr_svg(text: &str) -> Result<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|e| AppError::Other(format!("QR-код: {e}")))?;
    Ok(code
        .render::<qrcode::render::svg::Color<'_>>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build())
}

/// Ограничение перебора: после 5 неверных кодов — пауза, растущая с каждой ошибкой
/// (время — в секундах Unix, чтобы состояние можно было сохранить).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Throttle {
    pub failures: u32,
    pub blocked_until: Option<u64>,
}

impl Throttle {
    pub fn check(&self, now: u64) -> Result<()> {
        if let Some(t) = self.blocked_until {
            if t > now {
                return Err(AppError::Other(format!("Слишком много неверных кодов. Попробуйте через {} с", t - now)));
            }
        }
        Ok(())
    }

    pub fn fail(&mut self, now: u64) {
        self.failures += 1;
        if self.failures >= 5 {
            let secs = 30u64 << (self.failures - 5).min(6);
            self.blocked_until = Some(now + secs);
        }
    }

    pub fn ok(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_sha1_vectors() {
        // RFC 6238, приложение B: секрет "12345678901234567890", 8 цифр → берём младшие 6.
        let secret = b"12345678901234567890";
        for (t, want8) in [(59u64, 94287082u32), (1111111109, 7081804), (1111111111, 14050471), (1234567890, 89005924), (2000000000, 69279037)] {
            assert_eq!(code_at(secret, t / 30), want8 % 1_000_000, "t={t}");
        }
    }

    #[test]
    fn base32_roundtrip_and_known_value() {
        assert_eq!(base32_encode(b"12345678901234567890"), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        let s = new_secret().unwrap();
        assert_eq!(s.len(), 32);
        assert_eq!(base32_encode(&base32_decode(&s).unwrap()), s);
        assert_eq!(base32_decode("gezd gnbv").unwrap(), base32_decode("GEZDGNBV").unwrap());
    }

    #[test]
    fn verifies_with_window_and_rejects_reuse() {
        let secret = base32_encode(b"12345678901234567890");
        let raw = base32_decode(&secret).unwrap();
        let step = 1000;
        let code = |s: u64| format!("{:06}", code_at(&raw, s));
        assert_eq!(verify(&secret, &code(step), step, None), Some(step));
        assert_eq!(verify(&secret, &code(step - 1), step, None), Some(step - 1));
        assert_eq!(verify(&secret, &code(step + 1), step, None), Some(step + 1));
        assert_eq!(verify(&secret, &code(step - 2), step, None), None);
        assert_eq!(verify(&secret, &code(step), step, Some(step)), None, "тот же код второй раз");
        assert_eq!(verify(&secret, "12345", step, None), None);
        assert_eq!(verify(&secret, "abcdef", step, None), None);
    }

    #[test]
    fn recovery_codes_are_one_time() {
        let (codes, hashes) = new_recovery_codes().unwrap();
        assert_eq!(codes.len(), RECOVERY_CODES);
        let mut s = Stored { secret: new_secret().unwrap(), recovery: hashes, last_step: None, throttle: Throttle::default() };
        assert!(use_recovery(&mut s, &codes[3].to_uppercase()));
        assert!(!use_recovery(&mut s, &codes[3]));
        assert_eq!(s.recovery.len(), RECOVERY_CODES - 1);
        assert!(!use_recovery(&mut s, "zzzz-zzzz"));
    }

    #[test]
    fn throttles_after_five_failures() {
        let mut t = Throttle::default();
        for _ in 0..4 {
            t.fail(1000);
            assert!(t.check(1000).is_ok());
        }
        t.fail(1000);
        assert!(t.check(1000).is_err());
        assert!(t.check(1031).is_ok(), "пауза 30 с");
        t.ok();
        assert!(t.check(1000).is_ok());
    }

    /// Состояние переживает «перезапуск» (сохранение и загрузка из хранилища):
    /// пауза после ошибок не сбрасывается, принятый код не принимается повторно.
    #[test]
    fn state_survives_restart() {
        let secret = base32_encode(b"12345678901234567890");
        let raw = base32_decode(&secret).unwrap();
        let now = 1_700_000_000u64;
        let code = format!("{:06}", code_at(&raw, now / STEP));
        let mut s = Stored { secret: secret.clone(), recovery: vec![], last_step: None, throttle: Throttle::default() };
        assert!(apply(&mut s, &code, now).unwrap());
        let restarted = |s: &Stored| -> Stored { serde_json::from_str(&serde_json::to_string(s).unwrap()).unwrap() };
        let mut s = restarted(&s);
        assert!(!apply(&mut s, &code, now + 5).unwrap(), "повтор того же кода после перезапуска");
        for _ in 0..4 {
            let _ = apply(&mut s, "000000", now + 5);
        }
        let mut s = restarted(&s);
        assert!(apply(&mut s, &format!("{:06}", code_at(&raw, now / STEP + 1)), now + 6).is_err(), "пауза сохранилась");
        // Старые записи (0.14) без новых полей читаются.
        let old: Stored = serde_json::from_str(r#"{"secret":"GEZDGNBV","recovery":[]}"#).unwrap();
        assert!(old.last_step.is_none() && old.throttle.failures == 0);
    }

    #[test]
    fn builds_uri_and_qr() {
        let uri = otpauth_uri("GEZDGNBV", "ivanov@nrcki.ru");
        assert!(uri.starts_with("otpauth://totp/Atom%20Mail:ivanov@nrcki.ru?secret=GEZDGNBV&issuer=Atom%20Mail"));
        assert!(qr_svg(&uri).unwrap().starts_with("<?xml") || qr_svg(&uri).unwrap().contains("<svg"));
    }
}
