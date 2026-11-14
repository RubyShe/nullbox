//! Logging and OpenTelemetry setup.
//!
//! Export is off unless `OTEL_EXPORTER_OTLP_ENDPOINT` is set, in which case spans and log
//! events go out over OTLP/HTTP (protobuf). The endpoint, the `Authorization` header and
//! the rest are standard `OTEL_*` variables read by the exporters themselves, so no
//! credential passes through our code or `Config`. See `.env.example`.

use anyhow::Context;
use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, SpanExporter};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

const DEFAULT_SERVICE_NAME: &str = "nullbox-api";
const ENDPOINT_VAR: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// Targets that must not feed back into the exporter: they emit events while the
/// exporter itself is sending a batch.
const EXPORTER_TARGETS: [&str; 5] = ["hyper", "hyper_util", "h2", "reqwest", "opentelemetry"];

/// Flushes and shuts down the exporters on drop. Keep it alive until `main` returns.
#[must_use = "dropping the guard immediately shuts telemetry down"]
pub struct TelemetryGuard {
    tracer_provider: Option<SdkTracerProvider>,
    logger_provider: Option<SdkLoggerProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        // The subscriber may already be gone, so report on stderr.
        if let Some(provider) = self.tracer_provider.take()
            && let Err(error) = provider.shutdown()
        {
            eprintln!("failed to flush OpenTelemetry traces: {error}");
        }
        if let Some(provider) = self.logger_provider.take()
            && let Err(error) = provider.shutdown()
        {
            eprintln!("failed to flush OpenTelemetry logs: {error}");
        }
    }
}

/// Installs the global tracing subscriber. Call once, after `.env` is loaded.
pub fn init() -> anyhow::Result<TelemetryGuard> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=debug".into());
    let fmt = tracing_subscriber::fmt::layer();

    if !otlp_enabled() {
        tracing_subscriber::registry().with(env_filter).with(fmt).init();
        return Ok(TelemetryGuard { tracer_provider: None, logger_provider: None });
    }

    let resource = resource();

    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(SpanExporter::builder().with_http().build().context("building the OTLP span exporter")?)
        .build();
    let logger_provider = SdkLoggerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(LogExporter::builder().with_http().build().context("building the OTLP log exporter")?)
        .build();

    global::set_text_map_propagator(TraceContextPropagator::new());
    global::set_tracer_provider(tracer_provider.clone());

    let not_exporter = || filter_fn(|meta| !EXPORTER_TARGETS.iter().any(|target| meta.target().starts_with(target)));
    let traces = tracing_opentelemetry::layer()
        .with_tracer(tracer_provider.tracer(DEFAULT_SERVICE_NAME))
        .with_filter(not_exporter());
    let logs = OpenTelemetryTracingBridge::new(&logger_provider).with_filter(not_exporter());

    tracing_subscriber::registry().with(env_filter).with(fmt).with(traces).with(logs).init();

    // Log the destination only: the headers carry the API key.
    tracing::info!(endpoint = std::env::var(ENDPOINT_VAR).unwrap_or_default(), "exporting traces and logs over OTLP");
    Ok(TelemetryGuard { tracer_provider: Some(tracer_provider), logger_provider: Some(logger_provider) })
}

fn otlp_enabled() -> bool {
    let has_endpoint = std::env::var(ENDPOINT_VAR).is_ok_and(|value| !value.trim().is_empty());
    let sdk_disabled = std::env::var("OTEL_SDK_DISABLED").is_ok_and(|value| value.eq_ignore_ascii_case("true"));
    has_endpoint && !sdk_disabled
}

/// `OTEL_SERVICE_NAME` and `OTEL_RESOURCE_ATTRIBUTES` are honoured by the builder;
/// the service name falls back to `nullbox-api` when `OTEL_SERVICE_NAME` is unset.
fn resource() -> Resource {
    let builder = Resource::builder();
    if std::env::var_os("OTEL_SERVICE_NAME").is_some() {
        builder.build()
    } else {
        builder.with_service_name(DEFAULT_SERVICE_NAME).build()
    }
}
