//! Audit log: who did what, and who was refused. Append-only.
//!
//! Two writers feed it: the request middleware (`http::audit`) records every API call,
//! and [`AuditEventSink`] records domain events (a ticket was created, a status changed).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use nullbox_domain::DomainEvent;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use ts_rs::TS;
use uuid::Uuid;

use crate::events::EventSink;
use crate::repo::RepoError;

pub const DEFAULT_LIMIT: i64 = 200;
pub const MAX_LIMIT: i64 = 1000;

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct AuditEntry {
    pub id: String,
    /// RFC 3339 UTC.
    pub occurred_at: String,
    /// `http_request`, or the domain event kind (`ticket_created`, ...).
    pub kind: String,
    pub actor: Option<String>,
    pub client_ip: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub status: Option<i32>,
    /// Fingerprint of a presented token, e.g. `bearer:1a2b3c4d5e6f`. Never the token.
    pub credential: Option<String>,
    pub ticket_id: Option<String>,
    pub summary: String,
    pub detail: Option<String>,
}

impl AuditEntry {
    pub fn new(kind: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7().to_string(),
            occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            kind: kind.into(),
            actor: None,
            client_ip: None,
            method: None,
            path: None,
            status: None,
            credential: None,
            ticket_id: None,
            summary: summary.into(),
            detail: None,
        }
    }
}

#[async_trait]
pub trait AuditLog: Send + Sync {
    async fn record(&self, entry: AuditEntry) -> Result<(), RepoError>;

    /// Newest first. `query` is a case-insensitive substring match over the text columns.
    async fn search(&self, query: Option<&str>, limit: i64) -> Result<Vec<AuditEntry>, RepoError>;
}

pub struct SqliteAuditLog {
    pool: SqlitePool,
}

impl SqliteAuditLog {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AuditLog for SqliteAuditLog {
    async fn record(&self, entry: AuditEntry) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO audit_log (id, occurred_at, kind, actor, client_ip, method, path, status, credential,
                                    ticket_id, summary, detail)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(entry.id)
        .bind(entry.occurred_at)
        .bind(entry.kind)
        .bind(entry.actor)
        .bind(entry.client_ip)
        .bind(entry.method)
        .bind(entry.path)
        .bind(entry.status)
        .bind(entry.credential)
        .bind(entry.ticket_id)
        .bind(entry.summary)
        .bind(entry.detail)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn search(&self, query: Option<&str>, limit: i64) -> Result<Vec<AuditEntry>, RepoError> {
        let pattern = query.map(str::trim).filter(|q| !q.is_empty()).map(like_pattern);
        let rows = sqlx::query(
            r"SELECT id, occurred_at, kind, actor, client_ip, method, path, status, credential, ticket_id, summary, detail
              FROM audit_log
              WHERE ?1 IS NULL
                 OR summary    LIKE ?1 ESCAPE '\'
                 OR kind       LIKE ?1 ESCAPE '\'
                 OR actor      LIKE ?1 ESCAPE '\'
                 OR client_ip  LIKE ?1 ESCAPE '\'
                 OR path       LIKE ?1 ESCAPE '\'
                 OR credential LIKE ?1 ESCAPE '\'
                 OR ticket_id  LIKE ?1 ESCAPE '\'
              ORDER BY occurred_at DESC, id DESC
              LIMIT ?2",
        )
        .bind(pattern)
        .bind(limit.clamp(1, MAX_LIMIT))
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                Ok(AuditEntry {
                    id: row.try_get("id")?,
                    occurred_at: row.try_get("occurred_at")?,
                    kind: row.try_get("kind")?,
                    actor: row.try_get("actor")?,
                    client_ip: row.try_get("client_ip")?,
                    method: row.try_get("method")?,
                    path: row.try_get("path")?,
                    status: row.try_get("status")?,
                    credential: row.try_get("credential")?,
                    ticket_id: row.try_get("ticket_id")?,
                    summary: row.try_get("summary")?,
                    detail: row.try_get("detail")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(RepoError::from)
    }
}

/// `%term%` with LIKE wildcards in `term` escaped (backslash is the ESCAPE character).
fn like_pattern(term: &str) -> String {
    let mut out = String::with_capacity(term.len() + 2);
    out.push('%');
    for c in term.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Turns domain events into audit entries. `emit` is sync, so the write is spawned.
pub struct AuditEventSink {
    log: Arc<dyn AuditLog>,
}

impl AuditEventSink {
    pub fn new(log: Arc<dyn AuditLog>) -> Self {
        Self { log }
    }
}

impl EventSink for AuditEventSink {
    fn emit(&self, event: DomainEvent) {
        let entry = entry_for_event(&event);
        let log = self.log.clone();
        tokio::spawn(async move {
            if let Err(error) = log.record(entry).await {
                tracing::error!(target: "nullbox::audit", %error, "failed to write audit entry");
            }
        });
    }
}

fn entry_for_event(event: &DomainEvent) -> AuditEntry {
    let json = serde_json::to_value(event).unwrap_or_default();
    let kind = json.get("kind").and_then(|k| k.as_str()).unwrap_or("domain_event").to_owned();
    let mut entry = match event {
        DomainEvent::TicketCreated { ticket_id, number, severity, subject } => {
            let mut e =
                AuditEntry::new(kind, format!("ticket #{number} created (severity {}): {subject}", severity.as_str()));
            e.ticket_id = Some(ticket_id.to_string());
            e
        }
        DomainEvent::StatusChanged { ticket_id, from, to, actor } => {
            let mut e = AuditEntry::new(kind, format!("status {} -> {}", from.as_str(), to.as_str()));
            e.ticket_id = Some(ticket_id.to_string());
            e.actor.clone_from(actor);
            e
        }
        DomainEvent::MessageAdded { ticket_id, internal, actor, .. } => {
            let what = if *internal { "internal note" } else { "message" };
            let mut e = AuditEntry::new(kind, format!("{what} added"));
            e.ticket_id = Some(ticket_id.to_string());
            e.actor.clone_from(actor);
            e
        }
        DomainEvent::TicketYeeted { ticket_id, actor } => {
            let mut e = AuditEntry::new(kind, "ticket yeeted");
            e.ticket_id = Some(ticket_id.to_string());
            e.actor = Some(actor.clone());
            e
        }
    };
    entry.detail = Some(json.to_string());
    entry
}
