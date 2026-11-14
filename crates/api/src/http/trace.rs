//! Request spans shaped like OpenTelemetry HTTP server spans, so Elastic APM shows
//! each request as a transaction named after its route.

use std::time::Duration;

use axum::body::Body;
use axum::extract::MatchedPath;
use axum::http::{HeaderMap, Request, Response};
use opentelemetry::global;
use opentelemetry::propagation::Extractor;
use tower_http::trace::DefaultOnResponse;
use tracing::{Span, field};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Opens the server span and, when the caller sent a W3C `traceparent`, joins its trace.
pub fn make_span(request: &Request<Body>) -> Span {
    let method = request.method().as_str();
    // Route template, not the raw path: `{id}` stays low-cardinality.
    let route = request.extensions().get::<MatchedPath>().map(MatchedPath::as_str);
    let span = tracing::info_span!(
        "http.request",
        otel.name = %route.map_or_else(|| method.to_owned(), |route| format!("{method} {route}")),
        otel.kind = "server",
        otel.status_code = field::Empty,
        http.request.method = method,
        http.route = route,
        http.response.status_code = field::Empty,
    );
    let parent = global::get_text_map_propagator(|propagator| propagator.extract(&HeaderExtractor(request.headers())));
    // Only fails when OpenTelemetry is not installed, which just means there is nothing to join.
    let _ = span.set_parent(parent);
    span
}

/// Records the status on the span, then logs like tower-http's default.
#[derive(Clone, Default)]
pub struct OnResponse(DefaultOnResponse);

impl tower_http::trace::OnResponse<Body> for OnResponse {
    fn on_response(self, response: &Response<Body>, latency: Duration, span: &Span) {
        let status = response.status();
        span.record("http.response.status_code", status.as_u16());
        if status.is_server_error() {
            span.record("otel.status_code", "ERROR");
        }
        self.0.on_response(response, latency, span);
    }
}

struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|name| name.as_str()).collect()
    }
}
