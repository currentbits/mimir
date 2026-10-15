//! External MCP tool servers (e.g. the Playwright browser), proxied through our policy gate,
//! plus the credential vault. External tools are named `<server>__<tool>`; their lists are cached
//! in settings so building a prompt never starts a browser. A server process lives for one turn.
//! Remote servers (a URL) speak MCP's Streamable HTTP: POST each message, read JSON or SSE back. One that wants a
//! sign-in gets MCP's OAuth (sign_in below): Mimir finds its authorization server, registers itself there and signs in
//! with PKCE; the tokens live in the secret store under `oauth:mcp:<server>` (see oauth.rs).
//!
//! Vault: the model only ever writes a secret's NAME (e.g. GITHUB_PASSWORD); Playwright's
//! `--secrets` swaps in the value inside the browser and redacts it from responses. Our gate adds
//! the security: using a secret always needs approval, only on the secret's allowed sites, and we
//! redact values again before anything reaches the model or the run log.

use crate::db::Db;
use crate::tools::ToolCtx;
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

pub const BROWSER: &str = "browser";
/// A fixed version: it's handed saved logins to type, so it shouldn't change under you (and Mimir reads its output's
/// layout, e.g. the "### Page" section). Bump it deliberately.
const PLAYWRIGHT: &str = "@playwright/mcp@0.0.83";

/// Hosts the browser refuses, as Playwright URL globs (`*` is anything but `/`), so links, redirects and a page's own
/// requests can't reach this computer or the local network. The same ranges as tools::is_private_host.
/// ponytail: matches the URL's host, not where a name resolves (a public name pointing at 192.168.x.x still loads);
/// closing that needs a filtering proxy in front of the browser.
const LOCAL_ORIGINS: &str = "{localhost,*.localhost,*.local,*.internal}{,:*};0.*;10.*;127.*;169.254.*;192.168.*;\
    172.{16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31}.*;[::]{,:*};[::1]{,:*};[::ffff:*;[fc*;[fd*;[fe{8,9,a,b}*";

/// One-time Chromium download for the browser preset (quick no-op when already installed).
pub async fn install_browser() -> Result<()> {
    let npx = std::env::var("MIMIR_NPX").unwrap_or_else(|_| "npx".into());
    let out = tokio::process::Command::new(&npx)
        .args(["-y", PLAYWRIGHT, "install-browser", "chrome-for-testing"])
        .output()
        .await
        .with_context(|| format!("couldn't run {npx}; the browser needs Node.js"))?;
    if !out.status.success() {
        bail!("installing the browser failed: {}", crate::tools::truncate(&String::from_utf8_lossy(&out.stderr), 300));
    }
    Ok(())
}
const SEP: &str = "__";

/// A configured server: a command, a `url` (remote), or `preset: "playwright"` (the browser
/// command, built per bot at start). Env and header values live in the secret store.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
// Sent by the UI: a misspelled field is an error, not silently dropped. The saved list parses with this too, so a
// field removed later stays as an ignored one.
#[serde(deny_unknown_fields)]
pub struct ServerConf {
    pub name: String,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
}

impl ServerConf {
    /// The maps whose values belong in the secret store, by kind.
    fn secret_maps(&mut self) -> [(&'static str, &mut HashMap<String, String>); 2] {
        [("env", &mut self.env), ("header", &mut self.headers)]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Value,
    #[serde(default)]
    pub read_only: bool,
    /// Reaches things outside your computer (the web, a mail server): the server's openWorldHint, assumed when it
    /// doesn't say. A read-only tool like that can still carry data out in what it's asked to look up.
    #[serde(default = "yes")]
    pub open_world: bool,
}

fn yes() -> bool {
    true
}

/// A saved login: the value lives in the secret store, never in the database or a prompt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
// Like ServerConf: unknown fields are an error, here and in the saved list.
#[serde(deny_unknown_fields)]
pub struct VaultEntry {
    pub name: String,
    /// Sites it may be typed into (exact host or subdomain).
    pub domains: Vec<String>,
    /// Only this bot's (a login saved from its sign-in card); none = every bot's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot: Option<i64>,
    /// Has an authenticator key: Mimir types the current code for NAME_CODE.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub totp: bool,
}

pub fn servers(db: &Db) -> Vec<ServerConf> {
    db.setting("mcp_servers").ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn validate(s: &ServerConf) -> Result<()> {
    if s.name.is_empty() || !s.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') || s.name.contains(SEP) {
        bail!("server names are lowercase letters, digits and single underscores: {:?}", s.name);
    }
    if s.preset.is_none() && s.command.trim().is_empty() && s.url.is_empty() {
        bail!("server {} needs a command or a URL", s.name);
    }
    // Plain http only to this machine: header tokens would otherwise travel in the clear.
    if !(s.url.is_empty() || secure(&s.url)) {
        bail!("server {}'s URL must start with https:// (plain http only for localhost)", s.name);
    }
    Ok(())
}

pub fn set_servers(db: &Db, list: &[ServerConf]) -> Result<()> {
    list.iter().try_for_each(validate)?;
    db.set_setting("mcp_servers", &serde_json::to_string(list)?)
}

/// Where a tool server's env or header value lives: tokens belong in the secret store, not the database.
fn secret_name(kind: &str, server: &str, key: &str) -> String {
    format!("tool{kind}:{server}:{key}")
}

/// The env (or headers) a server starts with. Saved configs keep values in the secret store (the
/// config holds ""); a value still in the config (typed just now, or an older setup) is used as is.
fn values_of(server: &str, kind: &str, map: &HashMap<String, String>) -> Vec<(String, String)> {
    map.iter()
        .map(|(k, v)| (k.clone(), if v.is_empty() { crate::host::secret_get(&secret_name(kind, server, k)).unwrap_or_default() } else { v.clone() }))
        .collect()
}

/// Add or replace a tool server (not the browser). An empty env/header value keeps the one saved before.
/// The server is started once first to list its tools, so a broken setup is never saved.
pub async fn save_server(db: &Db, mut conf: ServerConf) -> Result<usize> {
    conf.name = conf.name.trim().to_string();
    if conf.name == BROWSER || conf.preset.is_some() {
        bail!("the browser is set up under Browser");
    }
    // No shell runs the command, so expand ~ ourselves.
    if let Some(home) = crate::user_home() {
        for a in conf.args.iter_mut() {
            if a == "~" || a.starts_with("~/") || a.starts_with("~\\") {
                *a = format!("{}{}", home.display(), &a[1..]);
            }
        }
    }
    validate(&conf)?;
    let n = refresh(db, &conf).await?;
    let mut list = servers(db);
    let mut old = list.iter().find(|s| s.name == conf.name).cloned().unwrap_or_default();
    let name = conf.name.clone();
    for ((kind, new), (_, old)) in conf.secret_maps().into_iter().zip(old.secret_maps()) {
        for k in old.keys().filter(|k| !new.contains_key(*k)) {
            crate::host::secret_set(&secret_name(kind, &name, k), "")?;
        }
        for (k, v) in new.iter_mut().filter(|(_, v)| !v.is_empty()) {
            crate::host::secret_set(&secret_name(kind, &name, k), v)?;
            v.clear();
        }
    }
    list.retain(|s| s.name != conf.name);
    list.push(conf);
    set_servers(db, &list)?;
    Ok(n)
}

pub fn delete_server(db: &Db, name: &str) -> Result<()> {
    let mut list = servers(db);
    for mut s in list.iter().filter(|s| s.name == name).cloned() {
        for (kind, map) in s.secret_maps() {
            map.keys().try_for_each(|k| crate::host::secret_set(&secret_name(kind, name, k), ""))?;
        }
    }
    crate::oauth::sign_out(&oauth_key(name))?;
    list.retain(|s| s.name != name);
    set_servers(db, &list)
}

// ---------------- signing in to a remote server (MCP authorization, 2025-06-18) ----------------

fn oauth_key(server: &str) -> String {
    format!("oauth:mcp:{server}")
}

/// Signed in to this server (it may have ended since; the next use says so).
pub fn signed_in(server: &str) -> bool {
    crate::oauth::load(&oauth_key(server)).is_some()
}

/// The server a sign-in in progress is for: saved once the sign-in is done.
static SIGNING_IN: std::sync::Mutex<Option<ServerConf>> = std::sync::Mutex::new(None);

/// https, or plain http to this computer only (a server of your own, a test). Parsed, not prefix-matched, so
/// `http://localhost.evil.com` and `http://localhost@evil.com` aren't this computer.
fn secure(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    match (u.scheme(), u.host()) {
        ("https", Some(_)) => true,
        ("http", Some(url::Host::Domain(d))) => d == "localhost",
        ("http", Some(url::Host::Ipv4(ip))) => ip.is_loopback(),
        ("http", Some(url::Host::Ipv6(ip))) => ip.is_loopback(),
        _ => false,
    }
}

async fn get_json(http: &reqwest::Client, url: &str) -> Option<Value> {
    if !secure(url) {
        return None;
    }
    let r = http.get(url).header("Accept", "application/json").send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.json().await.ok()
}

/// Where the metadata for `base` lives under a well-known name: `https://h/.well-known/<name><path>` (RFC 8414, 9728).
fn well_known(base: &url::Url, name: &str) -> String {
    let path = base.path().trim_end_matches('/');
    format!("{}/.well-known/{name}{path}", base.origin().ascii_serialization())
}

/// Begin signing in to a remote server (one that answered 401): find its authorization server from the server's
/// metadata, register Mimir as an app there (dynamic client registration), and return the address to open.
/// `listen`: the browser comes back to this computer (desktop); else the person pastes the address (a server).
pub async fn sign_in(conf: ServerConf, listen: bool) -> Result<String> {
    validate(&conf)?;
    if conf.url.is_empty() {
        bail!("only a server with a URL signs in");
    }
    // Metadata comes from the server: its redirects can't bounce Mimir onto this computer or the local network,
    // unless the server itself is there.
    let mut http = reqwest::Client::builder().timeout(Duration::from_secs(20));
    if !crate::tools::is_private_host(&conf.url) {
        http = http.redirect(crate::tools::public_redirects());
    }
    let http = http.build()?;
    let server = url::Url::parse(&conf.url)?;
    // The server's 401 says where its metadata is; else the well-known places (path first, then the root).
    let probe = http
        .post(&conf.url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .body(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": {"name": "mimir", "version": env!("CARGO_PKG_VERSION")}}}).to_string())
        .send()
        .await
        .with_context(|| format!("couldn't reach {}", conf.url))?;
    let hinted = probe.headers().get_all(reqwest::header::WWW_AUTHENTICATE).iter().filter_map(|v| v.to_str().ok()).find_map(resource_metadata);
    let mut places: Vec<String> = hinted.into_iter().collect();
    places.push(well_known(&server, "oauth-protected-resource"));
    places.push(format!("{}/.well-known/oauth-protected-resource", server.origin().ascii_serialization()));
    let mut prm = None;
    for p in places {
        if let Some(v) = get_json(&http, &p).await.filter(|v| v["authorization_servers"].is_array()) {
            prm = Some(v);
            break;
        }
    }
    // An older server (spec 2025-03-26) is its own authorization server.
    let issuer = prm.as_ref().and_then(|v| v["authorization_servers"][0].as_str().map(String::from)).unwrap_or_else(|| server.origin().ascii_serialization());
    let issuer = url::Url::parse(&issuer).context("the server names an authorization server that isn't a URL")?;
    let root = issuer.origin().ascii_serialization();
    let path = issuer.path().trim_end_matches('/');
    let mut meta = None;
    for p in [well_known(&issuer, "oauth-authorization-server"), well_known(&issuer, "openid-configuration"), format!("{root}{path}/.well-known/openid-configuration")] {
        if let Some(v) = get_json(&http, &p).await.filter(|v| v["authorization_endpoint"].is_string() && v["token_endpoint"].is_string()) {
            meta = Some(v);
            break;
        }
    }
    let meta = meta.with_context(|| format!("{} wants a sign-in, but doesn't say where (no OAuth metadata); give it a token in a header instead", conf.name))?;
    let (authorize, token) = (meta["authorization_endpoint"].as_str().unwrap_or(""), meta["token_endpoint"].as_str().unwrap_or("").to_string());
    if !secure(authorize) || !secure(&token) {
        bail!("{}'s sign-in addresses must start with https://", conf.name);
    }
    if meta["code_challenge_methods_supported"].as_array().is_some_and(|m| !m.iter().any(|x| x == "S256")) {
        bail!("{}'s sign-in doesn't support PKCE (S256), which Mimir needs", conf.name);
    }
    let register = meta["registration_endpoint"].as_str().filter(|r| secure(r)).with_context(|| {
        format!("{} doesn't let apps register themselves for a sign-in; make a token in its settings and add it as a header (Authorization: Bearer …)", conf.name)
    })?;
    let redirect = crate::oauth::redirect(listen, "127.0.0.1", "/callback").await?;
    let reg: Value = http
        .post(register)
        .json(&json!({"client_name": "Mimir", "redirect_uris": [redirect.uri], "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"], "token_endpoint_auth_method": "none"}))
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("{} didn't let Mimir register for a sign-in", conf.name))?
        .json()
        .await?;
    let client_id = reg["client_id"].as_str().context("the registration gave no client id")?.to_string();
    let scope = prm.as_ref().and_then(|v| v["scopes_supported"].as_array()).map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty());
    let resource = prm.as_ref().and_then(|v| v["resource"].as_str().map(String::from)).unwrap_or_else(|| conf.url.clone());
    // A token is only ever asked for this server: metadata naming another site is refused (RFC 9728, 3.3).
    if url::Url::parse(&resource).map(|r| r.origin()) != Ok(server.origin()) {
        bail!("{}'s sign-in metadata is for {resource}, not this server", conf.name);
    }
    let saved = crate::oauth::Saved { token_url: token, client_id, client_secret: reg["client_secret"].as_str().map(String::from), scope, resource: Some(resource), refresh: String::new() };
    let url = crate::oauth::begin(&oauth_key(&conf.name), &conf.name, authorize, saved, redirect, &[])?;
    *SIGNING_IN.lock().unwrap() = Some(conf);
    Ok(url)
}

/// `resource_metadata="…"` from a WWW-Authenticate header.
fn resource_metadata(h: &str) -> Option<String> {
    let i = h.find("resource_metadata=")? + "resource_metadata=".len();
    let rest = h[i..].trim_start_matches('"');
    Some(rest.split(['"', ',']).next()?.trim().to_string()).filter(|s| !s.is_empty())
}

/// The sign-in to a tool server is in progress (not another kind).
pub fn signing_in() -> bool {
    crate::oauth::pending_key().is_some_and(|k| k.starts_with("oauth:mcp:"))
}

/// After the sign-in: save the server (listing its tools with the new token). Returns its tool count.
pub async fn signed_in_save(db: &Db) -> Result<usize> {
    let conf = SIGNING_IN.lock().unwrap().take().context("no tool server sign-in in progress")?;
    save_server(db, conf).await
}

pub fn cached_tools(db: &Db, server: &str) -> Vec<ExtTool> {
    db.setting(&format!("mcp_tools:{server}")).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// (full name, tool) for every cached external tool of every configured server.
pub fn all_tools(db: &Db) -> Vec<(String, ExtTool)> {
    servers(db).into_iter().flat_map(|s| cached_tools(db, &s.name).into_iter().map(move |t| (format!("{}{SEP}{}", s.name, t.name), t))).collect()
}

pub fn split(full: &str) -> Option<(&str, &str)> {
    full.split_once(SEP)
}

pub fn find(db: &Db, full: &str) -> Option<ExtTool> {
    let (server, tool) = split(full)?;
    servers(db).iter().any(|s| s.name == server).then(|| cached_tools(db, server).into_iter().find(|t| t.name == tool)).flatten()
}

/// Built-in rule for an external tool. Browser: scripting the page is denied (it could read a
/// typed password back out), navigating asks (the main way data could leak to another site).
pub fn default_action(full: &str, read_only: bool) -> &'static str {
    match split(full) {
        Some((BROWSER, "browser_evaluate" | "browser_run_code_unsafe" | "browser_file_upload")) => "deny",
        Some((BROWSER, "browser_navigate")) => "ask",
        _ if read_only => "allow",
        Some((BROWSER, _)) => "allow",
        _ => "ask",
    }
}

// ---------------- vault ----------------

pub fn vault(db: &Db) -> Vec<VaultEntry> {
    db.setting("vault").ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// The saved logins a bot may use: everyone's, and its own.
pub fn vault_for(db: &Db, bot: i64) -> Vec<VaultEntry> {
    vault(db).into_iter().filter(|e| e.bot.is_none_or(|b| b == bot)).collect()
}

/// The site a sign-in is kept for: the registrable domain of a page's address ("accounts.google.com" → google.com,
/// while alice.github.io stays alice.github.io, since anyone can have a github.io site). None for an IP or a bare name.
pub fn site_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let Some(url::Host::Domain(host)) = u.host() else { return None };
    let host = host.trim_end_matches('.').to_lowercase();
    psl::domain_str(&host).filter(|d| d.contains('.')).map(String::from)
}

/// What a sign-in card's form warns about: the page's full address next to the site the login is kept for; whether the
/// bot came there after outside content or on someone else's words (`steered`); and, for a site with no saved login or
/// earlier sign-in, one that has them which this one looks like (`lookalike`); and an address with letters from
/// another alphabet (`foreign`: always, naming what it's made to look like when that's one of them).
pub fn signin_warnings(db: &Db, url: &str, site: &str, steered: bool) -> Value {
    let mut known: Vec<String> = vault(db).iter().flat_map(|e| e.domains.iter()).filter_map(|d| site_of(&format!("https://{d}/"))).collect();
    known.extend(db.signin_sites().unwrap_or_default());
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_lowercase)).unwrap_or_default();
    let foreign = crate::lookalike::foreign(&host);
    let lookalike = match known.iter().any(|k| k == site) {
        true => None,
        false => crate::lookalike::skeleton_of(site, known.iter().map(String::as_str)).or_else(|| crate::lookalike::of(site, known.iter().map(String::as_str))),
    };
    json!({"host": host, "site": site, "steered": steered, "lookalike": lookalike, "foreign": foreign})
}

/// The authenticator code of a saved login right now, if it has a key.
pub(crate) fn vault_code(name: &str) -> Option<String> {
    crate::host::secret_get(&format!("vault:{name}:totp")).and_then(|k| crate::totp::parse(&k).ok()).map(|t| t.now())
}

/// `totp`: an authenticator setup key or otpauth:// link to add (None or empty keeps what's there).
pub fn vault_save(db: &Db, entry: VaultEntry, value: Option<&str>, totp: Option<&str>) -> Result<()> {
    let name = entry.name.trim().to_string();
    if name.len() < 3 || !name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        bail!("use a name like GITHUB_PASSWORD (capitals, digits, underscores)");
    }
    let domains: Vec<String> = entry.domains.iter().map(|d| d.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_lowercase()).filter(|d| !d.is_empty()).collect();
    if domains.is_empty() {
        bail!("add at least one site this login may be used on");
    }
    let mut list = vault(db);
    let old = list.iter().find(|e| e.name == name).cloned();
    let totp = totp.map(str::trim).filter(|t| !t.is_empty());
    if let Some(t) = totp {
        crate::totp::parse(t)?;
    }
    if let Some(v) = value.filter(|v| !v.is_empty()) {
        crate::host::secret_set(&format!("vault:{name}"), v)?;
    } else if old.is_none() {
        bail!("a new login needs its secret value");
    }
    if let Some(t) = totp {
        crate::host::secret_set(&format!("vault:{name}:totp"), t)?;
    }
    list.retain(|e| e.name != name);
    // Changing a login in Settings keeps whose it is.
    let (bot, had) = old.map_or((None, false), |o| (o.bot, o.totp));
    list.push(VaultEntry { name, domains, bot: entry.bot.or(bot), totp: had || totp.is_some() });
    db.set_setting("vault", &serde_json::to_string(&list)?)
}

pub fn vault_delete(db: &Db, name: &str) -> Result<()> {
    let mut list = vault(db);
    list.retain(|e| e.name != name);
    for k in ["", ":user", ":totp"] {
        crate::host::secret_set(&format!("vault:{name}{k}"), "")?;
    }
    db.set_setting("vault", &serde_json::to_string(&list)?)
}

/// What a sign-in card saves: the login for one bot on one site, under a name like GITHUB_COM_BOT3.
pub fn vault_save_signin(db: &Db, bot: i64, site: &str, user: &str, password: &str, totp: Option<&str>) -> Result<String> {
    let name = format!("{}_BOT{bot}", site.to_uppercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_"));
    vault_save(db, VaultEntry { name: name.clone(), domains: vec![site.to_string()], bot: Some(bot), totp: false }, Some(password), totp)?;
    crate::host::secret_set(&format!("vault:{name}:user"), user)?;
    Ok(name)
}

/// Every NAME_CODE in the arguments, as the code it stands for.
fn fill_codes(v: &Value, codes: &[(String, String)]) -> Value {
    match v {
        Value::String(s) => Value::String(codes.iter().fold(s.clone(), |s, (n, c)| s.replace(n.as_str(), c))),
        Value::Array(a) => a.iter().map(|x| fill_codes(x, codes)).collect(),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), fill_codes(x, codes))).collect()),
        x => x.clone(),
    }
}

/// Every string anywhere in the arguments.
pub(crate) fn strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
        Value::Object(o) => o.values().for_each(|x| strings(x, out)),
        _ => {}
    }
}

/// Vault names a call refers to (any argument containing the name counts).
pub fn secrets_used(entries: &[VaultEntry], args: &Value) -> Vec<VaultEntry> {
    let mut all = vec![];
    strings(args, &mut all);
    entries.iter().filter(|e| all.iter().any(|s| s.contains(&e.name))).cloned().collect()
}

pub fn host_allowed(url: &str, domains: &[String]) -> bool {
    // The host the browser will really go to (a URL parser, not text before the first ':'), and none with a user name
    // or password in front of it, which can make "github.com:x@evil.example" look like github.com.
    let Ok(u) = url::Url::parse(url) else { return false };
    if !u.username().is_empty() || u.password().is_some() {
        return false;
    }
    let Some(host) = u.host_str() else { return false };
    let host = host.to_lowercase();
    domains.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// Replace secret values in text the model or the run log will see.
pub fn redact(text: &str, values: &[(String, String)]) -> String {
    values.iter().filter(|(_, v)| v.len() >= 4).fold(text.to_string(), |t, (n, v)| t.replace(v.as_str(), &format!("<secret>{n}</secret>")))
}

pub(crate) fn vault_values(db: &Db) -> Vec<(String, String)> {
    vault(db).into_iter().filter_map(|e| crate::host::secret_get(&format!("vault:{}", e.name)).map(|v| (e.name, v))).collect()
}

// ---------------- MCP client (stdio: newline-delimited JSON-RPC; remote: Streamable HTTP) ----------------

const PROTOCOL: &str = "2025-06-18";
const TIMEOUT: Duration = Duration::from_secs(180);

pub struct Client {
    io: Io,
    next: i64,
}

#[allow(clippy::large_enum_variant)] // one per running tool server; boxing buys nothing
enum Io {
    Stdio { _child: Child, stdin: ChildStdin, out: BufReader<ChildStdout> },
    Http(Http),
}

struct Http {
    client: reqwest::Client,
    url: String,
    headers: Vec<(String, String)>,
    session: Option<String>,
    version: Option<String>,
    /// Signed in (oauth.rs): the key its tokens live under, and the server's name for messages.
    oauth: Option<(String, String)>,
}

impl Client {
    async fn start(conf: &ServerConf, db: &Db, bot: i64) -> Result<Self> {
        let (io, secrets_file) = if conf.url.is_empty() {
            // The browser preset is pinned (PLAYWRIGHT); a command that fetches a package is checked first.
            if conf.preset.is_none() {
                let osv = std::env::var("MIMIR_OSV_API").unwrap_or_else(|_| "https://api.osv.dev".into());
                malware_check(db, &osv, &conf.name, &conf.command, &conf.args).await?;
            }
            // Off the async threads: bringing up the bots' computer can take seconds.
            let (conf, db) = (conf.clone(), db.clone());
            tokio::task::spawn_blocking(move || spawn(&conf, &db, bot)).await??
        } else {
            let headers = values_of(&conf.name, "header", &conf.headers);
            // A token typed in as a header wins over a sign-in.
            let key = oauth_key(&conf.name);
            let oauth = (signed_in(&conf.name) && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("authorization"))).then(|| (key, conf.name.clone()));
            (Io::Http(Http { client: reqwest::Client::builder().timeout(TIMEOUT).build()?, url: conf.url.clone(), headers, session: None, version: None, oauth }), None)
        };
        let mut c = Client { io, next: 0 };
        let init = c.request("initialize", json!({"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": {"name": "mimir", "version": env!("CARGO_PKG_VERSION")}})).await;
        // The server has read its secrets by now; don't leave them on disk.
        if let Some(f) = secrets_file {
            let _ = std::fs::remove_file(f);
        }
        let init = init?;
        if let Io::Http(h) = &mut c.io {
            h.version = init["protocolVersion"].as_str().map(Into::into);
        }
        c.notify("notifications/initialized").await?;
        Ok(c)
    }

    async fn notify(&mut self, method: &str) -> Result<()> {
        let msg = json!({"jsonrpc": "2.0", "method": method});
        match &mut self.io {
            Io::Stdio { stdin, .. } => {
                stdin.write_all(format!("{msg}\n").as_bytes()).await?;
                Ok(stdin.flush().await?)
            }
            Io::Http(h) => h.post(&msg).await.map(drop),
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let read = async {
            match &mut self.io {
                Io::Stdio { stdin, out, .. } => {
                    stdin.write_all(format!("{msg}\n").as_bytes()).await?;
                    stdin.flush().await?;
                    while let Some(line) = capped_line(out).await? {
                        if let Some(r) = reply(&line, id, method) {
                            return r;
                        }
                    }
                    bail!("tool server exited")
                }
                Io::Http(h) => {
                    let mut res = h.post(&msg).await?;
                    let sse = res.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|t| t.starts_with("text/event-stream"));
                    if !sse {
                        let mut body = vec![];
                        while let Some(chunk) = res.chunk().await? {
                            body.extend_from_slice(&chunk);
                            if body.len() > MAX_REPLY {
                                bail!("{method}: the server's answer is too big");
                            }
                        }
                        return reply(&String::from_utf8_lossy(&body), id, method).unwrap_or_else(|| bail!("{method}: no answer from the server"));
                    }
                    // ponytail: no resuming a dropped stream (Last-Event-ID); the call just fails.
                    let mut buf = vec![];
                    while let Some(chunk) = res.chunk().await? {
                        buf.extend_from_slice(&chunk);
                        if buf.len() > MAX_REPLY {
                            bail!("{method}: the server's answer is too big");
                        }
                        // Events end at a blank line: no need to look again until a line ends.
                        if !chunk.contains(&b'\n') {
                            continue;
                        }
                        if let Some(r) = sse_events(&mut buf).iter().find_map(|d| reply(d, id, method)) {
                            return r;
                        }
                    }
                    bail!("{method}: the server's stream ended without an answer")
                }
            }
        };
        tokio::time::timeout(TIMEOUT, read).await.context("tool server timed out")?
    }
}

/// The most a tool server may send for one answer: a server that keeps sending can't fill up memory.
const MAX_REPLY: usize = 16 * 1024 * 1024;

/// One line from a tool server, never more than MAX_REPLY (None at the end of its output).
async fn capped_line(r: &mut BufReader<ChildStdout>) -> Result<Option<String>> {
    use tokio::io::AsyncBufReadExt;
    let mut line = vec![];
    loop {
        let buf = r.fill_buf().await?;
        if buf.is_empty() {
            return Ok((!line.is_empty()).then(|| String::from_utf8_lossy(&line).into_owned()));
        }
        match buf.iter().position(|b| *b == b'\n') {
            Some(i) => {
                line.extend_from_slice(&buf[..i]);
                r.consume(i + 1);
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            None => {
                let n = buf.len();
                line.extend_from_slice(buf);
                r.consume(n);
                if line.len() > MAX_REPLY {
                    bail!("the tool server sent too much at once");
                }
            }
        }
    }
}

/// The answer to request `id`, if this JSON-RPC message is it (otherwise a notification, log, or noise).
fn reply(text: &str, id: i64, method: &str) -> Option<Result<Value>> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v["id"] != id {
        return None;
    }
    Some(if v["error"].is_null() { Ok(v["result"].clone()) } else { Err(anyhow!("{method}: {}", v["error"]["message"].as_str().unwrap_or("error"))) })
}

/// Take the complete events off the front of an SSE buffer: each one's data (lines joined by \n).
/// Events without data (comments, pings) are skipped; a partial event stays in the buffer.
/// ponytail: rescans the whole buffer per chunk, quadratic for a multi-MB event; keep a scan offset if that shows up.
fn sse_events(buf: &mut Vec<u8>) -> Vec<String> {
    let mut out = vec![];
    while let Some((end, next)) = (0..buf.len()).find_map(|i| [&b"\r\n\r\n"[..], b"\n\n", b"\r\r"].into_iter().find(|s| buf[i..].starts_with(s)).map(|s| (i, i + s.len()))) {
        let event = String::from_utf8_lossy(&buf[..end]).into_owned();
        buf.drain(..next);
        let data: Vec<&str> = event.lines().filter_map(|l| l.strip_prefix("data:")).map(|d| d.strip_prefix(' ').unwrap_or(d)).collect();
        if !data.is_empty() {
            out.push(data.join("\n"));
        }
    }
    out
}

impl Http {
    /// POST one message. Keeps the session id the server hands out on initialize.
    async fn post(&mut self, msg: &Value) -> Result<reqwest::Response> {
        let mut retried = false;
        let res = loop {
            let mut req = self.client.post(&self.url).header("Accept", "application/json, text/event-stream").header("Content-Type", "application/json").body(msg.to_string());
            for (k, v) in &self.headers {
                req = req.header(k, v);
            }
            if let Some((key, name)) = &self.oauth {
                req = req.bearer_auth(crate::oauth::access_token(key, name).await?);
            }
            if let Some(s) = &self.session {
                req = req.header("Mcp-Session-Id", s);
            }
            if let Some(v) = &self.version {
                req = req.header("MCP-Protocol-Version", v);
            }
            let res = req.send().await.with_context(|| format!("couldn't reach {}", self.url))?;
            // The access token was refused before it ran out (revoked, rotated): refresh once and try again.
            if res.status() == reqwest::StatusCode::UNAUTHORIZED
                && !retried
                && let Some((key, _)) = &self.oauth
            {
                crate::oauth::forget_access(key);
                retried = true;
                continue;
            }
            break res;
        };
        if let Some(s) = res.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            self.session = Some(s.into());
        }
        match res.status() {
            s if s.is_success() => Ok(res),
            s @ reqwest::StatusCode::UNAUTHORIZED if self.oauth.is_some() => bail!("{} refused the sign-in ({s}); sign in again in Settings → Tool servers", self.url),
            // The UI offers Sign in on "wants you to sign in".
            s @ reqwest::StatusCode::UNAUTHORIZED => bail!("{} wants you to sign in ({s}), or a token in a header (Authorization: Bearer …)", self.url),
            s @ reqwest::StatusCode::FORBIDDEN => bail!("{} refused the request ({s}); the token or sign-in doesn't allow it", self.url),
            s => bail!("{} answered {s}: {}", self.url, crate::tools::truncate(&res.text().await.unwrap_or_default(), 300)),
        }
    }
}

/// Start a stdio server. Returns the browser's secrets file too, to delete once it has started.
fn spawn(conf: &ServerConf, db: &Db, bot: i64) -> Result<(Io, Option<std::path::PathBuf>)> {
    let dir = db.dir().join("browser");
    let mut env = values_of(&conf.name, "env", &conf.env);
    let (command, args, secrets_file) = match conf.preset.as_deref() {
        Some("playwright") => {
            std::fs::create_dir_all(&dir)?;
            let mine: Vec<String> = vault_for(db, bot).into_iter().map(|e| e.name).collect();
            let file = write_secrets_file(&dir, &vault_values(db).into_iter().filter(|(n, _)| mine.contains(n)).collect::<Vec<_>>())?;
            let mut args: Vec<String> = vec!["-y".into(), PLAYWRIGHT.into()];
            if crate::computer::usable(db) {
                // The bots' computer is on: drive the Chromium on its screen, so you can watch (and take over).
                crate::computer::start(&db.dir(), (bot > 0).then_some(bot))?;
                args.extend(["--cdp-endpoint".into(), crate::computer::cdp_endpoint()?]);
                // A computer service wants its token on DevTools too: in the environment, not the command line `ps` shows.
                if let Some(h) = crate::computer::cdp_auth()? {
                    env.push(("PLAYWRIGHT_MCP_CDP_HEADERS".into(), format!("Authorization: {h}")));
                }
            } else {
                // Bundled Chromium (see install_browser), so Google Chrome needn't be installed.
                args.extend(["--headless".into(), "--browser".into(), "chromium".into()]);
                // A profile per bot keeps its logins between turns.
                args.extend(["--user-data-dir".into(), dir.join(format!("profile-{bot}")).display().to_string()]);
            }
            args.extend(["--blocked-origins".into(), LOCAL_ORIGINS.into()]);
            args.extend(["--output-dir".into(), dir.join("out").display().to_string()]);
            args.extend(["--secrets".into(), file.display().to_string()]);
            (std::env::var("MIMIR_NPX").unwrap_or_else(|_| "npx".into()), args, Some(file))
        }
        Some(p) => bail!("unknown preset {p}"),
        None => (conf.command.clone(), conf.args.clone(), None),
    };
    let mut child = tokio::process::Command::new(&command)
        .args(&args)
        .envs(env)
        .current_dir(db.dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("couldn't start tool server {} ({command})", conf.name))?;
    let io = Io::Stdio { stdin: child.stdin.take().context("no stdin")?, out: BufReader::new(child.stdout.take().context("no stdout")?), _child: child };
    Ok((io, secrets_file))
}

fn write_secrets_file(dir: &std::path::Path, values: &[(String, String)]) -> Result<std::path::PathBuf> {
    use std::io::Write;
    let path = dir.join(format!(".secrets-{}.env", crate::random_hex(8)?));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&path)?;
    for (k, v) in values {
        // dotenv: double-quoted with escapes, so any password survives.
        writeln!(f, "{k}=\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))?;
    }
    Ok(path)
}

/// The registry package a server's command fetches and runs (`npx -y pkg@1.2.3`, `uvx pkg`, `pipx run pkg`…):
/// (OSV ecosystem, name, exact version if pinned). None for anything else (a local program, a path, a git URL).
fn package_of(command: &str, args: &[String]) -> Option<(&'static str, String, Option<String>)> {
    let tool = std::path::Path::new(command).file_stem()?.to_str()?.to_ascii_lowercase();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    // The runner, the args after it, and its flags that name the package or take a value.
    let (eco, rest, pkg_flags, value_flags): (_, &[&str], &[&str], &[&str]) = match (tool.as_str(), a.as_slice()) {
        ("npx" | "bunx", r) => ("npm", r, &["-p", "--package"], &[]),
        ("npm", ["exec" | "x", r @ ..]) => ("npm", r, &["-p", "--package"], &["-w", "--workspace", "-c", "--call"]),
        ("pnpm" | "yarn", ["dlx", r @ ..]) | ("bun", ["x", r @ ..]) => ("npm", r, &["-p", "--package"], &[]),
        ("uvx", r) | ("uv", ["tool", "run", r @ ..]) => ("PyPI", r, &["--from"], &["--with", "-w", "--python", "-p", "--index", "--index-url", "--extra-index-url", "--with-requirements"]),
        ("pipx", ["run", r @ ..]) => ("PyPI", r, &["--spec"], &["--python", "--index-url", "--pip-args"]),
        _ => return None,
    };
    let mut spec = None;
    let mut i = 0;
    while i < rest.len() {
        let (flag, inline) = rest[i].split_once('=').map_or((rest[i], None), |(f, v)| (f, Some(v)));
        if pkg_flags.contains(&flag) || value_flags.contains(&flag) {
            let v = inline.or_else(|| rest.get(i + 1).copied());
            i += if inline.is_some() { 1 } else { 2 };
            if pkg_flags.contains(&flag) {
                spec = v;
                break;
            }
        } else if flag.starts_with('-') {
            i += 1;
        } else {
            spec = Some(rest[i]);
            break;
        }
    }
    let spec = spec?;
    if spec.is_empty() || spec.starts_with(['.', '/', '~', '\\']) || spec.contains(':') || (eco == "PyPI" && spec.contains('/')) {
        return None;
    }
    let pinned = |v: &str| (v.starts_with(|c: char| c.is_ascii_digit()) && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c))).then(|| v.to_string());
    if eco == "npm" {
        // A scoped name starts with @: the version's @ is the one after it.
        let at = spec[1..].find('@').map(|i| i + 1);
        let (name, version) = at.map_or((spec, None), |i| (&spec[..i], pinned(&spec[i + 1..])));
        return Some((eco, name.to_lowercase(), version));
    }
    let (name, version) = spec.split_once("==").or_else(|| spec.split_once('@')).map_or((spec, None), |(n, v)| (n, pinned(v.trim())));
    // Drop extras and any other specifier (`pkg[cli]>=1`); PyPI names compare normalized (PEP 503).
    let name = name.split(['[', '<', '>', '=', '!', '~', ';', ' ']).next().unwrap_or(name);
    Some((eco, name.to_lowercase().replace(['_', '.'], "-"), version))
}

/// The malicious-package advisory (OSV `MAL-…`) in an OSV query answer, if any.
fn malicious(answer: &Value) -> Option<String> {
    answer["vulns"].as_array()?.iter().find_map(|v| {
        let id = v["id"].as_str().unwrap_or("");
        let alias = v["aliases"].as_array().into_iter().flatten().filter_map(Value::as_str).find(|a| a.starts_with("MAL-"));
        if id.starts_with("MAL-") { Some(id.to_string()) } else { alias.map(|a| format!("{a} ({id})")) }
    })
}

/// How long a clean verdict holds before asking OSV again; a flagged one holds for good.
const OSV_FRESH: i64 = 24 * 3600;

/// Refuse to start a server whose package OSV.dev lists as malware. Verdicts are cached per package@version in
/// settings; when OSV can't be reached the server starts anyway (logged).
/// ponytail: an unpinned package is refused if any of its versions was ever flagged; pinning a clean one gets past that.
async fn malware_check(db: &Db, osv: &str, server: &str, command: &str, args: &[String]) -> Result<()> {
    let Some((eco, name, version)) = package_of(command, args) else { return Ok(()) };
    let shown = format!("{name}{}", version.as_deref().map(|v| format!("@{v}")).unwrap_or_default());
    let key = format!("osv:{eco}:{shown}");
    let cached = db.setting(&key)?.unwrap_or_default();
    let verdict = match cached.strip_prefix("ok:").and_then(|t| t.parse::<i64>().ok()) {
        Some(t) if crate::db::now() - t < OSV_FRESH => None,
        _ if cached.starts_with("MAL-") => Some(cached),
        _ => {
            let mut package = json!({"package": {"name": name, "ecosystem": eco}});
            if let Some(v) = &version {
                package["version"] = json!(v);
            }
            let asked = async {
                let res = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?.post(format!("{osv}/v1/query")).json(&package).send().await?.error_for_status()?;
                anyhow::Ok(res.json::<Value>().await?)
            };
            match asked.await {
                Ok(answer) => {
                    let found = malicious(&answer);
                    db.set_setting(&key, &found.clone().unwrap_or_else(|| format!("ok:{}", crate::db::now())))?;
                    found
                }
                Err(e) => {
                    eprintln!("tool server {server}: couldn't check {shown} for malware on OSV.dev, starting it anyway: {e:#}");
                    None
                }
            }
        }
    };
    match verdict {
        Some(id) => bail!("didn't start tool server {server}: its package {shown} is listed as malware ({id}, https://osv.dev/vulnerability/{})", id.split(' ').next().unwrap_or(&id)),
        None => Ok(()),
    }
}

/// Start a server once to (re)fill its tool cache. Called when servers are configured.
pub async fn refresh(db: &Db, conf: &ServerConf) -> Result<usize> {
    let mut c = Client::start(conf, db, 0).await?;
    let list = c.request("tools/list", json!({})).await?;
    let tools: Vec<ExtTool> = list["tools"]
        .as_array()
        .context("no tools")?
        .iter()
        .map(|t| ExtTool {
            name: t["name"].as_str().unwrap_or("").into(),
            description: t["description"].as_str().unwrap_or("").into(),
            input_schema: t["inputSchema"].clone(),
            read_only: t["annotations"]["readOnlyHint"].as_bool().unwrap_or(false),
            open_world: t["annotations"]["openWorldHint"].as_bool().unwrap_or(true),
        })
        .filter(|t| !t.name.is_empty())
        .collect();
    db.set_setting(&format!("mcp_tools:{}", conf.name), &serde_json::to_string(&tools)?)?;
    Ok(tools.len())
}

/// Run one external tool (after the policy gate). Starts the server on first use in this turn.
pub async fn call(ctx: &ToolCtx, full: &str, args: &Value) -> Result<String> {
    let (server, tool) = split(full).context("not an external tool")?;
    let conf = servers(&ctx.db).into_iter().find(|s| s.name == server).with_context(|| format!("tool server {server} is not configured"))?;

    let used = secrets_used(&vault_for(&ctx.db, ctx.bot_id), args);
    // Authenticator codes change every 30 seconds: worked out now, after the checks below, typed for NAME_CODE.
    let mut codes = vec![];
    if !used.is_empty() {
        if server != BROWSER {
            bail!("saved logins can only be typed into the browser");
        }
        // Where it'll be typed: a navigate's own target, else the page the browser reported last.
        let url = if tool == "browser_navigate" { args["url"].as_str().unwrap_or("").to_string() } else { ctx.page_url.lock().unwrap().clone().unwrap_or_default() };
        for e in &used {
            if !host_allowed(&url, &e.domains) {
                bail!("{} may only be used on {} (the page is {})", e.name, e.domains.join(", "), if url.is_empty() { "unknown" } else { &url });
            }
        }
        let names: Vec<&str> = used.iter().map(|e| e.name.as_str()).collect();
        crate::tools::wait_for_approval_with(ctx, full, &format!("Use saved login {} on {url}", names.join(", ")), "login", None).await?;
        codes = used.iter().filter(|e| e.totp).filter_map(|e| vault_code(&e.name).map(|c| (format!("{}_CODE", e.name), c))).collect();
    }
    let args = &fill_codes(args, &codes);

    if server == BROWSER && crate::computer::usable(&ctx.db) {
        // On the bots' computer: hold it like its own tools do, in this bot's browser profile.
        crate::tools::hold_computer(ctx).await?;
        let (home, bot) = (ctx.db.dir(), ctx.bot_id);
        tokio::task::spawn_blocking(move || crate::computer::start(&home, Some(bot))).await??;
    }
    let mut sessions = ctx.ext.lock().await;
    if !sessions.contains_key(server) {
        sessions.insert(server.to_string(), Client::start(&conf, &ctx.db, ctx.bot_id).await?);
    }
    let client = sessions.get_mut(server).expect("just inserted");
    let res = client.request("tools/call", json!({"name": tool, "arguments": args})).await?;
    let text: String = res["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");
    // Only from the browser's own "### Page" section: a page can print "Page URL: …" into a console message or its
    // text (which come first in the result) to pass for a site a saved login is allowed on.
    if let Some(url) = text.split("\n### Page").nth(1).and_then(|page| page.lines().find_map(|l| l.trim().trim_start_matches("- ").strip_prefix("Page URL: "))) {
        *ctx.page_url.lock().unwrap() = Some(url.trim().to_string());
    }
    let text = redact(&text, &[vault_values(&ctx.db), codes].concat());
    if res["isError"].as_bool() == Some(true) {
        bail!("{}", crate::tools::truncate(&text, 2000));
    }
    Ok(crate::tools::truncate(&text, 20_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_url_only_from_the_page_section() {
        let page = |text: &str| text.split("\n### Page").nth(1).and_then(|p| p.lines().find_map(|l| l.trim().trim_start_matches("- ").strip_prefix("Page URL: ")).map(String::from));
        let spoof = "### Result\nx\nPage URL: https://github.com/login\n### Page\n- Page URL: https://evil.example/\n- Page Title: Evil";
        assert_eq!(page(spoof).as_deref(), Some("https://evil.example/"));
        assert_eq!(page("### Result\nPage URL: https://github.com/login"), None, "no page section, no page");
    }

    #[test]
    fn signin_logins_stay_on_their_site_and_bot() {
        assert_eq!(site_of("https://accounts.google.com/signin?x=1").as_deref(), Some("google.com"));
        assert_eq!(site_of("https://www.bbc.co.uk/account").as_deref(), Some("bbc.co.uk"));
        assert_eq!(site_of("https://alice.github.io/login").as_deref(), Some("alice.github.io"), "a github.io site is someone's own");
        assert_eq!(site_of("http://192.168.1.10/login"), None);
        assert_eq!(site_of("http://localhost:8080/"), None);
        assert_eq!(site_of("about:blank"), None);
        let site = |u: &str| vec![site_of(u).unwrap()];
        assert!(host_allowed("https://mail.google.com/", &site("https://accounts.google.com/")));
        assert!(!host_allowed("https://google.com.evil.example/", &site("https://accounts.google.com/")));
        assert!(!host_allowed("https://bob.github.io/", &site("https://alice.github.io/")));

        let p = std::env::temp_dir().join(format!("mimir-vault-bots-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let e = |name: &str, bot| VaultEntry { name: name.into(), domains: vec!["github.com".into()], bot, totp: false };
        db.set_setting("vault", &serde_json::to_string(&[e("SHARED", None), e("GITHUB_COM_BOT3", Some(3))]).unwrap()).unwrap();
        let names = |bot| vault_for(&db, bot).into_iter().map(|e| e.name).collect::<Vec<_>>();
        assert_eq!(names(3), ["SHARED", "GITHUB_COM_BOT3"]);
        assert_eq!(names(4), ["SHARED"], "another bot doesn't get it");
        assert!(secrets_used(&vault_for(&db, 4), &json!({"text": "GITHUB_COM_BOT3"})).is_empty(), "nor can it type it");
        assert!(db.setting("vault").unwrap().unwrap().starts_with(r#"[{"name":"SHARED","domains":["github.com"]}"#), "older lists read and write the same");
        let codes = [("GH_CODE".to_string(), "123456".to_string())];
        assert_eq!(fill_codes(&json!({"fields": [{"value": "GH_CODE"}], "n": 1}), &codes), json!({"fields": [{"value": "123456"}], "n": 1}));
    }

    #[test]
    fn vault_rules() {
        let v = vec![VaultEntry { name: "GH_PASS".into(), domains: vec!["github.com".into()], bot: None, totp: false }];
        assert_eq!(secrets_used(&v, &json!({"ref":"e3","text":"GH_PASS"})).len(), 1);
        assert_eq!(secrets_used(&v, &json!({"fields":[{"value":"x GH_PASS y"}]})).len(), 1, "nested + substring");
        assert!(secrets_used(&v, &json!({"text":"hello"})).is_empty());
        assert!(host_allowed("https://github.com/login", &v[0].domains));
        assert!(host_allowed("https://gist.github.com/x", &v[0].domains));
        assert!(!host_allowed("https://github.com.evil.io/login", &v[0].domains));
        assert!(!host_allowed("https://evil.io/?github.com", &v[0].domains));
        assert!(!host_allowed("", &v[0].domains));
        assert_eq!(redact("pw is hunter22!", &[("GH_PASS".into(), "hunter22!".into())]), "pw is <secret>GH_PASS</secret>");
    }

    #[test]
    fn sse() {
        let mut buf = b": ping\n\nevent: message\ndata: {\"id\":1}\n\ndata: a\r\ndata:b\r\n\r\ndata: {\"par".to_vec();
        assert_eq!(sse_events(&mut buf), ["{\"id\":1}", "a\nb"]);
        assert_eq!(buf, b"data: {\"par", "a partial event waits for the rest");
        buf.extend_from_slice(b"tial\":true}\n\n");
        assert_eq!(sse_events(&mut buf), ["{\"partial\":true}"]);
        assert!(buf.is_empty());
        assert_eq!(reply(r#"{"jsonrpc":"2.0","id":2,"result":{"ok":1}}"#, 2, "m").unwrap().unwrap()["ok"], 1);
        assert!(reply(r#"{"jsonrpc":"2.0","method":"notifications/progress"}"#, 2, "m").is_none());
        assert!(reply(r#"{"id":2,"error":{"message":"nope"}}"#, 2, "m").unwrap().is_err());
    }

    /// `cargo test -p mimir-core remote_live -- --ignored` (DeepWiki; or MIMIR_MCP_URL/_TOOL/_TOKEN)
    #[tokio::test]
    #[ignore]
    async fn remote_live() {
        let p = std::env::temp_dir().join(format!("mimir-ext-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let url = std::env::var("MIMIR_MCP_URL").unwrap_or_else(|_| "https://mcp.deepwiki.com/mcp".into());
        let headers = std::env::var("MIMIR_MCP_TOKEN").map(|t| HashMap::from([("Authorization".into(), format!("Bearer {t}"))])).unwrap_or_default();
        let conf = ServerConf { name: "remote".into(), url, headers, ..Default::default() };
        let n = refresh(&db, &conf).await.unwrap();
        assert!(n > 0, "lists tools");
        let tool = std::env::var("MIMIR_MCP_TOOL").unwrap_or_else(|_| "read_wiki_structure".into());
        let mut c = Client::start(&conf, &db, 0).await.unwrap();
        let res = c.request("tools/call", json!({"name": tool, "arguments": {"repoName": "modelcontextprotocol/modelcontextprotocol"}})).await.unwrap();
        assert_ne!(res["isError"], true, "{res}");
        println!("{n} tools; {tool} → {}", crate::tools::truncate(&res.to_string(), 200));
    }

    #[test]
    fn sign_in_discovery_addresses() {
        let h = r#"Bearer error="invalid_token", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource/mcp""#;
        assert_eq!(resource_metadata(h).as_deref(), Some("https://mcp.example.com/.well-known/oauth-protected-resource/mcp"));
        assert_eq!(resource_metadata("Bearer realm=x"), None);
        let u = url::Url::parse("https://mcp.example.com/mcp/").unwrap();
        assert_eq!(well_known(&u, "oauth-protected-resource"), "https://mcp.example.com/.well-known/oauth-protected-resource/mcp");
        let root = url::Url::parse("https://auth.example.com").unwrap();
        assert_eq!(well_known(&root, "oauth-authorization-server"), "https://auth.example.com/.well-known/oauth-authorization-server");
        assert!(secure("https://a.example") && secure("http://127.0.0.1:9/x") && !secure("http://evil.example"));
        assert!(secure("http://localhost:8000/mcp") && secure("http://[::1]/") && secure("http://127.8.0.1/") && secure("http://LOCALHOST/"));
        for bad in ["http://localhost.evil.com/", "http://127.0.0.1.evil.com/", "http://localhost@evil.com/", "http://[::1].evil/", "ftp://localhost/", "localhost", "https://"] {
            assert!(!secure(bad), "{bad}");
        }
    }

    /// Against scripts/fake-mcp-oauth.py: the whole sign-in (metadata, registration, PKCE, token) then a tool list
    /// with the token, a refresh when it runs out, and a 401 retry. `python3 scripts/fake-mcp-oauth.py` then
    /// `cargo test -p mimir-core fake_mcp_oauth -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn fake_mcp_oauth() {
        let _one = crate::oauth::SERIAL.lock().await;
        let p = std::env::temp_dir().join(format!("mimir-mcp-oauth-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        crate::host::install_secrets(std::sync::Arc::new(crate::host::DbSecrets(db.clone())));
        let conf = ServerConf { name: "fake".into(), url: "http://127.0.0.1:8977/mcp".into(), ..Default::default() };
        let err = format!("{:#}", save_server(&db, conf.clone()).await.unwrap_err());
        assert!(err.contains("wants you to sign in"), "{err}");
        let url = sign_in(conf, true).await.unwrap();
        assert!(signing_in());
        // The fake authorization page approves at once and sends the browser back.
        let page = reqwest::get(&url).await.unwrap().text().await.unwrap();
        assert!(page.contains("Signed in to fake"), "{page}");
        crate::oauth::finish().await.unwrap();
        assert_eq!(signed_in_save(&db).await.unwrap(), 1);
        assert!(signed_in("fake"));
        // Tokens last 2 s on the fake: after 3 the next call refreshes first. Then the fake revokes it: 401, one retry.
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert_eq!(refresh(&db, &servers(&db)[0]).await.unwrap(), 1);
        reqwest::Client::new().post("http://127.0.0.1:8977/revoke").send().await.unwrap();
        assert_eq!(refresh(&db, &servers(&db)[0]).await.unwrap(), 1, "refused token: refreshed and retried");
        delete_server(&db, "fake").unwrap();
        assert!(!signed_in("fake"));
    }

    #[test]
    fn package_of_commands() {
        let p = |line: &str| {
            let mut w = line.split(' ').map(String::from);
            package_of(&w.next().unwrap(), &w.collect::<Vec<_>>()).map(|(e, n, v)| format!("{e} {n} {}", v.unwrap_or_default()))
        };
        assert_eq!(p("npx -y @modelcontextprotocol/server-filesystem ~/Notes").as_deref(), Some("npm @modelcontextprotocol/server-filesystem "));
        assert_eq!(p("/usr/local/bin/npx --yes @scope/pkg@1.2.3 --flag").as_deref(), Some("npm @scope/pkg 1.2.3"));
        assert_eq!(p("npx.cmd mcp-thing@latest").as_deref(), Some("npm mcp-thing "), "a tag isn't a pinned version");
        assert_eq!(p("npx -p left-pad@1.3.0 run").as_deref(), Some("npm left-pad 1.3.0"));
        assert_eq!(p("npm exec --yes -- pkg@^2").as_deref(), Some("npm pkg "));
        assert_eq!(p("pnpm dlx pkg@2.0.0").as_deref(), Some("npm pkg 2.0.0"));
        assert_eq!(p("bunx pkg").as_deref(), Some("npm pkg "));
        assert_eq!(p("uvx mcp-server-fetch").as_deref(), Some("PyPI mcp-server-fetch "));
        assert_eq!(p("uvx --python 3.12 Mcp_Server.Git==0.6.2").as_deref(), Some("PyPI mcp-server-git 0.6.2"));
        assert_eq!(p("uvx --from awslabs.core-mcp-server@1.0.1 run").as_deref(), Some("PyPI awslabs-core-mcp-server 1.0.1"));
        assert_eq!(p("uv tool run pkg[cli]>=1").as_deref(), Some("PyPI pkg "));
        assert_eq!(p("pipx run --spec pkg==1.0 tool").as_deref(), Some("PyPI pkg 1.0"));
        for none in ["node server.js", "npx ./local", "npx github:user/repo", "uvx git+https://x/y", "python -m pkg", "npm install x", "npx -y"] {
            assert_eq!(p(none), None, "{none}");
        }
    }

    #[test]
    fn malicious_verdict() {
        assert_eq!(malicious(&json!({"vulns": [{"id": "GHSA-x"}, {"id": "MAL-2025-1234"}]})).as_deref(), Some("MAL-2025-1234"));
        assert_eq!(malicious(&json!({"vulns": [{"id": "GHSA-y", "aliases": ["CVE-1", "MAL-2024-9"]}]})).as_deref(), Some("MAL-2024-9 (GHSA-y)"));
        assert_eq!(malicious(&json!({"vulns": [{"id": "GHSA-z"}]})), None, "an ordinary vulnerability doesn't block");
        assert_eq!(malicious(&json!({})), None);
    }

    /// Against a one-file fake OSV: a flagged package is refused, the verdict is cached, and OSV being down doesn't block.
    #[tokio::test]
    async fn malware_check_refuses_and_caches() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::AsyncReadExt;
        static ASKED: AtomicUsize = AtomicUsize::new(0);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let osv = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let mut req = vec![0; 4096];
                let n = s.read(&mut req).await.unwrap();
                ASKED.fetch_add(1, Ordering::SeqCst);
                let body = if String::from_utf8_lossy(&req[..n]).contains("evil-mcp") { r#"{"vulns":[{"id":"MAL-2025-42"}]}"# } else { "{}" };
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            }
        });
        let p = std::env::temp_dir().join(format!("mimir-osv-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let args = |a: &str| a.split(' ').map(String::from).collect::<Vec<_>>();
        let err = malware_check(&db, &osv, "x", "npx", &args("-y evil-mcp@1.0.0")).await.unwrap_err().to_string();
        assert!(err.contains("evil-mcp@1.0.0") && err.contains("MAL-2025-42"), "{err}");
        malware_check(&db, &osv, "x", "npx", &args("-y good-mcp")).await.unwrap();
        assert!(malware_check(&db, &osv, "x", "npx", &args("-y evil-mcp@1.0.0")).await.is_err());
        malware_check(&db, &osv, "x", "npx", &args("-y good-mcp")).await.unwrap();
        malware_check(&db, &osv, "x", "node", &args("server.js")).await.unwrap();
        assert_eq!(ASKED.load(Ordering::SeqCst), 2, "verdicts come from the cache the second time");
        malware_check(&db, "http://127.0.0.1:9", "x", "uvx", &args("other-pkg")).await.unwrap();
    }

    #[test]
    fn defaults() {
        assert_eq!(default_action("browser__browser_evaluate", false), "deny");
        assert_eq!(default_action("browser__browser_navigate", false), "ask");
        assert_eq!(default_action("browser__browser_click", false), "allow");
        assert_eq!(default_action("browser__browser_snapshot", true), "allow");
        assert_eq!(default_action("mail__send", false), "ask");
        assert_eq!(default_action("mail__list", true), "allow");
    }
}
