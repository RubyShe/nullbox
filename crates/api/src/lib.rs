//! Nullbox API: the HTTP layer, services and storage around `nullbox-domain`.
//!
//! Extension seams, each in one place:
//! - auth: [`http::extract::Actor`]
//! - authorization: [`policy::can`]
//! - audit: [`audit::AuditLog`] (domain events via [`audit::AuditEventSink`], requests via `http::audit`)
//! - webhooks/metrics: [`events::EventSink`]
//! - middleware (tracing, CORS, rate limits...): [`http::middleware`]
//! - storage: [`repo::TicketRepository`]
//! - settings: [`config::Config`]

pub mod audit;
pub mod config;
pub mod db;
pub mod dto;
pub mod error;
pub mod events;
pub mod http;
pub mod policy;
pub mod repo;
pub mod service;
pub mod telemetry;
pub mod token;

use std::sync::Arc;

pub use config::Config;
pub use error::ApiError;

/// Shared state for all handlers.
#[derive(Clone)]
pub struct AppState {
    pub tickets: Arc<service::TicketService>,
    pub audit: Arc<dyn audit::AuditLog>,
    pub config: Arc<Config>,
}

impl AppState {
    pub fn new(pool: sqlx::SqlitePool, config: Config) -> Self {
        let audit: Arc<dyn audit::AuditLog> = Arc::new(audit::SqliteAuditLog::new(pool.clone()));
        let repo = Arc::new(repo::SqliteTicketRepository::new(pool));
        let events = Arc::new(events::FanOutEventSink::new(vec![
            Arc::new(events::TracingEventSink),
            Arc::new(audit::AuditEventSink::new(audit.clone())),
        ]));
        Self { tickets: Arc::new(service::TicketService::new(repo, events)), audit, config: Arc::new(config) }
    }
}
