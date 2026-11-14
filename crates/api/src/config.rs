use std::net::SocketAddr;

/// Every setting, read from the environment (plus `.env` in dev). See `.env.example`.
#[derive(Debug, Clone)]
pub struct Config {
    /// `BIND_ADDR`, e.g. `0.0.0.0:3000` in a container.
    pub bind_addr: SocketAddr,
    /// `DATABASE_URL`, e.g. `sqlite://nullbox.db`.
    pub database_url: String,
    /// `CORS_ALLOWED_ORIGINS`, comma-separated. The web app runs on another origin (Vite, Tauri).
    pub cors_allowed_origins: Vec<String>,
    /// `PUBLIC_WEB_URL`: where the web app lives; used to build secret access links.
    pub public_web_url: String,
    /// `ADMIN_TOKEN`: unlocks `GET /api/admin/export`, the CEO's spreadsheet. Empty disables the export.
    pub admin_token: String,
    /// `STRIPE_API_KEY`: secret or restricted key. Unset = payments are switched off.
    pub stripe_api_key: Option<String>,
    /// `STRIPE_PRICE_ID`: the price of "Nothing to See Here".
    pub stripe_price_id: String,
    /// `STRIPE_CHECKOUT_MODE`: `subscription` for a recurring price, `payment` for a one-time price.
    pub stripe_checkout_mode: String,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid config value for {name}: {reason}")]
pub struct ConfigError {
    pub name: &'static str,
    pub reason: String,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let var = |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.to_owned());

        let bind_addr = var("BIND_ADDR", "127.0.0.1:3000")
            .parse()
            .map_err(|e: std::net::AddrParseError| ConfigError { name: "BIND_ADDR", reason: e.to_string() })?;

        Ok(Self {
            bind_addr,
            database_url: var("DATABASE_URL", "sqlite://nullbox.db"),
            cors_allowed_origins: var("CORS_ALLOWED_ORIGINS", "http://localhost:5173")
                .split(',')
                .map(|origin| origin.trim().to_owned())
                .filter(|origin| !origin.is_empty())
                .collect(),
            public_web_url: var("PUBLIC_WEB_URL", "http://localhost:5173").trim_end_matches('/').to_owned(),
            // No default: a built-in secret is a leaked secret. Empty keeps the export locked.
            admin_token: var("ADMIN_TOKEN", "").trim().to_owned(),
            stripe_api_key: std::env::var("STRIPE_API_KEY").ok().filter(|key| !key.trim().is_empty()),
            stripe_price_id: var("STRIPE_PRICE_ID", "price_1UWURXE6nIS5HOwPHs0C8GYV"),
            stripe_checkout_mode: var("STRIPE_CHECKOUT_MODE", "subscription"),
        })
    }
}
