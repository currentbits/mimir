//! `mimir server`: the self-hosted mode. Runs every bot 24/7 and serves the web UI plus
//! `POST /api/<cmd>` (the same commands the desktop app uses), behind an admin token.
//! Put it behind a TLS proxy (Caddy/Traefik) for anything beyond localhost.

use anyhow::{Context, Result};
use axum::extract::{DefaultBodyLimit, Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
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

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).is_some_and(|t| same(t, token))
}

/// Runs before the request body is read, so without the token nothing big is accepted.
async fn require_token(State(app): State<App>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if !authorized(req.headers(), &app.token) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "wrong or missing admin token"}))).into_response();
    }
    next.run(req).await
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
    let host = Arc::new(Host::new(db.clone(), home, Arc::new(DbSecrets(db)), tokio::runtime::Handle::current(), "server"));
    host.restart_all();

    let state = App { host, token: Arc::new(token.clone()) };
    let mut app = Router::new()
        // Photos and files dropped into the chat are sent base64 (up to 20 MB as files); the token is checked first.
        .route(
            "/api/{cmd}",
            post(api).layer(DefaultBodyLimit::max(32 * 1024 * 1024)).route_layer(axum::middleware::from_fn_with_state(state.clone(), require_token)),
        )
        .route("/healthz", get(|| async { "ok" }))
        // Token auth (no cookies), so any origin may call it: the desktop app in remote mode needs this.
        .layer(CorsLayer::new().allow_origin(Any).allow_methods([Method::POST]).allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]))
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
        eprintln!("admin token (shown once; `mimir admin-token` prints it again): {token}");
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
}
