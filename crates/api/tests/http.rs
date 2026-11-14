//! HTTP tests: the whole router against in-memory SQLite, via `oneshot`.
//! Copy a test, change the requests, done.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nullbox_api::{AppState, Config, db, http};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn app() -> Router {
    let pool = db::connect_in_memory().await.expect("in-memory db");
    let config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        database_url: "sqlite::memory:".into(),
        cors_allowed_origins: vec!["http://localhost:5173".into()],
        public_web_url: "http://localhost:5173".into(),
        admin_token: "test-admin-token".into(),
        cookie_secure: false,
    };
    let state = AppState::new(pool, config);
    let hash = nullbox_api::auth::hash_password("test-password".into()).await.unwrap();
    state.accounts.insert("山田".into(), hash).unwrap();
    http::router(state)
}

/// Send a request; returns status and JSON body (`Null` if empty).
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let body = match body {
        Some(json) => {
            req = req.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn login(app: &Router) -> String {
    let request = Request::builder().method("POST").uri("/api/auth/login")
        .header("content-type", "application/json").header("x-nullbox-request", "1")
        .body(Body::from(json!({"username": "山田", "password": "test-password"}).to_string())).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.headers().get("set-cookie").unwrap().to_str().unwrap().split(';').next().unwrap().to_owned()
}

#[tokio::test]
async fn customer_and_worker_round_trip() {
    let app = app().await;

    // Anonymous customer creates a ticket.
    let (status, created) = send(
        &app,
        "POST",
        "/api/tickets",
        &[],
        Some(json!({ "subject": "Printer on fire", "body": "Again.", "severity": "p1" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["ticket"]["id"].as_str().unwrap().to_owned();
    let token = created["access_token"].as_str().unwrap().to_owned();
    assert_eq!(created["ticket"]["number"], 1);
    assert!(created["ticket"]["anon"]["animal"].is_string());
    assert!(created["access_link"].as_str().unwrap().ends_with(&format!("/t/{id}#{token}")));

    // The secret link works; a wrong token looks like a missing ticket.
    let (status, thread) =
        send(&app, "GET", &format!("/api/public/tickets/{id}"), &[("x-ticket-token", &token)], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(thread["ticket"]["subject"], "Printer on fire");
    let (status, error) =
        send(&app, "GET", &format!("/api/public/tickets/{id}"), &[("x-ticket-token", "nope")], None).await;
    assert_eq!((status, &error["code"]), (StatusCode::NOT_FOUND, &json!("not_found")));

    let cookie = login(&app).await;
    let worker = [("cookie", cookie.as_str()), ("x-nullbox-request", "1")];
    // The worker sees it in the inbox, notes internally and yeets it.
    let (_, inbox) = send(&app, "GET", "/api/tickets", &worker, None).await;
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/tickets/{id}/messages"),
        &worker,
        Some(json!({ "body": "it's always DNS", "internal": true })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, yeeted) = send(&app, "POST", &format!("/api/tickets/{id}/yeet"), &worker, None).await;
    assert_eq!((status, &yeeted["status"]), (StatusCode::OK, &json!("resolved")));

    let (_, stats) = send(&app, "GET", "/api/stats/backlog", &worker, None).await;
    assert_eq!(stats, json!({ "open_count": 0, "beast_mood": "asleep", "oldest_open_age_secs": null }));

    // The customer never sees internal notes, and replying reopens the ticket.
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/public/tickets/{id}/messages"),
        &[("x-ticket-token", &token)],
        Some(json!({ "body": "Still on fire" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, thread) =
        send(&app, "GET", &format!("/api/public/tickets/{id}"), &[("x-ticket-token", &token)], None).await;
    assert_eq!(thread["ticket"]["status"], "open");
    let messages = thread["messages"].as_array().unwrap();
    assert!(messages.iter().all(|m| m["internal"] == false));
    assert!(
        messages
            .iter()
            .any(|m| m["system_event"] == json!({ "kind": "status_changed", "from": "resolved", "to": "open" }))
    );

    // The worker view does include the note, written by 山田.
    let (_, thread) = send(&app, "GET", &format!("/api/tickets/{id}"), &worker, None).await;
    let note = thread["messages"].as_array().unwrap().iter().find(|m| m["internal"] == true).unwrap();
    assert_eq!(note["author_name"], "山田");
}

#[tokio::test]
async fn errors_share_one_shape() {
    let app = app().await;

    let (status, error) = send(&app, "GET", "/api/tickets", &[], None).await;
    assert_eq!((status, &error["code"]), (StatusCode::UNAUTHORIZED, &json!("missing_actor")));

    let (status, error) =
        send(&app, "POST", "/api/tickets", &[], Some(json!({ "subject": " ", "body": "x", "severity": "p4" }))).await;
    assert_eq!((status, &error["code"]), (StatusCode::UNPROCESSABLE_ENTITY, &json!("validation_failed")));

    let cookie = login(&app).await;
    let worker = [("cookie", cookie.as_str()), ("x-nullbox-request", "1")];
    let (status, error) = send(&app, "GET", "/api/tickets/not-a-uuid", &worker, None).await;
    assert_eq!((status, &error["code"]), (StatusCode::NOT_FOUND, &json!("not_found")));

    let (status, error) = send(&app, "GET", "/api/teapot", &[], None).await;
    assert_eq!((status, &error["code"]), (StatusCode::IM_A_TEAPOT, &json!("im_a_teapot")));
}

#[tokio::test]
async fn admin_export_needs_the_admin_token() {
    let app = app().await;
    let ticket = json!({ "subject": "Export me", "body": "x", "severity": "p3", "customer_email": "a@example.com" });
    send(&app, "POST", "/api/tickets", &[], Some(ticket)).await;

    let (status, error) = send(&app, "GET", "/api/admin/export", &[], None).await;
    assert_eq!((status, &error["code"]), (StatusCode::FORBIDDEN, &json!("forbidden")));
    let (status, _) = send(&app, "GET", "/api/admin/export", &[("x-admin-token", "nope")], None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let admin = [("x-admin-token", "test-admin-token")];
    let (status, tickets) = send(&app, "GET", "/api/admin/export", &admin, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tickets[0]["customer_email"], "a@example.com");
}

#[tokio::test]
async fn session_and_customer_isolation() {
    let app = app().await;
    for headers in [&[][..], &[("x-actor-name", "fake")][..], &[("cookie", "nullbox_session=made-up")][..]] {
        assert_eq!(send(&app, "GET", "/api/tickets", headers, None).await.0, StatusCode::UNAUTHORIZED);
    }
    let (_, first) = send(&app, "POST", "/api/tickets", &[], Some(json!({"subject":"one","body":"x","severity":"p3"}))).await;
    let (_, second) = send(&app, "POST", "/api/tickets", &[], Some(json!({"subject":"two","body":"x","severity":"p3"}))).await;
    let id = second["ticket"]["id"].as_str().unwrap();
    let token = first["access_token"].as_str().unwrap();
    let headers = [("x-ticket-token", token)];
    assert_eq!(send(&app, "GET", &format!("/api/public/tickets/{id}"), &headers, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(send(&app, "POST", &format!("/api/public/tickets/{id}/messages"), &headers, Some(json!({"body":"x"}))).await.0, StatusCode::NOT_FOUND);
    let cookie = login(&app).await;
    let worker = [("cookie", cookie.as_str()), ("x-nullbox-request", "1")];
    assert_eq!(send(&app, "GET", "/api/tickets", &worker, None).await.0, StatusCode::OK);
    assert_eq!(send(&app, "POST", "/api/auth/logout", &worker, None).await.0, StatusCode::OK);
    assert_eq!(send(&app, "GET", "/api/tickets", &worker, None).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn short_password_is_rejected_before_signup() {
    let app = app().await;
    let (status, body) = send(&app, "POST", "/api/auth/signup", &[("x-nullbox-request", "1")],
        Some(json!({"username": "new-user", "password": "short"}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "password_length");
    assert_eq!(send(&app, "POST", "/api/auth/login", &[("x-nullbox-request", "1")],
        Some(json!({"username": "new-user", "password": "short"}))).await.0, StatusCode::UNAUTHORIZED);
}
