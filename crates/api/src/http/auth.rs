use axum::{extract::State, http::{header, HeaderMap, HeaderValue}, Json};
use serde::{Deserialize, Serialize};
use crate::{auth::{cookie_header, require_csrf_header}, http::extract::{Actor, ApiJson}, ApiError, AppState};
#[derive(Deserialize)]
pub struct Login { username: String, password: String }
#[derive(Serialize)]
pub struct Session { name: String }
pub async fn signup(State(state): State<AppState>, headers: HeaderMap, ApiJson(req): ApiJson<Login>) -> Result<Json<Session>, ApiError> {
    require_csrf_header(&headers)?;
    let name = req.username.trim();
    if name.is_empty() || name.chars().count() > 100 { return Err(ApiError::Validation("username must be 1 to 100 characters".into())); }
    crate::auth::check_password(&req.password).await?;
    let hash = crate::auth::hash_password(req.password).await?;
    state.accounts.insert(name.to_owned(), hash)?;
    Ok(Json(Session { name: name.to_owned() }))
}
pub async fn login(State(state): State<AppState>, headers: HeaderMap, ApiJson(req): ApiJson<Login>) -> Result<(HeaderMap, Json<Session>), ApiError> {
    require_csrf_header(&headers)?;
    let name = req.username.trim();
    let Some(hash) = state.accounts.password_hash(name)? else { return Err(ApiError::MissingActor); };
    if !crate::auth::verify_password(req.password, hash).await? { return Err(ApiError::MissingActor); }
    let token = state.sessions.create(name)?;
    state.sessions.remove(&headers);
    let mut response = HeaderMap::new();
    response.insert(header::SET_COOKIE, HeaderValue::from_str(&cookie_header(&token, state.config.cookie_secure, 28800)).map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?);
    Ok((response, Json(Session { name: name.to_owned() })))
}
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<(HeaderMap, Json<Session>), ApiError> {
    require_csrf_header(&headers)?;
    state.sessions.remove(&headers);
    let mut response = HeaderMap::new();
    response.insert(header::SET_COOKIE, HeaderValue::from_str(&cookie_header("", state.config.cookie_secure, 0)).map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?);
    Ok((response, Json(Session { name: String::new() })))
}
pub async fn session(actor: Actor) -> Json<Session> { Json(Session { name: actor.name }) }
