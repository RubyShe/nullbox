//! Ephemeral demo sessions. Restarting the API invalidates every session.
use std::{collections::HashMap, sync::Mutex, time::{Duration, Instant}};
use axum::http::HeaderMap;
use crate::{token, ApiError};

const LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);
const COOKIE: &str = "nullbox_session";
#[derive(Default)]
pub struct Sessions(Mutex<HashMap<String, (Instant, String)>>);
impl Sessions {
    pub fn create(&self, name: &str) -> Result<String, ApiError> {
        let value = token::generate().map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?;
        let mut sessions = self.0.lock().map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?;
        sessions.retain(|_, (expiry, _)| *expiry > Instant::now());
        if sessions.len() >= 1000 { return Err(ApiError::WorksOnMyMachine("session capacity reached".into())); }
        sessions.insert(token::hash(&value), (Instant::now() + LIFETIME, name.to_owned()));
        Ok(value)
    }
    pub fn identity(&self, headers: &HeaderMap) -> Option<String> {
        let Some(value) = cookie(headers) else { return None };
        let Ok(mut sessions) = self.0.lock() else { return None };
        let key = token::hash(value);
        match sessions.get(&key) {
            Some((expiry, name)) if *expiry > Instant::now() => Some(name.clone()),
            _ => { sessions.remove(&key); None }
        }
    }
    pub fn remove(&self, headers: &HeaderMap) {
        if let (Some(value), Ok(mut sessions)) = (cookie(headers), self.0.lock()) { sessions.remove(&token::hash(value)); }
    }
}
fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers.get_all(axum::http::header::COOKIE).iter().filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';')).filter_map(|v| v.trim().split_once('='))
        .find_map(|(name, value)| (name == COOKIE && !value.is_empty()).then_some(value))
}
pub fn cookie_header(value: &str, secure: bool, max_age: u64) -> String {
    format!("{COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}", if secure { "; Secure" } else { "" })
}
pub fn require_csrf_header(headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.get("x-nullbox-request").and_then(|v| v.to_str().ok()) == Some("1") { Ok(()) } else { Err(ApiError::Forbidden) }
}

/// Accounts live only for the lifetime of this demo process.
#[derive(Default)]
pub struct Accounts(Mutex<HashMap<String, String>>);
impl Accounts {
    pub fn insert(&self, name: String, hash: String) -> Result<(), ApiError> {
        let mut users = self.0.lock().map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?;
        if users.contains_key(&name) { return Err(ApiError::UsernameTaken); }
        if users.len() >= 1000 { return Err(ApiError::WorksOnMyMachine("account capacity reached".into())); }
        users.insert(name, hash);
        Ok(())
    }
    pub fn password_hash(&self, name: &str) -> Result<Option<String>, ApiError> {
        Ok(self.0.lock().map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?.get(name).cloned())
    }
}

pub async fn check_password(password: &str) -> Result<(), ApiError> {
    use sha1::{Digest, Sha1};
    if password.chars().count() < 12 || password.len() > 1024 {
        return Err(ApiError::PasswordLength);
    }
    let hash = format!("{:X}", Sha1::digest(password.as_bytes()));
    let response = reqwest::Client::builder().timeout(Duration::from_secs(5)).build()
        .map_err(|e| ApiError::ProbablyDns(e.to_string()))?
        .get(format!("https://api.pwnedpasswords.com/range/{}", &hash[..5]))
        .header("Add-Padding", "true").send().await
        .map_err(|e| ApiError::ProbablyDns(e.to_string()))?
        .error_for_status().map_err(|e| ApiError::ProbablyDns(e.to_string()))?
        .text().await.map_err(|e| ApiError::ProbablyDns(e.to_string()))?;
    if contains_breached_hash(&response, &hash[5..])? {
        return Err(ApiError::PasswordCompromised);
    }
    Ok(())
}

pub async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || {
        use argon2::{Argon2, password_hash::{PasswordHasher, SaltString}};
        let random = token::generate().map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?;
        let salt = SaltString::encode_b64(random.as_bytes()).map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?;
        Argon2::default().hash_password(password.as_bytes(), &salt).map(|v| v.to_string())
            .map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))
    }).await.map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?
}
pub async fn verify_password(password: String, hash: String) -> Result<bool, ApiError> {
    tokio::task::spawn_blocking(move || {
        use argon2::{Argon2, password_hash::{PasswordHash, PasswordVerifier}};
        let parsed = PasswordHash::new(&hash).map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?;
        Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
    }).await.map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?
}

fn contains_breached_hash(response: &str, suffix: &str) -> Result<bool, ApiError> {
    let mut found = false;
    let mut lines = 0;
    for line in response.lines() {
        let (candidate, count) = line.trim().split_once(':').ok_or_else(|| ApiError::ProbablyDns("invalid password check response".into()))?;
        if candidate.len() != 35 || !candidate.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::ProbablyDns("invalid password check response".into()));
        }
        let count: u64 = count.parse().map_err(|_| ApiError::ProbablyDns("invalid password check response".into()))?;
        lines += 1;
        found |= candidate.eq_ignore_ascii_case(suffix) && count > 0;
    }
    if lines == 0 { return Err(ApiError::ProbablyDns("empty password check response".into())); }
    Ok(found)
}
#[cfg(test)]
mod tests {
    use super::contains_breached_hash;
    #[test]
    fn breach_lookup_requires_a_positive_count_and_valid_response() {
        let hash = "0123456789ABCDEF0123456789ABCDEF012";
        assert!(contains_breached_hash(&format!("{hash}:23\r\n"), hash).unwrap());
        assert!(!contains_breached_hash(&format!("{hash}:0\r\n"), hash).unwrap());
        assert!(contains_breached_hash("bad", hash).is_err());
    }
}
