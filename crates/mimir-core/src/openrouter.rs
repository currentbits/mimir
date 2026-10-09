//! "Connect OpenRouter": OAuth PKCE with a one-shot localhost callback, so nobody copies keys around.
//! `start` listens on a random local port and returns the sign-in URL; the browser comes back to
//! http://localhost:<port>/?code=…, and `finish` trades that code for the user's key.

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(600);

/// The sign-in in progress (one at a time; starting again replaces it).
static PENDING: Mutex<Option<JoinHandle<Result<String>>>> = Mutex::new(None);

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Begin a sign-in; returns the URL to open in the browser.
pub async fn start() -> Result<String> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    let verifier = URL_SAFE_NO_PAD.encode(raw);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    // A secret path: only the browser coming back from OpenRouter knows it, so another program or a web page that
    // finds the port can't end the sign-in or slip in its own code.
    let secret = crate::random_hex(16)?;
    let url = format!(
        "https://openrouter.ai/auth?callback_url=http%3A%2F%2Flocalhost%3A{port}%2Fcb%2F{secret}&code_challenge={}&code_challenge_method=S256&key_label=Mimir",
        challenge(&verifier)
    );
    let task = tokio::spawn(async move { tokio::time::timeout(SIGN_IN_TIMEOUT, callback(listener, &verifier, &secret)).await.context("timed out waiting for OpenRouter")? });
    if let Some(old) = CANCEL.lock().unwrap().replace(task.abort_handle()) {
        old.abort();
    }
    if let Some(old) = PENDING.lock().unwrap().replace(task) {
        old.abort();
    }
    Ok(url)
}

/// The sign-in's abort handle, kept here because `finish` takes the task itself while it waits.
static CANCEL: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);

/// Stop a sign-in that's still waiting (Cancel): its callback port closes and `finish` returns "cancelled".
pub fn cancel() {
    if let Some(h) = CANCEL.lock().unwrap().take() {
        h.abort();
    }
    if let Some(t) = PENDING.lock().unwrap().take() {
        t.abort();
    }
}

/// Wait for the sign-in `start` began; returns the new API key.
pub async fn finish() -> Result<String> {
    let task = PENDING.lock().unwrap().take().context("no OpenRouter sign-in in progress")?;
    task.await.context("the sign-in was cancelled")?
}

/// Serve the browser's return trip: take the code, exchange it, show a page saying how it went.
async fn callback(listener: TcpListener, verifier: &str, secret: &str) -> Result<String> {
    let expected = format!("/cb/{secret}");
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        // A connection that sends nothing can't hold the port up.
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), sock.read(&mut buf)).await.ok().and_then(Result::ok).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        // "GET /cb/<secret>?code=… HTTP/1.1". Anything else (favicon, a wrong path) gets a 404 and the wait goes on.
        // ponytail: no percent-decoding; OpenRouter codes are URL-safe
        let path = req.split_whitespace().nth(1).unwrap_or("");
        let Some(code) = path.split_once('?').filter(|(p, _)| *p == expected).and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("code="))) else {
            respond(&mut sock, "404 Not Found", "").await;
            continue;
        };
        let result = exchange(code, verifier).await;
        let page = match &result {
            Ok(_) => page("Connected to OpenRouter", "Your bots can use it now. You can close this tab and go back to Mimir."),
            Err(e) => page("Couldn't connect to OpenRouter", &format!("{e:#}. Go back to Mimir and try again.")),
        };
        respond(&mut sock, "200 OK", &page).await;
        return result;
    }
}

async fn exchange(code: &str, verifier: &str) -> Result<String> {
    let v: Value = reqwest::Client::new()
        .post("https://openrouter.ai/api/v1/auth/keys")
        .json(&json!({"code": code, "code_verifier": verifier, "code_challenge_method": "S256"}))
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .json()
        .await
        .context("OpenRouter sent something unexpected")?;
    v["key"].as_str().map(String::from).with_context(|| format!("OpenRouter didn't hand over a key ({})", v["error"]["message"].as_str().unwrap_or("no reason given")))
}

async fn respond(sock: &mut TcpStream, status: &str, html: &str) {
    let msg = format!("HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{html}", html.len());
    let _ = sock.write_all(msg.as_bytes()).await;
}

fn page(title: &str, body: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;");
    format!(
        "<!doctype html><meta name=viewport content='width=device-width'><title>{t}</title>\
         <body style='font:15px -apple-system,system-ui,sans-serif;background:#f7f6f3;color:#1d1c1a;display:grid;place-items:center;height:90vh;margin:0'>\
         <div style='max-width:420px;padding:32px'><h1 style='font:600 26px ui-serif,Georgia,serif;margin:0 0 8px'>{t}</h1><p style='color:#5f5b55;line-height:1.5;margin:0'>{b}</p></div>",
        t = esc(title),
        b = esc(body)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_s256() {
        // RFC 7636, appendix B.
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    /// Cancel while the app waits: the wait ends right away and the callback port stops listening.
    #[tokio::test]
    async fn cancel_stops_a_waiting_sign_in() {
        let url = start().await.unwrap();
        let port = url.split("localhost%3A").nth(1).unwrap().split('%').next().unwrap().to_string();
        let waiting = tokio::spawn(finish());
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel();
        let err = tokio::time::timeout(Duration::from_secs(2), waiting).await.expect("finish returned").unwrap().unwrap_err();
        assert!(format!("{err:#}").contains("cancelled"), "{err:#}");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await.is_err(), "port closed");
    }

    /// Live (network): OpenRouter accepts our sign-in URL; the callback ignores strays and turns a
    /// bad code into a readable failure, in the browser and for the app.
    #[tokio::test]
    #[ignore]
    async fn live_sign_in_with_bad_code() {
        let url = start().await.unwrap();
        let auth = reqwest::get(&url).await.unwrap();
        assert!(auth.status().is_success(), "{}", auth.status());
        let port = url.split("localhost%3A").nth(1).unwrap().split('%').next().unwrap().to_string();
        let stray = reqwest::get(format!("http://127.0.0.1:{port}/favicon.ico")).await.unwrap();
        assert_eq!(stray.status(), 404);
        let html = reqwest::get(format!("http://127.0.0.1:{port}/?code=not-a-real-code")).await.unwrap().text().await.unwrap();
        assert!(html.contains("Couldn't connect to OpenRouter"), "{html}");
        let err = format!("{:#}", finish().await.unwrap_err());
        assert!(err.contains("OpenRouter"), "{err}");
        println!("browser page says: {}", html.split("<p").nth(1).unwrap_or(""));
        println!("app error: {err}");
    }
}
