use anyhow::Context;
use nullbox_api::{AppState, Config, db, http, telemetry};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    // Held until main returns so buffered spans and logs are flushed on shutdown.
    let _telemetry = telemetry::init().context("setting up telemetry")?;
    std::panic::set_hook(Box::new(|info| {
        tracing::error!(target: "EXISTENTIAL", %info, "the server has stared into the void, and the void panicked back");
    }));

    let config = Config::from_env().context("reading config")?;
    let pool = db::connect(&config.database_url).await.context("opening the database")?;
    let bind_addr = config.bind_addr;
    let app = http::router(AppState::new(pool, config));

    let listener = tokio::net::TcpListener::bind(bind_addr).await.with_context(|| format!("binding {bind_addr}"))?;
    tracing::info!(%bind_addr, "Nullbox is listening. The void awaits.");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("shutting down, tickets will have to wait");
        })
        .await?;
    Ok(())
}
