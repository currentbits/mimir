//! "Sign in with ChatGPT": bots run on the person's ChatGPT plan (Plus or Pro) through OpenAI's Responses API, with no
//! Codex CLI. OAuth PKCE with dynamic registration, as OpenAI documents for open-source apps
//! (developers.openai.com/siwc/token-sharing-open-source). The browser comes back to
//! http://127.0.0.1:<port>/auth/callback; on a server that address is the person's own computer, so they paste the
//! address their browser ended up on instead. The person sets how much Mimir may use each week in ChatGPT's settings.

use crate::db::Usage;
use crate::provider::Reply;
use crate::tools::{self, ToolCtx};
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(600);
/// The sign-in itself (account, client id, refresh token): lives as long as you stay signed in.
const SECRET: &str = "chatgpt_signin";
/// The hour-long access token, kept so the tools process and the next start needn't refresh at once.
const ACCESS: &str = "chatgpt_access";
const RESOURCE: &str = "https://api.openai.com/v1";
const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

fn auth_base() -> String {
    std::env::var("MIMIR_CHATGPT_AUTH").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "https://auth.openai.com".into())
}

fn api_base() -> String {
    std::env::var("MIMIR_CHATGPT_API").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| RESOURCE.into())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct Saved {
    client_id: String,
    /// Empty once OpenAI has ended the sign-in: sign in again (with the same client id).
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

fn forget_access() {
    let _ = crate::host::secret_set(ACCESS, "");
    *CACHE.lock().unwrap() = None;
}

/// Signed in, and as whom.
pub fn account() -> Option<String> {
    load().filter(|s| !s.refresh_token.is_empty()).map(|s| s.email)
}

/// This computer (or server) to OpenAI: made once and kept, signed in or not, as the docs ask.
fn host_id(db: &crate::db::Db) -> Result<String> {
    if let Some(id) = db.setting("chatgpt_host_id")?.filter(|s| !s.is_empty()) {
        return Ok(id);
    }
    let h = crate::random_hex(16)?;
    // A version 4 UUID: four bits of version, two of variant, the rest random.
    let v = format!("{}-{}-4{}-{:x}{}-{}", &h[..8], &h[8..12], &h[13..16], (u8::from_str_radix(&h[16..17], 16)? & 3) | 8, &h[17..20], &h[20..32]);
    let id = format!("urn:uuid:{v}");
    db.set_setting("chatgpt_host_id", &id)?;
    Ok(id)
}

// ---------------- signing in ----------------

struct Flow {
    verifier: String,
    state: String,
    nonce: String,
    redirect: String,
    /// The client id from an earlier sign-in on this computer, reused as the docs ask.
    client_id: Option<String>,
}

/// The sign-in in progress (one at a time; starting again replaces it).
static FLOW: Mutex<Option<Flow>> = Mutex::new(None);
static PENDING: Mutex<Option<JoinHandle<Result<String>>>> = Mutex::new(None);
static CANCEL: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(<sha2::Sha256 as sha2::Digest>::digest(verifier.as_bytes()))
}

/// Begin a sign-in; returns the address to open in the browser. `listen`: this machine's browser comes back here (the
/// desktop app); otherwise (a server) the person pastes the address their browser ended up on into `paste`.
pub async fn start(db: &crate::db::Db, listen: bool) -> Result<String> {
    cancel();
    let verifier = crate::random_hex(32)?;
    let (state, nonce) = (crate::random_hex(16)?, crate::random_hex(16)?);
    let listener = if listen { Some(TcpListener::bind("127.0.0.1:0").await?) } else { None };
    // Nothing listens on a server's behalf; 1455 is the port OpenAI's own examples use.
    let port = listener.as_ref().map(|l| l.local_addr().map(|a| a.port())).transpose()?.unwrap_or(1455);
    let redirect = format!("http://127.0.0.1:{port}/auth/callback");
    let earlier = load().map(|s| s.client_id).filter(|c| !c.is_empty());
    let mut url = url::Url::parse(&format!("{}/api/accounts/authorize", auth_base()))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", earlier.as_deref().unwrap_or("dynamic_agent_client"));
        if earlier.is_none() {
            q.append_pair("agent_name_hint", "Mimir");
        }
        q.append_pair("ext_agent_host_id", &host_id(db)?)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect)
            .append_pair("scope", SCOPES)
            .append_pair("resource", RESOURCE)
            .append_pair("state", &state)
            .append_pair("nonce", &nonce)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &challenge(&verifier));
    }
    *FLOW.lock().unwrap() = Some(Flow { verifier, state, nonce, redirect, client_id: earlier });
    if let Some(listener) = listener {
        let task = tokio::spawn(async move { tokio::time::timeout(SIGN_IN_TIMEOUT, callback(listener)).await.context("timed out waiting for ChatGPT")? });
        *CANCEL.lock().unwrap() = Some(task.abort_handle());
        *PENDING.lock().unwrap() = Some(task);
    }
    Ok(url.into())
}

/// Stop a sign-in that's still waiting: its callback port closes and `finish` returns "cancelled".
pub fn cancel() {
    if let Some(h) = CANCEL.lock().unwrap().take() {
        h.abort();
    }
    if let Some(t) = PENDING.lock().unwrap().take() {
        t.abort();
    }
}

/// Wait for the sign-in `start` began (desktop); returns the account's email.
pub async fn finish() -> Result<String> {
    let task = PENDING.lock().unwrap().take().context("no ChatGPT sign-in in progress")?;
    task.await.context("the sign-in was cancelled")?
}

/// Finish with the address the browser ended up on (http://127.0.0.1:…/auth/callback?code=…), pasted by the person.
pub async fn paste(address: &str) -> Result<String> {
    let url = url::Url::parse(address.trim()).context("paste the whole address from your browser's address bar (it starts with http://127.0.0.1)")?;
    let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    let out = complete(&pairs).await;
    if out.is_ok() {
        cancel();
    }
    out
}

/// Serve the browser's return trip: take the code, exchange it, show a page saying how it went.
async fn callback(listener: TcpListener) -> Result<String> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf)).await.ok().and_then(Result::ok).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        // "GET /auth/callback?code=… HTTP/1.1". Anything else (a favicon, a stray request) gets a 404 and the wait goes on.
        let path = req.split_whitespace().nth(1).unwrap_or("");
        let Some(query) = path.strip_prefix("/auth/callback?") else {
            answer(&mut sock, "404 Not Found", "").await;
            continue;
        };
        let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
        // Not from this sign-in (another page found the port): ignore it and keep waiting.
        let ours = FLOW.lock().unwrap().as_ref().is_some_and(|f| pairs.iter().any(|(k, v)| k == "state" && *v == f.state));
        if !ours {
            answer(&mut sock, "400 Bad Request", "").await;
            continue;
        }
        let result = complete(&pairs).await;
        let page = match &result {
            Ok(_) => page("Signed in with ChatGPT", "Your bots can use your ChatGPT plan now. You can close this tab and go back to Mimir."),
            Err(e) => page("Couldn't sign in with ChatGPT", &format!("{e:#}. Go back to Mimir and try again.")),
        };
        answer(&mut sock, "200 OK", &page).await;
        return result;
    }
}

/// The browser came back (or its address was pasted): check it's this sign-in's, trade the code for tokens, keep them.
async fn complete(pairs: &[(String, String)]) -> Result<String> {
    let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    if let Some(e) = get("error") {
        bail!("ChatGPT said no: {}", get("error_description").unwrap_or(e));
    }
    let flow = {
        let mut f = FLOW.lock().unwrap();
        // The state proves this answer belongs to the sign-in started here, not one someone else started.
        if f.as_ref().is_none_or(|f| get("state") != Some(f.state.as_str())) {
            bail!("that's not the address from this sign-in; start again and paste the newest one");
        }
        f.take().unwrap()
    };
    let code = get("code").context("the address has no sign-in code; paste the whole address")?;
    let client_id = get("client_id").map(String::from).or(flow.client_id).context("ChatGPT didn't give Mimir a client id; start again")?;
    let v = token_request(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", &client_id),
        ("code_verifier", &flow.verifier),
        ("redirect_uri", &flow.redirect),
        ("resource", RESOURCE),
    ])
    .await?
    .map_err(|e| anyhow::anyhow!("ChatGPT didn't accept the sign-in ({e})"))?;
    let claims = id_claims(v["id_token"].as_str().unwrap_or(""), &client_id, &flow.nonce)?;
    let email = claims["email"].as_str().unwrap_or("your ChatGPT account").to_string();
    let refresh = v["refresh_token"].as_str().context("ChatGPT gave no refresh token")?.to_string();
    store(&Saved { client_id, refresh_token: refresh, email: email.clone() })?;
    keep(access_of(&v)?);
    *MODEL.lock().unwrap() = None;
    Ok(email)
}

/// The ID token's claims, after checking it's for this sign-in. It comes straight from OpenAI's token endpoint over
/// TLS, which OpenID Connect accepts in place of checking its signature (Core 1.0, 3.1.3.7).
fn id_claims(token: &str, client_id: &str, nonce: &str) -> Result<Value> {
    let part = token.split('.').nth(1).context("ChatGPT gave no ID token")?;
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part.trim_end_matches('='))?).context("unreadable ID token")?;
    let aud_ok = claims["aud"] == client_id || claims["aud"].as_array().is_some_and(|a| a.iter().any(|x| x == client_id));
    if claims["iss"] != auth_base().as_str() || !aud_ok || claims["nonce"] != nonce || claims["exp"].as_i64().unwrap_or(0) < now() {
        bail!("the sign-in answer didn't check out (issuer, audience, nonce or expiry); start again");
    }
    Ok(claims)
}

fn access_of(v: &Value) -> Result<Access> {
    let token = v["access_token"].as_str().context("ChatGPT gave no access token")?.to_string();
    Ok(Access { token, expires_at: now() + v["expires_in"].as_i64().unwrap_or(3600) })
}

/// POST a form to the token endpoint. Ok(Err(code)) when OpenAI refused (invalid_grant…); Err when it couldn't be asked.
async fn token_request(form: &[(&str, &str)]) -> Result<std::result::Result<Value, String>> {
    let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(form).finish();
    let resp = reqwest::Client::new()
        .post(format!("{}/api/accounts/oauth/token", auth_base()))
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
        bail!("ChatGPT's sign-in service had a problem ({status}); try again in a moment");
    }
    Ok(Err(v["error"].as_str().or(v["error"]["code"].as_str()).unwrap_or(status.as_str()).to_string()))
}

/// Sign out: OpenAI ends the sign-in, and Mimir forgets it.
pub async fn sign_out() -> Result<()> {
    if let Some(s) = load().filter(|s| !s.refresh_token.is_empty()) {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([("token", s.refresh_token.as_str()), ("token_type_hint", "refresh_token"), ("client_id", s.client_id.as_str())])
            .finish();
        // ponytail: one try; if OpenAI can't be reached the sign-in stays listed under ChatGPT → Settings until removed there
        let _ = reqwest::Client::new()
            .post(format!("{}/api/accounts/oauth/revoke", auth_base()))
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .timeout(Duration::from_secs(15))
            .send()
            .await;
    }
    crate::host::secret_set(SECRET, "")?;
    forget_access();
    *MODEL.lock().unwrap() = None;
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
async fn access_token() -> Result<String> {
    if let Some(t) = fresh() {
        return Ok(t);
    }
    let _one = REFRESH.lock().await;
    if let Some(t) = fresh() {
        return Ok(t);
    }
    let saved = load().filter(|s| !s.refresh_token.is_empty()).context("sign in with ChatGPT first (Settings → AI keys)")?;
    let refresh = |s: &Saved| {
        let s = s.clone();
        async move { token_request(&[("grant_type", "refresh_token"), ("refresh_token", &s.refresh_token), ("client_id", &s.client_id)]).await }
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
            if let Some(r) = v["refresh_token"].as_str() {
                s.refresh_token = r.into();
            }
            store(&s)?;
            let a = access_of(&v)?;
            keep(a.clone());
            Ok(a.token)
        }
        Err(code) => {
            // Ended by OpenAI (signed out in ChatGPT, revoked, expired): keep the client id for signing in again.
            store(&Saved { refresh_token: String::new(), ..saved })?;
            forget_access();
            bail!("your ChatGPT sign-in has ended ({code}); sign in again in Settings → AI keys")
        }
    }
}

// ---------------- models ----------------

/// The models your plan offers in other apps: `{id, name, tools}` for the picker.
pub async fn models() -> Result<Vec<Value>> {
    let v: Value = reqwest::Client::new()
        .get(format!("{}/models", api_base()))
        .bearer_auth(access_token().await?)
        .timeout(Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"].as_str().is_none_or(|s| s == "list"))
        .filter_map(|m| {
            let id = m["slug"].as_str().or(m["id"].as_str())?;
            Some(json!({"id": id, "name": m["display_name"].as_str().unwrap_or(id), "tools": true}))
        })
        .collect())
}

/// The model a bot without one uses: the first your plan lists, looked up once.
static MODEL: Mutex<Option<String>> = Mutex::new(None);

async fn default_model() -> Result<String> {
    if let Some(m) = MODEL.lock().unwrap().clone() {
        return Ok(m);
    }
    let m = models().await?.first().and_then(|m| m["id"].as_str().map(String::from)).context("your ChatGPT plan lists no models Mimir can use")?;
    *MODEL.lock().unwrap() = Some(m.clone());
    Ok(m)
}

// ---------------- a reply ----------------

/// Web search turned out not to be offered on this route: stop asking for it.
static NO_WEB_SEARCH: AtomicBool = AtomicBool::new(false);

/// What went wrong, in words, from an error code and message (before or during the stream).
fn explain(status: Option<u16>, code: &str, msg: &str) -> anyhow::Error {
    let m = if msg.is_empty() { code } else { msg };
    match code {
        // "usage limit" is what the rest of Mimir recognises (a friendly message, routines that wait).
        "subscription_sharing_usage_limit_exceeded" => anyhow::anyhow!(
            "ChatGPT usage limit: Mimir has used what your ChatGPT plan allows it this week. Raise it in ChatGPT → Settings, or wait for it to reset"
        ),
        "subscription_sharing_user_not_eligible" => {
            anyhow::anyhow!("this ChatGPT account can't use its plan in other apps ({m}). It needs ChatGPT Plus or Pro")
        }
        "subscription_sharing_invalid_user" => anyhow::anyhow!("your ChatGPT sign-in has ended; sign in again in Settings → AI keys"),
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => anyhow::anyhow!("ChatGPT is busy ({m}); try again in a moment"),
        _ => match status {
            Some(401) => anyhow::anyhow!("ChatGPT didn't accept the sign-in ({m}); sign in again in Settings → AI keys"),
            Some(429) => anyhow::anyhow!("ChatGPT rate limit: {m}"),
            Some(s) => anyhow::anyhow!("ChatGPT {s}: {m}"),
            None => anyhow::anyhow!("ChatGPT: {m}"),
        },
    }
}

/// An error body: `{"error": {"code", "message"}}`, or `{"detail": "…"}` before the request was taken.
fn error_of(v: &Value) -> (String, String) {
    let e = if v["error"].is_object() { &v["error"] } else { v };
    let code = e["code"].as_str().or(e["type"].as_str()).unwrap_or("").to_string();
    let msg = e["message"].as_str().or(v["detail"].as_str()).unwrap_or("").to_string();
    (code, msg)
}

/// One streamed response, folded: output items, usage, model; or the error it ended with.
#[derive(Default)]
struct Turn {
    items: Vec<Value>,
    usage: Value,
    model: Option<String>,
    done: bool,
}

impl Turn {
    /// Folds one event; returns new text to show, if any.
    fn fold(&mut self, v: &Value) -> Result<Option<String>> {
        match v["type"].as_str().unwrap_or("") {
            "response.output_text.delta" => return Ok(v["delta"].as_str().filter(|s| !s.is_empty()).map(String::from)),
            "response.output_item.done" => self.items.push(v["item"].clone()),
            "response.completed" | "response.incomplete" => {
                let r = &v["response"];
                self.usage = r["usage"].clone();
                self.model = r["model"].as_str().map(String::from);
                if self.items.is_empty() {
                    self.items = r["output"].as_array().cloned().unwrap_or_default();
                }
                self.done = true;
            }
            "response.failed" => {
                let (code, msg) = error_of(&v["response"]["error"]);
                return Err(explain(None, &code, &msg));
            }
            "error" => {
                let (code, msg) = error_of(v);
                return Err(explain(None, &code, &msg));
            }
            _ => {}
        }
        Ok(None)
    }

    fn text(&self) -> String {
        self.items
            .iter()
            .filter(|i| i["type"] == "message")
            .flat_map(|i| i["content"].as_array().cloned().unwrap_or_default())
            .filter(|c| c["type"] == "output_text")
            .filter_map(|c| c["text"].as_str().map(String::from))
            .collect::<Vec<_>>()
            .join("")
    }
}

pub async fn respond(ctx: &ToolCtx, model: Option<&str>, system: &str, history: &[(String, String)], user: &str) -> Result<Reply> {
    let model = match model {
        Some(m) => m.to_string(),
        None => default_model().await?,
    };
    let mut input: Vec<Value> = history.iter().map(|(role, content)| json!({"role": role, "content": content})).collect();
    if ctx.images.is_empty() {
        input.push(json!({"role": "user", "content": user}));
    } else {
        let mut parts = vec![json!({"type": "input_text", "text": user})];
        for (mime, data) in crate::provider::inline_images(&ctx.images, 8_000_000)? {
            parts.push(json!({"type": "input_image", "image_url": format!("data:{mime};base64,{data}")}));
        }
        input.push(json!({"role": "user", "content": parts}));
    }
    let specs = tools::specs_for(ctx);
    // On this route, function tools have to come in a namespace. Not strict: Mimir's schemas have optional fields.
    let functions: Vec<Value> =
        specs.iter().map(|(name, description, parameters)| json!({"type": "function", "name": name, "description": description, "parameters": parameters, "strict": false})).collect();
    let mut web = !specs.is_empty() && !NO_WEB_SEARCH.load(Ordering::Relaxed);
    let http = reqwest::Client::builder().timeout(Duration::from_secs(600)).build()?;
    let mut usage = Usage { model: Some(model.clone()), ..Default::default() };
    let rounds = if specs.iter().any(|s| s.0 == "computer_screenshot") { 40 } else { crate::provider::MAX_TOOL_ROUNDS };
    // Screenshots in the conversation, oldest first: only the latest two are kept.
    let mut shots: Vec<usize> = vec![];
    let (mut retried_auth, mut busy) = (false, 0);
    let mut round = 0;
    while round < rounds {
        let mut body = json!({"model": model, "instructions": system, "input": input, "store": false, "stream": true, "include": ["reasoning.encrypted_content"]});
        let mut list = vec![];
        if !functions.is_empty() {
            list.push(json!({"type": "namespace", "name": "mimir", "description": "Mimir's tools: memory, routines, files, web pages, email, calendar and more.", "tools": functions}));
        }
        if web {
            list.push(json!({"type": "web_search"}));
        }
        if !list.is_empty() {
            body["tools"] = json!(list);
        }
        let resp = http.post(format!("{}/responses", api_base())).bearer_auth(access_token().await?).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let (code, msg) = error_of(&resp.json::<Value>().await.unwrap_or_default());
            // Web search isn't offered to every plan or model: once is enough to find out.
            if code == "subscription_sharing_unsupported_capability" && web {
                NO_WEB_SEARCH.store(true, Ordering::Relaxed);
                web = false;
                continue;
            }
            // The access token was ended early: refresh once and try again.
            if status == 401 && !retried_auth {
                retried_auth = true;
                forget_access();
                continue;
            }
            // Busy for a moment: wait a little, twice at most.
            if status == 503 && busy < 2 {
                busy += 1;
                tokio::time::sleep(Duration::from_secs(3 * busy)).await;
                continue;
            }
            return Err(explain(Some(status.as_u16()), &code, &msg));
        }
        crate::live::reset(ctx.bot_id, ctx.chat_id);
        let mut turn = Turn::default();
        let mut resp = resp;
        let mut bytes: Vec<u8> = vec![];
        while let Some(chunk) = resp.chunk().await? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > 32 * 1024 * 1024 {
                bail!("ChatGPT sent a reply too big to read");
            }
            while let Some(nl) = bytes.iter().position(|b| *b == b'\n') {
                let line = String::from_utf8_lossy(&bytes[..=nl]).into_owned();
                bytes.drain(..=nl);
                if let Some(data) = line.trim().strip_prefix("data:").map(str::trim)
                    && let Ok(v) = serde_json::from_str::<Value>(data)
                    && let Some(delta) = turn.fold(&v)?
                    && ctx.stream
                {
                    crate::live::push(ctx.bot_id, ctx.chat_id, &delta);
                }
            }
        }
        // Only a completed response counts: a stream that just stops isn't an answer.
        if !turn.done {
            bail!("ChatGPT stopped before the reply was finished; try again");
        }
        usage.tokens_in += turn.usage["input_tokens"].as_i64().unwrap_or(0);
        usage.tokens_out += turn.usage["output_tokens"].as_i64().unwrap_or(0);
        if let Some(m) = &turn.model {
            usage.model = Some(m.clone());
        }
        // Search results are pages anyone can write: the run has read outside content.
        if turn.items.iter().any(|i| i["type"] == "web_search_call") {
            ctx.taint();
        }
        let calls: Vec<Value> = turn.items.iter().filter(|i| i["type"] == "function_call").cloned().collect();
        if calls.is_empty() {
            return Ok(Reply { text: turn.text(), session: None, usage });
        }
        round += 1;
        // The reply so far goes back in, as it came (reasoning included): nothing is stored on OpenAI's side.
        // Without ids, which point at stored items.
        for mut item in turn.items {
            if let Some(o) = item.as_object_mut() {
                o.remove("id");
            }
            input.push(item);
        }
        for c in calls {
            let name = c["name"].as_str().unwrap_or("");
            let args: Value = serde_json::from_str(c["arguments"].as_str().unwrap_or("{}")).unwrap_or(json!({}));
            let out = tools::call(ctx, name, &args).await.unwrap_or_else(|e| format!("error: {e}"));
            let (shot, text) = crate::computer::tool_image(name, &out, &ctx.db.dir());
            input.push(json!({"type": "function_call_output", "call_id": c["call_id"], "output": text}));
            if let Some(data) = shot.and_then(|p| crate::computer::take_image(&p)) {
                while shots.len() >= 2 {
                    input[shots.remove(0)] = json!({"role": "user", "content": "(an earlier screenshot, removed)"});
                }
                shots.push(input.len());
                input.push(json!({"role": "user", "content": [
                    {"type": "input_text", "text": "The computer's screen:"},
                    {"type": "input_image", "image_url": format!("data:image/jpeg;base64,{data}")}
                ]}));
            }
        }
    }
    bail!("gave up after {rounds} tool rounds")
}

async fn answer(sock: &mut TcpStream, status: &str, html: &str) {
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

    fn jwt(claims: Value) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    #[test]
    fn id_token_must_be_for_this_sign_in() {
        let good = json!({"iss": "https://auth.openai.com", "aud": "oaiapp_1", "nonce": "n1", "exp": now() + 60, "email": "a@b.c"});
        assert_eq!(id_claims(&jwt(good.clone()), "oaiapp_1", "n1").unwrap()["email"], "a@b.c");
        let mut list = good.clone();
        list["aud"] = json!(["x", "oaiapp_1"]);
        assert!(id_claims(&jwt(list), "oaiapp_1", "n1").is_ok(), "audience as a list");
        assert!(id_claims(&jwt(good.clone()), "oaiapp_1", "other").is_err(), "nonce");
        assert!(id_claims(&jwt(good.clone()), "oaiapp_2", "n1").is_err(), "audience");
        let mut old = good.clone();
        old["exp"] = json!(now() - 5);
        assert!(id_claims(&jwt(old), "oaiapp_1", "n1").is_err(), "expired");
        let mut iss = good;
        iss["iss"] = json!("https://evil.example");
        assert!(id_claims(&jwt(iss), "oaiapp_1", "n1").is_err(), "issuer");
        assert!(id_claims("garbage", "oaiapp_1", "n1").is_err());
    }

    #[test]
    fn a_streamed_response_folds_into_text_calls_and_usage() {
        let mut t = Turn::default();
        let ev = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert_eq!(t.fold(&ev(r#"{"type":"response.output_text.delta","delta":"Hel"}"#)).unwrap().as_deref(), Some("Hel"));
        t.fold(&ev(r#"{"type":"response.output_item.done","item":{"type":"reasoning","id":"rs_1","encrypted_content":"x"}}"#)).unwrap();
        t.fold(&ev(r#"{"type":"response.output_item.done","item":{"type":"message","id":"m1","content":[{"type":"output_text","text":"Hello"}]}}"#)).unwrap();
        t.fold(&ev(r#"{"type":"response.output_item.done","item":{"type":"function_call","name":"memory_store","namespace":"mimir","call_id":"c1","arguments":"{}"}}"#)).unwrap();
        assert!(!t.done);
        t.fold(&ev(r#"{"type":"response.completed","response":{"model":"gpt-x","usage":{"input_tokens":12,"output_tokens":3},"output":[]}}"#)).unwrap();
        assert!(t.done);
        assert_eq!((t.text().as_str(), t.items.len(), t.usage["input_tokens"].as_i64(), t.model.as_deref()), ("Hello", 3, Some(12), Some("gpt-x")));
    }

    #[test]
    fn plan_errors_say_what_to_do() {
        let ev = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let e = Turn::default().fold(&ev(r#"{"type":"response.failed","response":{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"cap"}}}"#)).unwrap_err();
        // The rest of Mimir knows a usage limit by these words (a friendly reply; routines wait it out).
        assert!(format!("{e:#}").contains("usage limit"), "{e:#}");
        assert!(crate::friendly_error(&format!("{e:#}")).contains("usage limit"));
        let (code, msg) = error_of(&json!({"detail": "Not allowed"}));
        assert_eq!((code.as_str(), msg.as_str()), ("", "Not allowed"));
        assert!(format!("{}", explain(Some(401), &code, &msg)).contains("sign in again"));
        assert!(format!("{}", explain(Some(403), "subscription_sharing_user_not_eligible", "x")).contains("Plus or Pro"));
    }

    /// The desktop app's callback port: strays get a 404, an answer to someone else's sign-in is ignored, Cancel ends it.
    #[tokio::test]
    async fn the_callback_only_takes_this_sign_in() {
        let dir = std::env::temp_dir().join(format!("mimir-siwc-cb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Db::open(&dir.join("t.db")).unwrap();
        let url = url::Url::parse(&start(&db, true).await.unwrap()).unwrap();
        let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!((q["client_id"].as_str(), q["agent_name_hint"].as_str()), ("dynamic_agent_client", "Mimir"));
        assert_eq!(q["scope"], SCOPES);
        let back = q["redirect_uri"].clone();
        assert!(back.starts_with("http://127.0.0.1:") && back.ends_with("/auth/callback"), "{back}");
        let port = url::Url::parse(&back).unwrap().port().unwrap();
        assert_eq!(reqwest::get(format!("http://127.0.0.1:{port}/favicon.ico")).await.unwrap().status(), 404);
        assert_eq!(reqwest::get(format!("{back}?code=x&state=someone-elses")).await.unwrap().status(), 400);
        let waiting = tokio::spawn(finish());
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel();
        let err = tokio::time::timeout(Duration::from_secs(2), waiting).await.expect("finish returned").unwrap().unwrap_err();
        assert!(format!("{err:#}").contains("cancelled"), "{err:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_id_is_a_uuid_kept_for_good() {
        let dir = std::env::temp_dir().join(format!("mimir-siwc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Db::open(&dir.join("t.db")).unwrap();
        let id = host_id(&db).unwrap();
        let v = id.strip_prefix("urn:uuid:").unwrap();
        let parts: Vec<&str> = v.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12], "{v}");
        assert!(parts[2].starts_with('4') && "89ab".contains(&parts[3][..1]), "{v}");
        assert_eq!(host_id(&db).unwrap(), id);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
