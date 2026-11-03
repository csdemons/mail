//! Вход NTLMv2 (MS-NLMP) для Exchange: сервер Exchange в организации обычно
//! принимает на EWS вход Windows (NTLM/Negotiate), а простой вход (Basic) часто
//! выключен.
//!
//! Только NTLMv2 (LM и NTLMv1 не поддерживаются — они небезопасны). Поддерживается
//! привязка к TLS-каналу (Extended Protection, `MsvAvChannelBindings`): её
//! включает Exchange 2019 и новее, без неё вход отклоняется. Сообщение
//! AUTHENTICATE подписывается (MIC), как требуют современные серверы.

use hmac::{Hmac, Mac};
use md4::{Digest as _, Md4};
use md5::Md5;

type HmacMd5 = Hmac<Md5>;

const NEGOTIATE_UNICODE: u32 = 0x0000_0001;
const REQUEST_TARGET: u32 = 0x0000_0004;
const NEGOTIATE_NTLM: u32 = 0x0000_0200;
const NEGOTIATE_ALWAYS_SIGN: u32 = 0x0000_8000;
const NEGOTIATE_EXTENDED_SESSIONSECURITY: u32 = 0x0008_0000;
const NEGOTIATE_TARGET_INFO: u32 = 0x0080_0000;
const NEGOTIATE_VERSION: u32 = 0x0200_0000;
const NEGOTIATE_128: u32 = 0x2000_0000;

const FLAGS: u32 = NEGOTIATE_UNICODE | REQUEST_TARGET | NEGOTIATE_NTLM | NEGOTIATE_ALWAYS_SIGN | NEGOTIATE_EXTENDED_SESSIONSECURITY | NEGOTIATE_TARGET_INFO | NEGOTIATE_VERSION | NEGOTIATE_128;

const AV_EOL: u16 = 0;
const AV_FLAGS: u16 = 6;
const AV_TIMESTAMP: u16 = 7;
const AV_TARGET_NAME: u16 = 9;
const AV_CHANNEL_BINDINGS: u16 = 10;

/// Версия Windows в сообщениях (только для отладки на стороне сервера).
const VERSION: [u8; 8] = [10, 0, 0x61, 0x4a, 0, 0, 0, 15];

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect()
}

fn hmac_md5(key: &[u8], parts: &[&[u8]]) -> [u8; 16] {
    let mut m = HmacMd5::new_from_slice(key).expect("HMAC accepts any key length");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// NTOWFv2: ключ из пароля, имени пользователя и домена.
pub fn ntowf_v2(user: &str, domain: &str, password: &str) -> [u8; 16] {
    let nt_hash: [u8; 16] = Md4::digest(utf16(password)).into();
    hmac_md5(&nt_hash, &[&utf16(&user.to_uppercase()), &utf16(domain)])
}

/// Сообщение NEGOTIATE (тип 1).
pub fn negotiate() -> Vec<u8> {
    let mut m = Vec::with_capacity(40);
    m.extend_from_slice(b"NTLMSSP\0");
    m.extend_from_slice(&1u32.to_le_bytes());
    m.extend_from_slice(&FLAGS.to_le_bytes());
    // DomainNameFields и WorkstationFields — пустые (смещение — конец сообщения).
    m.extend_from_slice(&[0, 0, 0, 0, 40, 0, 0, 0]);
    m.extend_from_slice(&[0, 0, 0, 0, 40, 0, 0, 0]);
    m.extend_from_slice(&VERSION);
    m
}

/// Разобранное сообщение CHALLENGE (тип 2).
#[derive(Debug, Clone)]
pub struct Challenge {
    pub flags: u32,
    pub server_challenge: [u8; 8],
    pub target_info: Vec<u8>,
    raw: Vec<u8>,
}

fn field(msg: &[u8], at: usize) -> Option<&[u8]> {
    let len = u16::from_le_bytes(msg.get(at..at + 2)?.try_into().ok()?) as usize;
    let off = u32::from_le_bytes(msg.get(at + 4..at + 8)?.try_into().ok()?) as usize;
    msg.get(off..off.checked_add(len)?)
}

pub fn parse_challenge(msg: &[u8]) -> Option<Challenge> {
    if msg.len() < 32 || &msg[..8] != b"NTLMSSP\0" || u32::from_le_bytes(msg[8..12].try_into().ok()?) != 2 {
        return None;
    }
    let flags = u32::from_le_bytes(msg[20..24].try_into().ok()?);
    let server_challenge: [u8; 8] = msg[24..32].try_into().ok()?;
    let target_info = if msg.len() >= 48 { field(msg, 40).unwrap_or_default().to_vec() } else { Vec::new() };
    Some(Challenge { flags, server_challenge, target_info, raw: msg.to_vec() })
}

/// Пары AV (TargetInfo) — список (идентификатор, значение) без завершающей пары.
fn av_pairs(info: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= info.len() {
        let id = u16::from_le_bytes([info[i], info[i + 1]]);
        let len = u16::from_le_bytes([info[i + 2], info[i + 3]]) as usize;
        i += 4;
        if id == AV_EOL || i + len > info.len() {
            break;
        }
        out.push((id, info[i..i + len].to_vec()));
        i += len;
    }
    out
}

fn encode_av(pairs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (id, v) in pairs {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(v.len() as u16).to_le_bytes());
        out.extend_from_slice(v);
    }
    out.extend_from_slice(&[0, 0, 0, 0]);
    out
}

/// Значение `MsvAvChannelBindings`: MD5 структуры gss_channel_bindings с
/// данными приложения `tls-server-end-point:<хэш сертификата сервера>` (RFC 5929).
pub fn channel_bindings(cert_der: &[u8]) -> [u8; 16] {
    let hash = cert_end_point_hash(cert_der);
    let mut app = b"tls-server-end-point:".to_vec();
    app.extend_from_slice(&hash);
    let mut s = vec![0u8; 16];
    s.extend_from_slice(&(app.len() as u32).to_le_bytes());
    s.extend_from_slice(&app);
    Md5::digest(&s).into()
}

/// Хэш сертификата для tls-server-end-point: алгоритм подписи сертификата,
/// но не слабее SHA-256 (MD5 и SHA-1 заменяются на SHA-256).
fn cert_end_point_hash(der: &[u8]) -> Vec<u8> {
    use sha2::Digest as _;
    // OID в DER: sha384WithRSAEncryption, ecdsa-with-SHA384, sha512WithRSAEncryption, ecdsa-with-SHA512.
    const SHA384: [&[u8]; 2] = [&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0c], &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x03]];
    const SHA512: [&[u8]; 2] = [&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0d], &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x04]];
    let alg = signature_algorithm(der);
    if alg.as_deref().is_some_and(|a| SHA512.contains(&a)) {
        sha2::Sha512::digest(der).to_vec()
    } else if alg.as_deref().is_some_and(|a| SHA384.contains(&a)) {
        sha2::Sha384::digest(der).to_vec()
    } else {
        sha2::Sha256::digest(der).to_vec()
    }
}

/// OID алгоритма подписи сертификата (второй элемент SEQUENCE Certificate).
fn signature_algorithm(der: &[u8]) -> Option<Vec<u8>> {
    fn header(b: &[u8]) -> Option<(u8, usize, usize)> {
        let tag = *b.first()?;
        let first = *b.get(1)? as usize;
        if first < 0x80 {
            return Some((tag, 2, first));
        }
        let n = first & 0x7f;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *b.get(2 + i)? as usize;
        }
        Some((tag, 2 + n, len))
    }
    let (tag, h, _) = header(der)?;
    if tag != 0x30 {
        return None;
    }
    let body = &der[h..];
    let (_, th, tlen) = header(body)?; // tbsCertificate
    let rest = body.get(th + tlen..)?;
    let (atag, ah, _) = header(rest)?; // AlgorithmIdentifier
    if atag != 0x30 {
        return None;
    }
    let (otag, oh, olen) = header(&rest[ah..])?;
    if otag != 0x06 {
        return None;
    }
    rest.get(ah + oh..ah + oh + olen).map(<[u8]>::to_vec)
}

/// Сообщение AUTHENTICATE (тип 3). `user` — имя без домена; `domain` — домен
/// Windows (может быть пустым, если имя вида user@domain). `cert_der` — сертификат
/// сервера для привязки к каналу. `negotiate_msg` — отправленное сообщение типа 1 (для MIC).
#[allow(clippy::too_many_arguments)]
pub fn authenticate(
    challenge: &Challenge,
    negotiate_msg: &[u8],
    user: &str,
    domain: &str,
    password: &str,
    workstation: &str,
    cert_der: Option<&[u8]>,
    spn: Option<&str>,
    client_challenge: [u8; 8],
    now_filetime: u64,
) -> Vec<u8> {
    let key = ntowf_v2(user, domain, password);
    let mut pairs = av_pairs(&challenge.target_info);
    let server_time = pairs.iter().find(|(id, _)| *id == AV_TIMESTAMP).and_then(|(_, v)| v.as_slice().try_into().ok()).map(u64::from_le_bytes);
    let timestamp = server_time.unwrap_or(now_filetime);
    let with_mic = server_time.is_some();
    // Флаги: MIC присутствует (0x2).
    pairs.retain(|(id, _)| *id != AV_FLAGS && *id != AV_CHANNEL_BINDINGS && *id != AV_TARGET_NAME);
    if with_mic {
        pairs.push((AV_FLAGS, 2u32.to_le_bytes().to_vec()));
    }
    if let Some(cert) = cert_der {
        pairs.push((AV_CHANNEL_BINDINGS, channel_bindings(cert).to_vec()));
    }
    if let Some(spn) = spn {
        pairs.push((AV_TARGET_NAME, utf16(spn)));
    }
    let info = encode_av(&pairs);

    let mut temp = vec![1u8, 1, 0, 0, 0, 0, 0, 0];
    temp.extend_from_slice(&timestamp.to_le_bytes());
    temp.extend_from_slice(&client_challenge);
    temp.extend_from_slice(&[0, 0, 0, 0]);
    temp.extend_from_slice(&info);
    temp.extend_from_slice(&[0, 0, 0, 0]);
    let proof = hmac_md5(&key, &[&challenge.server_challenge, &temp]);
    let mut nt = proof.to_vec();
    nt.extend_from_slice(&temp);
    // С MIC ответ LM — нули (24 байта), как требует MS-NLMP.
    let lm: Vec<u8> = if with_mic {
        vec![0u8; 24]
    } else {
        let mut lm = hmac_md5(&key, &[&challenge.server_challenge, &client_challenge]).to_vec();
        lm.extend_from_slice(&client_challenge);
        lm
    };
    let session_key = hmac_md5(&key, &[&proof]);

    let domain_b = utf16(domain);
    let user_b = utf16(user);
    let ws_b = utf16(workstation);
    let flags = challenge.flags & FLAGS | NEGOTIATE_UNICODE;
    // Заголовок 88 байт: подпись, тип, 6 полей, флаги, версия, MIC.
    let header_len = 88usize;
    let mut payload = Vec::new();
    let mut fields = Vec::new();
    for data in [&lm, &nt, &domain_b, &user_b, &ws_b, &Vec::new()] {
        let off = header_len + payload.len();
        fields.push((data.len() as u16, off as u32));
        payload.extend_from_slice(data);
    }
    let mut m = Vec::with_capacity(header_len + payload.len());
    m.extend_from_slice(b"NTLMSSP\0");
    m.extend_from_slice(&3u32.to_le_bytes());
    for (len, off) in &fields {
        m.extend_from_slice(&len.to_le_bytes());
        m.extend_from_slice(&len.to_le_bytes());
        m.extend_from_slice(&off.to_le_bytes());
    }
    m.extend_from_slice(&flags.to_le_bytes());
    m.extend_from_slice(&VERSION);
    m.extend_from_slice(&[0u8; 16]); // MIC
    m.extend_from_slice(&payload);
    if with_mic {
        let mic = hmac_md5(&session_key, &[negotiate_msg, &challenge.raw, &m]);
        m[72..88].copy_from_slice(&mic);
    }
    m
}

/// Текущее время в формате FILETIME (сотни наносекунд с 1601 года).
pub fn filetime_now() -> u64 {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() / 100).unwrap_or(0) as u64;
    unix + 116_444_736_000_000_000
}

/// Имя для NTLM: `DOMAIN\user` → (user, DOMAIN); `user@domain` и `user` — без домена.
pub fn split_login(login: &str) -> (String, String) {
    match login.split_once('\\') {
        Some((d, u)) => (u.to_string(), d.to_string()),
        None => (login.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    // Пример из MS-NLMP, раздел 4.2.4 (NTLMv2 Authentication).
    #[test]
    fn ms_nlmp_vectors() {
        let key = ntowf_v2("User", "Domain", "Password");
        assert_eq!(hex(&key), "0c868a403bfd7a93a3001ef22ef02e3f");
        let server_challenge = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
        let client_challenge = [0xaa; 8];
        let info = encode_av(&[(2, utf16("Domain")), (1, utf16("Server"))]);
        let mut temp = vec![1u8, 1, 0, 0, 0, 0, 0, 0];
        temp.extend_from_slice(&0u64.to_le_bytes());
        temp.extend_from_slice(&client_challenge);
        temp.extend_from_slice(&[0, 0, 0, 0]);
        temp.extend_from_slice(&info);
        temp.extend_from_slice(&[0, 0, 0, 0]);
        let proof = hmac_md5(&key, &[&server_challenge, &temp]);
        assert_eq!(hex(&proof), "68cd0ab851e51c96aabc927bebef6a1c");
        assert_eq!(hex(&hmac_md5(&key, &[&proof])), "8de40ccadbc14a82f15cb0ad0de95ca3");
        let lm = hmac_md5(&key, &[&server_challenge, &client_challenge]);
        assert_eq!(hex(&lm), "86c35097ac9cec102554764a57cccc19");
    }

    #[test]
    fn messages_round_trip() {
        let neg = negotiate();
        assert_eq!(&neg[..8], b"NTLMSSP\0");
        assert_eq!(neg.len(), 40);
        // Сервер: вызов с TargetInfo (домен, время) — клиент добавляет MIC и привязку к каналу.
        let info = encode_av(&[(2, utf16("CORP")), (AV_TIMESTAMP, 5u64.to_le_bytes().to_vec())]);
        let mut ch = b"NTLMSSP\0".to_vec();
        ch.extend_from_slice(&2u32.to_le_bytes());
        ch.extend_from_slice(&[0, 0, 0, 0, 56, 0, 0, 0]);
        ch.extend_from_slice(&FLAGS.to_le_bytes());
        ch.extend_from_slice(&[7; 8]);
        ch.extend_from_slice(&[0; 8]);
        ch.extend_from_slice(&(info.len() as u16).to_le_bytes());
        ch.extend_from_slice(&(info.len() as u16).to_le_bytes());
        ch.extend_from_slice(&56u32.to_le_bytes());
        ch.extend_from_slice(&VERSION);
        ch.extend_from_slice(&info);
        let c = parse_challenge(&ch).unwrap();
        assert_eq!(c.server_challenge, [7; 8]);
        let auth = authenticate(&c, &neg, "ivanov", "CORP", "Пароль-1", "PC", Some(b"cert"), Some("HTTP/mail.corp.ru"), [1; 8], 9);
        assert_eq!(u32::from_le_bytes(auth[8..12].try_into().unwrap()), 3);
        let nt = field(&auth, 20).unwrap();
        // NTProofStr (16) + заголовок blob (28) + AV: время сервера, флаги MIC, привязка канала, SPN.
        let pairs = av_pairs(&nt[16 + 28..]);
        assert!(pairs.iter().any(|(id, v)| *id == AV_TIMESTAMP && v == &5u64.to_le_bytes()));
        assert!(pairs.iter().any(|(id, v)| *id == AV_FLAGS && v == &2u32.to_le_bytes()));
        assert!(pairs.iter().any(|(id, v)| *id == AV_CHANNEL_BINDINGS && v == &channel_bindings(b"cert")));
        assert_eq!(field(&auth, 28).unwrap(), utf16("CORP").as_slice());
        assert_eq!(field(&auth, 36).unwrap(), utf16("ivanov").as_slice());
        assert_eq!(field(&auth, 12).unwrap(), &[0u8; 24][..]);
        assert_ne!(&auth[72..88], &[0u8; 16]);
        assert!(parse_challenge(b"garbage").is_none());
        assert_eq!(split_login("CORP\\ivanov"), ("ivanov".to_string(), "CORP".to_string()));
        assert_eq!(split_login("ivanov@corp.ru"), ("ivanov@corp.ru".to_string(), String::new()));
    }
}
