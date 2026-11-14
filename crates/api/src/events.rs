use nullbox_domain::DomainEvent;

/// Where domain events go: the log and the audit trail (see `FanOutEventSink`).
/// Email, webhooks and metrics are more implementations of this trait.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: DomainEvent);
}

pub struct TracingEventSink;

impl EventSink for TracingEventSink {
    fn emit(&self, event: DomainEvent) {
        let payload = serde_json::to_string(&event).unwrap_or_default();
        tracing::info!(target: "nullbox::events", %payload, "domain event");
    }
}

/// Delivers every event to each sink in turn.
pub struct FanOutEventSink {
    sinks: Vec<std::sync::Arc<dyn EventSink>>,
}

impl FanOutEventSink {
    pub fn new(sinks: Vec<std::sync::Arc<dyn EventSink>>) -> Self {
        Self { sinks }
    }
}

impl EventSink for FanOutEventSink {
    fn emit(&self, event: DomainEvent) {
        for sink in &self.sinks {
            sink.emit(event.clone());
        }
    }
}
