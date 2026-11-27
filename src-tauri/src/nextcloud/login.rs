//! Вход по Login Flow v2: начало и ожидание подтверждения в браузере.
//!
//! Часть модуля `nextcloud` — общий обзор в `mod.rs`.

use super::*;

#[derive(Debug, Clone)]
pub struct LoginFlow {
    pub login_url: String,
    pub poll_endpoint: Url,
    pub poll_token: String,
}

pub async fn login_flow_start(base: &Url) -> Result<LoginFlow> {
    let client = http(Duration::from_secs(30))?;
    let url = base.join("index.php/login/v2").expect("valid path");
    let resp = client.post(url).send().await.map_err(net_error)?;
    if !resp.status().is_success() {
        return Err(status_error(resp.status(), "Вход в Nextcloud"));
    }
    let v: Value = crate::netio::json_limited(resp, crate::netio::SMALL_LIMIT).await?;
    let s = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_string);
    let endpoint = s("/poll/endpoint").ok_or_else(|| AppError::Other("Nextcloud не вернул адрес проверки входа".into()))?;
    let poll_endpoint = Url::parse(&endpoint).map_err(|e| AppError::Other(e.to_string()))?;
    // Ответ сервера не должен уводить пароль приложения на чужой хост.
    if poll_endpoint.origin() != base.origin() {
        return Err(AppError::Other("Nextcloud вернул адрес на другом сервере — вход прерван".into()));
    }
    Ok(LoginFlow {
        login_url: s("/login").ok_or_else(|| AppError::Other("Nextcloud не вернул адрес входа".into()))?,
        poll_endpoint,
        poll_token: s("/poll/token").ok_or_else(|| AppError::Other("Nextcloud не вернул токен входа".into()))?,
    })
}

/// `Ok(None)` — пользователь ещё не подтвердил вход в браузере.
pub async fn login_flow_poll(base: &Url, flow: &LoginFlow) -> Result<Option<NcCreds>> {
    let client = http(Duration::from_secs(30))?;
    let resp = client
        .post(flow.poll_endpoint.clone())
        .form(&[("token", flow.poll_token.as_str())])
        .send()
        .await
        .map_err(net_error)?;
    if resp.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(status_error(resp.status(), "Вход в Nextcloud"));
    }
    let v: Value = crate::netio::json_limited(resp, crate::netio::SMALL_LIMIT).await?;
    let login = v["loginName"].as_str().unwrap_or_default().to_string();
    let app_password = v["appPassword"].as_str().unwrap_or_default().to_string();
    if login.is_empty() || app_password.is_empty() {
        return Err(AppError::Other("Nextcloud вернул неполные данные входа".into()));
    }
    Ok(Some(NcCreds { base: base.clone(), login, app_password }))
}

// ---------------------------------------------------------------- клиент

