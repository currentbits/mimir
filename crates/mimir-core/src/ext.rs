//! External MCP tool servers (e.g. the Playwright browser), proxied through our policy gate,
//! plus the credential vault. External tools are named `<server>__<tool>`; their lists are cached
//! in settings so building a prompt never starts a browser. A server process lives for one turn.
//! Remote servers (a URL) speak MCP's Streamable HTTP: POST each message, read JSON or SSE back.
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
pub struct VaultEntry {
    pub name: String,
    /// Sites it may be typed into (exact host or subdomain).
    pub domains: Vec<String>,
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
    let local_http = ["http://localhost", "http://127.0.0.1", "http://[::1]"].iter().any(|p| s.url.starts_with(p));
    if !(s.url.is_empty() || s.url.starts_with("https://") || local_http) {
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
    list.retain(|s| s.name != name);
    set_servers(db, &list)
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

pub fn vault_save(db: &Db, entry: VaultEntry, value: Option<&str>) -> Result<()> {
    let name = entry.name.trim().to_string();
    if name.len() < 3 || !name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        bail!("use a name like GITHUB_PASSWORD (capitals, digits, underscores)");
    }
    let domains: Vec<String> = entry.domains.iter().map(|d| d.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_lowercase()).filter(|d| !d.is_empty()).collect();
    if domains.is_empty() {
        bail!("add at least one site this login may be used on");
    }
    let mut list = vault(db);
    let exists = list.iter().any(|e| e.name == name);
    if let Some(v) = value.filter(|v| !v.is_empty()) {
        crate::host::secret_set(&format!("vault:{name}"), v)?;
    } else if !exists {
        bail!("a new login needs its secret value");
    }
    list.retain(|e| e.name != name);
    list.push(VaultEntry { name, domains });
    db.set_setting("vault", &serde_json::to_string(&list)?)
}

pub fn vault_delete(db: &Db, name: &str) -> Result<()> {
    let mut list = vault(db);
    list.retain(|e| e.name != name);
    crate::host::secret_set(&format!("vault:{name}"), "")?;
    db.set_setting("vault", &serde_json::to_string(&list)?)
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
}

impl Client {
    async fn start(conf: &ServerConf, db: &Db, bot: i64) -> Result<Self> {
        let (io, secrets_file) = if conf.url.is_empty() {
            // Off the async threads: bringing up the bots' computer can take seconds.
            let (conf, db) = (conf.clone(), db.clone());
            tokio::task::spawn_blocking(move || spawn(&conf, &db, bot)).await??
        } else {
            let headers = values_of(&conf.name, "header", &conf.headers);
            (Io::Http(Http { client: reqwest::Client::builder().timeout(TIMEOUT).build()?, url: conf.url.clone(), headers, session: None, version: None }), None)
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
        let mut req = self.client.post(&self.url).header("Accept", "application/json, text/event-stream").header("Content-Type", "application/json").body(msg.to_string());
        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        if let Some(s) = &self.session {
            req = req.header("Mcp-Session-Id", s);
        }
        if let Some(v) = &self.version {
            req = req.header("MCP-Protocol-Version", v);
        }
        let res = req.send().await.with_context(|| format!("couldn't reach {}", self.url))?;
        if let Some(s) = res.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            self.session = Some(s.into());
        }
        // ponytail: no OAuth; servers that need a login take a token header (Authorization: Bearer …).
        match res.status() {
            s if s.is_success() => Ok(res),
            s @ (reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN) => bail!("{} refused the request ({s}); it needs a token, e.g. an Authorization header", self.url),
            s => bail!("{} answered {s}: {}", self.url, crate::tools::truncate(&res.text().await.unwrap_or_default(), 300)),
        }
    }
}

/// Start a stdio server. Returns the browser's secrets file too, to delete once it has started.
fn spawn(conf: &ServerConf, db: &Db, bot: i64) -> Result<(Io, Option<std::path::PathBuf>)> {
    let dir = db.dir().join("browser");
    let (command, args, secrets_file) = match conf.preset.as_deref() {
        Some("playwright") => {
            std::fs::create_dir_all(&dir)?;
            let file = write_secrets_file(&dir, &vault_values(db))?;
            let mut args: Vec<String> = vec!["-y".into(), PLAYWRIGHT.into()];
            if crate::computer::usable(db) {
                // The bots' computer is on: drive the Chromium on its screen, so you can watch (and take over).
                crate::computer::start(&db.dir())?;
                args.extend(["--cdp-endpoint".into(), crate::computer::cdp_endpoint()?]);
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
        .envs(values_of(&conf.name, "env", &conf.env))
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

    let used = secrets_used(&vault(&ctx.db), args);
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
    let text = redact(&text, &vault_values(&ctx.db));
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
    fn vault_rules() {
        let v = vec![VaultEntry { name: "GH_PASS".into(), domains: vec!["github.com".into()] }];
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
    fn defaults() {
        assert_eq!(default_action("browser__browser_evaluate", false), "deny");
        assert_eq!(default_action("browser__browser_navigate", false), "ask");
        assert_eq!(default_action("browser__browser_click", false), "allow");
        assert_eq!(default_action("browser__browser_snapshot", true), "allow");
        assert_eq!(default_action("mail__send", false), "ask");
        assert_eq!(default_action("mail__list", true), "allow");
    }
}
