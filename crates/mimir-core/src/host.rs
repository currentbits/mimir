//! Runs every bot and answers the UI's commands. One implementation, two front doors:
//! the desktop app calls `Host::call` in-process; `mimir server` exposes it as
//! `POST /api/<cmd>` for browsers and for the desktop app in remote mode.

use crate::db::{Bot, Db};
use crate::provider::Provider;
use crate::runtime::{self, SharedStatus, Status};
use crate::ext;
use crate::tools::{self, ToolCtx};
use crate::{Trigger, turn};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Where channel tokens and the API key live: the OS keychain on desktop, the server's
/// own database when self-hosted.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
    /// "" deletes.
    fn set(&self, name: &str, value: &str) -> Result<()>;
}

/// Secrets in the settings table. For self-hosting: the DB file is the server's data, so
/// keep it private (the server creates it 0600).
// ponytail: plaintext at rest on the user's own server; add envelope encryption (MIMIR_SECRET_KEY) if hosted for others
pub struct DbSecrets(pub Db);

impl SecretStore for DbSecrets {
    fn get(&self, name: &str) -> Option<String> {
        self.0.setting(&format!("secret:{name}")).ok().flatten().filter(|s| !s.is_empty())
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.0.set_setting(&format!("secret:{name}"), value)
    }
}


/// The process's secret store, for code that runs outside a Host (the MCP subprocess a CLI
/// provider spawns, which may need to start another bot for `ask_bot`). Set by each entry point.
static SECRETS: std::sync::OnceLock<Arc<dyn SecretStore>> = std::sync::OnceLock::new();

pub fn install_secrets(s: Arc<dyn SecretStore>) {
    let _ = SECRETS.set(s);
}

pub fn secret_get(name: &str) -> Option<String> {
    SECRETS.get()?.get(name)
}

pub fn secret_set(name: &str, value: &str) -> Result<()> {
    SECRETS.get().context("no secret store in this process")?.set(name, value)
}

/// Accounts that take an API key: (secret name, env var fallback).
/// (`brave` is Brave Search's key, for the bots' web_search, and `x` an X API bearer token, for x_search: not AI accounts.
/// `anthropic` isn't an account of its own either: Claude bots go on to it when the plan says no, see provider::api_credits.)
const KEYED: [(&str, &str, &str); 6] = [
    ("openai", "openai_api_key", "OPENAI_API_KEY"),
    ("anthropic", "anthropic_api_key", "ANTHROPIC_API_KEY"),
    ("openrouter", OPENROUTER_KEY, "OPENROUTER_API_KEY"),
    ("xai", "xai_api_key", "XAI_API_KEY"),
    ("brave", "brave_api_key", "BRAVE_API_KEY"),
    ("x", "x_bearer_token", "X_BEARER_TOKEN"),
];
const OPENROUTER_KEY: &str = "openrouter_api_key";

/// API key for an account kind (`openai`, `openrouter`): the secret store, else the env var.
pub fn api_key(kind: &str) -> Option<String> {
    let (_, secret, env) = KEYED.iter().find(|(k, ..)| *k == kind)?;
    secret_get(secret).or_else(|| std::env::var(env).ok().filter(|k| !k.trim().is_empty()))
}
const TG: &str = "telegram_bot_token";
const SLACK_BOT: &str = "slack_bot_token";
const SLACK_APP: &str = "slack_app_token";
const DISCORD: &str = "discord_bot_token";
const SIGNAL: &str = "signal_number";
const SIGNAL_API: &str = "signal_api";
const WHATSAPP: &str = "whatsapp_number_id";
const WHATSAPP_TOKEN: &str = "whatsapp_token";
const WHATSAPP_SECRET: &str = "whatsapp_app_secret";
/// Not a secret, but kept with the bot's other channel settings.
const IMESSAGE: &str = "imessage_chat";
const CHANNEL_SECRETS: [&str; 10] = [TG, SLACK_BOT, SLACK_APP, DISCORD, SIGNAL, SIGNAL_API, WHATSAPP, WHATSAPP_TOKEN, WHATSAPP_SECRET, IMESSAGE];
/// Chat id of the built-in chat in the app (no messaging app needed).
pub const LOCAL_CHAT: i64 = 0;

/// Secret name for a bot's channel token. Bot 1's Telegram token keeps its pre-multi-bot
/// name so existing desktop installs don't lose it.
/// "server" (mimir server) or "desktop": what's running this process, for things only one of them does.
static MODE: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

pub fn is_server() -> bool {
    MODE.get() == Some(&"server")
}

pub fn is_desktop() -> bool {
    MODE.get() == Some(&"desktop")
}

fn key(bot: i64, name: &str) -> String {
    if bot == 1 && name == TG { name.into() } else { format!("{name}:{bot}") }
}

/// A bot's channel tokens from the process's secret store, for code outside a Host (`mimir doctor`).
pub(crate) fn channels_of(bot: i64) -> runtime::Channels {
    let s = |n| secret_get(&key(bot, n));
    runtime::Channels {
        telegram: s(TG),
        slack: s(SLACK_BOT).zip(s(SLACK_APP)),
        discord: s(DISCORD),
        signal: s(SIGNAL).map(|n| (s(SIGNAL_API).unwrap_or_default(), n)),
        whatsapp: s(WHATSAPP).zip(s(WHATSAPP_TOKEN)).zip(s(WHATSAPP_SECRET)).map(|((i, t), a)| (i, t, a)),
        // Messages on this Mac: never on a server.
        imessage: s(IMESSAGE).filter(|_| !is_server()),
    }
}

struct Live {
    status: SharedStatus,
    task: Option<tokio::task::JoinHandle<()>>,
}

pub struct Host {
    pub db: Db,
    home: PathBuf,
    secrets: Arc<dyn SecretStore>,
    rt: tokio::runtime::Handle,
    /// "desktop" or "server"; also names the built-in chat.
    mode: &'static str,
    live: Mutex<HashMap<i64, Live>>,
    /// Chat turns in progress in the app, by (bot, chat): how to stop one, and what was asked.
    running: Mutex<HashMap<(i64, i64), (tokio::task::AbortHandle, String)>>,
    /// Your inputs on the bots' computer, one at a time and in order (the app sends them without waiting).
    input: tokio::sync::Mutex<()>,
}

#[derive(Serialize)]
struct BotView {
    #[serde(flatten)]
    bot: Bot,
    has_telegram_token: bool,
    has_slack: bool,
    has_discord: bool,
    has_signal: bool,
    has_whatsapp: bool,
    has_imessage: bool,
}

/// Write-only channel secrets: None = keep, "" = remove, else replace.
#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Secrets {
    telegram_token: Option<String>,
    slack_bot_token: Option<String>,
    slack_app_token: Option<String>,
    discord_token: Option<String>,
    signal_number: Option<String>,
    signal_api: Option<String>,
    whatsapp_number_id: Option<String>,
    whatsapp_token: Option<String>,
    whatsapp_app_secret: Option<String>,
    imessage_chat: Option<String>,
}

#[derive(Serialize)]
struct Rule {
    tool: String,
    description: String,
    action: String,
    default: &'static str,
}

fn arg<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T> {
    serde_json::from_value(args.get(name).cloned().unwrap_or(Value::Null)).with_context(|| format!("bad or missing argument '{name}'"))
}

fn ok<T: Serialize>(v: T) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}

/// Every future `Host::call` awaits goes through here: `make` builds it on this function's stack and it's boxed, so
/// the call's poll frame holds a pointer. Debug builds give each future awaited in place a slot of its own in that
/// frame, one per command, and those add up to more than a test thread's 2 MB stack (Windows CI). Boxing in place
/// (`Box::pin(f)`) doesn't help: `f` is built in the frame first.
fn boxed<'a, F: std::future::Future + Send + 'a>(make: impl FnOnce() -> F) -> futures_util::future::BoxFuture<'a, F::Output> {
    Box::pin(make())
}

/// A token for the server's chat API (`/v1/chat/completions`): it reaches one bot and nothing else. Only its hash is
/// kept, so the token is shown once.
#[derive(Serialize, serde::Deserialize)]
struct ChatToken {
    id: String,
    name: String,
    bot_id: i64,
    created: i64,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    hash: String,
}
// ponytail: one JSON setting, read-modify-write; fine for the handful of tokens one admin makes by hand
const CHAT_TOKENS: &str = "chat_api_tokens";

fn token_hash(token: &str) -> String {
    format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(token.as_bytes()))
}

fn chat_tokens(db: &Db) -> Result<Vec<ChatToken>> {
    Ok(db.setting(CHAT_TOKENS)?.map(|s| serde_json::from_str(&s)).transpose()?.unwrap_or_default())
}

fn set_chat_tokens(db: &Db, t: &[ChatToken]) -> Result<()> {
    db.set_setting(CHAT_TOKENS, &serde_json::to_string(t)?)
}

/// Chat API chats: one per token, ids from a range no other chat uses (Telegram's are below 2^52, topics from 8e15,
/// other apps from 9e15). A token id is 12 hex digits, so they fit.
const API_CHATS: i64 = 7_000_000_000_000_000;

/// The bot a chat API token reaches, if it's one of ours (and the bot still exists), and the token's own chat. Not
/// the built-in chat, and not a private one: whoever holds a token (or talks to the speaker that uses it) isn't
/// necessarily you, so no email, calendar, meetings or computer, and nothing of the app's conversation.
pub fn chat_token_bot(db: &Db, token: &str) -> Option<(i64, i64)> {
    let h = token_hash(token);
    let t = chat_tokens(db).ok()?.into_iter().find(|t| t.hash == h)?;
    db.bot(t.bot_id).ok()?;
    let chat = API_CHATS + i64::from_str_radix(&t.id, 16).ok()?;
    // Listed with the chats (its own channel, so it's never taken for a Telegram one), to see what it was asked.
    db.pair_api(chat, &t.id, &format!("Chat API: {}", t.name)).ok()?;
    Some((t.bot_id, chat))
}

/// A chat API token's chat: what's said there isn't taken as your own words.
pub fn is_api_chat(chat: i64) -> bool {
    (API_CHATS..API_CHATS + 1_000_000_000_000_000).contains(&chat)
}

impl Host {
    pub fn new(db: Db, home: PathBuf, secrets: Arc<dyn SecretStore>, rt: tokio::runtime::Handle, mode: &'static str) -> Self {
        let _ = MODE.set(mode);
        install_secrets(secrets.clone());
        crate::say::load(&db);
        if let Err(e) = db.incognito_leftovers().and_then(|_| db.prune_old()).and_then(|_| db.close_interrupted()) {
            eprintln!("cleanup: {e:#}");
        }
        // Private files a crash left behind (a run's instructions), and photos and files older than 60 days.
        let _ = std::fs::remove_dir_all(home.join("tmp"));
        let _ = std::fs::remove_file(crate::provider::grok_rules(&home.join("workspace-incognito")));
        for dir in std::fs::read_dir(home.join("workspace")).into_iter().flatten().flatten() {
            let _ = std::fs::remove_file(crate::provider::grok_rules(&dir.path()));
        }
        if let Ok(entries) = std::fs::read_dir(home.join("media")) {
            let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60 * 86400);
            for e in entries.flatten() {
                if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < old) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        // The built-in chat exists from the start, so its memories count before the first message.
        if let Err(e) = db.pair(LOCAL_CHAT, if mode == "server" { "Web chat" } else { "This computer" }) {
            eprintln!("built-in chat: {e:#}");
        }
        // A copy of the database once a day, after checking it (looked at hourly: a laptop sleeps through the night).
        {
            let db = db.clone();
            rt.spawn(async move {
                loop {
                    if crate::backup::due(&db) {
                        let d = db.clone();
                        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || crate::backup::snapshot(&d, "")).await {
                            eprintln!("backup: {e:#}");
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                }
            });
        }
        // The bots' computer (self-hosted Linux server): bring it up if it's switched on, and back up if it dies.
        if crate::computer::available() {
            let (db, home) = (db.clone(), home.clone());
            rt.spawn(async move {
                loop {
                    if crate::computer::enabled(&db) {
                        // Both block (a TCP connect or xdotool, then up to ~13 s of starting).
                        let h = home.clone();
                        let up = move || if crate::computer::running() { Ok(()) } else { crate::computer::start(&h, None) };
                        if let Ok(Err(e)) = tokio::task::spawn_blocking(up).await {
                            eprintln!("computer: {e:#}");
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                }
            });
        }
        Host { db, home, secrets, rt, mode, live: Mutex::new(HashMap::new()), running: Mutex::new(HashMap::new()), input: tokio::sync::Mutex::new(()) }
    }

    fn channels(&self, bot: i64) -> runtime::Channels {
        let s = |n| self.secrets.get(&key(bot, n));
        runtime::Channels {
        telegram: s(TG),
        slack: s(SLACK_BOT).zip(s(SLACK_APP)),
        discord: s(DISCORD),
        signal: s(SIGNAL).map(|n| (s(SIGNAL_API).unwrap_or_default(), n)),
        whatsapp: s(WHATSAPP).zip(s(WHATSAPP_TOKEN)).zip(s(WHATSAPP_SECRET)).map(|((i, t), a)| (i, t, a)),
        // Messages on this Mac: never on a server.
        imessage: s(IMESSAGE).filter(|_| !is_server()),
    }
    }

    fn provider(&self, bot: i64) -> Result<Provider> {
        Provider::for_bot(&self.db.bot(bot)?, self.home.clone())
    }

    pub fn stop(&self, bot: i64) {
        if let Some(Live { task: Some(h), .. }) = self.live.lock().unwrap().remove(&bot) {
            h.abort();
        }
    }

    pub fn stop_all(&self) {
        let ids: Vec<i64> = self.live.lock().unwrap().keys().copied().collect();
        ids.into_iter().for_each(|b| self.stop(b));
    }

    /// (Re)start one bot with its current settings. Runs even without channels, so
    /// routines from the built-in chat still fire.
    pub fn restart(&self, bot: i64) {
        self.stop(bot);
        let status: SharedStatus = Default::default();
        let task = match self.provider(bot) {
            Err(err) => {
                *status.lock().unwrap() = Status { bot_id: bot, error: Some(format!("{err:#}")), ..Default::default() };
                None
            }
            Ok(p) => {
                let (db, st, ch, p) = (self.db.clone(), status.clone(), self.channels(bot), Arc::new(p));
                Some(self.rt.spawn(async move {
                    // A crash (a panic anywhere in the bot) starts it again, saying so; an error (a bad token) stops it.
                    loop {
                        let run = std::panic::AssertUnwindSafe(runtime::serve(db.clone(), bot, p.clone(), ch.clone(), st.clone()));
                        let why = match futures_util::FutureExt::catch_unwind(run).await {
                            Ok(Ok(())) => return,
                            Ok(Err(err)) => {
                                st.lock().unwrap_or_else(|e| e.into_inner()).error = Some(format!("{err:#}"));
                                return;
                            }
                            Err(panic) => panic.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| panic.downcast_ref::<String>().cloned()).unwrap_or_default(),
                        };
                        eprintln!("bot #{bot} crashed ({why}); starting it again");
                        let at = chrono::Local::now().format("%H:%M");
                        st.lock().unwrap_or_else(|e| e.into_inner()).error = Some(format!("Stopped after a crash at {at} and started again: {why}"));
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }))
            }
        };
        self.live.lock().unwrap().insert(bot, Live { status, task });
    }

    pub fn restart_all(&self) {
        for b in self.db.bots().unwrap_or_default() {
            self.restart(b.id);
        }
    }

    /// One UI command. Arguments are a JSON object with snake_case keys.
    /// A reply task ended: forget its Stop handle, unless a newer reply in the same chat has replaced it.
    fn finished(&self, key: (i64, i64), task: tokio::task::Id) {
        let mut running = self.running.lock().unwrap();
        if running.get(&key).is_some_and(|(h, _)| h.id() == task) {
            running.remove(&key);
        }
    }

    /// "" off, "limit" once the plan says no, "routines" that and its routines from the start (get without `mode`).
    // ponytail: out of `call`, whose one stack frame holds every arm's locals and is at Windows' limit in debug tests
    fn api_credits(&self, a: &Value) -> Result<Value> {
        let key = format!("api_credits:{}", arg::<i64>(a, "bot_id")?);
        match arg::<Option<String>>(a, "mode")?.as_deref() {
            None => {}
            Some("") => self.db.set_setting(&key, "")?,
            Some(m @ ("limit" | "routines")) if api_key("anthropic").is_some() => self.db.set_setting(&key, m)?,
            Some("limit" | "routines") => bail!("add your Anthropic API key in Settings → AI keys first"),
            Some(m) => bail!("unknown mode '{m}'"),
        }
        ok(self.db.setting(&key)?.unwrap_or_default())
    }

    pub async fn call(&self, cmd: &str, a: &Value) -> Result<Value> {
        let scope = |a: &Value| -> Result<String> { Ok(format!("chat:{}", arg::<i64>(a, "chat_id")?)) };
        match cmd {
            "info" => {
                // Existing installs (anything has run) count as onboarded.
                let onboarded = self.db.setting("onboarded")?.is_some() || !self.db.runs(None, 1)?.is_empty();
                ok(json!({"mode": self.mode, "version": env!("CARGO_PKG_VERSION"), "onboarded": onboarded}))
            }
            "onboarding_done" => ok(self.db.set_setting("onboarded", "1")?),
            // Which AI accounts work here: installed / signed in / plan.
            "accounts" => {
                use crate::provider::detect;
                let h = &self.home;
                let (c, x, gr, r, g, l, k, cg) = boxed(|| async {
                    tokio::join!(
                        detect("claude", h),
                        detect("codex", h),
                        detect("grok", h),
                        detect("openrouter", h),
                        detect("xai", h),
                        detect("local", h),
                        detect("openai", h),
                        detect("chatgpt", h)
                    )
                })
                .await;
                ok(json!({"claude": c, "codex": x, "grok": gr, "openrouter": r, "xai": g, "local": l, "openai": k, "chatgpt": cg}))
            }
            // Sign in to Grok for Mimir: returns the device-code URL to open; `accounts` shows when it's done.
            "grok_login_cancel" => {
                crate::provider::grok_login_cancel();
                ok(())
            }
            "openrouter_cancel" => {
                crate::openrouter::cancel();
                ok(())
            }
            "grok_login" => {
                let (url, code) = boxed(|| crate::provider::grok_login(&self.home)).await?;
                ok(json!({"url": url, "code": code}))
            }
            // Model ids for the picker (API accounts; [] for the CLI ones).
            "models" => {
                let kind: String = arg(a, "provider")?;
                let (base_url, key) = (arg::<Option<String>>(a, "base_url")?, api_key(&kind));
                ok(boxed(|| crate::provider::models(&kind, base_url.as_deref(), key.as_deref())).await?)
            }
            // A real round-trip with an account + model before saving it.
            "test_account" => {
                let kind: String = arg(a, "provider")?;
                let p = Provider::new(&kind, arg(a, "model")?, arg(a, "base_url")?, api_key(&kind), self.home.clone())?;
                let (reply, secs) = boxed(|| p.check(&self.db)).await?;
                ok(json!({"reply": crate::tools::truncate(&reply, 200), "seconds": (secs * 10.0).round() / 10.0, "provider": p.name()}))
            }
            "statuses" => {
                // A bot another bot made (bot_create, in the AI CLI's own tool process) starts here, so its routines run.
                // Not while this app's bots are switched off (it shows a server's).
                if self.db.setting("local_bots")?.as_deref() != Some("off") {
                    let running: std::collections::HashSet<i64> = self.live.lock().unwrap().keys().copied().collect();
                    for b in self.db.bots()?.into_iter().filter(|b| !running.contains(&b.id)) {
                        self.restart(b.id);
                    }
                }
                let mut v: Vec<Status> = self.live.lock().unwrap().values().map(|l| l.status.lock().unwrap().clone()).collect();
                v.sort_by_key(|s| s.bot_id);
                ok(v)
            }
            "bots" => ok(self
                .db
                .bots()?
                .into_iter()
                .map(|b| {
                    let ch = self.channels(b.id);
                    BotView { has_telegram_token: ch.telegram.is_some(), has_slack: ch.slack.is_some(), has_discord: ch.discord.is_some(), has_signal: ch.signal.is_some(), has_whatsapp: ch.whatsapp.is_some(), has_imessage: ch.imessage.is_some(), bot: b }
                })
                .collect::<Vec<_>>()),
            "save_bot" => {
                // The UI sends back the bot as `bots` listed it: the channel flags are that list's, not the bot's.
                let mut bot = a.get("bot").cloned().unwrap_or_default();
                if let Some(o) = bot.as_object_mut() {
                    for k in ["has_telegram_token", "has_slack", "has_discord", "has_signal", "has_whatsapp", "has_imessage"] {
                        o.remove(k);
                    }
                }
                let bot: Bot = arg(&json!({"bot": bot}), "bot")?;
                let secrets: Secrets = arg::<Option<Secrets>>(a, "secrets")?.unwrap_or_default();
                Provider::for_bot(&bot, self.home.clone())?;
                let unknown: Vec<&str> =
                    bot.tools.split(',').map(str::trim).filter(|t| !t.is_empty() && !t.ends_with("__*") && !tools::all_specs(&self.db).iter().any(|(n, ..)| n == t)).collect();
                if !unknown.is_empty() {
                    bail!("unknown tools: {}", unknown.join(", "));
                }
                if let Some(chat) = secrets.imessage_chat.as_deref().filter(|c| !c.trim().is_empty()) {
                    if is_server() {
                        bail!("iMessage works only in the Mac app");
                    }
                    crate::imessage::IMessage::new(chat)?;
                }
                // A working folder: checked and stored as it really is (links resolved); its files copied in if asked.
                let mut bot = bot;
                let was = (bot.id > 0).then(|| self.db.bot(bot.id).ok()).flatten().map(|b| b.workdir).unwrap_or_default();
                if !bot.workdir.trim().is_empty() && bot.workdir != was {
                    bot.workdir = crate::files::check_workdir(&self.db, &bot.workdir)?;
                }
                let id = self.db.bot_save(&bot)?;
                if arg::<Option<bool>>(a, "copy_files")?.unwrap_or(false) && !bot.workdir.is_empty() && bot.workdir != was {
                    crate::files::copy_into_workdir(&self.db, id, std::path::Path::new(&bot.workdir))?;
                }
                for (name, value) in
                    [(TG, secrets.telegram_token), (SLACK_BOT, secrets.slack_bot_token), (SLACK_APP, secrets.slack_app_token), (DISCORD, secrets.discord_token), (SIGNAL, secrets.signal_number),
                    (SIGNAL_API, secrets.signal_api),
                    (WHATSAPP, secrets.whatsapp_number_id),
                    (WHATSAPP_TOKEN, secrets.whatsapp_token),
                    (WHATSAPP_SECRET, secrets.whatsapp_app_secret),
                    (IMESSAGE, secrets.imessage_chat),
                ]
                {
                    if let Some(v) = value {
                        self.secrets.set(&key(id, name), v.trim())?;
                    }
                }
                self.restart(id);
                ok(id)
            }
            "delete_bot" => {
                let id: i64 = arg(a, "id")?;
                // The AI CLIs' copies of its conversations, and its browser profile (logins, cookies), go too.
                if let Ok(p) = self.provider(id) {
                    self.db.sessions_of(id)?.iter().for_each(|s| p.forget_session(s));
                }
                let _ = std::fs::remove_dir_all(self.home.join("browser").join(format!("profile-{id}")));
                self.db.bot_delete(id)?;
                self.stop(id);
                CHANNEL_SECRETS.iter().try_for_each(|n| self.secrets.set(&key(id, n), ""))?;
                // Its chat API tokens too, so a later bot given the same id can't be reached with them.
                set_chat_tokens(&self.db, &chat_tokens(&self.db)?.into_iter().filter(|t| t.bot_id != id).collect::<Vec<_>>())?;
                ok(())
            }
            // Chat API tokens (server): list them, make one (its token is in the answer, once), or delete one.
            "chat_tokens" => ok(chat_tokens(&self.db)?.into_iter().map(|t| ChatToken { hash: String::new(), ..t }).collect::<Vec<_>>()),
            "chat_token_add" => {
                let (bot_id, name): (i64, String) = (arg(a, "bot_id")?, arg(a, "name")?);
                let name = name.trim().to_string();
                if name.is_empty() || name.chars().count() > 60 {
                    bail!("give it a name of up to 60 characters, like the app that will use it");
                }
                self.db.bot(bot_id).context("no such bot")?;
                let token = format!("mimir_{}", crate::random_hex(24)?);
                let mut all = chat_tokens(&self.db)?;
                all.push(ChatToken { id: crate::random_hex(6)?, name, bot_id, created: crate::db::now(), hash: token_hash(&token) });
                set_chat_tokens(&self.db, &all)?;
                ok(token)
            }
            "chat_token_delete" => {
                let id: String = arg(a, "id")?;
                set_chat_tokens(&self.db, &chat_tokens(&self.db)?.into_iter().filter(|t| t.id != id).collect::<Vec<_>>())?;
                ok(())
            }
            // Which API keys are saved, by account kind.
            "api_keys" => ok(KEYED.iter().map(|(k, secret, _)| (*k, self.secrets.get(secret).is_some())).collect::<HashMap<_, _>>()),
            // One-click OpenRouter sign-in (desktop: the callback comes back to this machine's browser).
            // `openrouter_connect` returns the URL to open; `openrouter_connect_wait` resolves once approved.
            "openrouter_connect" => {
                if self.mode == "server" {
                    bail!("connecting works in the desktop app; paste a key from openrouter.ai/keys instead");
                }
                ok(json!({"url": boxed(crate::openrouter::start).await?}))
            }
            "openrouter_connect_wait" => {
                let key = boxed(crate::openrouter::finish).await?;
                self.secrets.set(OPENROUTER_KEY, &key)?;
                self.restart_all();
                ok(())
            }
            // Sign in with ChatGPT (bots on your ChatGPT plan, no CLI). `chatgpt_sign_in` returns the address to open;
            // in the desktop app `chatgpt_sign_in_wait` resolves when the browser comes back. A server can't receive that
            // (it comes back to your own computer), so you paste the address you ended up on: `chatgpt_sign_in_paste`.
            "chatgpt_sign_in" => {
                let paste = self.mode == "server";
                ok(json!({"url": boxed(|| crate::chatgpt::start(&self.db, !paste)).await?, "paste": paste}))
            }
            "chatgpt_sign_in_wait" | "chatgpt_sign_in_paste" => {
                let account = if cmd == "chatgpt_sign_in_wait" {
                    boxed(crate::chatgpt::finish).await?
                } else {
                    let url: String = arg(a, "url")?;
                    boxed(|| crate::chatgpt::paste(&url)).await?
                };
                self.restart_all();
                ok(json!({"account": account}))
            }
            // Sign in with your Grok plan (xAI's API on the plan, no Grok Build). Returns the address and code to confirm
            // there; the sign-in finishes in the background, and `accounts` (xai) shows when it's done or why it stopped.
            // The app then says `grok_plan_signed_in`, so the bots pick it up.
            "grok_plan_sign_in" => {
                let (url, code) = boxed(crate::grok_plan::start).await?;
                ok(json!({"url": url, "code": code}))
            }
            "grok_plan_signed_in" => {
                self.restart_all();
                ok(())
            }
            "grok_plan_sign_in_cancel" => {
                crate::grok_plan::cancel();
                ok(())
            }
            "grok_plan_sign_out" => {
                crate::grok_plan::sign_out()?;
                self.restart_all();
                ok(())
            }
            "chatgpt_sign_in_cancel" => {
                crate::chatgpt::cancel();
                ok(())
            }
            "chatgpt_sign_out" => {
                boxed(crate::chatgpt::sign_out).await?;
                self.restart_all();
                ok(())
            }
            "set_api_key" => {
                let kind = arg::<Option<String>>(a, "kind")?.unwrap_or_else(|| "openai".into());
                let (_, secret, _) = KEYED.iter().find(|(k, ..)| *k == kind).with_context(|| format!("no API key for {kind}"))?;
                self.secrets.set(secret, arg::<String>(a, "key")?.trim())?;
                self.restart_all();
                ok(())
            }
            // The bots' mailboxes: a list. Saving one signs in once first; the same address again replaces it.
            "email_accounts" => ok(json!({
                "accounts": crate::email::accounts(&self.db).into_iter().map(|a| json!({"user": a.user, "host": a.host, "smtp": a.smtp_host, "smtp_port": a.smtp_port, "outlook": a.oauth.is_some()})).collect::<Vec<_>>(),
                "outlook_client_id": self.db.setting("outlook_client_id")?.unwrap_or_default(),
                "outlook_tenant": self.db.setting("outlook_tenant")?.unwrap_or_default(),
            })),
            "email_account_save" => {
                let port: u16 = arg::<Option<u16>>(a, "port")?.unwrap_or(993);
                let mut acc = crate::email::Account::new(&arg::<String>(a, "host")?, port, &arg::<String>(a, "user")?, &arg::<String>(a, "password")?);
                // Outgoing mail: as given, else worked out from the incoming server; port 465 unless given (587 = STARTTLS).
                let smtp: String = arg::<Option<String>>(a, "smtp_host")?.unwrap_or_default();
                let smtp_port = arg::<Option<u16>>(a, "smtp_port")?.unwrap_or(465);
                if !smtp.trim().is_empty() {
                    acc.smtp_host = smtp.trim().into();
                }
                acc.smtp_port = smtp_port;
                let n = boxed(|| crate::email::add(&self.db, &acc, &smtp, smtp_port)).await?;
                ok(json!({"user": acc.user, "messages": n}))
            }
            "email_account_remove" => ok(crate::email::remove(&self.db, &arg::<String>(a, "user")?)?),
            // Outlook / Microsoft 365: Microsoft's sign-in with the person's own app. Like ChatGPT's: the desktop app waits
            // for the browser (`_wait`), a server takes the pasted address (`_paste`). The client ID is kept for next time.
            "outlook_sign_in" => {
                let (client_id, tenant): (String, String) = (arg(a, "client_id")?, arg::<Option<String>>(a, "tenant")?.unwrap_or_default());
                let paste = self.mode == "server";
                let url = boxed(|| crate::email::outlook_sign_in(&client_id, &tenant, !paste)).await?;
                self.db.set_setting("outlook_client_id", client_id.trim())?;
                self.db.set_setting("outlook_tenant", tenant.trim())?;
                ok(json!({"url": url, "paste": paste}))
            }
            "outlook_sign_in_wait" | "outlook_sign_in_paste" => {
                if !crate::email::outlook_signing_in() {
                    bail!("no Outlook sign-in in progress; start again");
                }
                let answer = if cmd == "outlook_sign_in_wait" {
                    boxed(crate::oauth::finish).await?
                } else {
                    let url: String = arg(a, "url")?;
                    boxed(|| crate::oauth::paste(&url)).await?
                };
                let (user, n) = boxed(|| crate::email::outlook_signed_in(&self.db, &answer)).await?;
                ok(json!({"user": user, "messages": n}))
            }
            // Stop whichever sign-in (Outlook, a tool server) is waiting for the browser.
            "sign_in_cancel" => {
                crate::oauth::cancel();
                ok(())
            }
            // Mail tasks (forward or cc your bot): on or off. Turned on, mail already there isn't taken (each folder is
            // read from now on). They use the first mailbox.
            "email_tasks" => {
                if let Some(on) = arg::<Option<bool>>(a, "on")? {
                    self.db.set_setting("email_tasks", if on { "on" } else { "off" })?;
                    for f in ["INBOX", crate::email::TASK_FOLDER] {
                        self.db.set_setting(&format!("mail_task_seen:{f}"), "")?;
                    }
                }
                ok(self.db.setting("email_tasks")?.as_deref() == Some("on"))
            }
            // The line added to emails a bot sends to someone you haven't written to ("" = none).
            "email_disclose" => ok(crate::email::disclose(&self.db, arg(a, "line")?)?),
            // The memory folder: Markdown copies of your memories; changes made there wait in Needs you.
            "memory_folder" => {
                if let Some(dir) = arg::<Option<String>>(a, "dir")? {
                    crate::vault::set(&self.db, Some(&dir))?;
                }
                ok(crate::vault::folder(&self.db).map(|p| p.to_string_lossy().to_string()).unwrap_or_default())
            }
            // Webhooks out: the address, which events go to it, and whether the text goes too. The secret the bodies
            // are signed with is made (in the keychain) the first time an address is saved, or anew with `rotate`.
            "webhook_out" => {
                if let Some(url) = arg::<Option<String>>(a, "url")? {
                    let url = url.trim();
                    if !url.is_empty() && url::Url::parse(url).map_or(true, |u| !["http", "https"].contains(&u.scheme())) {
                        bail!("not a web address (http:// or https://)");
                    }
                    let events: Vec<String> = arg::<Option<Vec<String>>>(a, "events")?.unwrap_or_default().into_iter().filter(|e| crate::hooks::EVENTS.contains(&e.as_str())).collect();
                    self.db.set_setting("webhook_out_url", url)?;
                    self.db.set_setting("webhook_out_events", &events.join(","))?;
                    self.db.set_setting("webhook_out_text", if arg::<Option<bool>>(a, "text")?.unwrap_or(false) { "on" } else { "off" })?;
                    if !url.is_empty() && (self.secrets.get(crate::hooks::SECRET).is_none() || arg::<Option<bool>>(a, "rotate")?.unwrap_or(false)) {
                        self.secrets.set(crate::hooks::SECRET, &crate::random_hex(24)?)?;
                    }
                }
                let (url, events, text) = crate::hooks::settings(&self.db);
                ok(json!({"url": url, "events": events, "text": text, "secret": self.secrets.get(crate::hooks::SECRET)}))
            }
            // A test body to the saved address, sent now (not through the outbox): says what went wrong.
            "webhook_out_test" => {
                let url = crate::hooks::settings(&self.db).0.context("save an address first")?;
                let body = json!({"event": "test", "at": crate::db::now(), "summary": "A test from Mimir"}).to_string();
                boxed(|| crate::hooks::post(&url, &body)).await?;
                ok(())
            }
            // Adding events to your calendar (calendar_create) over CalDAV: the server, the login and which calendar.
            // Saving signs in and lists the calendars; no password given keeps the saved one (or the email's, same login).
            "caldav_settings" => {
                if let Some(server) = arg::<Option<String>>(a, "server")? {
                    if server.trim().is_empty() {
                        self.db.set_setting("caldav_url", "")?;
                        self.secrets.set(crate::caldav::SECRET, "")?;
                        return ok(Value::Null);
                    }
                    let user = arg::<String>(a, "user")?.trim().to_string();
                    let given = arg::<Option<String>>(a, "password")?.unwrap_or_default();
                    let pass = if given.is_empty() { crate::caldav::password(&self.db, &user).context("the app password for this calendar")? } else { given.clone() };
                    let cals = boxed(|| crate::caldav::discover(&server, &user, &pass)).await?;
                    let pick = arg::<Option<String>>(a, "calendar")?;
                    let (url, name) = cals.iter().find(|c| pick.as_deref() == Some(c.0.as_str())).unwrap_or(&cals[0]).clone();
                    for (k, v) in [("caldav_server", server.trim()), ("caldav_url", &url), ("caldav_user", &user), ("caldav_name", &name)] {
                        self.db.set_setting(k, v)?;
                    }
                    if !given.is_empty() {
                        self.secrets.set(crate::caldav::SECRET, &given)?;
                    }
                    return ok(json!({"name": name, "url": url, "calendars": cals.iter().map(|(u, n)| json!({"url": u, "name": n})).collect::<Vec<_>>()}));
                }
                let email = crate::email::account(&self.db);
                ok(match crate::caldav::account(&self.db) {
                    Some(c) => json!({"name": c.name, "user": c.user, "url": c.url, "server": self.db.setting("caldav_server")?.unwrap_or_default()}),
                    None => json!({"guess": email.as_ref().map(|e| crate::caldav::guess(&e.host)).unwrap_or(""), "user": email.map(|e| e.user).unwrap_or_default()}),
                })
            }
            // Web search for the bots (web_search): Brave Search's key (set_api_key, kind brave) or a SearXNG address; and
            // whether an X bearer token is saved (x_search, set_api_key kind x).
            "search_settings" => {
                if let Some(url) = arg::<Option<String>>(a, "searxng_url")? {
                    let url = url.trim().trim_end_matches('/').to_string();
                    if !(url.is_empty() || url.starts_with("http://") || url.starts_with("https://")) {
                        bail!("a SearXNG address starts with http:// or https://");
                    }
                    self.db.set_setting("searxng_url", &url)?;
                }
                ok(json!({"brave": api_key("brave").is_some(), "x": api_key("x").is_some(), "searxng_url": self.db.setting("searxng_url")?.unwrap_or_default()}))
            }
            // Scanned PDFs read by a local model behind Ollama (ocr.rs): its name (empty: off) and Ollama's address.
            "ocr_settings" => {
                if let Some(model) = arg::<Option<String>>(a, "model")? {
                    self.db.set_setting("ocr_model", model.trim())?;
                }
                if let Some(url) = arg::<Option<String>>(a, "url")? {
                    let url = url.trim().trim_end_matches('/').to_string();
                    if !(url.is_empty() || url.starts_with("http://") || url.starts_with("https://")) {
                        bail!("Ollama's address starts with http:// or https://");
                    }
                    self.db.set_setting("ocr_url", &url)?;
                }
                ok(json!({"model": self.db.setting("ocr_model")?.unwrap_or_default(), "url": self.db.setting("ocr_url")?.unwrap_or_default(), "default_url": crate::ocr::DEFAULT_URL}))
            }
            "chats" => ok(self.db.chats()?),
            // Each topic in a private Telegram chat its own conversation (read per message, so no restart).
            "telegram_topics" => ok(self.db.setting(&format!("telegram_topics:{}", arg::<i64>(a, "bot_id")?))?.as_deref() == Some("on")),
            // The account a bot's routines run on while its own is at a usage limit (get without `provider`; "" turns it off).
            "fallback" => {
                let bot: i64 = arg(a, "bot_id")?;
                if let Some(kind) = arg::<Option<String>>(a, "provider")? {
                    let model = arg::<Option<String>>(a, "model")?.unwrap_or_default();
                    if !kind.is_empty() {
                        // Checked like a bot's own account: a key that isn't there, a local model without a name.
                        Provider::new(&kind, Some(model.clone()), None, api_key(&kind), self.home.clone())?;
                    }
                    self.db.set_fallback(bot, &kind, &model)?;
                }
                ok(self.db.fallback(bot)?.map(|(provider, model)| json!({"provider": provider, "model": model})))
            }
            // A Claude bot on the Anthropic API key (the monthly API credits): see api_credits.
            "api_credits" => self.api_credits(a),
            // The request-match check: off until false asks are measured (scripts/e2e-flows.py turns it on).
            "set_ask_check" => ok(self.db.set_setting("ask_check", if arg(a, "on")? { "on" } else { "off" })?),
            "set_telegram_topics" => ok(self.db.set_setting(&format!("telegram_topics:{}", arg::<i64>(a, "bot_id")?), if arg(a, "on")? { "on" } else { "off" })?),
            // Has this conversation read untrusted content (so changes ask first)? And: start a fresh one.
            // Whether it read outside content, and its title and "so far" line (tidy::catch_up), if it has them yet.
            "conversation" => {
                let (bot, chat): (i64, i64) = (arg(a, "bot_id")?, arg(a, "chat_id")?);
                let c: Value = self.db.setting(&crate::tidy::catch_up_key(bot, chat))?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
                ok(json!({"tainted": self.db.conv_tainted(bot, chat)?, "title": c["title"], "so_far": c["so_far"]}))
            }
            // Each bot's title for its chat on this computer, for the sidebar.
            "chat_titles" => ok(self
                .db
                .bots()?
                .into_iter()
                .filter_map(|b| {
                    let c: Value = serde_json::from_str(&self.db.setting(&crate::tidy::catch_up_key(b.id, LOCAL_CHAT)).ok()??).ok()?;
                    Some(json!({"bot_id": b.id, "title": c["title"].as_str()?}))
                })
                .collect::<Vec<_>>()),
            "new_conversation" => {
                let (bot, chat): (i64, i64) = (arg(a, "bot_id")?, arg(a, "chat_id")?);
                // A fresh start gets a title of its own once its first exchange is in.
                self.db.setting_take(&crate::tidy::catch_up_key(bot, chat))?;
                if let (Some(s), Ok(p)) = (self.db.session(bot, chat)?, self.provider(bot)) {
                    p.forget_session(&s);
                }
                ok(self.db.set_session(bot, chat, None)?)
            }
            // Branch from a message: a fresh conversation that opens with the chat up to it.
            "branch" => {
                let (bot, chat): (i64, i64) = (arg(a, "bot_id")?, arg(a, "chat_id")?);
                ok(crate::branch(&self.db, &self.provider(bot)?, bot, chat, chat, Some(arg(a, "message_id")?))?)
            }
            // Delete a chat's history (conversation, run log, corrections, the AI CLIs' copies); memories stay.
            "clear_history" => {
                let chat: i64 = arg(a, "chat_id")?;
                for b in self.db.bots()? {
                    if let (Some(s), Ok(p)) = (self.db.session(b.id, chat)?, self.provider(b.id)) {
                        p.forget_session(&s);
                    }
                }
                ok(self.db.chat_history_clear(chat)?)
            }
            // Everything Mimir keeps about you, as one JSON file (memories, conversations, meetings, skills, routines,
            // bots without their tokens).
            "export_all" => ok(self.db.export_all()?),
            // Daily copies of the database (backups/ in the data folder), and putting one back.
            "doctor" => ok(boxed(|| crate::doctor::run(&self.home)).await),
            "backups" => ok(json!({"dir": crate::backup::dir(&self.db), "list": crate::backup::list(&self.db)})),
            "backup_now" => ok(crate::backup::snapshot(&self.db, "")?),
            "backup_restore" => {
                let name: String = arg(a, "name")?;
                // Bots stopped around it, so nothing writes into the old database mid-restore; started again either way.
                self.stop_all();
                let done = crate::backup::restore(&self.db, &name);
                self.restart_all();
                ok(done?)
            }
            // Someone else's chat (a guest: nothing of yours there), or yours.
            "chat_guest" => ok(self.db.chat_set_guest(arg(a, "chat_id")?, arg(a, "guest")?)?),
            // A guest's chat or a group reads (and adds to) the household's memories; a family group lets everyone in it talk.
            "chat_household" => ok(self.db.chat_set_household(arg(a, "chat_id")?, arg(a, "on")?)?),
            "chat_family" => ok(self.db.chat_set_family(arg(a, "chat_id")?, arg(a, "on")?)?),
            // One bot keeps a memory to itself, or all bots share it again (bot_id 0).
            "memory_for_bot" => ok(self.db.set_for_bot(&scope(a)?, arg(a, "id")?, arg(a, "bot_id")?)?),
            "entries" => ok(self.db.entries(&scope(a)?)?),
            "search" => ok(self.db.search(&scope(a)?, 0, &arg::<String>(a, "query")?, 50)?),
            "forget" => ok(self.db.forget(&scope(a)?, arg(a, "id")?)?),
            "pin" => ok(self.db.pin(&scope(a)?, arg(a, "id")?, arg(a, "pinned")?)?),
            "background" => ok(self.db.background(&scope(a)?, arg(a, "id")?, arg(a, "on")?)?),
            // An end date (YYYY-MM-DD, through that day), or none: past it, a memory leaves standing memory.
            "set_until" => {
                let until = arg::<Option<String>>(a, "until").unwrap_or(None).filter(|u| !u.is_empty()).map(|u| tools::end_of_day(&u)).transpose()?;
                ok(self.db.set_until(&scope(a)?, arg(a, "id")?, until)?)
            }
            "skills" => ok(self.db.skills(&scope(a)?)?),
            // The chat's file workspace, as the bots keep it: list, read, delete.
            "files" => ok(crate::files::list(&self.db, arg(a, "chat_id")?, "")?.into_iter().map(|(path, size)| json!({"path": path, "size": size})).collect::<Vec<_>>()),
            // Where it is (path and machine), and on the desktop, open it in Finder / Explorer.
            "workspace" => {
                let root = crate::files::root(&self.db, arg(a, "chat_id")?);
                ok(json!({"path": root.display().to_string(), "machine": crate::files::machine(), "exists": root.is_dir(), "can_open": is_desktop()}))
            }
            "open_workspace" => {
                if !is_desktop() {
                    bail!("the folder is on the server; open it there");
                }
                let root = crate::files::root(&self.db, arg(a, "chat_id")?);
                std::fs::create_dir_all(&root)?;
                ok(crate::files::reveal(&root)?)
            }
            // A bot's working folder: where (and on which machine), whether it's there, and open it on the desktop.
            "bot_folder" => {
                let bot = self.db.bot(arg(a, "bot_id")?)?;
                let ws = crate::files::bot_ws(bot.id);
                let root = crate::files::root(&self.db, ws);
                let problem = crate::files::workspace(&self.db, bot.id, 0, true, true).err().map(|e| format!("{e:#}"));
                ok(json!({"path": root.display().to_string(), "chosen": !bot.workdir.is_empty(), "machine": crate::files::machine(), "problem": problem, "can_open": is_desktop()}))
            }
            "open_bot_folder" => {
                if !is_desktop() {
                    bail!("the folder is on the server; open it there");
                }
                let bot: i64 = arg(a, "bot_id")?;
                crate::files::workspace(&self.db, bot, 0, true, true)?;
                let root = crate::files::root(&self.db, crate::files::bot_ws(bot));
                std::fs::create_dir_all(&root)?;
                ok(crate::files::reveal(&root)?)
            }
            "file" => ok(crate::files::read(&self.db, arg(a, "chat_id")?, &arg::<String>(a, "path")?)?.0),
            "delete_file" => ok(crate::files::delete(&self.db, arg(a, "chat_id")?, &arg::<String>(a, "path")?)?),
            // A file a bot sent to the chat ([[file:…]] in its reply), as base64, to show or save.
            "sent_file" => {
                use base64::Engine;
                let (path, name) = crate::files::sent_path(&self.home, arg(a, "chat_id")?, &arg::<String>(a, "name")?).context("that file is gone (sent files are kept 60 days)")?;
                ok(json!({"name": name, "data": base64::engine::general_purpose::STANDARD.encode(std::fs::read(path)?)}))
            }
            // Mimir's skill library for a chat: each skill, which bots there already have it, and per bot the tools it
            // needs that the bot can't use here (a calendar not linked, a tool off for that bot…).
            "skill_library" => {
                let chat: i64 = arg(a, "chat_id")?;
                let have = self.db.skills(&format!("chat:{chat}"))?;
                let bots = self.db.bots()?;
                ok(crate::library::all()
                    .into_iter()
                    .map(|k| {
                        let added: Vec<i64> = have.iter().filter(|s| s.status == "active" && s.name.eq_ignore_ascii_case(&k.name)).map(|s| s.bot_id).collect();
                        let missing: serde_json::Map<String, Value> = bots
                            .iter()
                            .map(|b| {
                                let usable: Vec<String> = tools::specs_for(&ToolCtx::new(self.db.clone(), b.id, chat)).into_iter().map(|t| t.0).collect();
                                (b.id.to_string(), json!(k.needs.iter().filter(|t| !usable.iter().any(|u| u == *t)).collect::<Vec<_>>()))
                            })
                            .collect();
                        // Routines run a skill when their prompt is its command.
                        let schedule = k.schedule.map(|(cron, label)| json!({"cron": cron, "label": label, "prompt": format!("/{}", crate::skill_slug(&k.name))}));
                        json!({"slug": k.slug, "category": k.category, "name": k.name, "description": k.description, "body": k.body, "needs": k.needs, "added": added, "missing": missing, "schedule": schedule})
                    })
                    .collect::<Vec<_>>())
            }
            "skill_add" => {
                let k = crate::library::get(&arg::<String>(a, "slug")?).context("no such skill in the library")?;
                ok(self.db.skill_from_library(&scope(a)?, arg(a, "bot_id")?, &k.name, &k.description, &k.body)?)
            }
            // Teach a bot a skill from a link or pasted text: a proposal to read and accept.
            "teach_skill" => {
                let bot: i64 = arg(a, "bot_id")?;
                let (p, chat, source): (_, i64, String) = (self.provider(bot)?, arg(a, "chat_id")?, arg(a, "source")?);
                ok(boxed(|| crate::tidy::teach(&self.db, &p, bot, chat, &source)).await?)
            }
            // A skill as a SKILL.md, to share or use in another agent.
            "export_skill" => {
                let id: i64 = arg(a, "id")?;
                let k = self.db.skills(&scope(a)?)?.into_iter().find(|k| k.id == id).context("no such skill")?;
                ok(crate::tidy::skill_md(&k))
            }
            "restore_skill" => ok(self.db.skill_restore(&scope(a)?, arg(a, "id")?)?),
            "removed_skills" => ok(self.db.skills_removed(&scope(a)?)?),
            // For all bots in this chat (bot_id 0), or one bot.
            "share_skill" => ok(self.db.skill_share(&scope(a)?, arg(a, "id")?, arg(a, "bot_id")?)?),
            "decide_skill" => ok(self.db.skill_decide(&scope(a)?, arg(a, "id")?, arg(a, "accept")?)?),
            "remove_skill" => ok(self.db.skill_remove(&scope(a)?, arg(a, "id")?)?),
            "undo_memory" => ok(self.db.undo(&scope(a)?, arg(a, "id")?)?),
            // A button in a tool server's card (MCP Apps): its tool, through the tool gate.
            "card_call" => ok(crate::ext::card_call(&self.db, &arg::<String>(a, "key")?, &arg::<String>(a, "name")?, &a["arguments"]).await?),
            // The weekly look back: how the last four weeks went, and when you last reviewed what was learned.
            "learning" => {
                let chat: i64 = arg(a, "chat_id")?;
                let seen = self.db.setting(&format!("learned_seen:{chat}"))?.and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                ok(json!({"weeks": self.db.weeks(chat, 4, crate::db::now())?, "seen": seen}))
            }
            "learned_seen" => ok(self.db.set_setting(&format!("learned_seen:{}", arg::<i64>(a, "chat_id")?), &crate::db::now().to_string())?),
            "confirm_memory" => ok(self.db.confirm(&scope(a)?, arg(a, "id")?)?),
            // Gone for good, with its earlier versions (forget keeps it in History).
            "erase" => ok(self.db.erase(&scope(a)?, arg(a, "id")?)?),
            // Memories pasted from another assistant become suggestions to accept or dismiss.
            "import_memory" => {
                let bot = crate::tidy::memory_bot(&self.db).context("no bots")?;
                let (p, chat, text): (_, i64, String) = (self.provider(bot)?, arg(a, "chat_id")?, arg(a, "text")?);
                ok(boxed(|| crate::tidy::import(&self.db, &p, bot, chat, &text)).await?)
            }
            // Other agents on this computer (Claude Code, Codex, Hermes, OpenClaw) and what they have; `import_agent` brings
            // one in as suggestions, skill proposals and paused routines.
            "import_sources" => ok(crate::imports::sources()),
            "import_agent" => {
                let bot = crate::tidy::memory_bot(&self.db).context("no bots")?;
                let (p, chat, source): (_, i64, String) = (self.provider(bot)?, arg(a, "chat_id")?, arg(a, "source")?);
                ok(boxed(|| crate::imports::run(&self.db, &p, bot, chat, &source)).await?)
            }
            // A bot as one file (no secrets, memories or chats); reading one back shows what it would add, and only
            // `import_bot` adds it: in the built-in chat, skills as proposals, routines paused.
            "export_bot" => ok(crate::imports::export_bot(&self.db, arg(a, "id")?)?),
            "import_bot_preview" => ok(crate::imports::preview(&self.db, &arg::<String>(a, "text")?, arg::<Option<i64>>(a, "duplicate_of")?)?),
            "import_bot" => {
                let (id, done) = crate::imports::import_bot(&self.db, &arg::<String>(a, "text")?, LOCAL_CHAT, arg::<Option<bool>>(a, "rules")?.unwrap_or(false), arg::<Option<i64>>(a, "duplicate_of")?)?;
                // An AI account this Mimir can't use: the one your first bot uses instead.
                let mut bot = self.db.bot(id)?;
                if Provider::for_bot(&bot, self.home.clone()).is_err()
                    && let Some(b) = self.db.bots()?.into_iter().find(|b| b.id != id)
                {
                    (bot.provider, bot.model, bot.base_url) = (b.provider, b.model, b.base_url);
                    self.db.bot_save(&bot)?;
                }
                self.restart(id);
                ok(json!({"id": id, "skills": done.skills, "routines": done.routines}))
            }
            // Add a memory, or correct one (`supersedes`); history is kept either way.
            "remember" => ok(self.db.store_by(
                &scope(a)?,
                &arg::<String>(a, "kind")?,
                &arg::<String>(a, "title")?,
                &arg::<String>(a, "body")?,
                "edited in app",
                arg(a, "supersedes")?,
                &crate::db::Prov::by("you"),
            )?.0),
            "suggestions" => ok(self.db.suggestions(&scope(a)?)?),
            // The one-time "get to know me" pass: whether it can run (email or calendar connected) and has; `run` starts it.
            "know_me" => {
                if arg::<Option<bool>>(a, "run")?.unwrap_or(false) {
                    let bot = crate::tidy::memory_bot(&self.db).context("no bots")?;
                    let (p, chat): (_, i64) = (self.provider(bot)?, arg(a, "chat_id")?);
                    return ok(boxed(|| crate::tidy::get_to_know(&self.db, &p, bot, chat)).await?);
                }
                let ready = crate::email::account(&self.db).is_some() || secret_get(crate::calendar::SECRET).is_some();
                ok(json!({"ready": ready, "done": self.db.setting(crate::tidy::KNOW_ME)?.is_some()}))
            }
            // Replay past corrections against a suggested preference (on request: it costs model calls).
            "check_suggestion" => {
                let bot = crate::tidy::memory_bot(&self.db).context("no bots")?;
                let provider = self.provider(bot)?;
                let (chat, id): (i64, i64) = (arg(a, "chat_id")?, arg(a, "id")?);
                ok(boxed(|| crate::tidy::check(&self.db, &provider, bot, chat, id)).await?)
            }
            // The same replay for a preference already accepted, on the memory bot's own model (after switching models).
            "check_memory" => {
                let bot = crate::tidy::memory_bot(&self.db).context("no bots")?;
                let provider = self.provider(bot)?;
                let (chat, id): (i64, i64) = (arg(a, "chat_id")?, arg(a, "id")?);
                ok(boxed(|| crate::tidy::check_memory(&self.db, &provider, bot, chat, id)).await?)
            }
            "decide_suggestion" => ok(self.db.suggestion_decide(&scope(a)?, arg(a, "id")?, arg(a, "accept")?)?),
            // Keep the other one of a pair (duplicate, contradiction), or forget a stale memory rather than move it.
            "forget_for_suggestion" => ok(self.db.suggestion_forget(&scope(a)?, arg(a, "id")?, arg(a, "entry_id")?)?),
            // Run the tidy-up now (the UI's "Tidy up now"); returns how many suggestions were queued.
            "tidy" => {
                let bot = crate::tidy::memory_bot(&self.db).context("no bots")?;
                let (p, chat): (_, i64) = (self.provider(bot)?, arg(a, "chat_id")?);
                ok(boxed(|| crate::tidy::run(&self.db, &p, bot, chat)).await?)
            }
            // ---- external tools & saved logins ----
            // Configured servers with their tool counts. Env and headers show names only; values stay in the secret store.
            "tool_servers" => {
                let sensitive = tools::sensitive_list(&self.db);
                // `update`: a newer settled release than the one its package is pinned to (or a pin, for one that isn't).
                let mut servers: Vec<Value> = vec![];
                for s in ext::servers(&self.db) {
                    let keys = |m: std::collections::HashMap<String, String>| m.into_keys().collect::<std::collections::BTreeSet<_>>();
                    let update = if s.preset.is_none() { boxed(|| ext::update_for(&self.db, &s)).await } else { None };
                    servers.push(json!({"name": s.name, "preset": s.preset, "command": s.command, "args": s.args, "url": s.url, "env": keys(s.env), "headers": keys(s.headers), "tools": ext::cached_tools(&self.db, &s.name).len(), "signed_in": ext::signed_in(&s.name), "sensitive": sensitive.contains(&s.name), "suggest_sensitive": tools::looks_sensitive(&s.name), "update": update, "unconfirmed": ext::claimed_tools(&self.db, &s.name).iter().filter(|t| t.read_only).count() - ext::cached_tools(&self.db, &s.name).iter().filter(|t| t.read_only).count()}));
                }
                ok(json!({"servers": servers, "tools": ext::all_tools(&self.db).len()}))
            }
            // Add or replace one server: {name, command, args, env} or {name, url, headers}. Its package is pinned to a
            // release (see ext::pin). Starts it once; returns its tool count.
            "tool_server_save" => {
                let server = arg(a, "server")?;
                ok(boxed(|| ext::save_server(&self.db, server)).await?)
            }
            // Hosted servers in the official MCP registry, on their publisher's own domain (see ext::registry_pick).
            "tool_server_registry" => {
                let q: String = arg(a, "q")?;
                ok(boxed(|| ext::registry_search(&q)).await?)
            }
            "tool_server_update" => {
                let name: String = arg(a, "name")?;
                ok(boxed(|| ext::update_server(&self.db, &name)).await?)
            }
            // Sensitive sources: once a run has read one, everything that goes out asks. Only you set them.
            "sensitive" => ok(tools::sensitive_list(&self.db)),
            "set_sensitive" => {
                let (name, on): (String, bool) = (arg(a, "name")?, arg(a, "on")?);
                let known = ext::servers(&self.db).iter().any(|s| s.name == name)
                    || tools::all_specs(&self.db).iter().any(|(n, ..)| *n == name)
                    || name.strip_prefix("chat:").is_some_and(|c| c.parse::<i64>().is_ok());
                // Turning one off always works: its server may be gone (deleted before this was cleaned up with it).
                if on && !known {
                    bail!("no tool server, tool or memory called {name}");
                }
                let mut list: Vec<String> = tools::sensitive_list(&self.db).into_iter().filter(|s| *s != name).collect();
                if on {
                    list.push(name);
                }
                ok(self.db.set_setting("sensitive", &serde_json::to_string(&list)?)?)
            }
            "tool_server_delete" => ok(ext::delete_server(&self.db, &arg::<String>(a, "name")?)?),
            // One server's tools with the server's read-only claims, which count once confirmed (`tool_server_read_only`).
            "tool_server_tools" => {
                let name: String = arg(a, "name")?;
                let trusted = ext::cached_tools(&self.db, &name);
                let tools: Vec<Value> = ext::claimed_tools(&self.db, &name)
                    .into_iter()
                    .map(|t| json!({"name": t.name, "description": t.description, "says_read_only": t.read_only, "read_only": trusted.iter().any(|o| o.name == t.name && o.read_only)}))
                    .collect();
                ok(tools)
            }
            "tool_server_read_only" => ok(ext::confirm_read_only(&self.db, &arg::<String>(a, "name")?, &arg::<Vec<String>>(a, "tools")?)?),
            // A remote server that wants a sign-in (MCP's OAuth): returns the address to open; then `_wait` (desktop) or
            // `_paste` (a server: the address the browser ended up on) finishes it and saves the server. Returns its tool count.
            // `client`: optional [client id, secret] of an app the person registered at a service that doesn't register apps.
            "tool_server_sign_in" => {
                let paste = self.mode == "server";
                let server = arg(a, "server")?;
                let client = arg(a, "client")?;
                ok(json!({"url": boxed(|| ext::sign_in(server, !paste, client)).await?, "paste": paste}))
            }
            "tool_server_sign_in_wait" | "tool_server_sign_in_paste" => {
                if !ext::signing_in() {
                    bail!("no tool server sign-in in progress; start again");
                }
                if cmd == "tool_server_sign_in_wait" {
                    boxed(crate::oauth::finish).await?
                } else {
                    let url: String = arg(a, "url")?;
                    boxed(|| crate::oauth::paste(&url)).await?
                };
                ok(boxed(|| ext::signed_in_save(&self.db)).await?)
            }
            // The browser = Playwright's MCP server (needs Node). Enabling starts it once to list its tools.
            "browser_enable" => {
                let on: bool = arg(a, "enabled")?;
                let mut list = ext::servers(&self.db);
                list.retain(|s| s.name != ext::BROWSER);
                if on {
                    // With the bots' computer on, the browser is its Chromium: nothing to download.
                    if !crate::computer::usable(&self.db) {
                        boxed(ext::install_browser).await?;
                    }
                    let conf = ext::ServerConf { name: ext::BROWSER.into(), preset: Some("playwright".into()), ..Default::default() };
                    let n = boxed(|| ext::refresh(&self.db, &conf)).await.context("couldn't start the browser (is Node.js installed?)")?;
                    list.push(conf);
                    ext::set_servers(&self.db, &list)?;
                    return ok(n);
                }
                ext::set_servers(&self.db, &list)?;
                ok(0)
            }
            // ---- meetings and calls (recorded and transcribed by the desktop app) ----
            "meeting" => {
                let id: i64 = arg(a, "id")?;
                let m = self.db.meetings()?.into_iter().find(|m| m["id"] == id).context("no such meeting")?;
                let lines: Vec<Value> = self.db.meeting_lines(id)?.into_iter().map(|(ts, who, text)| json!({"ts": ts, "who": who, "text": text})).collect();
                ok(json!({"meeting": m, "lines": lines}))
            }
            "meeting_speaker" => ok(self.db.meeting_speaker(arg(a, "id")?, &arg::<String>(a, "key")?, &arg::<String>(a, "name")?)?),
            "meeting_summary" => ok(self.db.meeting_summary(arg(a, "id")?, &arg::<String>(a, "summary")?)?),
            "meeting_notes" => ok(self.db.meeting_notes(arg(a, "id")?, &arg::<String>(a, "notes")?)?),
            "meeting_rename" => ok(self.db.meeting_update(arg(a, "id")?, Some(&arg::<String>(a, "title")?), None)?),
            "meeting_delete" => ok(self.db.meeting_delete(arg(a, "id")?)?),
            // ---- the bots' computer (self-hosted Linux server) ----
            "computer" => {
                let running = tokio::task::spawn_blocking(crate::computer::running).await?;
                ok(json!({"available": crate::computer::available(), "enabled": crate::computer::enabled(&self.db), "running": running}))
            }
            "computer_enable" => {
                let on: bool = arg(a, "on")?;
                if on && !crate::computer::available() {
                    bail!("the bots' computer runs on a self-hosted Linux server (the Mimir Docker image has everything it needs)");
                }
                self.db.set_setting("computer", if on { "on" } else { "off" })?;
                let home = self.home.clone();
                if on {
                    tokio::task::spawn_blocking(move || crate::computer::start(&home, None)).await??;
                } else {
                    crate::computer::stop(&self.home);
                }
                ok(json!({"enabled": on, "running": tokio::task::spawn_blocking(crate::computer::running).await?}))
            }
            // Sign a bot out of every site on the computer.
            "computer_forget" => {
                let (home, bot) = (self.home.clone(), arg::<i64>(a, "botId")?);
                ok(tokio::task::spawn_blocking(move || crate::computer::forget(&home, bot)).await??)
            }
            // The live view: the screen as a JPEG (base64), for watching in the app.
            "computer_screen" => {
                if !crate::computer::usable(&self.db) {
                    bail!("the computer is off");
                }
                use base64::Engine;
                let jpg = tokio::task::spawn_blocking(crate::computer::screenshot_jpeg).await??;
                // Who's driving: "you" (took over), a bot's name, or nobody.
                let using = self.db.computer_holder()?.map(|(who, name)| if who == "you" { "you".to_string() } else { name });
                let teaching = crate::computer::teaching().map(|(secs, steps)| json!({"seconds": secs, "steps": steps, "limit": crate::computer::TEACH_SECS}));
                ok(json!({"image": base64::engine::general_purpose::STANDARD.encode(jpg), "width": crate::computer::WIDTH, "height": crate::computer::HEIGHT, "using": using, "teaching": teaching}))
            }
            // Teach by showing: record what you do while you're in control, then a bot drafts a skill from it.
            "computer_teach" => match arg::<String>(a, "action")?.as_str() {
                "start" => {
                    if !crate::computer::usable(&self.db) {
                        bail!("the computer is off");
                    }
                    self.db.computer_lease("you", "you", crate::computer::TEACH_SECS as i64)?;
                    crate::computer::teach_start();
                    crate::computer::sniff(&self.home, crate::computer::TEACH_SECS);
                    ok(true)
                }
                "stop" => {
                    let shown = crate::computer::teach_stop(6).context("it isn't recording")?;
                    self.db.computer_release("you")?;
                    if shown.steps.is_empty() {
                        shown.shots.iter().for_each(|p| drop(std::fs::remove_file(p)));
                        bail!("nothing was recorded: click into the screen and do the task, then press Done");
                    }
                    let bot: i64 = arg(a, "bot_id")?;
                    let (p, chat): (_, i64) = (self.provider(bot)?, arg(a, "chat_id")?);
                    ok(boxed(|| crate::tidy::teach_shown(&self.db, &p, bot, chat, shown)).await?)
                }
                _ => {
                    if let Some(s) = crate::computer::teach_stop(0) {
                        s.shots.iter().for_each(|p| drop(std::fs::remove_file(p)));
                    }
                    ok(self.db.computer_release("you")?)
                }
            },
            // Hand the computer back to the bots after taking over.
            "computer_release" => ok(self.db.computer_release("you")?),
            // Taking over: your own click, typing, keys or scroll on the computer.
            "computer_input" => {
                if !crate::computer::usable(&self.db) {
                    bail!("the computer is off");
                }
                let _in_order = self.input.lock().await;
                // Taking over: bots pause until a minute after your last input (or Give back); while showing, until it ends.
                let secs = if crate::computer::teaching().is_some() { crate::computer::TEACH_SECS as i64 } else { 60 };
                self.db.computer_lease("you", "you", secs)?;
                crate::computer::sniff(&self.home, secs as u64);
                boxed(|| crate::computer::teach_note(&self.home, a)).await;
                let a = a.clone();
                tokio::task::spawn_blocking(move || -> Result<()> {
                    let n = |k: &str| a.get(k).and_then(Value::as_i64).unwrap_or(0);
                    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    match s("kind").as_str() {
                        "click" => crate::computer::click(n("x"), n("y"), &s("button"), a.get("double").and_then(Value::as_bool).unwrap_or(false)),
                        "type" => crate::computer::type_text(&s("text")),
                        "key" => crate::computer::key(&s("keys")),
                        "scroll" => crate::computer::scroll(n("x"), n("y"), s("direction") == "down", n("amount").max(3)),
                        k => bail!("unknown input {k}"),
                    }
                })
                .await??;
                ok(true)
            }
            "vault" => ok(ext::vault(&self.db)),
            "vault_save" => ok(ext::vault_save(&self.db, arg(a, "entry")?, arg::<Option<String>>(a, "value")?.as_deref(), arg::<Option<String>>(a, "totp").unwrap_or(None).as_deref())?),
            // The masked form a bot's sign-in card links to. What you type goes from here into the page on the
            // computer's screen (only while it's on the card's site) and into the secret store as that bot's login
            // for the site: never into a chat, the database or a prompt. The card is used once, and runs out with it.
            // The newest sign-in card waiting, for the form: its site, the page's full address and its warnings.
            "signin_card" => match self.db.approvals_pending()?.into_iter().rfind(|a| a.kind == "signin") {
                Some(ap) => {
                    let mut card = self.db.signin_card(ap.id)?;
                    card["id"] = json!(ap.id);
                    card["bot_id"] = json!(ap.bot_id);
                    card["site"] = json!(ap.site);
                    ok(card)
                }
                None => ok(Value::Null),
            },
            "signin_submit" => {
                let text = |k: &str| arg::<Option<String>>(a, k).map(Option::unwrap_or_default);
                let (user, password, code, key) = (text("user")?, text("password")?, text("code")?.trim().to_string(), text("totp")?.trim().to_string());
                // A mistyped key is said before the card is used up.
                let totp = (!key.is_empty()).then(|| crate::totp::parse(&key)).transpose()?;
                if user.is_empty() && password.is_empty() && code.is_empty() && totp.is_none() {
                    bail!("fill in your sign-in first");
                }
                let ap = self.db.approval_claim(arg(a, "id")?)?.context("this sign-in has been used or has run out; ask the bot again")?;
                let site = ap.site.clone().unwrap_or_default();
                let login = crate::computer::Login { user, password, code, totp };
                let dir = self.db.dir();
                let signing_in = boxed(|| crate::computer::sign_in(&dir, ap.bot_id, &login, std::slice::from_ref(&site)));
                let out = match tokio::time::timeout(std::time::Duration::from_secs(60), signing_in).await {
                    Ok(r) => r,
                    Err(_) => Err(anyhow::anyhow!("the page didn't answer in time")),
                };
                // Kept only once it worked, so a mistyped password isn't saved.
                let kept = match &out {
                    Ok(true) if !login.password.is_empty() => ext::vault_save_signin(&self.db, ap.bot_id, &site, &login.user, &login.password, Some(&key)).map(Some),
                    _ => Ok(None),
                };
                self.db.approval_signed_in(ap.id, matches!(out, Ok(true)) && kept.is_ok())?;
                let signed_in = out?;
                ok(json!({"signed_in": signed_in, "saved": kept?}))
            }
            "vault_delete" => ok(ext::vault_delete(&self.db, &arg::<String>(a, "name")?)?),
            // Which bots are working right now, on what (one entry per bot, its newest run), and the step a reply is
            // on: their avatars come alive in the sidebar.
            "busy" => {
                let mut seen = std::collections::HashSet::new();
                ok(self
                    .db
                    .busy()?
                    .into_iter()
                    .filter(|(bot, ..)| seen.insert(*bot))
                    .map(|(bot, chat, trigger)| json!({"bot_id": bot, "chat_id": chat, "trigger": trigger, "step": crate::live::step(bot, chat)}))
                    .collect::<Vec<_>>())
            }
            // The reply being written right now (streams while send_local is running).
            "draft" => ok(crate::live::get(arg(a, "bot_id")?, arg(a, "chat_id")?)),
            // A reply in progress: the text so far, what it's doing now, the steps it has finished, and its checklist.
            "live" => {
                let (bot, chat): (i64, i64) = (arg(a, "bot_id")?, arg(a, "chat_id")?);
                let done = match self.db.running_run(bot, chat)? {
                    // Ticking the checklist shows as the checklist itself.
                    Some(run) => self.db.tool_calls(run)?.into_iter().filter(|c| c.tool != "progress_set").map(|c| json!({"summary": c.summary, "ok": c.ok, "decision": c.decision})).collect(),
                    None => vec![],
                };
                let checklist: Vec<Value> = crate::tools::progress(&self.db, bot, chat).into_iter().map(|(step, done)| json!({"step": step, "done": done})).collect();
                ok(json!({"draft": crate::live::get(bot, chat), "step": crate::live::step(bot, chat), "done": done, "checklist": checklist}))
            }
            // Stop from the Runs list: a reply in a messaging app (it records itself as stopped and says so there), or
            // one in the app (below).
            "stop_run" => {
                let id: i64 = arg(a, "id")?;
                let (bot, chat) = self.db.run_where(id)?.context("no such run")?;
                if let Some(h) = crate::runtime::TURNS.lock().unwrap().remove(&(bot, chat)) {
                    h.abort();
                    return ok(true);
                }
                Box::pin(self.call("stop", &json!({"bot_id": bot, "chat_id": chat}))).await
            }
            // A message sent while the app's reply is being written: true if that reply takes it in (it changes
            // course), false if nothing there takes one now (send it as a message of its own).
            "steer" => {
                let (bot, chat, text): (i64, i64, String) = (arg(a, "bot_id")?, arg(a, "chat_id")?, arg(a, "text")?);
                ok(chat != crate::INCOGNITO_CHAT && !text.trim().is_empty() && crate::live::steer(bot, chat, text.trim()))
            }
            // Stop the reply in progress: kill the turn, keep what was asked and written so far.
            "stop" => {
                let (bot, chat): (i64, i64) = (arg(a, "bot_id")?, arg(a, "chat_id")?);
                let Some((handle, asked)) = self.running.lock().unwrap().remove(&(bot, chat)) else { return ok(false) };
                let partial = crate::live::get(bot, chat).unwrap_or_default();
                let steered = crate::live::steered(bot, chat);
                handle.abort();
                crate::live::end(bot, chat);
                if chat == crate::INCOGNITO_CHAT {
                    return ok(partial);
                }
                self.db.run_stop(bot, chat, &asked)?;
                for t in std::iter::once(&asked).chain(&steered) {
                    self.db.log(bot, chat, "user", t, None)?;
                }
                let note = if partial.trim().is_empty() { "_Stopped before replying._".to_string() } else { format!("{}\n\n_Stopped._", partial.trim_end()) };
                self.db.log(bot, chat, "assistant", &note, None)?;
                ok(true)
            }
            "routines" => ok(self.db.routines(arg(a, "chat_id")?)?),
            "routine_stats" => ok(self.db.routine_stats(arg(a, "chat_id")?, crate::db::now())?),
            "add_routine" => {
                let (chat, cron): (i64, String) = (arg(a, "chat_id")?, arg(a, "cron")?);
                crate::events::ready(&self.db, chat, &cron)?;
                let r = self.db.routine_create(arg(a, "bot_id")?, chat, &cron, &arg::<String>(a, "prompt")?, arg::<Option<bool>>(a, "quiet")?.unwrap_or(false))?;
                if arg::<Option<bool>>(a, "first_only")?.unwrap_or(false) && crate::events::on(&cron)?.is_some() {
                    self.db.routine_set_once(r.id)?;
                }
                ok(r)
            }
            // A webhook routine's address (on this server) and the secret its caller signs with.
            "webhook" => {
                let (chat, id): (i64, i64) = (arg(a, "chat_id")?, arg(a, "id")?);
                self.db.routines(chat)?.into_iter().find(|r| r.id == id && crate::events::is_hook(&r.cron)).context("no such webhook routine")?;
                ok(json!({"path": format!("/hook/{id}"), "secret": crate::events::webhook_secret(&self.db, id)?}))
            }
            // Run a routine now, as a test: the result lands in the chat whatever its delivery setting.
            "run_routine" => {
                let (chat, id): (i64, i64) = (arg(a, "chat_id")?, arg(a, "id")?);
                let mut r = self.db.routines(chat)?.into_iter().find(|r| r.id == id).context("no such routine")?;
                r.delivery = "now".into();
                let provider = self.provider(r.bot_id)?;
                let ctx = ToolCtx::new(self.db.clone(), r.bot_id, chat);
                let reply = boxed(|| turn(&provider, &ctx, Trigger::Routine(&r))).await?;
                ok(if reply.trim().is_empty() { "Nothing needed your attention.".to_string() } else { reply })
            }
            // A rehearsal of a routine or a skill: it runs for real, except that anything that would change or send
            // something (or ask you) only says what it would do. Nothing lands in the chat.
            "rehearse" => {
                let chat: i64 = arg(a, "chat_id")?;
                let mut r = match arg::<Option<i64>>(a, "routine_id")? {
                    Some(id) => self.db.routines(chat)?.into_iter().find(|r| r.id == id).context("no such routine")?,
                    None => {
                        let (bot, skill): (i64, i64) = (arg(a, "bot_id")?, arg(a, "skill_id")?);
                        let k = self.db.skills_active(&format!("chat:{chat}"), bot)?.into_iter().find(|k| k.id == skill).context("no such skill for this bot")?;
                        let input = arg::<Option<String>>(a, "input")?.unwrap_or_default();
                        crate::db::Routine {
                            id: 0,
                            bot_id: bot,
                            chat_id: chat,
                            cron: String::new(),
                            prompt: format!("/{} {}", crate::skill_slug(&k.name), input.trim()),
                            next_fire: 0,
                            quiet: false,
                            paused: false,
                            delivery: String::new(),
                            tainted: false,
                            once: true,
                            watch: String::new(),
                            retries: 0,
                            urgent: String::new(),
                            ends: 0,
                            followup: false,
                            goal_held: false,
                            for_goal: false,
                        }
                    }
                };
                r.delivery = "log".into();
                r.prompt.push_str(
                    "\n\n[Rehearsal run: do this exactly as in a real run, calling your tools as you normally would. It's safe: Mimir \
                     catches every tool call that would change or send something and answers with what would have happened instead. \
                     Then reply with what you found and what you did.]",
                );
                let provider = self.provider(r.bot_id)?;
                let mut ctx = ToolCtx::new(self.db.clone(), r.bot_id, chat);
                ctx.rehearsal = true;
                let reply = boxed(|| turn(&provider, &ctx, Trigger::Routine(&r))).await?;
                let run = self.db.runs(Some(chat), 20)?.into_iter().find(|x| x.trigger == "rehearsal" && x.bot_id == r.bot_id).map(|x| x.id);
                let (would, looked) = match run {
                    Some(run) => self.db.rehearsal_steps(run)?,
                    None => (vec![], vec![]),
                };
                ok(json!({"reply": reply, "would": would, "looked": looked}))
            }
            // A photo pasted or dropped into the chat: base64 in, a path under <home>/media out.
            // A PDF, Word, Excel or text file dropped into the app's chat: its text, wrapped for the bot as outside
            // content. The conversation is marked as having read outside content, like a web page. A scanned PDF
            // gives {text, images}: its pages saved as photos to send with the message.
            "read_document" => {
                use base64::Engine;
                let (bot, chat, name): (i64, i64, String) = (arg(a, "bot_id")?, arg(a, "chat_id")?, arg(a, "name")?);
                let bytes = base64::engine::general_purpose::STANDARD.decode(arg::<String>(a, "data")?).context("bad file data")?;
                if bytes.len() > 20 * 1024 * 1024 {
                    bail!("that file is over 20 MB");
                }
                let said = arg::<String>(a, "text").unwrap_or_default();
                let text = match boxed(|| crate::read_document_text(&name, "", bytes.clone())).await {
                    None => bail!("I can read PDFs, Word and Excel files and text files, not this kind of file yet"),
                    Some(r) => r?,
                };
                let pages = if text.split_whitespace().count() < 5 && crate::is_pdf(&name, "") { boxed(|| crate::read_pdf_pages(bytes)).await } else { vec![] };
                if text.split_whitespace().count() < 5 && pages.is_empty() {
                    bail!("that file has no text I can read (a scanned PDF?); send photos of the pages instead");
                }
                if chat != crate::INCOGNITO_CHAT {
                    self.db.conv_taint(bot, chat)?;
                }
                if pages.is_empty() {
                    return ok(crate::with_document(&said, &name, &text));
                }
                // Read by a local model where one is set; what it couldn't read still goes as photos.
                let (read, pages) = boxed(|| crate::ocr::read_pages(&self.db, pages)).await;
                if pages.is_empty() {
                    return ok(crate::with_document(&said, &name, &read));
                }
                let dir = self.home.join("media");
                std::fs::create_dir_all(&dir)?;
                let mut images = vec![];
                for (i, p) in pages.iter().enumerate() {
                    let path = dir.join(format!("{}-{}-p{}.jpg", crate::db::now(), p.len(), i + 1));
                    std::fs::write(&path, p)?;
                    images.push(path.display().to_string());
                }
                let text = if read.is_empty() { crate::with_scan(&said, &name, images.len()) } else { crate::with_document(&said, &name, &read) };
                ok(json!({"text": text, "images": images}))
            }
            "save_media" => {
                use base64::Engine;
                let ext: String = arg(a, "ext")?;
                if !["png", "jpg", "jpeg", "gif", "webp"].contains(&ext.as_str()) {
                    bail!("only photos (png, jpg, gif, webp) for now");
                }
                let bytes = base64::engine::general_purpose::STANDARD.decode(arg::<String>(a, "data")?).context("bad image data")?;
                if bytes.len() > 15 * 1024 * 1024 {
                    bail!("that photo is over 15 MB");
                }
                let dir = self.home.join("media");
                std::fs::create_dir_all(&dir)?;
                let path = dir.join(format!("{}-{}.{ext}", crate::db::now(), bytes.len()));
                std::fs::write(&path, bytes)?;
                ok(path.display().to_string())
            }
            "reset_watch_noise" => ok(self.db.routine_reset_noise(arg(a, "chat_id")?, arg(a, "id")?)?),
            "set_watch" => ok(self.db.routine_set_watch(arg(a, "chat_id")?, arg(a, "id")?, &arg::<String>(a, "watch")?)?),
            // The data a watched page loaded on the bots' computer while you were in control, to watch instead.
            "watch_offer" => {
                let offers = crate::computer::page_data(arg(a, "bot_id")?, &arg::<String>(a, "url")?);
                ok(offers.into_iter().map(|(url, holds)| json!({"url": url, "holds": holds})).collect::<Vec<_>>())
            }
            // Only the person sets this (no bot tool does): what the bot read can't widen what comes through.
            "set_ends" => ok(self.db.routine_set_ends(arg(a, "chat_id")?, arg(a, "id")?, arg(a, "ends")?)?),
            "set_urgent" => ok(self.db.routine_set_urgent(arg(a, "chat_id")?, arg(a, "id")?, &arg::<String>(a, "rule")?)?),
            // Pinned cards: the chat's, and your own edits to them (a tick writes back as yours; bots see it next read).
            "cards" => ok(self.db.cards(arg(a, "chat_id")?)?),
            "card_tick" => ok(self.db.card_tick(arg(a, "chat_id")?, arg(a, "id")?, arg(a, "line")?, &arg::<String>(a, "text")?)?),
            "card_delete" => ok(self.db.card_delete(arg(a, "chat_id")?, arg(a, "id")?)?),
            // The chat's goal, and yours to pause, resume, mark done or remove: local, whatever the AI is doing. Pausing or
            // removing it also cuts off tasks it handed to other bots.
            "goal" => ok(self.db.goal(arg(a, "chat_id")?)?),
            "goal_status" => {
                crate::runtime::stop_handed(&self.db.goal_status(arg(a, "chat_id")?, &arg::<String>(a, "status")?, "")?);
                ok(self.db.goal(arg(a, "chat_id")?)?)
            }
            "goal_delete" => {
                crate::runtime::stop_handed(&self.db.goal_delete(arg(a, "chat_id")?)?);
                ok(true)
            }
            "set_delivery" => ok(self.db.routine_set_delivery(arg(a, "chat_id")?, arg(a, "id")?, &arg::<String>(a, "delivery")?)?),
            // How full standing memory is (the Memory tab's meter).
            "standing_memory" => {
                let (used, budget, count) = self.db.standing_usage(&scope(a)?)?;
                ok(json!({"used": used, "budget": budget, "count": count}))
            }
            // The model for the background memory tidy-up (get without `model`, set with it; blank = the bot's own).
            // A lighter model for every routine, per kind of account its bots use (a model name only means something
            // to its own kind). Set with kind and model ("" = the bot's own model).
            "routine_model" => {
                if let (Some(kind), Some(m)) = (arg::<Option<String>>(a, "kind")?, arg::<Option<String>>(a, "model")?) {
                    if !["claude", "codex", "grok", "api"].contains(&kind.as_str()) {
                        bail!("unknown kind of account");
                    }
                    self.db.set_setting(&format!("routine_model:{kind}"), m.trim())?;
                }
                let mut kinds: Vec<&str> = self.db.bots()?.iter().filter_map(|b| self.provider(b.id).ok()).map(|p| p.kind()).collect();
                kinds.sort();
                kinds.dedup();
                let model = |k: &str| self.db.setting(&format!("routine_model:{k}")).ok().flatten().unwrap_or_default();
                ok(kinds.into_iter().map(|k| json!({"kind": k, "model": model(k)})).collect::<Vec<_>>())
            }
            "tidy_model" => {
                // Kept per kind of account (the memory bot's): a model name only means something to its own kind.
                let kind = crate::tidy::memory_bot(&self.db).and_then(|b| self.provider(b).ok()).map_or("claude", |p| p.kind());
                let key = format!("tidy_model:{kind}");
                if let Some(m) = arg::<Option<String>>(a, "model")? {
                    self.db.set_setting(&key, m.trim())?;
                }
                ok(json!({"model": self.db.setting(&key)?.unwrap_or_default(), "kind": kind}))
            }
            // The daily digest time, "HH:MM" (get without `time`, set with it).
            "digest_time" => {
                if let Some(t) = arg::<Option<String>>(a, "time")? {
                    let ok_time = t.split_once(':').is_some_and(|(h, m)| h.parse::<u32>().is_ok_and(|h| h < 24) && m.parse::<u32>().is_ok_and(|m| m < 60));
                    if !ok_time {
                        bail!("use a time like 08:00");
                    }
                    self.db.set_setting("digest_time", &t)?;
                }
                ok(self.db.setting("digest_time")?.unwrap_or_else(|| "08:00".into()))
            }
            // Quiet hours, "HH:MM-HH:MM" or "" for none (get without `hours`, set with it).
            "quiet_hours" => {
                if let Some(h) = arg::<Option<String>>(a, "hours")? {
                    let valid = |t: &str| t.split_once(':').is_some_and(|(h, m)| h.parse::<u32>().is_ok_and(|h| h < 24) && m.parse::<u32>().is_ok_and(|m| m < 60));
                    if !h.is_empty() && !h.split_once('-').is_some_and(|(a, b)| valid(a) && valid(b) && a != b) {
                        bail!("use a range like 22:00-07:00");
                    }
                    self.db.set_setting("quiet_hours", &h)?;
                }
                ok(self.db.setting("quiet_hours")?.unwrap_or_default())
            }
            // The private chat that also gets every chat's routine asks, failed routines and memory suggestions: get
            // without `chat`, set with it (0 = none). Returns the one in use and the chats that can be it.
            "decisions_chat" => {
                if let Some(chat) = arg::<Option<i64>>(a, "chat")? {
                    if chat == 0 {
                        self.db.set_setting("decisions_chat", "")?;
                    } else if crate::runtime::decisions_ok(&self.db, chat) {
                        self.db.set_setting("decisions_chat", &chat.to_string())?;
                    } else {
                        bail!("pick one of your own private chats, not a group or a guest's chat");
                    }
                }
                let options: Vec<_> = self.db.chats()?.into_iter().filter(|c| crate::runtime::decisions_ok(&self.db, c.chat_id)).collect();
                ok(json!({"chat": crate::runtime::decisions_chat(&self.db), "options": options}))
            }
            // The language Mimir writes in, in chat apps: a code from say::LANGS, or "" for the system's (get without
            // `lang`, set with it).
            "language" => {
                if let Some(l) = arg::<Option<String>>(a, "lang")? {
                    if !l.is_empty() && !crate::say::LANGS.iter().any(|(c, _)| *c == l) {
                        bail!("Mimir doesn't speak {l} yet");
                    }
                    self.db.set_setting("language", &l)?;
                    crate::say::load(&self.db);
                }
                ok(json!({
                    "lang": self.db.setting("language")?.unwrap_or_default(),
                    "system": crate::say::system(),
                    "langs": crate::say::LANGS,
                }))
            }
            // The weekly "what I learned" message in chat apps: on unless switched off (get without `on`, set with it).
            // Idle hours before a messaging chat's next message starts a new session (get without `hours`; 0 = never).
            "fresh_after_hours" => {
                if let Some(h) = arg::<Option<i64>>(a, "hours")? {
                    self.db.set_setting("fresh_after_hours", &h.clamp(0, 24 * 30).to_string())?;
                }
                ok(self.db.setting("fresh_after_hours")?.and_then(|v| v.parse::<i64>().ok()).unwrap_or(crate::FRESH_AFTER_HOURS))
            }
            "weekly_review" => {
                if let Some(on) = arg::<Option<bool>>(a, "on")? {
                    self.db.set_setting("weekly_review", if on { "on" } else { "off" })?;
                }
                ok(self.db.setting("weekly_review")?.as_deref() != Some("off"))
            }
            // Health, beliefs, money and the like in proposed memories: off unless switched on (get without `on`).
            "remember_sensitive" => {
                if let Some(on) = arg::<Option<bool>>(a, "on")? {
                    self.db.set_setting(crate::tidy::SENSITIVE, if on { "on" } else { "off" })?;
                }
                ok(self.db.setting(crate::tidy::SENSITIVE)?.as_deref() == Some("on"))
            }
            "update_routine" => {
                let (chat, id, cron): (i64, i64, String) = (arg(a, "chat_id")?, arg(a, "id")?, arg(a, "cron")?);
                // A new trigger needs what it runs on; pausing one whose mailbox went away still works.
                if self.db.routine(id)?.is_none_or(|r| r.cron != cron.trim()) {
                    crate::events::ready(&self.db, chat, &cron)?;
                }
                // A new end comes with the schedule, so the two are checked together (0 clears it).
                ok(self.db.routine_update(chat, id, &cron, &arg::<String>(a, "prompt")?, arg(a, "quiet")?, arg(a, "paused")?, arg(a, "ends")?)?)
            }
            "delete_routine" => ok(self.db.routine_delete(arg(a, "chat_id")?, arg(a, "id")?)?),
            // With bot_id: that bot's own conversation in the chat (a bot opened from the sidebar); without, everyone's.
            // Paging back: before_id, the 200 before it; from_id, everything from it on (a search hit, and what's below).
            "messages" => {
                let from: Option<i64> = arg(a, "from_id")?;
                // ponytail: from_id loads up to 20k at once; page it if chats ever get that long
                ok(self.db.messages_of(arg(a, "chat_id")?, arg(a, "bot_id")?, arg(a, "before_id")?, from.unwrap_or(0), if from.is_some() { 20_000 } else { 200 })?)
            }
            // The chat's messages matching `query`, best first (for one bot only, with bot_id).
            "chat_search" => {
                let bot: Option<i64> = arg(a, "bot_id")?;
                let hits = self.db.chat_search(arg(a, "chat_id")?, &arg::<String>(a, "query")?, None, (0, i64::MAX), 60)?;
                ok(hits.into_iter().filter(|h| bot.is_none_or(|b| h.bot_id == b)).take(30)
                    .map(|h| json!({"id": h.id, "ts": h.ts, "bot_id": h.bot_id, "role": h.role, "excerpt": h.excerpt})).collect::<Vec<_>>())
            }
            "runs" => ok(self.db.runs(Some(arg(a, "chat_id")?), 200)?),
            // Last 30 days for one chat.
            // What other bots did for a run (tasks it handed over): their runs with each step.
            "run_children" => {
                let runs = self.db.child_runs(arg(a, "run_id")?)?;
                let out: Vec<Value> = runs.into_iter().map(|r| {
                    let steps = self.db.tool_calls(r.id).unwrap_or_default();
                    json!({"run": r, "steps": steps})
                }).collect();
                ok(out)
            }
            "run_totals" => ok(self.db.run_totals(Some(arg(a, "chat_id")?), crate::db::now() - 30 * 86400)?),
            "tool_calls" => ok(self.db.tool_calls(arg(a, "run_id")?)?),
            "approvals" => ok(self.db.approvals_pending()?),
            // Approve once, deny, or approve and keep allowing (`always`: "tool" or "site"). Guarded and
            // saved-login asks can't become rules: those protections must keep asking. Approving can carry your edits
            // to the card's text (`edits`: field → text), denying a `reason` the bot is told.
            "decide" => {
                let always: Option<String> = arg(a, "always").unwrap_or(None);
                let (id, approve): (i64, bool) = (arg(a, "id")?, arg(a, "approve")?);
                let edits: Option<Value> = arg(a, "edits").unwrap_or(None);
                let reason: Option<String> = arg(a, "reason").unwrap_or(None);
                if approve && let Some(edits) = edits.filter(|e| !e.is_null()) {
                    let Some(ap) = self.db.approval(id)?.filter(|ap| ap.status == "pending") else { return ok(false) };
                    tools::edit_card(&self.db, &ap, &edits)?;
                }
                if !approve && let Some(r) = reason.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
                    self.db.approval_set_reason(id, &tools::truncate(r, 500))?;
                }
                // Before approving: the waiting tool may go on (and the run end) the moment it's approved.
                if approve && always.as_deref() == Some("run") && !self.db.approval_allow_rest_of_run(id, None)? {
                    bail!("this one can only be approved once (it isn't a plain ask from a reply that's still going)");
                }
                let Some(ap) = self.db.approval_decide(id, approve, None)? else { return ok(false) };
                tools::after_decision(&self.db, &ap);
                match always.as_deref() {
                    None | Some("run") => {}
                    Some("site") if ap.site.is_none() => bail!("this request isn't about a website"),
                    Some(_) => tools::always_allow(&self.db, &ap)?,
                }
                ok(true)
            }
            // Everything that needs the person: approvals waiting, ones that ran out, failed runs, memory suggestions.
            "inbox" => {
                let now = crate::db::now();
                // The app is about to list every pending ask: each counts as shown (a timeout then says nobody answered).
                self.db.approvals_shown_in_app()?;
                let counts: Vec<Value> = self.db.suggestion_counts()?.into_iter().map(|(chat, n)| json!({"chat_id": chat, "count": n})).collect();
                ok(json!({
                    "approvals": self.db.approvals_pending()?,
                    "missed": self.db.approvals_missed(now - 86400)?,
                    "failed": self.db.runs_failed(now - 7 * 86400)?,
                    // Routines waiting: for a usage limit to reset, or for tomorrow after a daily cap; and people over their
                    // allowance. A bot running on its fallback account isn't waiting.
                    "holds": self.db.holds(now)?.into_iter().filter(|(key, bot, routine, _)| routine.is_some() || key.starts_with("hold:person:") || !self.db.on_fallback(*bot, now).is_ok_and(|f| f.is_some())).map(|(key, bot, routine, h)| {
                        let prompt = routine.and_then(|id| self.db.routine_prompt(id).ok().flatten()).map(|p| crate::tools::truncate(&p, 60));
                        let person = key.strip_prefix("hold:person:").and_then(|p| p.parse().ok()).map(|p| self.db.chat_title(p).unwrap_or_default());
                        json!({"key": key, "bot_id": bot, "routine_id": routine, "routine": prompt, "person": person, "until": h.until, "why": h.why, "cap": h.waive.is_some()})
                    }).collect::<Vec<_>>(),
                    "suggestions": counts,
                    // Goals waiting on the person: over their budget (go on?), or blocked on them.
                    "goals": self.db.goals_waiting()?,
                    // Replies to a bot's email that it handed to you instead of answering.
                    "mail": crate::email::handed_over(&self.db),
                    // The daily check found the database damaged: restore a backup.
                    "damage": self.db.setting(crate::backup::DAMAGE)?,
                    // Plain asks you keep allowing: stop asking?
                    "rule_hints": tools::rule_hints(&self.db)?,
                    "unconfirmed": self.db.unconfirmed_counts()?.into_iter().map(|(chat, n)| json!({"chat_id": chat, "count": n})).collect::<Vec<_>>(),
                    // A newer release than this server runs (looked up by the bots' loops twice a day).
                    "server_update": is_server().then(|| self.db.setting("update_latest").ok().flatten()).flatten()
                        .filter(|v| crate::server_update::newer(v, crate::server_update::CURRENT))
                        .map(|v| json!({"current": crate::server_update::CURRENT, "latest": v, "how": crate::server_update::how()})),
                }))
            }
            // The server's own updates: whether it tells you about a new version (in your chat and Needs you).
            "update_notice" => {
                if let Some(on) = arg::<Option<bool>>(a, "on")? {
                    self.db.set_setting("update_notice", if on { "on" } else { "off" })?;
                }
                ok(json!({
                    "server": is_server(),
                    "on": self.db.setting("update_notice")?.as_deref() != Some("off"),
                    "current": crate::server_update::CURRENT,
                    "latest": self.db.setting("update_latest")?,
                }))
            }
            // The daily anonymous usage count (usage_stats.rs): on unless switched off; `sample` is exactly what's sent.
            "usage_stats" => {
                if let Some(on) = arg::<Option<bool>>(a, "on")? {
                    self.db.set_setting("usage_stats", if on { "on" } else { "off" })?;
                }
                ok(json!({"on": crate::usage_stats::on(&self.db), "sample": crate::usage_stats::batch(&self.db)?["events"]}))
            }
            // Run now instead of waiting for a hold to end (the limit was raised, another plan…); a cap's, for the rest of today.
            "hold_clear" => {
                let key: String = arg(a, "key")?;
                if !key.starts_with("hold:") {
                    bail!("not a hold");
                }
                ok(self.db.hold_clear(&key, true)?)
            }
            // Pause all bots (no bot_id) or one, until a time (or until resumed); on: false resumes. Without `on`, what's paused.
            "pause" => {
                let bot: Option<i64> = arg(a, "bot_id")?;
                match arg::<Option<bool>>(a, "on")? {
                    Some(true) => {
                        let until: i64 = arg::<Option<i64>>(a, "until")?.unwrap_or(0);
                        if until != 0 && until <= crate::db::now() {
                            bail!("that time has already passed");
                        }
                        self.db.pause_set(bot, until)?
                    }
                    Some(false) => self.db.pause_end(bot)?,
                    None => {}
                }
                ok(self.db.pauses(crate::db::now())?.into_iter().map(|(bot, until)| json!({"bot_id": bot, "until": until})).collect::<Vec<_>>())
            }
            // Daily token caps (get without values, set with them; 0 or null = no cap).
            "caps" => {
                for (k, name) in crate::db::CAPS.iter().zip(["routine", "bot", "day", "person"]) {
                    if let Some(v) = a.get(name).filter(|v| !v.is_null()) {
                        let n = v.as_i64().filter(|n| *n >= 0).context("a cap is a number of tokens")?;
                        self.db.set_setting(k, &n.to_string())?;
                    }
                }
                let [routine, bot, day, person] = self.db.caps()?;
                ok(json!({"routine": routine, "bot": bot, "day": day, "person": person}))
            }
            // Where people reach this server's web app (the app tells it on load): links in messages, like to the live
            // computer when a bot needs you there.
            "public_url" => {
                let url: String = arg(a, "url")?;
                let url = url.trim().trim_end_matches('/');
                if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 300 || url.contains(char::is_whitespace) {
                    bail!("not a web address");
                }
                // Opened on the server itself (or through a tunnel) after the real address: the real one stays.
                let local = |u: &str| url::Url::parse(u).ok().and_then(|u| u.host_str().map(str::to_lowercase)).is_some_and(|h| h == "localhost" || h.starts_with("127.") || h == "[::1]");
                if local(url) && self.db.setting("public_url")?.is_some_and(|known| !local(&known)) {
                    return ok(false);
                }
                ok(self.db.set_setting("public_url", url).map(|_| true)?)
            }
            // The AI plan's usage windows, where the AI reports them (Claude Code does).
            "ai_windows" => ok(crate::provider::windows(&self.db)),
            "inbox_dismiss" => {
                let (kind, id): (String, i64) = (arg(a, "kind")?, arg(a, "id")?);
                if kind == "mail" { crate::email::handed_done(&self.db, id)? } else { self.db.inbox_dismiss(&kind, id)? }
                ok(())
            }
            "rule_hint_dismiss" => ok(tools::rule_hint_dismiss(&self.db, &arg::<String>(a, "tool")?, arg::<Option<String>>(a, "site")?.as_deref())?),
            "site_rules" => ok(self.db.site_rules()?.into_iter().map(|(tool, host, action)| json!({"tool": tool, "host": host, "action": action})).collect::<Vec<_>>()),
            "set_site_rule" => ok(self.db.set_site_rule(&arg::<String>(a, "tool")?, &arg::<String>(a, "host")?, &arg::<String>(a, "action")?)?),
            "rules" => ok(tools::all_specs(&self.db)
                .into_iter()
                .map(|(tool, description, _)| {
                    let default = tools::default_for(&self.db, &tool);
                    Ok(Rule { action: self.db.rule(&tool)?.unwrap_or_else(|| default.into()), tool, description, default })
                })
                .collect::<Result<Vec<_>>>()?),
            "set_rule" => {
                let (tool, action): (String, String) = (arg(a, "tool")?, arg(a, "action")?);
                if !["always", "allow", "ask", "deny", "default"].contains(&action.as_str()) || !tools::all_specs(&self.db).iter().any(|(n, ..)| *n == tool) {
                    bail!("invalid rule {tool}={action}");
                }
                // Picking the built-in default stores no override, so future default changes apply.
                let action = if action == tools::default_for(&self.db, &tool) { "default".into() } else { action };
                ok(self.db.set_rule(&tool, &action)?)
            }
            // The built-in chat (no messaging app needed).
            // Incognito: the window keeps the conversation and sends it back each turn; nothing is stored.
            "send_incognito" => {
                let bot: i64 = arg(a, "bot_id")?;
                let provider = self.provider(bot)?;
                let text: String = arg(a, "text")?;
                let history: Vec<(String, String)> = arg::<Option<Vec<(String, String)>>>(a, "history")?.unwrap_or_default();
                if history.is_empty() {
                    self.db.incognito_reset(bot)?;
                }
                let db = self.db.clone();
                let task = tokio::spawn(boxed(move || async move { crate::incognito_turn(&provider, &ToolCtx::new(db, bot, crate::INCOGNITO_CHAT), &history, &text).await }));
                let me = task.id();
                self.running.lock().unwrap().insert((bot, crate::INCOGNITO_CHAT), (task.abort_handle(), String::new()));
                let out = task.await;
                self.finished((bot, crate::INCOGNITO_CHAT), me);
                // Finished, failed or stopped: what it asked to do goes too.
                self.db.incognito_forget_approvals(bot)?;
                match out {
                    Ok(reply) => ok(reply?),
                    Err(e) if e.is_cancelled() => bail!("stopped"),
                    Err(e) => bail!("the reply crashed: {e}"),
                }
            }
            "send_local" => {
                let bot: i64 = arg(a, "bot_id")?;
                // The built-in chat, or a chat API token's own (the server passes it).
                let chat = arg::<Option<i64>>(a, "chat_id")?.unwrap_or(LOCAL_CHAT);
                if chat != LOCAL_CHAT && !is_api_chat(chat) {
                    bail!("send_local answers in the built-in chat or a chat API one");
                }
                let provider = self.provider(bot)?;
                let text: String = arg(a, "text")?;
                // Photos pasted or dropped into the app (saved by save_media); only files inside media/ are accepted.
                let media = self.home.join("media");
                // Resolved first, so "media/../mimir.db" can't pass for a file in media/.
                let media = media.canonicalize().unwrap_or(media);
                let images: Vec<PathBuf> = arg::<Option<Vec<String>>>(a, "images")?
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|p| PathBuf::from(p).canonicalize().ok())
                    .filter(|p| p.starts_with(&media) && p.is_file())
                    .collect();
                // Run as a task so Stop can cancel it (dropping it kills the CLI process too).
                let (db, asked) = (self.db.clone(), text.clone());
                let task = tokio::spawn(boxed(move || async move {
                    let mut ctx = ToolCtx::new(db, bot, chat);
                    ctx.images = images;
                    turn(&provider, &ctx, Trigger::Message(&text)).await
                }));
                let me = task.id();
                self.running.lock().unwrap().insert((bot, chat), (task.abort_handle(), asked));
                let out = task.await;
                self.finished((bot, chat), me);
                match out {
                    Ok(reply) => ok(reply?),
                    Err(e) if e.is_cancelled() => bail!("stopped"),
                    Err(e) => bail!("the reply crashed: {e}"),
                }
            }
            _ => bail!("unknown command {cmd}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mem(Mutex<HashMap<String, String>>);
    impl SecretStore for Mem {
        fn get(&self, n: &str) -> Option<String> {
            self.0.lock().unwrap().get(n).cloned().filter(|s| !s.is_empty())
        }
        fn set(&self, n: &str, v: &str) -> Result<()> {
            self.0.lock().unwrap().insert(n.into(), v.into());
            Ok(())
        }
    }

    /// A data folder of the test's own, removed at the end (Host::new tidies tmp/ and media/ in it).
    struct Home(std::path::PathBuf);
    impl Home {
        fn new(name: &str) -> Home {
            let dir = std::env::temp_dir().join(format!("mimir-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Home(dir)
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Every command the UI sends is one the host or the desktop app handles: a name with no handler is a button
    /// that does nothing.
    #[test]
    fn every_ui_command_has_a_handler() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap();
        // ponytail: reads the source, not a list. The host's names are the quoted ones before `=>` in Host::call;
        // the desktop's, any word between quotes in its `api` (extra words only make this check looser).
        let host = read("crates/mimir-core/src/host.rs");
        let call = &host[host.find("pub async fn call").unwrap()..host.find("#[cfg(test)]").unwrap()];
        let desktop = read("crates/mimir-desktop/src/main.rs");
        let api = &desktop[desktop.find("async fn api(").unwrap()..desktop.find("fn toggle_quick").unwrap()];
        let quoted = |s: &str| s.split('"').filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c == '_')).map(String::from).collect::<Vec<_>>();
        let mut known: std::collections::HashSet<String> = quoted(api).into_iter().collect();
        for line in call.lines().filter(|l| l.trim_start().starts_with('"')) {
            if let Some((pat, _)) = line.split_once("=>") {
                known.extend(quoted(pat));
            }
        }
        let (mut missing, mut seen) = (vec![], 0);
        let files = std::fs::read_dir(root.join("ui/src")).unwrap().chain(std::fs::read_dir(root.join("ui/src/views")).unwrap());
        for path in files.map(|f| f.unwrap().path()).filter(|p| p.is_file()) {
            let src = std::fs::read_to_string(&path).unwrap();
            // invoke('name'…), invoke<T>('name'…), localInvoke('name'…); not api.ts's own tauriInvoke('api', …).
            for (i, _) in src.match_indices("nvoke").filter(|(i, _)| !src[..*i].ends_with("tauriI")) {
                let rest = &src[i + 5..];
                let rest = if rest.starts_with('<') { &rest[rest.find(">(").map_or(0, |j| j + 1)..] } else { rest };
                if let Some(name) = rest.strip_prefix("('").and_then(|r| r.split('\'').next()) {
                    seen += 1;
                    if !known.contains(name) {
                        missing.push(format!("{}: {name}", path.display()));
                    }
                }
            }
        }
        assert!(known.len() > 100 && seen > 100, "found the handlers and the calls");
        assert!(missing.is_empty(), "UI commands nothing handles: {missing:?}");
    }

    #[tokio::test]
    async fn the_skill_library() {
        let home = Home::new("library");
        let host = Host::new(Db::open(&home.0.join("mimir.db")).unwrap(), home.0.clone(), Arc::new(Mem(Default::default())), tokio::runtime::Handle::current(), "server");
        let lib = host.call("skill_library", &json!({"chat_id": 0})).await.unwrap();
        let lib = lib.as_array().unwrap();
        assert_eq!(lib.len(), crate::library::all().len());
        let prep = lib.iter().find(|k| k["slug"] == "meeting-prep").unwrap();
        assert_eq!(prep["missing"]["1"], json!(["calendar_list"]), "no calendar linked: it says so");
        assert!(lib.iter().find(|k| k["slug"] == "summarize-link").unwrap()["missing"]["1"].as_array().unwrap().is_empty());
        // Added for all bots: an accepted skill, shown as added; adding it again changes nothing.
        let id = host.call("skill_add", &json!({"chat_id": 0, "slug": "summarize-link", "bot_id": 0})).await.unwrap();
        assert!(id.as_i64().is_some());
        let skills = host.call("skills", &json!({"chat_id": 0})).await.unwrap();
        assert!(skills.as_array().unwrap().iter().any(|k| k["name"] == "Summarise a link" && k["status"] == "active" && k["kind"] == "library"));
        let lib = host.call("skill_library", &json!({"chat_id": 0})).await.unwrap();
        assert_eq!(lib.as_array().unwrap().iter().find(|k| k["slug"] == "summarize-link").unwrap()["added"], json!([0]));
        assert_eq!(host.call("skill_add", &json!({"chat_id": 0, "slug": "summarize-link", "bot_id": 0})).await.unwrap(), Value::Null);
        assert!(host.call("skill_add", &json!({"chat_id": 0, "slug": "nope", "bot_id": 0})).await.is_err());
    }

    #[tokio::test]
    async fn a_deleted_servers_sensitive_marks_go_with_it() {
        let home = Home::new("sensitive-stale");
        let host = Host::new(Db::open(&home.0.join("mimir.db")).unwrap(), home.0.clone(), Arc::new(Mem(Default::default())), tokio::runtime::Handle::current(), "server");
        host.db.set_setting("mcp_servers", r#"[{"name":"bank","command":"/nonexistent/bank"},{"name":"notes","command":"/nonexistent/notes"}]"#).unwrap();
        host.db.set_setting("mcp_tools:bank", r#"[{"name":"balance","read_only":true}]"#).unwrap();
        for name in ["bank", "bank__balance", "notes"] {
            host.call("set_sensitive", &json!({"name": name, "on": true})).await.unwrap();
        }
        host.call("tool_server_delete", &json!({"name": "bank"})).await.unwrap();
        assert_eq!(tools::sensitive_list(&host.db), ["notes"], "the server and its tools, not the others");
        // One left behind (deleted before this cleanup existed) can still be turned off, but nothing unknown turned on.
        host.db.set_setting("sensitive", r#"["gone","notes"]"#).unwrap();
        host.call("set_sensitive", &json!({"name": "gone", "on": false})).await.unwrap();
        assert_eq!(tools::sensitive_list(&host.db), ["notes"]);
        assert!(host.call("set_sensitive", &json!({"name": "gone", "on": true})).await.is_err());
    }

    #[tokio::test]
    async fn a_sign_in_is_never_typed_into_another_bots_browser() {
        let home = Home::new("signin-bots");
        let host = Host::new(Db::open(&home.0.join("mimir.db")).unwrap(), home.0.clone(), Arc::new(Mem(Default::default())), tokio::runtime::Handle::current(), "server");
        let db = &host.db;
        // Bot 3 waits for its card to be filled in; its hold on the computer runs out, and bot 4 takes the computer
        // (its Chromium on the screen now).
        db.set_setting("computer_lease", &json!({"who": "run:1", "name": "Bot 3", "until": crate::db::now() - 1}).to_string()).unwrap();
        assert!(db.computer_lease("run:2", "Bot 4", 120).unwrap().is_none(), "bot 4 takes the computer");
        std::fs::create_dir_all(home.0.join("computer")).unwrap();
        std::fs::write(home.0.join("computer").join("browser"), format!("4 {}", std::process::id())).unwrap();
        assert_ne!(crate::computer::screen_bot(&home.0), Some(3));
        // The person sends the form: refused before anything reaches a page, the card is used up, nothing is kept.
        let card = db.approval_create(3, 0, "computer_signin", "Sign in to github.com", 300, "signin", Some("github.com")).unwrap();
        let err = host.call("signin_submit", &json!({"id": card, "user": "me@example.com", "password": "hunter2-gh"})).await.unwrap_err();
        assert!(format!("{err:#}").contains("isn't on this bot's browser") && format!("{err:#}").contains("nothing was typed"), "{err:#}");
        assert_eq!(db.approval_status(card).unwrap(), "denied", "the bot hears it failed");
        assert!(ext::vault(db).is_empty(), "and no login is saved");
        // The saved-login path refuses the same way.
        let login = crate::computer::Login { user: "me".into(), password: "hunter2-gh".into(), ..Default::default() };
        assert!(crate::computer::sign_in(&home.0, 3, &login, &["github.com".into()]).await.is_err());
    }

    #[tokio::test]
    async fn a_saved_login_that_stopped_working_gives_way_to_a_card() {
        let home = Home::new("signin-stale");
        let host = Host::new(Db::open(&home.0.join("mimir.db")).unwrap(), home.0.clone(), Arc::new(Mem(Default::default())), tokio::runtime::Handle::current(), "server");
        let db = &host.db;
        let url = "https://github.com/login";
        ext::vault_save_signin(db, 3, "github.com", "me", "old-pass", None).unwrap();
        let e = ext::signin_login(db, 3, url).unwrap();
        assert_eq!(e.name, "GITHUB_COM_BOT3");
        assert!(ext::signin_login(db, 4, url).is_none(), "another bot's isn't used");
        // The site turned it down: the next computer_signin doesn't pick it again, so the person gets a card.
        ext::login_failed_set(db, &e.name).unwrap();
        assert!(ext::signin_login(db, 3, url).is_none());
        // What they sign in with on the card replaces it (and an older one of this bot's for the site under another name).
        let old = ext::VaultEntry { name: "GH_OLD".into(), domains: vec!["github.com".into()], bot: Some(3), totp: false };
        ext::vault_save(db, old, Some("older-pass"), None).unwrap();
        ext::vault_save_signin(db, 3, "github.com", "me", "new-pass", None).unwrap();
        assert_eq!(ext::signin_login(db, 3, url).map(|e| e.name).as_deref(), Some("GITHUB_COM_BOT3"));
        assert_eq!(secret_get("vault:GITHUB_COM_BOT3").as_deref(), Some("new-pass"));
        assert_eq!(ext::vault(db).iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["GITHUB_COM_BOT3"]);
    }

    #[tokio::test]
    async fn a_bot_as_one_file() {
        let home = Home::new("botfile");
        let host = Host::new(Db::open(&home.0.join("mimir.db")).unwrap(), home.0.clone(), Arc::new(Mem(Default::default())), tokio::runtime::Handle::current(), "server");
        // A bot with secrets of every kind: a messaging-app token, a tool server's key and token (an older setup, in
        // the config itself), and a saved login one of its routines mentions.
        let look = r##"{"color":"#336699","shape":"round"}"##;
        let bot = json!({"id":0,"name":"Ops","persona":"You keep the servers up.","provider":"claude","model":"","base_url":"","tools":"web_fetch,email_send,routine_create,mytools__*","look":look});
        let id = host.call("save_bot", &json!({"bot": bot, "secrets": {"slack_bot_token": "xoxb-SLACKSECRET"}})).await.unwrap().as_i64().unwrap();
        let server = crate::ext::ServerConf {
            name: "mytools".into(),
            command: "npx".into(),
            args: vec!["mytools".into(), "--token".into(), "ghp_abcdefghijklmnopqrstuvwxyz0123".into()],
            env: [("API_KEY".to_string(), "LIVEKEYVALUE".to_string())].into(),
            url: String::new(),
            ..Default::default()
        };
        crate::ext::set_servers(&host.db, &[server]).unwrap();
        crate::ext::vault_save(&host.db, crate::ext::VaultEntry { name: "SHOP_PASSWORD".into(), domains: vec!["shop.example".into()], bot: None, totp: false }, Some("hunter2-shop"), None).unwrap();
        host.db.routine_create(id, 0, "0 9 * * 1", "Check the status page", false).unwrap();
        host.db.routine_create(id, 0, "0 9 * * 2", "Log in with hunter2-shop and check orders", false).unwrap();
        host.call("skill_add", &json!({"chat_id": 0, "slug": "summarize-link", "bot_id": id})).await.unwrap();
        host.call("set_rule", &json!({"tool": "email_send", "action": "allow"})).await.unwrap();
        host.call("set_rule", &json!({"tool": "web_fetch", "action": "ask"})).await.unwrap();

        let file = host.call("export_bot", &json!({"id": id})).await.unwrap();
        let text = file.to_string();
        for s in ["SLACKSECRET", "LIVEKEYVALUE", "ghp_abc", "hunter2"] {
            assert!(!text.contains(s), "{s} leaked: {text}");
        }
        assert_eq!(file["servers"][0]["env"]["API_KEY"], "", "the key's name stays, so you know what to fill in");
        assert_eq!((file["skills"].as_array().unwrap().len(), file["routines"].as_array().unwrap().len()), (1, 1));
        assert_eq!(file["rules"], json!({"email_send": "allow", "web_fetch": "ask"}));

        // On a Mimir with the default rules: sending email without asking is looser (never used), asking before
        // reading a page is stricter.
        for t in ["email_send", "web_fetch"] {
            host.call("set_rule", &json!({"tool": t, "action": "default"})).await.unwrap();
        }
        let p = host.call("import_bot_preview", &json!({"text": text})).await.unwrap();
        assert_eq!(p["name"], "Ops (copy)");
        assert_eq!(
            p["rules"],
            json!([{"tool": "email_send", "action": "allow", "yours": "ask", "looser": true}, {"tool": "web_fetch", "action": "ask", "yours": "allow", "looser": false}])
        );
        assert_eq!(p["servers"][0]["here"], true);
        assert_eq!(host.db.bots().unwrap().len(), 2, "a preview adds nothing");

        let done = host.call("import_bot", &json!({"text": text})).await.unwrap();
        let copy = host.db.bot(done["id"].as_i64().unwrap()).unwrap();
        assert_eq!((copy.name.as_str(), copy.persona.as_str(), copy.tools.as_str(), copy.look.as_str()), ("Ops (copy)", "You keep the servers up.", "web_fetch,email_send,routine_create,mytools__*", look));
        let r: Vec<_> = host.db.routines(0).unwrap().into_iter().filter(|r| r.bot_id == copy.id).collect();
        assert!(r.len() == 1 && r[0].paused && r[0].tainted && r[0].cron == "0 9 * * 1", "{r:?}");
        assert!(host.db.skills("chat:0").unwrap().iter().any(|k| k.bot_id == copy.id && k.status == "proposed" && k.name == "Summarise a link"));
        assert_eq!(host.db.rule("email_send").unwrap(), None, "its rules only when you say so");
        host.call("import_bot", &json!({"text": text, "rules": true})).await.unwrap();
        assert_eq!(host.db.rule("web_fetch").unwrap().as_deref(), Some("ask"), "a stricter rule is used");
        assert_eq!(host.db.rule("email_send").unwrap(), None, "a looser one never is");
        assert!(host.db.bots().unwrap().iter().any(|b| b.name == "Ops (copy 2)"));
        assert!(host.call("import_bot_preview", &json!({"text": "{\"name\": \"x\"}"})).await.is_err(), "not a bot file");
    }

    #[tokio::test]
    async fn dispatch() {
        let home = Home::new("host");
        let mem = Arc::new(Mem(Default::default()));
        let host = Host::new(Db::open(&home.0.join("mimir.db")).unwrap(), home.0.clone(), mem.clone(), tokio::runtime::Handle::current(), "server");

        assert_eq!(host.call("info", &json!({})).await.unwrap()["mode"], "server");
        let bot = json!({"id":0,"name":"Ops","persona":"","provider":"claude","model":"","base_url":"","tools":""});
        // A misspelled field inside an argument is an error, not dropped; `bots`' channel flags sent back are fine.
        assert!(host.call("save_bot", &json!({"bot": bot, "secrets": {"slackBotToken": "x"}})).await.is_err());
        let mut typo = bot.clone();
        typo["baseUrl"] = json!("");
        assert!(host.call("save_bot", &json!({"bot": typo})).await.is_err());
        // Half a Slack pair: the token is stored, but no channel starts (a test opens no connections).
        let id = host.call("save_bot", &json!({"bot": bot, "secrets": {"slack_bot_token": " x "}})).await.unwrap();
        assert_eq!(mem.get(&key(id.as_i64().unwrap(), SLACK_BOT)).as_deref(), Some("x"));
        let bots = host.call("bots", &json!({})).await.unwrap();
        assert_eq!(bots[1]["name"], "Ops");
        assert_eq!((bots[1]["has_slack"].as_bool(), bots[0]["has_slack"].as_bool()), (Some(false), Some(false)));
        assert_eq!(host.call("save_bot", &json!({"bot": bots[1], "secrets": {}})).await.unwrap(), id);
        // Saving starts the bot; asking for statuses starts any not running yet (one a bot made in its tool process).
        assert_eq!(host.call("statuses", &json!({})).await.unwrap().as_array().unwrap().len(), 2);

        let mem = host.call("remember", &json!({"chat_id":0,"kind":"knowledge","title":"t","body":"b","supersedes":null})).await.unwrap();
        assert_eq!(host.call("entries", &json!({"chat_id":0})).await.unwrap()[0]["id"], mem);
        assert!(host.call("entries", &json!({})).await.unwrap_err().to_string().contains("chat_id"));
        assert!(host.call("set_rule", &json!({"tool":"rm -rf","action":"allow"})).await.is_err());
        assert!(host.call("nope", &json!({})).await.is_err());

        // A chat API token reaches its bot and only that; the list never shows it; it goes with its bot.
        let bot = id.as_i64().unwrap();
        assert!(host.call("chat_token_add", &json!({"bot_id": bot, "name": " "})).await.is_err());
        assert!(host.call("chat_token_add", &json!({"bot_id": 999, "name": "Shortcuts"})).await.is_err());
        let token = host.call("chat_token_add", &json!({"bot_id": bot, "name": "Shortcuts"})).await.unwrap();
        let token = token.as_str().unwrap();
        let list = host.call("chat_tokens", &json!({})).await.unwrap();
        // Its own chat, not the built-in one, and listed with the chats.
        let chat = API_CHATS + i64::from_str_radix(list[0]["id"].as_str().unwrap(), 16).unwrap();
        assert_eq!(chat_token_bot(&host.db, token), Some((bot, chat)));
        assert!(is_api_chat(chat) && !host.db.chat_private(chat));
        assert!(host.db.chats().unwrap().iter().any(|c| c.chat_id == chat && c.channel == "api" && c.title == "Chat API: Shortcuts"));
        assert_eq!(chat_token_bot(&host.db, &token[..token.len() - 1]), None);
        assert_eq!((list[0]["name"].as_str(), list[0]["bot_id"].as_i64(), list[0].get("hash")), (Some("Shortcuts"), Some(bot), None));
        assert!(!list.to_string().contains(token));
        let other = host.call("chat_token_add", &json!({"bot_id": 1, "name": "HA"})).await.unwrap();
        host.call("chat_token_delete", &json!({"id": list[0]["id"]})).await.unwrap();
        assert_eq!(chat_token_bot(&host.db, token), None);
        host.call("chat_token_add", &json!({"bot_id": bot, "name": "Again"})).await.unwrap();

        host.call("delete_bot", &json!({"id": id})).await.unwrap();
        assert_eq!(host.call("bots", &json!({})).await.unwrap().as_array().unwrap().len(), 1);
        let list = host.call("chat_tokens", &json!({})).await.unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1, "the deleted bot's token went with it");
        assert_eq!(chat_token_bot(&host.db, other.as_str().unwrap()).map(|t| t.0), Some(1));
        host.stop_all();
    }
}
