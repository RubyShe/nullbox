//! MCP server at `/mcp`: lets an AI agent work the help desk (read the inbox, look up a
//! ticket, reply, change status). Streamable HTTP transport, JSON responses, no SSE.
//!
//! - Auth: `Authorization: Bearer <MCP_TOKEN>`. No token configured = every request is
//!   refused, so the door is closed by default. Checked before the body is parsed.
//! - Protocol: `initialize` negotiates a version and hands out an `Mcp-Session-Id`.
//!   Sessions are not required afterwards, so stateless clients (2026-07-28) work too.
//! - Tools call `TicketService` through `policy::ensure`, exactly like the worker routes,
//!   acting as the worker named by `MCP_ACTOR_NAME`.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use nullbox_domain::{Severity, TicketStatus};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::extract::Actor;
use crate::dto::UpdateTicketRequest;
use crate::policy::{self, Action, Resource};
use crate::{ApiError, AppState};

/// Newest first. An unknown requested version gets the first entry.
const SUPPORTED_VERSIONS: [&str; 4] = ["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26"];
const SESSION_HEADER: &str = "mcp-session-id";

// JSON-RPC error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

#[derive(Debug, Deserialize)]
struct RpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: Option<String>,
    #[serde(default)]
    params: Value,
}

/// `POST /mcp`: one JSON-RPC message per request.
pub async fn handle(State(state): State<AppState>, headers: HeaderMap, body: String) -> Response {
    if !authorized(&state, &headers) {
        tracing::warn!(target: "mcp", "refused MCP request without a valid bearer token");
        return unauthorized();
    }

    let request: RpcRequest = match serde_json::from_str(&body) {
        Ok(request) => request,
        Err(e) => return rpc_error(Value::Null, PARSE_ERROR, &format!("parse error: {e}")).into_response(),
    };
    let (Some("2.0"), Some(method)) = (request.jsonrpc.as_deref(), request.method.as_deref()) else {
        return rpc_error(request.id.unwrap_or(Value::Null), INVALID_REQUEST, "expected a JSON-RPC 2.0 request")
            .into_response();
    };

    // Notifications (no id) get no body, just 202.
    let Some(id) = request.id else {
        tracing::debug!(target: "mcp", %method, "notification");
        return StatusCode::ACCEPTED.into_response();
    };

    tracing::info!(target: "mcp", %method, "MCP request");
    match method {
        "initialize" => initialize(id, &request.params),
        "ping" => rpc_result(id, json!({})).into_response(),
        "tools/list" => rpc_result(id, json!({ "tools": tools() })).into_response(),
        "tools/call" => call_tool(&state, id, &request.params).await,
        other => rpc_error(id, METHOD_NOT_FOUND, &format!("method not found: {other}")).into_response(),
    }
}

/// `GET /mcp` would open an SSE stream; this server does not offer one.
pub async fn no_stream(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST, DELETE")]).into_response()
}

/// `DELETE /mcp`: the client ends its session. Nothing is stored per session, so just acknowledge.
pub async fn end_session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    StatusCode::NO_CONTENT.into_response()
}

fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(expected) = state.config.mcp_token.as_deref() else { return false };
    let Some(given) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
    else {
        return false;
    };
    // Compare fixed-length digests so the comparison time does not leak the token.
    Sha256::digest(given.trim().as_bytes()) == Sha256::digest(expected.as_bytes())
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, HeaderValue::from_static(r#"Bearer realm="nullbox-mcp""#))],
        Json(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32001, "message": "unauthorized" } })),
    )
        .into_response()
}

fn initialize(id: Value, params: &Value) -> Response {
    let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
    let version = SUPPORTED_VERSIONS.iter().find(|v| **v == requested).copied().unwrap_or(SUPPORTED_VERSIONS[0]);
    let session = Uuid::now_v7().to_string();
    tracing::info!(target: "mcp", protocol_version = %version, session = %session, "MCP session started");
    let result = json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "nullbox", "title": "Nullbox help desk", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Nullbox is a support help desk. Use list_tickets to see the inbox, get_ticket to read a \
                         ticket with its messages, reply_to_ticket to answer the customer (or add an internal note), \
                         and update_ticket to change status, severity or assignee. Keep the Backlog Beast asleep.",
    });
    let mut response = rpc_result(id, result).into_response();
    if let Ok(value) = HeaderValue::from_str(&session) {
        response.headers_mut().insert(SESSION_HEADER, value);
    }
    response
}

fn tools() -> Value {
    let ticket_id = json!({ "type": "string", "format": "uuid", "description": "Ticket id (UUID) from list_tickets." });
    let status =
        json!({ "type": "string", "enum": ["open", "in_progress", "waiting_on_customer", "resolved", "closed"] });
    json!([
        {
            "name": "list_tickets",
            "title": "List tickets",
            "description": "List tickets in the worker inbox. Without a status, all active tickets (open, in progress, waiting on customer).",
            "inputSchema": { "type": "object", "properties": { "status": status }, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "get_ticket",
            "title": "Get ticket",
            "description": "Read one ticket with all its messages, including internal notes.",
            "inputSchema": { "type": "object", "properties": { "ticket_id": ticket_id }, "required": ["ticket_id"], "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "reply_to_ticket",
            "title": "Reply to ticket",
            "description": "Send a reply to the customer, or add an internal note that the customer never sees.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ticket_id": ticket_id,
                    "body": { "type": "string", "minLength": 1, "description": "Message text." },
                    "internal": { "type": "boolean", "default": false, "description": "true = internal note, hidden from the customer." }
                },
                "required": ["ticket_id", "body"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false }
        },
        {
            "name": "update_ticket",
            "title": "Update ticket",
            "description": "Change a ticket's status, severity or assignee. An empty assignee_name unassigns.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ticket_id": ticket_id,
                    "status": status,
                    "severity": { "type": "string", "enum": ["p1", "p2", "p3", "p4"] },
                    "assignee_name": { "type": "string" }
                },
                "required": ["ticket_id"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false }
        },
        {
            "name": "get_backlog_stats",
            "title": "Backlog stats",
            "description": "How many tickets are open, the age of the oldest one, and the Backlog Beast's mood.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        }
    ])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    status: Option<TicketStatus>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TicketArgs {
    ticket_id: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyArgs {
    ticket_id: Uuid,
    body: String,
    #[serde(default)]
    internal: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateArgs {
    ticket_id: Uuid,
    status: Option<TicketStatus>,
    severity: Option<Severity>,
    assignee_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

async fn call_tool(state: &AppState, id: Value, params: &Value) -> Response {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return rpc_error(id, INVALID_PARAMS, "tools/call needs a tool name").into_response();
    };
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let actor = Actor { name: state.config.mcp_actor_name.clone() };
    tracing::info!(target: "mcp", tool = %name, actor = %actor.name, "MCP tool call");

    let outcome = match name {
        "list_tickets" => {
            run(args, |a: ListArgs| async move {
                policy::ensure(&actor, Action::ListTickets, Resource::Inbox)?;
                to_value(state.tickets.list(a.status).await?)
            })
            .await
        }
        "get_ticket" => {
            run(args, |a: TicketArgs| async move {
                policy::ensure(&actor, Action::ViewTicket, Resource::Ticket(a.ticket_id))?;
                to_value(state.tickets.worker_thread(a.ticket_id).await?)
            })
            .await
        }
        "reply_to_ticket" => {
            run(args, |a: ReplyArgs| async move {
                let action = if a.internal { Action::AddInternalNote } else { Action::ReplyToTicket };
                policy::ensure(&actor, action, Resource::Ticket(a.ticket_id))?;
                to_value(state.tickets.worker_message(&actor.name, a.ticket_id, &a.body, a.internal).await?)
            })
            .await
        }
        "update_ticket" => {
            run(args, |a: UpdateArgs| async move {
                policy::ensure(&actor, Action::UpdateTicket, Resource::Ticket(a.ticket_id))?;
                let req =
                    UpdateTicketRequest { status: a.status, severity: a.severity, assignee_name: a.assignee_name };
                to_value(state.tickets.update(&actor.name, a.ticket_id, req).await?)
            })
            .await
        }
        "get_backlog_stats" => {
            run(args, |_: NoArgs| async move {
                policy::ensure(&actor, Action::ViewBacklogStats, Resource::Inbox)?;
                to_value(state.tickets.backlog_stats().await?)
            })
            .await
        }
        other => return rpc_error(id, INVALID_PARAMS, &format!("unknown tool: {other}")).into_response(),
    };

    // Tool failures are results with isError, so the agent can read them and adapt.
    let result = match outcome {
        Ok(value) => json!({
            "content": [{ "type": "text", "text": value.to_string() }],
            "structuredContent": if value.is_object() { value.clone() } else { json!({ "items": value }) },
            "isError": false
        }),
        Err(message) => json!({ "content": [{ "type": "text", "text": message }], "isError": true }),
    };
    rpc_result(id, result).into_response()
}

/// Parse the arguments, run the tool, and turn any failure into a readable message.
async fn run<A, F, Fut>(args: Value, tool: F) -> Result<Value, String>
where
    A: for<'de> Deserialize<'de>,
    F: FnOnce(A) -> Fut,
    Fut: Future<Output = Result<Value, ApiError>>,
{
    let parsed: A = serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
    tool(parsed).await.map_err(|e| match e {
        ApiError::WorksOnMyMachine(_) | ApiError::ProbablyDns(_) => {
            tracing::error!(target: "mcp", error = %e, "MCP tool failed");
            "internal error".to_owned()
        }
        other => other.to_string(),
    })
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, ApiError> {
    serde_json::to_value(value).map_err(|e| ApiError::WorksOnMyMachine(e.to_string()))
}

fn rpc_result(id: Value, result: Value) -> Json<Value> {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn rpc_error(id: Value, code: i64, message: &str) -> Json<Value> {
    Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }))
}
