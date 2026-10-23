//! OAuth 2 sign-in with PKCE (authorization code, a public client), for tool servers (MCP) and Outlook mail.
//! The browser comes back to a one-shot listener on this computer; a server can't receive that (it comes back to the
//! person's own computer), so there they paste the address their browser ended up on, as with ChatGPT.
//! Tokens live in the secret store: the sign-in (client, refresh token) under its key, the access token under
//! `<key>:access`, each in parts when long (see `put`).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(600);
/// Where a server's sign-in says it comes back to: nothing listens there, the person pastes the address instead.
/// Also the one port of an app the person registered themselves (it lists this exact address).
const PASTE_PORT: u16 = 33418;

/// A sign-in as kept: enough to refresh it. `refresh` is empty once it has ended (or the server gave none).
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Saved {
    pub token_url: String,
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Sent again when refreshing (Microsoft wants the scopes; MCP servers the resource).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(default)]
    pub refresh: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Access {
    token: String,
    expires_at: i64,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Most characters in one secret: Windows' credential store holds 2,560 bytes, which is 1,280 characters as it keeps
/// them, and Microsoft's tokens are longer than that.
const PART: usize = 1000;

/// Keep a secret, in parts under `<key>#1`, `<key>#2`… when it's long (`<key>` then says how many). "" deletes.
fn put(key: &str, value: &str) -> Result<()> {
    let old = crate::host::secret_get(key).and_then(|v| v.strip_prefix("parts:").and_then(|n| n.parse::<usize>().ok())).unwrap_or(0);
    let parts: Vec<String> = value.chars().collect::<Vec<_>>().chunks(PART).map(|c| c.iter().collect()).collect();
    if parts.len() <= 1 {
        crate::host::secret_set(key, value)?;
    } else {
        for (i, p) in parts.iter().enumerate() {
            crate::host::secret_set(&format!("{key}#{}", i + 1), p)?;
        }
        crate::host::secret_set(key, &format!("parts:{}", parts.len()))?;
    }
    let kept = if parts.len() > 1 { parts.len() } else { 0 };
    (kept + 1..=old).try_for_each(|i| crate::host::secret_set(&format!("{key}#{i}"), ""))
}

fn get(key: &str) -> Option<String> {
    let v = crate::host::secret_get(key)?;
    match v.strip_prefix("parts:").and_then(|n| n.parse::<usize>().ok()) {
        Some(n) => (1..=n).map(|i| crate::host::secret_get(&format!("{key}#{i}"))).collect(),
        None => Some(v),
    }
}

pub fn load(key: &str) -> Option<Saved> {
    serde_json::from_str(&get(key)?).ok()
}

fn store(key: &str, s: &Saved) -> Result<()> {
    put(key, &serde_json::to_string(s)?)
}

static CACHE: Mutex<Option<HashMap<String, Access>>> = Mutex::new(None);

fn keep(key: &str, a: Access) {
    // ponytail: if the store refuses it, it stays in memory only; the tools process then refreshes for itself
    let _ = put(&format!("{key}:access"), &serde_json::to_string(&a).unwrap_or_default());
    CACHE.lock().unwrap().get_or_insert_default().insert(key.into(), a);
}

/// Drop the access token (a server refused it): the next use refreshes.
pub fn forget_access(key: &str) {
    let _ = put(&format!("{key}:access"), "");
    CACHE.lock().unwrap().get_or_insert_default().remove(key);
}

/// Forget the sign-in altogether.
pub fn sign_out(key: &str) -> Result<()> {
    forget_access(key);
    put(key, "")
}

// ---------------- signing in ----------------

/// Where the browser will come back to, picked before the sign-in starts (dynamic registration needs it).
pub struct Redirect {
    pub uri: String,
    listener: Option<TcpListener>,
}

/// `listen`: this computer's browser comes back here (the desktop app); else (a server) the address is pasted.
/// `host`: "127.0.0.1" (RFC 8252) or "localhost" (what Microsoft lets you register); `path` like "/callback" or "".
/// `fixed`: listen on PASTE_PORT, not any free port (an app the person registered names the exact address).
pub async fn redirect(listen: bool, fixed: bool, host: &str, path: &str) -> Result<Redirect> {
    let at = if fixed { PASTE_PORT } else { 0 };
    let listener = if listen { Some(TcpListener::bind(("127.0.0.1", at)).await.with_context(|| format!("port {at} on this computer is busy; close what's using it and try again"))?) } else { None };
    let port = listener.as_ref().map(|l| l.local_addr().map(|a| a.port())).transpose()?.unwrap_or(PASTE_PORT);
    Ok(Redirect { uri: format!("http://{host}:{port}{path}"), listener })
}

struct Flow {
    key: String,
    saved: Saved,
    verifier: String,
    state: String,
    redirect: String,
    /// Shown on the page the browser lands on.
    what: String,
    /// The authorization server's `issuer`, and whether it said it puts it on its answer (RFC 9207).
    issuer: Option<(String, bool)>,
}

/// The sign-in in progress (one at a time; starting another replaces it).
static FLOW: Mutex<Option<Flow>> = Mutex::new(None);
static PENDING: Mutex<Option<JoinHandle<Result<Value>>>> = Mutex::new(None);
static CANCEL: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);

/// Begin a sign-in that will be kept under `key`; returns the address to open. `what` names it for the person
/// ("Notion", "Outlook"). `extra` goes on the authorization address (prompt=…, login_hint=…). `issuer`: see `Flow`.
pub fn begin(key: &str, what: &str, authorize: &str, saved: Saved, redirect: Redirect, extra: &[(&str, &str)], issuer: Option<(String, bool)>) -> Result<String> {
    cancel();
    let verifier = crate::random_hex(32)?;
    let state = crate::random_hex(16)?;
    let mut url = url::Url::parse(authorize).context("the sign-in address isn't a URL")?;
    if url.scheme() != "https" && !crate::tools::is_private_host(authorize) {
        bail!("the sign-in address must start with https://");
    }
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", &saved.client_id)
            .append_pair("redirect_uri", &redirect.uri)
            .append_pair("state", &state)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &crate::chatgpt::challenge(&verifier));
        if let Some(s) = &saved.scope {
            q.append_pair("scope", s);
        }
        if let Some(r) = &saved.resource {
            q.append_pair("resource", r);
        }
        for (k, v) in extra {
            q.append_pair(k, v);
        }
    }
    *FLOW.lock().unwrap() = Some(Flow { key: key.into(), saved, verifier, state, redirect: redirect.uri, what: what.into(), issuer });
    if let Some(listener) = redirect.listener {
        let what = what.to_string();
        let task = tokio::spawn(async move { tokio::time::timeout(SIGN_IN_TIMEOUT, callback(listener)).await.with_context(|| format!("timed out waiting for {what}"))? });
        *CANCEL.lock().unwrap() = Some(task.abort_handle());
        *PENDING.lock().unwrap() = Some(task);
    }
    Ok(url.into())
}

/// Stop a sign-in that's still waiting: its port closes and `finish` returns "cancelled".
pub fn cancel() {
    if let Some(h) = CANCEL.lock().unwrap().take() {
        h.abort();
    }
    if let Some(t) = PENDING.lock().unwrap().take() {
        t.abort();
    }
}

/// Wait for the sign-in `begin` started (desktop). Returns the token endpoint's answer (an ID token, if any, is in it).
pub async fn finish() -> Result<Value> {
    let task = PENDING.lock().unwrap().take().context("no sign-in in progress")?;
    task.await.context("the sign-in was cancelled")?
}

/// Finish with the address the browser ended up on (http://127.0.0.1:…?code=…), pasted by the person.
pub async fn paste(address: &str) -> Result<Value> {
    let url = url::Url::parse(address.trim()).context("paste the whole address from your browser's address bar (it starts with http://)")?;
    let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    let out = complete(&pairs).await;
    if out.is_ok() {
        cancel();
    }
    out
}

/// The key the sign-in in progress will be kept under (to tell which sign-in a paste belongs to).
pub fn pending_key() -> Option<String> {
    FLOW.lock().unwrap().as_ref().map(|f| f.key.clone())
}

async fn callback(listener: TcpListener) -> Result<Value> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf)).await.ok().and_then(Result::ok).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        // "GET /callback?code=…&state=… HTTP/1.1". A favicon or a stray request gets a 404 and the wait goes on.
        let path = req.split_whitespace().nth(1).unwrap_or("");
        let pairs: Vec<(String, String)> = path.split_once('?').map(|(_, q)| url::form_urlencoded::parse(q.as_bytes()).into_owned().collect()).unwrap_or_default();
        let Some(state) = pairs.iter().find(|(k, _)| k == "state").map(|(_, v)| v.clone()) else {
            crate::chatgpt::answer(&mut sock, "404 Not Found", "").await;
            continue;
        };
        // Not from this sign-in (another page found the port): ignore it and keep waiting.
        let what = FLOW.lock().unwrap().as_ref().filter(|f| f.state == state).map(|f| f.what.clone());
        let Some(what) = what else {
            crate::chatgpt::answer(&mut sock, "400 Bad Request", "").await;
            continue;
        };
        let result = complete(&pairs).await;
        let page = match &result {
            Ok(_) => crate::chatgpt::page(&format!("Signed in to {what}"), "You can close this tab and go back to Mimir."),
            Err(e) => crate::chatgpt::page(&format!("Couldn't sign in to {what}"), &format!("{e:#}. Go back to Mimir and try again.")),
        };
        crate::chatgpt::answer(&mut sock, "200 OK", &page).await;
        return result;
    }
}

/// The browser came back: check it's this sign-in's, trade the code for tokens, keep them.
async fn complete(pairs: &[(String, String)]) -> Result<Value> {
    let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    if let Some(e) = get("error") {
        bail!("the sign-in was refused: {}", get("error_description").unwrap_or(e));
    }
    let flow = {
        let mut f = FLOW.lock().unwrap();
        // The state proves this answer belongs to the sign-in started here, not one someone else started.
        if f.as_ref().is_none_or(|f| get("state") != Some(f.state.as_str())) {
            bail!("that's not the address from this sign-in; start again and paste the newest one");
        }
        // The answer names the server that gave it: an answer from another one (a mix-up attack) is refused.
        if let Some((issuer, always)) = &f.as_ref().unwrap().issuer
            && (get("iss").is_some_and(|i| i != issuer) || (*always && get("iss").is_none()))
        {
            bail!("the sign-in answer came from a different server than the one asked; start again");
        }
        f.take().unwrap()
    };
    let code = get("code").context("the address has no sign-in code; paste the whole address")?;
    let mut form = vec![("grant_type", "authorization_code"), ("code", code), ("redirect_uri", &flow.redirect), ("code_verifier", &flow.verifier)];
    let v = token_request(&flow.saved, &mut form).await?.map_err(|e| anyhow::anyhow!("{} didn't accept the sign-in ({e})", flow.what))?;
    let mut saved = flow.saved;
    saved.refresh = v["refresh_token"].as_str().unwrap_or("").into();
    store(&flow.key, &saved)?;
    keep(&flow.key, access_of(&v)?);
    Ok(v)
}

fn access_of(v: &Value) -> Result<Access> {
    let token = v["access_token"].as_str().context("no access token in the answer")?.to_string();
    // No lifetime given: use it until a server refuses it (callers then forget_access and ask again).
    Ok(Access { token, expires_at: v["expires_in"].as_i64().map_or(i64::MAX / 2, |s| now() + s) })
}

/// POST the token endpoint with the client's own fields added. Ok(Err(code)) when it refused; Err when it couldn't be asked.
async fn token_request<'a>(s: &'a Saved, form: &mut Vec<(&'a str, &'a str)>) -> Result<std::result::Result<Value, String>> {
    form.push(("client_id", &s.client_id));
    if let Some(c) = &s.client_secret {
        form.push(("client_secret", c));
    }
    if let Some(r) = &s.resource {
        form.push(("resource", r));
    }
    let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(form.iter()).finish();
    let resp = reqwest::Client::new()
        .post(&s.token_url)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(reqwest::header::ACCEPT, "application/json")
        .body(body)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .with_context(|| format!("couldn't reach {}", s.token_url))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or_default();
    if status.is_success() {
        return Ok(Ok(v));
    }
    if status.is_server_error() {
        bail!("the sign-in service had a problem ({status}); try again in a moment");
    }
    Ok(Err(v["error_description"].as_str().or(v["error"].as_str()).unwrap_or(status.as_str()).to_string()))
}

// ---------------- tokens ----------------

/// One refresh at a time: each may replace the refresh token, and a second with the old one would fail.
static REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn fresh(key: &str) -> Option<String> {
    let mut c = CACHE.lock().unwrap();
    let c = c.get_or_insert_default();
    if !c.contains_key(key)
        && let Some(a) = get(&format!("{key}:access")).and_then(|s| serde_json::from_str::<Access>(&s).ok())
    {
        c.insert(key.into(), a);
    }
    c.get(key).filter(|a| a.expires_at - 120 > now()).map(|a| a.token.clone())
}

/// A working access token for the sign-in kept under `key`, refreshed when it's about to run out.
/// `what` names it in the error when the sign-in has ended ("sign in to … again").
pub async fn access_token(key: &str, what: &str) -> Result<String> {
    if let Some(t) = fresh(key) {
        return Ok(t);
    }
    let _one = REFRESH.lock().await;
    if let Some(t) = fresh(key) {
        return Ok(t);
    }
    let ended = || anyhow::anyhow!("the sign-in to {what} has ended; sign in again in Settings");
    let saved = load(key).filter(|s| !s.refresh.is_empty()).ok_or_else(ended)?;
    let refresh = |s: Saved| async move {
        let mut form = vec![("grant_type", "refresh_token"), ("refresh_token", s.refresh.as_str())];
        if let Some(sc) = &s.scope {
            form.push(("scope", sc));
        }
        token_request(&s, &mut form).await
    };
    let mut got = refresh(saved.clone()).await?;
    if got.is_err()
        && let Some(newer) = load(key).filter(|s| s.refresh != saved.refresh && !s.refresh.is_empty())
    {
        // Another Mimir process (a CLI bot's tools process) refreshed first: use its new token.
        got = refresh(newer).await?;
    }
    match got {
        Ok(v) => {
            let mut s = load(key).unwrap_or(saved);
            if let Some(r) = v["refresh_token"].as_str() {
                s.refresh = r.into();
            }
            store(key, &s)?;
            let a = access_of(&v)?;
            keep(key, a.clone());
            Ok(a.token)
        }
        Err(code) => {
            // Ended (revoked, expired, password changed): keep the client for signing in again.
            store(key, &Saved { refresh: String::new(), ..saved })?;
            forget_access(key);
            Err(ended().context(code))
        }
    }
}

/// An ID token's claims, without checking its signature: it came straight from the token endpoint over TLS, which
/// OpenID Connect accepts in place of that (Core 1.0, 3.1.3.7). Checks it was made for `client_id`.
pub fn id_claims(token: &str, client_id: &str) -> Result<Value> {
    use base64::Engine;
    let part = token.split('.').nth(1).context("no ID token in the answer")?;
    let claims: Value = serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part.trim_end_matches('='))?).context("unreadable ID token")?;
    let aud_ok = claims["aud"] == client_id || claims["aud"].as_array().is_some_and(|a| a.iter().any(|x| x == client_id));
    if !aud_ok || claims["exp"].as_i64().unwrap_or(0) < now() {
        bail!("the sign-in answer didn't check out (audience or expiry); start again");
    }
    Ok(claims)
}

/// Tests that sign in take turns: there's one sign-in at a time, and starting one cancels the other.
#[cfg(test)]
pub(crate) static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// A token endpoint that answers every POST with `answer` (one connection per request).
    async fn token_server(answers: Vec<String>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}/token", l.local_addr().unwrap().port());
        tokio::spawn(async move {
            for a in answers {
                let (mut s, _) = l.accept().await.unwrap();
                let mut b = vec![0u8; 8192];
                let _ = s.read(&mut b).await;
                let msg = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{a}", a.len());
                tokio::io::AsyncWriteExt::write_all(&mut s, msg.as_bytes()).await.unwrap();
            }
        });
        url
    }

    fn secrets() {
        let p = std::env::temp_dir().join(format!("mimir-oauth-{}-{}.db", std::process::id(), crate::random_hex(4).unwrap()));
        crate::host::install_secrets(std::sync::Arc::new(crate::host::DbSecrets(crate::db::Db::open(&p).unwrap())));
    }

    /// The browser comes back to the listener: strays are turned away, the code is traded, tokens are kept and the
    /// expired one is refreshed with the rotated refresh token.
    #[tokio::test]
    async fn sign_in_then_refresh() {
        let _one = SERIAL.lock().await;
        secrets();
        let token_url = token_server(vec![
            r#"{"access_token":"a1","refresh_token":"r1","expires_in":1}"#.into(),
            r#"{"access_token":"a2","refresh_token":"r2","expires_in":3600}"#.into(),
        ])
        .await;
        let r = redirect(true, false, "127.0.0.1", "/callback").await.unwrap();
        let back = r.uri.clone();
        let saved = Saved { token_url, client_id: "c1".into(), scope: Some("s".into()), ..Default::default() };
        let url = url::Url::parse(&begin("oauth:test", "Test", "https://auth.example/authorize", saved, r, &[], None).unwrap()).unwrap();
        let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!((q["client_id"].as_str(), q["redirect_uri"].as_str(), q["code_challenge_method"].as_str(), q["scope"].as_str()), ("c1", back.as_str(), "S256", "s"));
        assert_eq!(reqwest::get(format!("{back}?x=1")).await.unwrap().status(), 404);
        assert_eq!(reqwest::get(format!("{back}?code=x&state=someone-elses")).await.unwrap().status(), 400);
        let page = reqwest::get(format!("{back}?code=c&state={}", q["state"])).await.unwrap().text().await.unwrap();
        assert!(page.contains("Signed in to Test"), "{page}");
        assert_eq!(finish().await.unwrap()["access_token"], "a1");
        assert_eq!(load("oauth:test").unwrap().refresh, "r1");
        // a1 runs out within the two-minute margin: refreshed, and the new refresh token is kept.
        assert_eq!(access_token("oauth:test", "Test").await.unwrap(), "a2");
        assert_eq!(load("oauth:test").unwrap().refresh, "r2");
        assert_eq!(access_token("oauth:test", "Test").await.unwrap(), "a2", "cached");
        sign_out("oauth:test").unwrap();
        assert!(format!("{:#}", access_token("oauth:test", "Test").await.unwrap_err()).contains("sign in again"));
    }

    /// RFC 9207: an answer naming another issuer, or none when the server said it always does, is refused.
    #[tokio::test]
    async fn the_answer_comes_from_the_server_asked() {
        let _one = SERIAL.lock().await;
        secrets();
        let token_url = token_server(vec![r#"{"access_token":"a1"}"#.into()]).await;
        let r = redirect(false, false, "127.0.0.1", "/callback").await.unwrap();
        let back = r.uri.clone();
        let saved = Saved { token_url, client_id: "c1".into(), ..Default::default() };
        let url = url::Url::parse(&begin("oauth:iss", "Test", "https://auth.example/authorize", saved, r, &[], Some(("https://auth.example".into(), true))).unwrap()).unwrap();
        let state = url.query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
        assert!(paste(&format!("{back}?code=c&state={state}&iss=https%3A%2F%2Fevil.example")).await.is_err(), "another issuer");
        assert!(paste(&format!("{back}?code=c&state={state}")).await.is_err(), "promised but missing");
        assert_eq!(paste(&format!("{back}?code=c&state={state}&iss=https%3A%2F%2Fauth.example")).await.unwrap()["access_token"], "a1");
        sign_out("oauth:iss").unwrap();
    }

    /// A long secret goes in parts and comes back whole; a shorter one later leaves no parts behind.
    #[test]
    fn long_secrets_in_parts() {
        secrets();
        let long = "x".repeat(2500);
        put("oauth:long", &long).unwrap();
        assert_eq!(crate::host::secret_get("oauth:long").as_deref(), Some("parts:3"));
        assert_eq!(get("oauth:long").as_deref(), Some(long.as_str()));
        put("oauth:long", "short").unwrap();
        assert_eq!(get("oauth:long").as_deref(), Some("short"));
        assert!(crate::host::secret_get("oauth:long#1").is_none() && crate::host::secret_get("oauth:long#3").is_none());
        put("oauth:long", "").unwrap();
        assert!(get("oauth:long").is_none());
    }

    #[test]
    fn id_token_is_for_this_client() {
        use base64::Engine;
        let jwt = |c: Value| format!("e30.{}.sig", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(c.to_string()));
        let good = serde_json::json!({"aud": "c1", "exp": now() + 60, "preferred_username": "me@outlook.com"});
        assert_eq!(id_claims(&jwt(good.clone()), "c1").unwrap()["preferred_username"], "me@outlook.com");
        assert!(id_claims(&jwt(good), "c2").is_err());
        assert!(id_claims("junk", "c1").is_err());
    }
}
