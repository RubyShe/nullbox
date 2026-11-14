//! Request audit middleware: one audit entry per API call, refused ones included.
//!
//! It wraps the whole router, so a request is recorded whether a handler, an extractor or
//! a token check turned it away. Raw tokens are never stored, only a short fingerprint.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use percent_encoding::percent_decode_str;
use serde_json::json;

use super::extract::{ACTOR_HEADER, ADMIN_TOKEN_HEADER, TICKET_TOKEN_HEADER};
use crate::audit::AuditEntry;
use crate::{AppState, token};

pub async fn record(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let audited =
        request.method() != Method::OPTIONS && path != "/api/healthz" && (path.starts_with("/api/") || path == "/mcp");
    if !audited {
        return next.run(request).await;
    }

    let method = request.method().to_string();
    let actor = request
        .headers()
        .get(ACTOR_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| percent_decode_str(raw).decode_utf8().ok())
        .map(|name| name.trim().chars().take(100).collect::<String>())
        .filter(|name| !name.is_empty());
    let credential = credentials(request.headers());
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|info| info.0.ip().to_string());
    let forwarded = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|ip| ip.trim().to_owned())
        .filter(|ip| !ip.is_empty());

    let response = next.run(request).await;
    let status = response.status();

    let refused = matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN);
    let via = credential.as_deref().map(|c| format!(" [{c}]")).unwrap_or_default();
    let summary = if refused {
        format!("refused {method} {path} ({}){via}", status.as_u16())
    } else {
        format!("{method} {path} -> {}{via}", status.as_u16())
    };

    let mut entry = AuditEntry::new("http_request", summary);
    entry.actor = actor;
    entry.client_ip = forwarded.clone().or_else(|| peer.clone());
    entry.method = Some(method);
    entry.path = Some(path);
    entry.status = Some(i32::from(status.as_u16()));
    entry.credential = credential;
    entry.detail = Some(json!({ "peer": peer, "x_forwarded_for": forwarded }).to_string());

    if let Err(error) = state.audit.record(entry).await {
        tracing::error!(target: "nullbox::audit", %error, "failed to write audit entry");
    }
    response
}

/// Fingerprints of every credential the caller presented, e.g. `bearer:1a2b3c4d5e6f`.
fn credentials(headers: &HeaderMap) -> Option<String> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).filter(|v| !v.is_empty());
    let mut found = Vec::new();
    if let Some(value) = header(AUTHORIZATION.as_str()) {
        match value.split_once(' ') {
            Some((scheme, secret)) => found.push(fingerprint(&scheme.to_ascii_lowercase(), secret.trim())),
            None => found.push(fingerprint("authorization", value)),
        }
    }
    if let Some(value) = header(ADMIN_TOKEN_HEADER) {
        found.push(fingerprint("admin-token", value));
    }
    if let Some(value) = header(TICKET_TOKEN_HEADER) {
        found.push(fingerprint("ticket-token", value));
    }
    (!found.is_empty()).then(|| found.join(", "))
}

fn fingerprint(label: &str, secret: &str) -> String {
    format!("{label}:{}", &token::hash(secret)[..12])
}
