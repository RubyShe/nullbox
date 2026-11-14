//! Admin endpoints. One so far: the export the CEO pastes into a spreadsheet every Monday.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use nullbox_domain::Ticket;
use serde::Deserialize;

use super::extract::{ADMIN_TOKEN_HEADER, ApiQuery};
use crate::audit::{AuditEntry, DEFAULT_LIMIT};
use crate::{ApiError, AppState};

/// Every ticket, with customer names and emails. Guarded by `X-Admin-Token` or
/// `Authorization: Bearer`, which must match `ADMIN_TOKEN` (see `config.rs`).
/// An empty `ADMIN_TOKEN` locks the export for everyone.
pub async fn export(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Vec<Ticket>>, ApiError> {
    require_admin(&state, &headers)?;
    Ok(Json(state.tickets.export().await?))
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    q: Option<String>,
    limit: Option<i64>,
}

/// The audit trail, newest first. `q` is a substring match; same guard as the export.
pub async fn audit_log(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiQuery(query): ApiQuery<AuditQuery>,
) -> Result<Json<Vec<AuditEntry>>, ApiError> {
    require_admin(&state, &headers)?;
    Ok(Json(state.audit.search(query.q.as_deref(), query.limit.unwrap_or(DEFAULT_LIMIT)).await?))
}

fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = state.config.admin_token.as_str();
    let presented = presented_token(headers);
    if expected.is_empty() || !presented.is_some_and(|token| constant_time_eq(token.as_bytes(), expected.as_bytes())) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

fn presented_token(headers: &HeaderMap) -> Option<&str> {
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    header(ADMIN_TOKEN_HEADER).or_else(|| {
        let value = header(AUTHORIZATION.as_str())?;
        let (scheme, token) = value.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
    })
}

/// Compares without stopping at the first differing byte.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}
