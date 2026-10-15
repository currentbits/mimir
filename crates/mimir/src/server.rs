//! `mimir server`: the self-hosted mode. Runs every bot 24/7 and serves the web UI plus
//! `POST /api/<cmd>` (the same commands the desktop app uses), behind an admin token, and an OpenAI-compatible
//! `/v1/chat/completions` whose tokens each reach one bot.
//! Put it behind a TLS proxy (Caddy/Traefik) for anything beyond localhost.

use anyhow::{Context, Result};
use axum::extract::{DefaultBodyLimit, Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use mimir_core::db::Db;
use mimir_core::host::{DbSecrets, Host};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct App {
    host: Arc<Host>,
    token: Arc<String>,
}

/// MIMIR_ADMIN_TOKEN, else one generated on first run and kept in the database.
pub fn admin_token(db: &Db) -> Result<(String, bool)> {
    if let Ok(t) = std::env::var("MIMIR_ADMIN_TOKEN")
        && !t.trim().is_empty()
    {
        // It's the only lock on the server: a short one can be guessed.
        if t.trim().len() < 24 {
            anyhow::bail!("MIMIR_ADMIN_TOKEN is too short: use at least 24 characters (for example `openssl rand -hex 24`)");
        }
        return Ok((t.trim().to_string(), false));
    }
    if let Some(t) = db.setting("admin_token")? {
        return Ok((t, false));
    }
    let t = mimir_core::random_hex(24)?;
    db.set_setting("admin_token", &t)?;
    Ok((t, true))
}

/// Constant-time so the token can't be guessed byte by byte from response timing.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "))
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    bearer(headers).is_some_and(|t| same(t, token))
}

/// Runs before the request body is read, so without the token nothing big is accepted.
async fn require_token(State(app): State<App>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if !authorized(req.headers(), &app.token) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "wrong or missing admin token"}))).into_response();
    }
    next.run(req).await
}

/// The bot a chat API token reaches (set by `require_chat_token`).
#[derive(Clone, Copy)]
struct ChatBot(i64);

/// Errors the way OpenAI's API words them, so clients show the message.
fn oai_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": {"message": message, "type": "invalid_request_error"}}))).into_response()
}

/// The chat API takes its own tokens, each for one bot; the admin token isn't one of them. Checked before the body
/// is read, like the admin API.
async fn require_chat_token(State(app): State<App>, mut req: Request, next: Next) -> Response {
    match bearer(req.headers()).and_then(|t| mimir_core::host::chat_token_bot(&app.host.db, t)) {
        Some(bot) => {
            req.extensions_mut().insert(ChatBot(bot));
            next.run(req).await
        }
        None => oai_error(StatusCode::UNAUTHORIZED, "wrong or missing token: make one under Settings, Chat API"),
    }
}

/// The text of the last user message: a string, or the text parts of a list of parts.
fn last_user_text(req: &Value) -> String {
    let Some(m) = req["messages"].as_array().and_then(|m| m.iter().rev().find(|m| m["role"] == "user")) else { return String::new() };
    match &m["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// OpenAI-compatible chat for Shortcuts, Home Assistant and the like: one bot, its built-in chat. Mimir keeps the
/// conversation itself, so only the last user message is the turn; the rest of `messages` is ignored.
async fn chat_completions(State(app): State<App>, Extension(ChatBot(bot)): Extension<ChatBot>, body: axum::body::Bytes) -> Response {
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return oai_error(StatusCode::BAD_REQUEST, &format!("bad request body: {e}")),
    };
    let text = last_user_text(&req);
    if text.trim().is_empty() {
        return oai_error(StatusCode::BAD_REQUEST, "no user message to answer");
    }
    let reply = match app.host.call("send_local", &json!({"bot_id": bot, "text": text})).await {
        Ok(Value::String(s)) => s,
        Ok(v) => v.to_string(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"message": format!("{e:#}"), "type": "server_error"}}))).into_response(),
    };
    let model = app.host.db.bot(bot).map(|b| b.name).unwrap_or_default();
    let (id, created) = (format!("chatcmpl-{}", mimir_core::random_hex(12).unwrap_or_default()), mimir_core::db::now());
    // ponytail: the whole reply as one chunk once it's done; real token streaming if a client needs it
    if req["stream"] == true {
        let chunk = |delta: Value, finish: Value| {
            format!("data: {}\n\n", json!({"id": id, "object": "chat.completion.chunk", "created": created, "model": model, "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}))
        };
        let body = chunk(json!({"role": "assistant", "content": reply}), Value::Null) + &chunk(json!({}), json!("stop")) + "data: [DONE]\n\n";
        return ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response();
    }
    Json(json!({
        "id": id, "object": "chat.completion", "created": created, "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": reply}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0},
    }))
    .into_response()
}

/// The one "model" a token reaches: its bot. Clients ask for this to check the connection.
async fn models(State(app): State<App>, Extension(ChatBot(bot)): Extension<ChatBot>) -> Response {
    let name = app.host.db.bot(bot).map(|b| b.name).unwrap_or_default();
    Json(json!({"object": "list", "data": [{"id": name, "object": "model", "created": 0, "owned_by": "mimir"}]})).into_response()
}

/// A routine's webhook (Routines → When called). No admin token: its own secret, as the bearer token or an HMAC of the
/// body in X-Hub-Signature-256 (how GitHub signs; X-Signature-256 too).
async fn hook(State(app): State<App>, Path(id): Path<i64>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let sig = ["x-hub-signature-256", "x-signature-256"].iter().find_map(|h| headers.get(*h)).and_then(|v| v.to_str().ok());
    match mimir_core::events::webhook(&app.host.db, id, bearer(&headers), sig, &body) {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"ok": true}))).into_response(),
        Err(mimir_core::events::Refused::Paused) => (StatusCode::CONFLICT, Json(json!({"error": "this routine is paused"}))).into_response(),
        Err(mimir_core::events::Refused::Repeat) => (StatusCode::CONFLICT, Json(json!({"error": "this call was already received"}))).into_response(),
        Err(mimir_core::events::Refused::Secret) => (StatusCode::UNAUTHORIZED, Json(json!({"error": "wrong or missing secret"}))).into_response(),
    }
}

async fn api(State(app): State<App>, Path(cmd): Path<String>, body: axum::body::Bytes) -> Response {
    let args: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({"error": format!("bad request body: {e}")}))).into_response(),
        }
    };
    match app.host.call(&cmd, &args).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": format!("{e:#}")}))).into_response(),
    }
}

/// Every response: the web UI can't be framed by another site (clickjacking an Approve button), loads only its own
/// scripts, and browsers don't guess content types.
async fn harden(req: Request, next: Next) -> Response {
    let asset = req.uri().path().starts_with("/assets/");
    let mut r = next.run(req).await;
    // Vite names each built file after its content, so a browser may keep one forever. Not a missing one, which the
    // fallback answers with index.html.
    let keep = asset && r.status().is_success() && r.headers().get(header::CONTENT_TYPE).is_none_or(|t| !t.as_bytes().starts_with(b"text/html"));
    let h = r.headers_mut();
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"),
    );
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    // Anything else, nothing a browser or a proxy in between should keep: index.html updates with the server, and
    // answers are private.
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(if keep { "public, max-age=31536000, immutable" } else { "no-store" }));
    r
}

fn ui_dir() -> Option<PathBuf> {
    let candidates = [std::env::var("MIMIR_UI_DIR").ok(), Some("ui/dist".into()), Some("/app/ui".into())];
    candidates.into_iter().flatten().map(PathBuf::from).find(|d| d.join("index.html").is_file())
}

pub async fn run(db: Db, home: PathBuf, listen: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The database holds memories and channel tokens: owner-only.
        let _ = std::fs::set_permissions(home.join("mimir.db"), std::fs::Permissions::from_mode(0o600));
    }
    let (token, fresh) = admin_token(&db)?;
    // For `mimir doctor`: whether it answers beyond this machine.
    db.set_setting("server_listen", listen)?;
    let host = Arc::new(Host::new(db.clone(), home, Arc::new(DbSecrets(db)), tokio::runtime::Handle::current(), "server"));
    host.restart_all();

    let state = App { host, token: Arc::new(token.clone()) };
    let mut app = Router::new()
        // Photos and files dropped into the chat are sent base64 (up to 20 MB as files); the token is checked first.
        .route(
            "/api/{cmd}",
            post(api).layer(DefaultBodyLimit::max(32 * 1024 * 1024)).route_layer(axum::middleware::from_fn_with_state(state.clone(), require_token)),
        )
        .route("/v1/chat/completions", post(chat_completions).route_layer(axum::middleware::from_fn_with_state(state.clone(), require_chat_token)))
        .route("/v1/models", get(models).route_layer(axum::middleware::from_fn_with_state(state.clone(), require_chat_token)))
        .route("/hook/{id}", post(hook).layer(DefaultBodyLimit::max(256 * 1024)))
        .route("/healthz", get(|| async { "ok" }))
        // Token auth (no cookies), so any origin may call it: the desktop app in remote mode needs this.
        .layer(CorsLayer::new().allow_origin(Any).allow_methods([Method::GET, Method::POST]).allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]))
        .with_state(state);
    match ui_dir() {
        Some(dir) => {
            eprintln!("serving web UI from {}", dir.display());
            app = app.fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html"))));
        }
        None => eprintln!("web UI not found (set MIMIR_UI_DIR to ui/dist); serving the API only"),
    }

    let app = app.layer(CompressionLayer::new()).layer(axum::middleware::from_fn(harden));
    let listener = tokio::net::TcpListener::bind(listen).await.with_context(|| format!("can't listen on {listen}"))?;
    eprintln!("mimir server on http://{listen}");
    if fresh {
        // Not the token itself: logs are kept and shipped where the token shouldn't go.
        eprintln!("made an admin token: run `mimir admin-token` to see it");
    }
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_check() {
        let mut h = HeaderMap::new();
        assert!(!authorized(&h, "abc"));
        h.insert(header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        assert!(authorized(&h, "abc"));
        assert!(!authorized(&h, "abd"));
        assert!(!authorized(&h, "abcd"));
        h.insert(header::AUTHORIZATION, "abc".parse().unwrap());
        assert!(!authorized(&h, "abc"), "Bearer prefix required");
    }

    #[test]
    fn last_user_message() {
        let req = json!({"messages": [{"role": "system", "content": "s"}, {"role": "user", "content": "first"}, {"role": "assistant", "content": "a"},
            {"role": "user", "content": [{"type": "text", "text": "lights"}, {"type": "image_url"}, {"type": "text", "text": "off"}]}]});
        assert_eq!(last_user_text(&req), "lights\noff");
        assert_eq!(last_user_text(&json!({"messages": [{"role": "user", "content": "hi"}]})), "hi");
        assert_eq!(last_user_text(&json!({"messages": [{"role": "system", "content": "s"}]})), "");
        assert_eq!(last_user_text(&json!({})), "");
    }
}
