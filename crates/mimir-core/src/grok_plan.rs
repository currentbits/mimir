//! "Sign in with your Grok plan": bots on the `xai` account run on the person's Grok plan (The X Subscription,
//! SuperGrok or X Premium+) through xAI's own API, with no Grok Build and no API key. xAI's device-code sign-in
//! (RFC 8628) at auth.x.ai, with Grok Build's public client, as xAI set it up for other apps (x.ai/news/grok-opencode).
//! A device code works the same on a server: the person confirms the code in any browser. xAI decides which plans
//! may use the API this way; a 403 says the plan isn't one of them. `grok -p` stays as the other way in.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;

const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
/// `api:access` lets the token call api.x.ai; without it the sign-in works but no request does.
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// The sign-in (refresh token, account): lives as long as you stay signed in.
const SECRET: &str = "grok_plan_signin";
/// The short-lived access token, kept so the tools process and the next start needn't refresh at once.
const ACCESS: &str = "grok_plan_access";

fn auth_base() -> String {
    std::env::var("MIMIR_XAI_AUTH").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "https://auth.x.ai".into())
}

/// Where the plan's token goes: xAI's API, never a bot's own address.
pub fn api_base() -> String {
    std::env::var("MIMIR_XAI_API").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "https://api.x.ai/v1".into())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct Saved {
    /// Empty once xAI has ended the sign-in.
    refresh_token: String,
    email: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Access {
    token: String,
    expires_at: i64,
}

fn load() -> Option<Saved> {
    serde_json::from_str(&crate::host::secret_get(SECRET)?).ok()
}

fn store(s: &Saved) -> Result<()> {
    crate::host::secret_set(SECRET, &serde_json::to_string(s)?)
}

static CACHE: Mutex<Option<Access>> = Mutex::new(None);

fn keep(a: Access) {
    // ponytail: Windows' credential store holds at most 2,560 bytes; a bigger token stays in memory only
    let _ = crate::host::secret_set(ACCESS, &serde_json::to_string(&a).unwrap_or_default());
    *CACHE.lock().unwrap() = Some(a);
}

pub(crate) fn forget_access() {
    let _ = crate::host::secret_set(ACCESS, "");
    *CACHE.lock().unwrap() = None;
}

/// Signed in, and as whom.
pub fn account() -> Option<String> {
    load().filter(|s| !s.refresh_token.is_empty()).map(|s| s.email)
}

// ---------------- signing in ----------------

/// The sign-in waiting for its code to be confirmed (one at a time), and why the last one stopped.
static PENDING: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);
static FAILED: Mutex<Option<String>> = Mutex::new(None);

/// Why the last sign-in didn't finish (declined, ran out, a plan xAI refused), for the settings page.
pub fn sign_in_error() -> Option<String> {
    FAILED.lock().unwrap().clone()
}

/// Stop a sign-in that's still waiting (Cancel, or starting another).
pub fn cancel() {
    if let Some(h) = PENDING.lock().unwrap().take() {
        h.abort();
    }
}

/// Begin a sign-in: returns (address to open, code to confirm there). It keeps waiting in the background until the
/// code is confirmed, declined or runs out; `account` (or `sign_in_error`) says how it went.
pub async fn start() -> Result<(String, String)> {
    cancel();
    *FAILED.lock().unwrap() = None;
    let v = post(&format!("{}/oauth2/device/code", auth_base()), &[("client_id", CLIENT_ID), ("scope", SCOPE)])
        .await?
        .map_err(|(_, e)| anyhow!("xAI didn't start the sign-in ({e})"))?;
    let device = v["device_code"].as_str().context("xAI gave no sign-in code")?.to_string();
    let code = v["user_code"].as_str().context("xAI gave no sign-in code")?.to_string();
    let url = v["verification_uri_complete"].as_str().or(v["verification_uri"].as_str()).context("xAI gave no sign-in address")?.to_string();
    let interval = v["interval"].as_u64().unwrap_or(5).clamp(1, 60);
    let expires = v["expires_in"].as_u64().unwrap_or(600).min(1800);
    let task = tokio::spawn(async move {
        if let Err(e) = poll(&device, interval, expires).await {
            *FAILED.lock().unwrap() = Some(format!("{e:#}"));
        }
    });
    *PENDING.lock().unwrap() = Some(task.abort_handle());
    Ok((url, code))
}

/// Ask until the code is confirmed (RFC 8628 §3.4–3.5). A moment's trouble reaching xAI just means asking again.
async fn poll(device: &str, mut interval: u64, expires: u64) -> Result<()> {
    let until = std::time::Instant::now() + Duration::from_secs(expires);
    while std::time::Instant::now() < until {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        match post(&format!("{}/oauth2/token", auth_base()), &[("grant_type", DEVICE_GRANT), ("client_id", CLIENT_ID), ("device_code", device)]).await {
            Err(_) => {}
            Ok(Ok(v)) => return signed_in(&v),
            Ok(Err((_, e))) if e == "authorization_pending" => {}
            Ok(Err((_, e))) if e == "slow_down" => interval += 5,
            Ok(Err((_, e))) if e == "access_denied" => bail!("the sign-in was declined at x.ai"),
            Ok(Err((_, e))) if e == "expired_token" => break,
            Ok(Err((403, e))) => bail!("{}", not_on_plan(&e)),
            Ok(Err((_, e))) => bail!("xAI didn't accept the sign-in ({e})"),
        }
    }
    bail!("the code ran out before it was confirmed; start again")
}

fn signed_in(v: &Value) -> Result<()> {
    let refresh = v["refresh_token"].as_str().filter(|s| !s.is_empty()).context("xAI gave no refresh token")?;
    // Only shown, so read as it came from xAI's token endpoint over TLS.
    let email = v["id_token"].as_str().and_then(claims).and_then(|c| c["email"].as_str().map(String::from)).unwrap_or_else(|| "your Grok account".into());
    store(&Saved { refresh_token: refresh.into(), email })?;
    keep(access_of(v)?);
    Ok(())
}

fn claims(jwt: &str) -> Option<Value> {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(jwt.split('.').nth(1)?.trim_end_matches('=')).ok()?).ok()
}

fn access_of(v: &Value) -> Result<Access> {
    let token = v["access_token"].as_str().context("xAI gave no access token")?.to_string();
    // expires_in, else the token's own expiry, else an hour.
    let expires_at = v["expires_in"].as_i64().map(|s| now() + s).or_else(|| claims(&token)?["exp"].as_i64()).unwrap_or(now() + 3600);
    Ok(Access { token, expires_at })
}

/// What a 403 from xAI means here: signed in, but this plan may not use xAI's API.
pub(crate) fn not_on_plan(detail: &str) -> String {
    format!(
        "xAI doesn't let this Grok plan use its API from other apps ({detail}). Use an xAI API key, or the Grok account through Grok Build, instead"
    )
}

/// POST a form to auth.x.ai. Ok(Err((status, error code))) when xAI refused; Err when it couldn't be asked.
async fn post(url: &str, form: &[(&str, &str)]) -> Result<std::result::Result<Value, (u16, String)>> {
    let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(form).finish();
    let resp = reqwest::Client::new()
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .timeout(Duration::from_secs(30))
        .send()
        .await?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or_default();
    if status.is_success() {
        return Ok(Ok(v));
    }
    if status.is_server_error() {
        bail!("xAI's sign-in service had a problem ({status}); try again in a moment");
    }
    let code = v["error"].as_str().or(v["error_description"].as_str()).or(v["code"].as_str()).unwrap_or(status.as_str());
    Ok(Err((status.as_u16(), code.to_string())))
}

/// Sign out: Mimir forgets the sign-in. ponytail: not revoked at xAI; it ends when the refresh token goes unused.
pub fn sign_out() -> Result<()> {
    cancel();
    crate::host::secret_set(SECRET, "")?;
    forget_access();
    Ok(())
}

// ---------------- tokens ----------------

/// Only one refresh at a time: each one replaces the refresh token, and a second with the old one would fail.
static REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn fresh() -> Option<String> {
    let mut c = CACHE.lock().unwrap();
    if c.is_none() {
        *c = crate::host::secret_get(ACCESS).and_then(|s| serde_json::from_str(&s).ok());
    }
    c.as_ref().filter(|a| a.expires_at - 120 > now()).map(|a| a.token.clone())
}

/// A working access token, refreshed when it's about to run out.
pub async fn access_token() -> Result<String> {
    if let Some(t) = fresh() {
        return Ok(t);
    }
    let _one = REFRESH.lock().await;
    if let Some(t) = fresh() {
        return Ok(t);
    }
    let saved = load().filter(|s| !s.refresh_token.is_empty()).context("sign in with your Grok plan first (Settings → AI keys)")?;
    let refresh = |s: &Saved| {
        let r = s.refresh_token.clone();
        async move { post(&format!("{}/oauth2/token", auth_base()), &[("grant_type", "refresh_token"), ("client_id", CLIENT_ID), ("refresh_token", &r)]).await }
    };
    let mut got = refresh(&saved).await?;
    if got.is_err()
        && let Some(newer) = load().filter(|s| s.refresh_token != saved.refresh_token && !s.refresh_token.is_empty())
    {
        // Another Mimir process (the tools process of a bot on a CLI account) refreshed first: use its new token.
        got = refresh(&newer).await?;
    }
    match got {
        Ok(v) => {
            let mut s = load().unwrap_or(saved);
            // xAI hands out a new refresh token each time; keep the old one if it ever doesn't.
            if let Some(r) = v["refresh_token"].as_str().filter(|r| !r.is_empty()) {
                s.refresh_token = r.into();
            }
            store(&s)?;
            let a = access_of(&v)?;
            keep(a.clone());
            Ok(a.token)
        }
        // A plan xAI won't serve: signing in again wouldn't help, so the sign-in stays.
        Err((403, e)) => bail!("{}", not_on_plan(&e)),
        Err((_, e)) => {
            store(&Saved { refresh_token: String::new(), ..saved })?;
            forget_access();
            bail!("your Grok plan sign-in has ended ({e}); sign in again in Settings → AI keys")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_token_says_when_it_runs_out() {
        let jwt = |c: Value| format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(c.to_string()));
        let a = access_of(&json!({"access_token": "t", "expires_in": 900})).unwrap();
        assert!((a.expires_at - now() - 900).abs() < 5);
        let exp = now() + 21600;
        assert_eq!(access_of(&json!({"access_token": jwt(json!({"exp": exp}))})).unwrap().expires_at, exp, "from the token itself");
        assert!(access_of(&json!({"access_token": "opaque"})).unwrap().expires_at > now() + 3000, "an hour when nobody says");
        assert!(access_of(&json!({})).is_err());
        assert_eq!(claims(&jwt(json!({"email": "a@b.c"}))).unwrap()["email"], "a@b.c");
        assert!(claims("garbage").is_none());
    }

    #[test]
    fn a_refused_plan_says_what_to_do_instead() {
        let m = not_on_plan("caller does not have permission");
        assert!(m.contains("API key") && m.contains("Grok Build") && m.contains("permission"), "{m}");
    }
}
