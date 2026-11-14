//! Payments: a Stripe Checkout for "Nothing to See Here". Stripe hosts the card form,
//! so a card number never reaches this server. Keys come from `Config` (`STRIPE_*`).

use axum::Json;
use axum::extract::{Path, State};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Serialize;
use serde_json::Value;

use crate::{ApiError, AppState};

const STRIPE_API: &str = "https://api.stripe.com/v1";

#[derive(Debug, Serialize)]
pub struct CheckoutCreated {
    pub id: String,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct CheckoutStatus {
    pub id: String,
    pub status: String,
    pub payment_status: String,
}

/// Opens a Checkout Session and returns Stripe's hosted page URL. After paying,
/// Stripe sends the buyer back to `/checkout/success` on our own site.
pub async fn create_checkout(State(state): State<AppState>) -> Result<Json<CheckoutCreated>, ApiError> {
    let config = &state.config;
    let key = config.stripe_api_key.clone().ok_or(ApiError::NotFound)?;
    let web = &config.public_web_url;
    let form = encode_form(&[
        ("mode", config.stripe_checkout_mode.as_str()),
        ("line_items[0][price]", config.stripe_price_id.as_str()),
        ("line_items[0][quantity]", "1"),
        ("success_url", &format!("{web}/checkout/success?session_id={{CHECKOUT_SESSION_ID}}")),
        ("cancel_url", &format!("{web}/checkout/cancel")),
    ]);
    let session = stripe(key, reqwest::Method::POST, "/checkout/sessions".to_owned(), Some(form)).await?;
    let id = field(&session, "id")?;
    let url = field(&session, "url")?;
    tracing::info!(target: "payments", checkout_session = %id, "checkout session created");
    Ok(Json(CheckoutCreated { id, url }))
}

/// Lets the success page confirm the payment with Stripe instead of trusting the URL.
pub async fn get_checkout(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CheckoutStatus>, ApiError> {
    if !id.starts_with("cs_") || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ApiError::Validation("not a checkout session id".into()));
    }
    let key = state.config.stripe_api_key.clone().ok_or(ApiError::NotFound)?;
    let session = stripe(key, reqwest::Method::GET, format!("/checkout/sessions/{id}"), None).await?;
    let status =
        CheckoutStatus { id, status: field(&session, "status")?, payment_status: field(&session, "payment_status")? };
    tracing::info!(target: "payments", checkout_session = %status.id, payment_status = %status.payment_status, "checkout session checked");
    Ok(Json(status))
}

fn encode_form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| {
            format!("{}={}", utf8_percent_encode(k, NON_ALPHANUMERIC), utf8_percent_encode(v, NON_ALPHANUMERIC))
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn field(value: &Value, name: &str) -> Result<String, ApiError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ApiError::WorksOnMyMachine(format!("stripe response without `{name}`")))
}

/// One call to the Stripe REST API. The blocking client runs off the async runtime.
async fn stripe(key: String, method: reqwest::Method, path: String, form: Option<String>) -> Result<Value, ApiError> {
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let mut request =
            reqwest::blocking::Client::new().request(method, format!("{STRIPE_API}{path}")).bearer_auth(key);
        if let Some(form) = form {
            request = request.header("Content-Type", "application/x-www-form-urlencoded").body(form);
        }
        let response = request.send().map_err(|e| e.to_string())?;
        let status = response.status();
        let body: Value =
            serde_json::from_str(&response.text().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("stripe {status}: {}", body["error"]["message"].as_str().unwrap_or("unknown error")));
        }
        Ok(body)
    })
    .await
    .map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))?;
    result.map_err(|e| {
        tracing::error!(target: "payments", error = %e, "stripe call failed");
        ApiError::WorksOnMyMachine(e)
    })
}
