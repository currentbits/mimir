//! Model providers. `Claude` / `Codex` / `Grok` drive the user's own logged-in CLI
//! (`claude -p`, `codex exec` = ChatGPT subscription, `grok -p` = SuperGrok / X Premium+); we never touch their
//! credentials. `OpenAi` speaks /chat/completions: the `openai` account (OpenAI or any
//! compatible endpoint), `openrouter`, and `local` (Ollama / LM Studio on this machine). `ChatGpt` is your ChatGPT
//! plan signed in from Mimir ("Sign in with ChatGPT"), on the Responses API: see chatgpt.rs.

use crate::db::Usage;
use crate::tools::{self, ToolCtx};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

pub enum Provider {
    Claude { model: Option<String>, home: PathBuf },
    Codex { model: Option<String>, home: PathBuf },
    Grok { model: Option<String>, home: PathBuf },
    OpenAi { base: String, key: String, model: String },
    ChatGpt { model: Option<String> },
}

#[derive(Debug)]
pub struct Reply {
    pub text: String,
    /// CLI session/thread id to resume next turn.
    pub session: Option<String>,
    pub usage: Usage,
}

// Long enough to wait out an approval (tools::APPROVAL_TTL_SECS) mid-turn.
const TURN_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const MAX_TOOL_ROUNDS: usize = 8;
pub const OLLAMA: &str = "http://localhost:11434/v1";
const LM_STUDIO: &str = "http://localhost:1234/v1";

/// Accounts that speak /chat/completions: (default endpoint, default model).
fn api_defaults(kind: &str) -> Option<(&'static str, Option<&'static str>)> {
    match kind {
        "openai" => Some(("https://api.openai.com/v1", Some("gpt-5-mini"))),
        // openrouter/auto picks a model per request; any OpenRouter model id works.
        "openrouter" => Some(("https://openrouter.ai/api/v1", Some("openrouter/auto"))),
        // Grok, on xAI's OpenAI-compatible API.
        "xai" => Some(("https://api.x.ai/v1", Some("grok-4.7"))),
        // No default model: it's whichever one you downloaded.
        "local" => Some((OLLAMA, None)),
        _ => None,
    }
}

impl Provider {
    /// The same account with another model (e.g. a cheaper one for background work). None or blank keeps the model.
    pub fn with_model(&self, model: Option<&str>) -> Provider {
        let m = model.map(str::trim).filter(|m| !m.is_empty()).map(str::to_string);
        match self {
            Provider::Claude { model, home } => Provider::Claude { model: m.or_else(|| model.clone()), home: home.clone() },
            Provider::Codex { model, home } => Provider::Codex { model: m.or_else(|| model.clone()), home: home.clone() },
            Provider::Grok { model, home } => Provider::Grok { model: m.or_else(|| model.clone()), home: home.clone() },
            Provider::OpenAi { base, key, model } => Provider::OpenAi { base: base.clone(), key: key.clone(), model: m.unwrap_or_else(|| model.clone()) },
            Provider::ChatGpt { model } => Provider::ChatGpt { model: m.or_else(|| model.clone()) },
        }
    }

    /// MIMIR_PROVIDER=claude (default) | codex | openrouter | local | openai. See README for the env vars.
    pub fn from_env(home: PathBuf) -> Result<Self> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let kind = var("MIMIR_PROVIDER").unwrap_or_else(|| "claude".into());
        Self::new(&kind, var("MIMIR_MODEL"), var("OPENAI_BASE_URL"), crate::host::api_key(&kind), home)
    }

    pub fn new(kind: &str, model: Option<String>, base: Option<String>, key: Option<String>, home: PathBuf) -> Result<Self> {
        let model = model.filter(|m| !m.trim().is_empty());
        match kind {
            "claude" => Ok(Provider::Claude { model, home }),
            "codex" => Ok(Provider::Codex { model, home }),
            "grok" => Ok(Provider::Grok { model, home }),
            "chatgpt" if crate::chatgpt::account().is_none() => bail!("sign in with ChatGPT first (Settings → AI keys)"),
            "chatgpt" => Ok(Provider::ChatGpt { model }),
            _ => {
                let Some((default_base, default_model)) = api_defaults(kind) else {
                    bail!("unknown provider '{kind}' (use claude, chatgpt, codex, grok, openrouter, xai, local or openai)")
                };
                match kind {
                    "openrouter" if key.is_none() => bail!("add your OpenRouter key in Settings first"),
                    "xai" if key.is_none() => bail!("add your xAI key in Settings first"),
                    _ => {}
                }
                Ok(Provider::OpenAi {
                    base: base.filter(|b| !b.trim().is_empty()).unwrap_or_else(|| default_base.into()),
                    key: key.unwrap_or_default(),
                    model: model.or(default_model.map(String::from)).context("pick a model for this bot")?,
                })
            }
        }
    }

    /// Provider for a configured bot. API keys are per account kind, shared across bots (one per user).
    pub fn for_bot(bot: &crate::db::Bot, home: PathBuf) -> Result<Self> {
        let opt = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
        Self::new(&bot.provider, opt(&bot.model), opt(&bot.base_url), crate::host::api_key(&bot.provider), home)
    }

    pub fn name(&self) -> String {
        match self {
            Provider::Claude { model, .. } => format!("claude -p ({})", model.as_deref().unwrap_or("default model")),
            Provider::Codex { model, .. } => format!("codex exec ({})", model.as_deref().unwrap_or("default model")),
            Provider::Grok { model, .. } => format!("grok -p ({})", model.as_deref().unwrap_or("default model")),
            Provider::OpenAi { base, model, .. } => format!("{model} @ {base}"),
            Provider::ChatGpt { model } => format!("ChatGPT plan ({})", model.as_deref().unwrap_or("default model")),
        }
    }

    /// Which kind of account: claude, codex, grok or api (settings that name a model are per kind).
    pub fn kind(&self) -> &'static str {
        match self {
            Provider::Claude { .. } => "claude",
            Provider::Codex { .. } => "codex",
            Provider::Grok { .. } => "grok",
            Provider::OpenAi { .. } => "api",
            Provider::ChatGpt { .. } => "chatgpt",
        }
    }

    /// The AI CLI's own copy of a conversation (its transcript, kept in its folder), deleted when the conversation
    /// starts fresh, its history is deleted or the bot goes. Only files and folders named with that session's id.
    pub fn forget_session(&self, session: &str) {
        let home = match self {
            Provider::Claude { home, .. } | Provider::Codex { home, .. } | Provider::Grok { home, .. } => home.clone(),
            Provider::OpenAi { .. } | Provider::ChatGpt { .. } => return,
        };
        forget_session(&home, self.kind(), session);
    }

    pub fn uses_history(&self) -> bool {
        matches!(self, Provider::OpenAi { .. } | Provider::ChatGpt { .. })
    }

    /// One reply. A message the person sends while it's being written steers it: `claude -p` takes it in mid-run
    /// (see claude); the others can't, so the run stops and starts over from the same session with it added.
    pub async fn run(&self, ctx: &ToolCtx, system: &str, history: &[(String, String)], user: &str, session: Option<String>) -> Result<Reply> {
        let (b, c) = (ctx.bot_id, ctx.chat_id);
        let bell = if ctx.stream && !matches!(self, Provider::Claude { .. }) { crate::live::bell(b, c) } else { None };
        let Some(bell) = bell else { return self.run_once(ctx, system, history, user, session).await };
        let mut user = user.to_string();
        loop {
            let more = {
                let run = self.run_once(ctx, system, history, &user, session.clone());
                tokio::pin!(run);
                loop {
                    tokio::select! {
                        r = &mut run => match crate::live::last_call(b, c) {
                            more if more.is_empty() => return r,
                            more => break more,
                        },
                        _ = bell.notified() => match crate::live::steers(b, c) {
                            more if more.is_empty() => {}
                            more => break more,
                        },
                    }
                }
            };
            crate::live::turned(b, c);
            user = steered(&user, &more);
        }
    }

    async fn run_once(&self, ctx: &ToolCtx, system: &str, history: &[(String, String)], user: &str, session: Option<String>) -> Result<Reply> {
        match self {
            Provider::Claude { model, home } | Provider::Codex { model, home } | Provider::Grok { model, home } => {
                let go = |sid: Option<&str>| {
                    let (m, sid) = (model.as_deref(), sid.map(String::from));
                    async move {
                        match self {
                            Provider::Claude { .. } => claude(ctx, home, m, system, user, sid.as_deref()).await,
                            Provider::Codex { .. } => codex(ctx, home, m, system, user, sid.as_deref()).await,
                            _ => grok(ctx, home, m, system, user, sid.as_deref()).await,
                        }
                    }
                };
                // A session that can't be reopened is the caller's to retry: it has the chat to start the new one with.
                go(session.as_deref()).await
            }
            // One deadline for the whole reply, however many tool rounds it takes.
            Provider::OpenAi { base, key, model } => tokio::time::timeout(TURN_TIMEOUT, openai(ctx, base, key, model, system, history, user)).await.context("the AI service took too long")?,
            Provider::ChatGpt { model } => {
                tokio::time::timeout(TURN_TIMEOUT, crate::chatgpt::respond(ctx, model.as_deref(), system, history, user)).await.context("ChatGPT took too long")?
            }
        }
    }
}

/// The message a run starts over with after the person sent more while it was being written.
fn steered(user: &str, more: &[String]) -> String {
    format!(
        "{user}\n\n<sent_while_you_were_answering>\nThe person sent this while you were still working on the message above, so you're \
         starting over. It may change or replace what they asked: answer with it in mind.\n{}\n</sent_while_you_were_answering>",
        more.join("\n\n")
    )
}

/// What `claude -p` is sent for a message that arrives mid-run, after interrupting the answer in progress (the
/// session keeps what was written so far).
fn steer_line(text: &str) -> String {
    let text = format!("[Sent while you were still answering. It may change or replace what I asked: answer with it in mind.]\n{text}");
    json!({"type": "user", "message": {"role": "user", "content": text}}).to_string() + "\n"
}

async fn claude(ctx: &ToolCtx, home: &PathBuf, model: Option<&str>, system: &str, user: &str, session: Option<&str>) -> Result<Reply> {
    let exe = std::env::current_exe()?;
    // Tools off: no tools server at all (with --strict-mcp-config nothing else loads), so the run doesn't pay its startup.
    let mcp = if ctx.tools_off {
        json!({"mcpServers": {}})
    } else {
        json!({"mcpServers": {"mimir": {
            "command": exe,
            "args": ctx.mcp_args(),
            "env": {"MIMIR_HOME": home}
        }}})
    };
    let workdir = home.join("workspace");
    std::fs::create_dir_all(&workdir)?;

    let system_file = Private::write(home, system)?;
    let mut cmd = tokio::process::Command::new(std::env::var("MIMIR_CLAUDE_BIN").unwrap_or_else(|_| "claude".into()));
    cmd.current_dir(&workdir)
        // stream-json + partial messages: text arrives as it's written; the final `result` line has the same shape as --output-format json.
        .args(["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages"])
        // Messages in as stream-json too, so one sent mid-run can steer it (stdin stays open until the answer is in).
        .args(["--input-format", "stream-json"])
        .args(["--restricted", "--strict-mcp-config", "--permission-mode", "dontAsk"])
        .arg("--mcp-config")
        .arg(mcp.to_string())
        // From a file, not the command line: the instructions hold your memories, and any account on the computer can
        // read a process's command line.
        .arg("--append-system-prompt-file")
        .arg(&system_file.0)
        // Only WebSearch from the built-in set. Page fetches go through our web_fetch so they
        // taint the turn; side effects go through our policy-gated MCP tools.
        // (A WebSearch marks the run as having read outside content: see searched_web.)
        .args(["--tools", if ctx.images.is_empty() { "WebSearch" } else { "WebSearch,Read" }])
        // With a photo attached, Read is allowed for Mimir's media folder only (nothing else on disk).
        .args(["--allowedTools", &if ctx.images.is_empty() { "mcp__mimir,WebSearch".to_string() } else { format!("mcp__mimir,WebSearch,Read(/{}/**)", home.join("media").display()) }])
        // Our tools can legitimately take minutes (waiting on an approval, a handed-over task).
        .env("MCP_TOOL_TIMEOUT", (TURN_TIMEOUT.as_millis() as u64).to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(m) = model {
        cmd.args(["--model", m]);
    }
    if !ctx.images.is_empty() {
        // The media folder sits outside the CLI's working directory; make it reachable (Read-only, per the rule above).
        cmd.arg("--add-dir").arg(home.join("media"));
    }
    if let Some(s) = session {
        cmd.args(["--resume", s]);
    }
    if ctx.incognito() {
        // Claude Code otherwise keeps every session's transcript on disk.
        cmd.arg("--no-session-persistence");
    }

    let mut child = cmd.spawn().context("failed to start `claude` — is Claude Code installed and logged in?")?;
    // Prompt via stdin so it can't be mistaken for a flag value.
    let mut stdin = child.stdin.take();
    let user = with_image_note(user, &ctx.images);
    stdin.as_mut().context("no stdin")?.write_all(json!({"type": "user", "message": {"role": "user", "content": user}}).to_string().as_bytes()).await?;
    stdin.as_mut().context("no stdin")?.write_all(b"\n").await?;

    let mut lines = tokio::io::BufReader::new(child.stdout.take().context("no stdout")?).lines();
    let mut stderr = child.stderr.take().context("no stderr")?;
    let (bot, chat, stream) = (ctx.bot_id, ctx.chat_id, ctx.stream);
    // Only a reply to a person can be steered; anything else ends at its first answer.
    let bell = if stream { crate::live::bell(bot, chat) } else { None };
    if bell.is_none() {
        stdin = None;
    }
    // ponytail: one that never rings stands in when there's none (that branch is off then anyway).
    let bell = bell.unwrap_or_default();
    let read = async {
        let mut result = None;
        // Each message sent gets a `result` (an interrupted one too, as an error): tokens add up, cost is the process's so far.
        let mut used = Usage::default();
        // Messages sent that have no `result` yet: the answer is in when that's none.
        let mut asked = 1;
        loop {
            let line = tokio::select! {
                line = lines.next_line() => line,
                // Interrupt the answer in progress, then the new message: it answers that, with the old one in its session.
                _ = bell.notified(), if stdin.is_some() => {
                    let more = crate::live::steers(bot, chat);
                    if let (Some(w), false) = (stdin.as_mut(), more.is_empty()) {
                        w.write_all((json!({"type": "control_request", "request_id": crate::random_hex(4)?, "request": {"subtype": "interrupt"}}).to_string() + "\n").as_bytes()).await?;
                        for m in &more {
                            w.write_all(steer_line(m).as_bytes()).await?;
                        }
                        asked += more.len();
                        crate::live::turned(bot, chat);
                    }
                    continue;
                }
            };
            let Some(line) = line? else { break };
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            match v["type"].as_str() {
                Some("result") => {
                    let u = claude_usage(&v);
                    (used.tokens_in, used.tokens_out, used.cache_read) = (used.tokens_in + u.tokens_in, used.tokens_out + u.tokens_out, used.cache_read + u.cache_read);
                    (used.model, used.cost_usd) = (u.model.or(used.model), u.cost_usd);
                    result = Some((v, used.clone()));
                    asked = asked.saturating_sub(1);
                    // The answer is in. One that arrived just now goes in as a message of its own (it's idle: nothing
                    // to interrupt); with none, stdin closes and the CLI exits.
                    if stdin.is_some() && asked == 0 {
                        match crate::live::last_call(bot, chat) {
                            more if more.is_empty() => stdin = None,
                            more => {
                                for m in &more {
                                    stdin.as_mut().context("no stdin")?.write_all(steer_line(m).as_bytes()).await?;
                                }
                                asked += more.len();
                                crate::live::turned(bot, chat);
                            }
                        }
                    }
                }
                // The plan's usage windows (5-hour, weekly): shown with routines, and when a limit resets.
                Some("rate_limit_event") => {
                    let _ = ctx.db.set_setting(WINDOWS, &json!({"at": crate::db::now(), "info": v["rate_limit_info"]}).to_string());
                }
                Some("stream_event") if stream => on_claude_event(bot, chat, &v["event"]),
                // Search results are pages anyone can write: the run has read outside content.
                Some("assistant") if searched_web(&v) => ctx.taint(),
                _ => {}
            }
        }
        anyhow::Ok(result)
    };
    let result = tokio::time::timeout(TURN_TIMEOUT, read).await.context("claude timed out")??;
    drop(stdin);
    let status = child.wait().await?;
    let Some((v, usage)) = result else {
        let mut err = String::new();
        let _ = tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut err).await;
        bail!("claude exited {status} without a result: {}", tools::truncate(&err, 500));
    };
    if v["is_error"].as_bool().unwrap_or(false) || !status.success() {
        bail!("claude error: {}", v["result"].as_str().unwrap_or("unknown error"));
    }
    Ok(Reply { text: v["result"].as_str().unwrap_or("").to_string(), session: v["session_id"].as_str().map(String::from), usage })
}

/// The HOME `grok` runs under: Mimir's own, so the person's grok/Claude/Cursor plugins, skills, hooks and MCP
/// servers never load into a bot. Signing in to Grok for Mimir puts its login here (not in ~/.grok).
pub fn grok_home(home: &std::path::Path) -> PathBuf {
    home.join("grok")
}

fn grok_bin() -> String {
    std::env::var("MIMIR_GROK_BIN").unwrap_or_else(|_| "grok".into())
}

/// `grok` with Mimir's isolated HOME, no self-updates.
fn grok_cmd(home: &std::path::Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(grok_bin());
    // USERPROFILE too: on Windows that's where a program finds its home folder.
    cmd.env("HOME", grok_home(home)).env("USERPROFILE", grok_home(home)).env("GROK_DISABLE_AUTOUPDATER", "1").stdin(Stdio::null()).kill_on_drop(true);
    cmd
}

/// The grok config: our MCP server only. It's shared by every run, so the per-run arguments travel in
/// MIMIR_MCP_ARGS (`mimir mcp --from-env` reads them); the server gets the person's real HOME back.
fn grok_config(home: &std::path::Path) -> Result<()> {
    let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let real_home = crate::user_home().map(|h| format!("HOME = {0}\nUSERPROFILE = {0}\n", q(&h.to_string_lossy()))).unwrap_or_default();
    let toml = format!(
        "[mcp_servers.mimir]\ncommand = {}\nargs = [\"mcp\", \"--from-env\"]\n\n[mcp_servers.mimir.env]\nMIMIR_HOME = {}\n{real_home}",
        q(&std::env::current_exe()?.to_string_lossy()),
        q(&home.to_string_lossy()),
    );
    let dir = grok_home(home).join(".grok");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("config.toml");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(toml.as_str()) {
        // Unique per write: two runs starting at once mustn't share (and half-publish) one temp file.
        let tmp = dir.join(format!("config.toml.{}", crate::random_hex(6)?));
        std::fs::write(&tmp, toml)?;
        std::fs::rename(tmp, path)?;
    }
    Ok(())
}

/// `grok -p` (Grok Build) on the user's SuperGrok / X Premium+ login. Same stream format as `claude -p`.
/// Locked down like Claude: only web search plus our MCP tools (grok reaches MCP through search_tool/use_tool);
/// no shell, files, subagents, plan mode or grok's own scheduler.
async fn grok(ctx: &ToolCtx, home: &std::path::Path, model: Option<&str>, system: &str, user: &str, session: Option<&str>) -> Result<Reply> {
    grok_config(home)?;
    // Photos go inline (grok has no file access here), shrunk to fit the command line: Linux caps one argument at
    // 128 KiB, Windows the whole line at ~32K characters (the system prompt and the message count too).
    let room = if cfg!(windows) { 30_000usize.saturating_sub(system.len() + user.len()) } else { 120_000usize.saturating_sub(user.len()) };
    let (mut prompt, mut note) = (vec![], String::new());
    if !ctx.images.is_empty() {
        if room < 4_000 {
            note = format!("\n\n[The person attached {} image(s), too large to pass to this AI account here.]", ctx.images.len());
        } else {
            for (mime, data) in inline_images(&ctx.images, room - 2_000)? {
                prompt.push(json!({"type": "image", "mimeType": mime, "data": data}));
            }
        }
    }
    prompt.insert(0, json!({"type": "text", "text": format!("{user}{note}")}));
    let out = grok_turn(ctx, home, model, system, &prompt, session).await;
    // Incognito leaves nothing behind, whether the turn worked, failed or timed out.
    if ctx.incognito() {
        forget_incognito_sessions(home);
    }
    match out? {
        Ok(r) => Ok(r),
        // grok ends the whole turn when the model reaches for a tool that isn't here (Claude would hand back a
        // denial instead). Resume once, telling it so, rather than failing the message.
        Err(sid) => {
            let note = [json!({"type": "text", "text": "[Mimir: that tool isn't available here. Use only web search and Mimir's tools, and answer my previous message.]"})];
            // The retry starts a fresh tools server: if the first try read outside content, say so (the server also
            // checks the run in the database).
            if ctx.run_id.is_some_and(|r| ctx.db.run_tainted(r).unwrap_or(true)) {
                ctx.pre_taint();
            }
            match grok_turn(ctx, home, model, system, &note, Some(&sid)).await? {
                Ok(r) => Ok(r),
                Err(_) => bail!("grok kept reaching for a tool Mimir doesn't offer"),
            }
        }
    }
}

/// One `grok` run. Err(session) when grok stopped because a tool was refused, so the caller can resume it.
async fn grok_turn(ctx: &ToolCtx, home: &std::path::Path, model: Option<&str>, system: &str, prompt: &[Value], session: Option<&str>) -> Result<Result<Reply, String>> {
    // Incognito gets its own folder, so its transcript can be found and deleted afterwards. Otherwise a folder per bot
    // and chat, so the instructions in it are only ever that conversation's (grok resumes a session from any folder).
    let workdir = if ctx.incognito() { home.join("workspace-incognito") } else { home.join("workspace").join(format!("{}-{}", ctx.bot_id, ctx.chat_id)) };
    std::fs::create_dir_all(&workdir)?;
    let mut cmd = grok_cmd(home);
    cmd.current_dir(&workdir);
    // The instructions (with standing memories) from a private rules file in that folder, not the command line, which
    // any account on the computer can read. Windows keeps --rules: there other accounts can't read a command line,
    // and grok's form for a trusted Windows folder is untested.
    // Runs in one folder take turns (a hand-off from another bot runs in the tools server, a separate process; the
    // tidy-up and the connection test use the same folders with other instructions): a lock file, held to the end.
    let _turn = if cfg!(unix) {
        std::fs::create_dir_all(workdir.join(".grok"))?;
        let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(workdir.join(".grok").join("mimir.lock"))?;
        Some(tokio::task::spawn_blocking(move || f.lock().map(|_| f)).await??)
    } else {
        None
    };
    let _rules = if cfg!(unix) {
        grok_trust(home, &workdir)?;
        Some(Private::at(&grok_rules(&workdir), system)?)
    } else {
        cmd.arg(format!("--rules={system}"));
        None
    };
    // The message from a private file, not the command line.
    let message = Private::write(home, &match prompt {
        [only] if only["type"] == "text" => only["text"].as_str().unwrap_or("").to_string(),
        _ => String::new(),
    })?;
    match prompt {
        [only] if only["type"] == "text" => cmd.arg("--prompt-file").arg(&message.0),
        // `--flag=value` so a message starting with "-" can't be read as a flag.
        _ => cmd.arg(format!("--prompt-json={}", Value::from(prompt.to_vec()))),
    };
    cmd.args(["--output-format", "streaming-messages-json", "--include-partial-messages"])
        .args(["--permission-mode", "dontAsk", "--allow", "mcp__mimir", "--tools", "web_search,search_tool,use_tool", "--no-subagents", "--no-plan"])
        .env("MIMIR_MCP_ARGS", serde_json::to_string(&ctx.mcp_args())?)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(m) = model {
        cmd.args(["--model", m]);
    }
    if let Some(s) = session {
        cmd.args(["--resume", s]);
    }
    let mut child = cmd.spawn().context("failed to start `grok` — is Grok Build installed?")?;
    let mut lines = tokio::io::BufReader::new(child.stdout.take().context("no stdout")?).lines();
    // Read stderr alongside stdout: a chatty grok must never block on a full pipe.
    let mut stderr = child.stderr.take().context("no stderr")?;
    let errs = tokio::spawn(async move {
        let mut s = String::new();
        let _ = tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut s).await;
        s
    });
    let (bot, chat, stream) = (ctx.bot_id, ctx.chat_id, ctx.stream);
    let read = async {
        let mut result = None;
        while let Some(line) = lines.next_line().await? {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            match v["type"].as_str() {
                Some("result") => result = Some(v),
                // The plan's usage windows (5-hour, weekly): shown with routines, and when a limit resets.
                Some("rate_limit_event") => {
                    let _ = ctx.db.set_setting(WINDOWS, &json!({"at": crate::db::now(), "info": v["rate_limit_info"]}).to_string());
                }
                Some("stream_event") if stream => on_claude_event(bot, chat, &v["event"]),
                // Search results are pages anyone can write: the run has read outside content.
                Some("assistant") if searched_web(&v) => ctx.taint(),
                _ => {}
            }
        }
        anyhow::Ok(result)
    };
    let result = tokio::time::timeout(TURN_TIMEOUT, read).await.context("grok timed out")??;
    let status = child.wait().await?;
    let sid = result.as_ref().and_then(|v| v["session_id"].as_str()).map(String::from);
    let Some(v) = result else {
        let err = errs.await.unwrap_or_default();
        if err.to_lowercase().contains("not authenticated") || err.to_lowercase().contains("not logged in") {
            bail!("grok isn't signed in for Mimir yet: sign in to Grok in Settings");
        }
        bail!("grok exited {status} without a result: {}", tools::truncate(&err, 500));
    };
    if v["is_error"].as_bool().unwrap_or(false) {
        let errs: Vec<&str> = v["errors"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        if errs == ["cancelled"]
            && let Some(sid) = sid
            && !ctx.incognito()
        {
            return Ok(Err(sid));
        }
        let detail = if errs.is_empty() { v["subtype"].as_str().unwrap_or("unknown error").to_string() } else { errs.join("; ") };
        // grok's own advice (grok login …) would sign in the wrong grok home; Mimir keeps its own login.
        if detail.to_lowercase().contains("not signed in") || detail.to_lowercase().contains("not authenticated") {
            bail!("grok isn't signed in for Mimir yet: sign in to Grok in Settings → AI keys");
        }
        bail!("grok error: {detail}");
    }
    Ok(Ok(Reply { text: v["result"].as_str().unwrap_or("").to_string(), session: sid, usage: claude_usage(&v) }))
}

/// Photos for a model that takes them inline, as (mime, base64): the original when it fits, else a smaller JPEG,
/// shrinking until every one fits its share of `budget` base64 characters.
pub(crate) fn inline_images(images: &[PathBuf], budget: usize) -> Result<Vec<(String, String)>> {
    use base64::Engine;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let each = budget / images.len().max(1);
    images
        .iter()
        .map(|p| {
            let bytes = std::fs::read(p)?;
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            let mime = match ext.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                "gif" => "image/gif",
                _ => "",
            };
            if !mime.is_empty() && bytes.len().div_ceil(3) * 4 <= each {
                return Ok((mime.to_string(), b64(&bytes)));
            }
            let img = image::load_from_memory(&bytes).context("couldn't read that image")?;
            for side in [1568u32, 1024, 768, 512, 384, 256, 160] {
                let fit = if img.width().max(img.height()) > side { img.resize(side, side, image::imageops::FilterType::Triangle) } else { img.clone() };
                let mut out = std::io::Cursor::new(vec![]);
                image::DynamicImage::ImageRgb8(fit.to_rgb8()).write_to(&mut out, image::ImageFormat::Jpeg)?;
                let enc = b64(out.get_ref());
                if enc.len() <= each {
                    return Ok(("image/jpeg".to_string(), enc));
                }
            }
            bail!("that image is too large to send")
        })
        .collect()
}

/// Where grok reads a working folder's rules from (only once the folder is trusted).
pub(crate) fn grok_rules(workdir: &std::path::Path) -> PathBuf {
    workdir.join(".grok").join("rules").join("mimir.md")
}

/// Trust a working folder in Mimir's grok home, so grok reads the rules in it. Trust isn't inherited by subfolders,
/// so each bot-and-chat folder gets its own entry.
fn grok_trust(home: &std::path::Path, dir: &std::path::Path) -> Result<()> {
    // grok keys folders by their real path (/tmp is /private/tmp on macOS); a JSON string is a valid TOML key.
    let key = format!("[folders.{}]", serde_json::to_string(&std::fs::canonicalize(dir)?.to_string_lossy())?);
    let path = grok_home(home).join(".grok").join("trusted_folders.toml");
    let mut f = std::fs::OpenOptions::new().create(true).read(true).append(true).open(&path)?;
    // Locked across processes (grok also runs from the tools server): two runs mustn't both add the same folder, which
    // would make the file invalid TOML. Read through the locked handle: on Windows a lock keeps out other handles.
    f.lock()?;
    let mut now = String::new();
    std::io::Read::read_to_string(&mut f, &mut now)?;
    if !now.contains(&key) {
        std::io::Write::write_all(&mut f, format!("\n{key}\ntrusted = true\ndecided_at = {}\n", crate::db::now()).as_bytes())?;
    }
    Ok(())
}

/// Grok keeps transcripts in sessions/<url-encoded cwd>/<id>. After an incognito turn, every session from the
/// incognito folder goes, matched on the decoded folder name (so path prefixes like Windows' \\?\ don't matter).
fn forget_incognito_sessions(home: &std::path::Path) {
    let decode = |s: &str| -> String {
        let b = s.as_bytes();
        let (mut out, mut i) = (vec![], 0);
        while i < b.len() {
            // Bytes, not str slices: a folder name may hold multi-byte characters.
            if b[i] == b'%'
                && i + 2 < b.len()
                && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    let sessions = grok_home(home).join(".grok").join("sessions");
    for dir in std::fs::read_dir(&sessions).into_iter().flatten().flatten() {
        let name = decode(&dir.file_name().to_string_lossy());
        if name.trim_end_matches(['/', '\\']).ends_with("workspace-incognito") {
            let _ = std::fs::remove_dir_all(dir.path());
        }
    }
}

/// The Grok sign-in waiting for its code to be confirmed (one at a time).
static GROK_LOGIN: std::sync::Mutex<Option<tokio::task::AbortHandle>> = std::sync::Mutex::new(None);

/// Stop a Grok sign-in that's still waiting (Cancel, or starting another).
pub fn grok_login_cancel() {
    if let Some(h) = GROK_LOGIN.lock().unwrap_or_else(|e| e.into_inner()).take() {
        h.abort();
    }
}

/// Sign in to Grok for Mimir (device code, so it works on a server too). Returns (url, code) once grok prints
/// them; the login keeps waiting in the background until it's approved in the browser or times out.
pub async fn grok_login(home: &std::path::Path) -> Result<(String, String)> {
    let mut cmd = grok_cmd(home);
    cmd.args(["login", "--device-auth"]).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("failed to start `grok` — is Grok Build installed?")?;
    let stderr = child.stderr.take().context("no stderr")?;
    // The task owns the process (kill_on_drop) from the start, so Cancel works even before the code shows up,
    // and starting another sign-in stops this one.
    let (tx, rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        // grok prints the sign-in URL and code on stderr; keep reading after, so it never blocks on a full pipe.
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        let mut tx = Some(tx);
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(url) = line.split_whitespace().find(|w| w.starts_with("https://"))
                && let Some(tx) = tx.take()
            {
                let _ = tx.send((url.to_string(), url.split("user_code=").nth(1).unwrap_or("").to_string()));
            }
        }
        let _ = child.wait().await;
    });
    if let Some(old) = GROK_LOGIN.lock().unwrap_or_else(|e| e.into_inner()).replace(task.abort_handle()) {
        old.abort();
    }
    match tokio::time::timeout(Duration::from_secs(20), rx).await {
        Ok(Ok(v)) => Ok(v),
        _ => {
            task.abort();
            bail!("grok didn't offer a sign-in code")
        }
    }
}

pub fn forget_session(home: &std::path::Path, kind: &str, session: &str) {
    // A session id is a long token; anything else (or anything path-like) isn't looked for.
    if session.len() < 8 || !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return;
    }
    let user = crate::user_home();
    let roots: Vec<PathBuf> = match kind {
        "claude" => user.map(|h| h.join(".claude").join("projects")).into_iter().collect(),
        "codex" => user.map(|h| h.join(".codex").join("sessions")).into_iter().collect(),
        "grok" => vec![grok_home(home).join(".grok").join("sessions")],
        _ => vec![],
    };
    fn sweep(dir: &std::path::Path, needle: &str, depth: u8) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let (path, name) = (e.path(), e.file_name().to_string_lossy().into_owned());
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            if name.contains(needle) {
                let _ = if is_dir { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
            } else if is_dir && depth > 0 {
                sweep(&path, needle, depth - 1);
            }
        }
    }
    for root in roots {
        sweep(&root, session, 3);
    }
}

/// A file only this account can read, deleted when dropped (a run that ends, fails or is stopped).
struct Private(PathBuf);

impl Private {
    fn write(home: &std::path::Path, text: &str) -> Result<Private> {
        Private::at(&home.join("tmp").join(format!("{}.txt", crate::random_hex(8)?)), text)
    }

    /// The same at a fixed place (replacing what a crash left there).
    fn at(path: &std::path::Path, text: &str) -> Result<Private> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(path);
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        std::io::Write::write_all(&mut o.open(path)?, text.as_bytes())?;
        Ok(Private(path.to_path_buf()))
    }
}

impl Drop for Private {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// An assistant message (Claude's or grok's stream-json) that used the CLI's own web search.
fn searched_web(v: &Value) -> bool {
    v["message"]["content"].as_array().into_iter().flatten().any(|c| {
        matches!(c["type"].as_str(), Some("tool_use" | "server_tool_use")) && c["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case("websearch") || n == "web_search" || n == "WebFetch")
    })
}

/// A Claude stream event → the live draft. Each new assistant message replaces the previous one's text.
fn on_claude_event(bot: i64, chat: i64, e: &Value) {
    match e["type"].as_str() {
        Some("message_start") => crate::live::reset(bot, chat),
        Some("content_block_delta") if e["delta"]["type"] == "text_delta" => {
            crate::live::set_step(bot, chat, None);
            crate::live::push(bot, chat, e["delta"]["text"].as_str().unwrap_or(""))
        }
        // A tool call is starting (it runs in the MCP subprocess; the run log records it when done).
        Some("content_block_start") if matches!(e["content_block"]["type"].as_str(), Some("tool_use" | "server_tool_use")) => {
            crate::live::set_step(bot, chat, Some(crate::live::step_label(e["content_block"]["name"].as_str().unwrap_or("a tool"))))
        }
        _ => {}
    }
}

fn claude_usage(v: &Value) -> Usage {
    let u = &v["usage"];
    let n = |k: &str| u[k].as_i64().unwrap_or(0);
    Usage {
        model: v["modelUsage"].as_object().and_then(|m| m.keys().next().cloned()),
        tokens_in: n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens"),
        tokens_out: n("output_tokens"),
        cost_usd: v["total_cost_usd"].as_f64(),
        cache_read: n("cache_read_input_tokens"),
        estimated: false,
    }
}

/// Is this AI account usable here? For onboarding: installed, signed in, which plan.
pub async fn detect(kind: &str, home: &std::path::Path) -> Value {
    async fn run(bin: &str, args: &[&str]) -> Option<std::process::Output> {
        let out = tokio::process::Command::new(bin).args(args).stdin(Stdio::null()).kill_on_drop(true).output();
        tokio::time::timeout(Duration::from_secs(15), out).await.ok()?.ok()
    }
    let text = |o: &std::process::Output| String::from_utf8_lossy(&o.stdout).trim().to_string();
    match kind {
        "claude" => {
            let bin = std::env::var("MIMIR_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
            let Some(v) = run(&bin, &["--version"]).await.filter(|o| o.status.success()) else {
                return json!({"installed": false});
            };
            let auth: Value = run(&bin, &["auth", "status", "--json"]).await.and_then(|o| serde_json::from_slice(&o.stdout).ok()).unwrap_or_default();
            // null when this CLI couldn't say (an older version without `auth status`).
            json!({"installed": true, "version": text(&v), "signed_in": auth["loggedIn"],
                   "plan": auth["subscriptionType"], "account": auth["email"]})
        }
        "codex" => {
            let bin = std::env::var("MIMIR_CODEX_BIN").unwrap_or_else(|_| "codex".into());
            let Some(v) = run(&bin, &["--version"]).await.filter(|o| o.status.success()) else {
                return json!({"installed": false});
            };
            let status = run(&bin, &["login", "status"]).await;
            let (ok, how) = status.map(|o| (o.status.success(), format!("{} {}", text(&o), String::from_utf8_lossy(&o.stderr).trim()))).unwrap_or((false, String::new()));
            let how = how.to_lowercase();
            let signed_in = if how.contains("not logged in") { json!(false) } else if ok && how.contains("logged in") { json!(true) } else { Value::Null };
            json!({"installed": true, "version": text(&v), "signed_in": signed_in, "plan": null})
        }
        // Signed in for Mimir (its own grok home), not the person's own ~/.grok.
        "grok" => {
            let out = |args: &'static [&'static str]| {
                let mut c = grok_cmd(home);
                c.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
                async move { tokio::time::timeout(Duration::from_secs(15), c.output()).await.ok()?.ok() }
            };
            let Some(v) = out(&["--version"]).await.filter(|o| o.status.success()) else {
                return json!({"installed": false});
            };
            let models = out(&["models"]).await.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
            json!({"installed": true, "version": text(&v), "signed_in": models.contains("You are logged in")})
        }
        // A model server on this machine: Ollama, else LM Studio.
        "local" => {
            for (server, base) in [("Ollama", OLLAMA), ("LM Studio", LM_STUDIO)] {
                if let Ok(models) = models("local", Some(base), None).await {
                    return json!({"installed": true, "signed_in": !models.is_empty(), "server": server, "base": base, "models": models});
                }
            }
            json!({"installed": false})
        }
        // Signed in from Mimir: nothing to install.
        "chatgpt" => {
            let account = crate::chatgpt::account();
            json!({"installed": true, "signed_in": account.is_some(), "account": account})
        }
        _ => json!({"installed": true, "signed_in": crate::host::api_key(kind).is_some()}),
    }
}

/// Chat models an API account offers, for the model picker: `{id, tools}`, where `tools` says whether
/// the model can call tools (remember things, run routines), if known. OpenRouter's list is public;
/// we ask for tool-capable models only. Ollama reports each model's capabilities.
pub async fn models(kind: &str, base: Option<&str>, key: Option<&str>) -> Result<Vec<Value>> {
    if kind == "chatgpt" {
        return crate::chatgpt::models().await;
    }
    let Some((default_base, _)) = api_defaults(kind) else { return Ok(vec![]) };
    let base = base.filter(|b| !b.trim().is_empty()).unwrap_or(default_base).trim_end_matches('/');
    let url = if kind == "openrouter" { format!("{base}/models?supported_parameters=tools") } else { format!("{base}/models") };
    let http = reqwest::Client::builder().timeout(Duration::from_secs(if kind == "local" { 2 } else { 15 })).build()?;
    let mut req = http.get(url);
    if let Some(k) = key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let v: Value = req.send().await?.error_for_status()?.json().await?;
    let mut out = vec![];
    // xAI also lists its image and video models (grok-imagine-…), which can't chat.
    let chat = |id: &&str| !(id.contains("embed") || kind == "xai" && id.contains("imagine"));
    for id in v["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str()).filter(chat) {
        let tools = match kind {
            "openrouter" => Some(true),
            "local" => ollama_tools(&http, base, id).await,
            _ => None,
        };
        out.push(json!({"id": id, "tools": tools}));
    }
    Ok(out)
}

/// Ollama's `/api/show` lists a model's capabilities; other servers don't have it (None).
async fn ollama_tools(http: &reqwest::Client, base: &str, model: &str) -> Option<bool> {
    let root = base.strip_suffix("/v1")?;
    let v: Value = http.post(format!("{root}/api/show")).json(&json!({"model": model})).send().await.ok()?.json().await.ok()?;
    Some(v["capabilities"].as_array()?.iter().any(|c| c == "tools"))
}

impl Provider {
    /// One tiny real round-trip, for "Test" buttons. Returns (reply, seconds).
    pub async fn check(&self, db: &crate::db::Db) -> Result<(String, f64)> {
        let mut ctx = ToolCtx::new(db.clone(), 1, crate::host::LOCAL_CHAT);
        ctx.tools_off = true;
        let t = std::time::Instant::now();
        let r = self.run(&ctx, "You are a connection test. Reply with exactly: OK", &[], "ping", None).await?;
        Ok((r.text.trim().to_string(), t.elapsed().as_secs_f64()))
    }
}

/// The settings key for what Claude Code last said about the plan's usage windows.
pub const WINDOWS: &str = "ai_windows:claude";

/// The plan's usage windows as Claude Code last reported them: [(name, share used 0–1, resets at)], with when.
pub fn windows(db: &crate::db::Db) -> Option<Value> {
    let v: Value = serde_json::from_str(&db.setting(WINDOWS).ok()??).ok()?;
    let list: Vec<Value> = v["info"]["unifiedWindows"]
        .as_object()?
        .iter()
        .map(|(name, w)| json!({"name": name, "used": w["utilization"], "resets_at": w["resetsAt"]}))
        .collect();
    Some(json!({"at": v["at"], "status": v["info"]["status"], "windows": list}))
}

/// When a usage limit that stopped a run resets: for a Claude bot, what Claude Code reported (a rejected request's reset
/// time, or a full window's); else a reset time in the error itself ("…|1791650400"). None when nobody said.
pub fn quota_reset(db: &crate::db::Db, kind: &str, error: &str, now: i64) -> Option<i64> {
    let from_cli = db.setting(WINDOWS).ok().flatten().filter(|_| kind == "claude").and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| {
        let info = &v["info"];
        let full = info["unifiedWindows"].as_object().into_iter().flatten().filter(|(_, w)| w["utilization"].as_f64().is_some_and(|u| u >= 1.0)).filter_map(|(_, w)| w["resetsAt"].as_i64()).max();
        if info["status"] == "rejected" { info["resetsAt"].as_i64().or(full) } else { full }
    });
    let in_error = error.split('|').filter_map(|p| p.trim().get(..10)).find_map(|d| d.parse::<i64>().ok().filter(|t| *t > 1_600_000_000));
    from_cli.or(in_error).filter(|t| *t > now)
}

impl Provider {
    /// Before a routine spends anything: is the AI account there? Only definite answers count (an older CLI that can't
    /// say whether it's signed in is taken as signed in), so a routine never stops on a guess.
    pub async fn ready(&self) -> Result<()> {
        let (kind, home) = match self {
            Provider::Claude { home, .. } => ("claude", home),
            Provider::Codex { home, .. } => ("codex", home),
            Provider::Grok { home, .. } => ("grok", home),
            Provider::OpenAi { .. } => return Ok(()),
            // Signed in from Mimir: nothing to start, just whether the sign-in is still there.
            Provider::ChatGpt { .. } if crate::chatgpt::account().is_none() => bail!("chatgpt isn't signed in, so the routine didn't run (nothing was spent)"),
            Provider::ChatGpt { .. } => return Ok(()),
        };
        // A check that passed holds for 10 minutes: a quiet check every 15 minutes shouldn't start the CLI twice more each
        // time. A failed one is checked again at the next run.
        static PASSED: std::sync::Mutex<Vec<(&str, std::time::Instant)>> = std::sync::Mutex::new(vec![]);
        if PASSED.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(k, t)| *k == kind && t.elapsed() < Duration::from_secs(600)) {
            return Ok(());
        }
        let v = detect(kind, home).await;
        if v["installed"] == false {
            bail!("failed to start `{kind}`: it isn't installed here, so the routine didn't run (nothing was spent)");
        }
        // grok's sign-in check reads its model list's wording, which is too loose to stop a routine on.
        if kind != "grok" && v["signed_in"] == false {
            bail!("{kind} isn't signed in, so the routine didn't run (nothing was spent)");
        }
        let mut passed = PASSED.lock().unwrap_or_else(|e| e.into_inner());
        passed.retain(|(k, _)| *k != kind);
        passed.push((kind, std::time::Instant::now()));
        Ok(())
    }
}

pub(crate) fn is_usage_limit(e: &anyhow::Error) -> bool {
    let m = format!("{e:#}").to_lowercase();
    m.contains("usage limit") || m.contains("rate limit") || m.contains("extra usage")
}

/// `codex exec --json` on the user's ChatGPT login. Locked down: no shell/exec, apps, browser,
/// computer use or plugins; read-only sandbox; our tools arrive via MCP.
// Built against codex-cli 0.159's JSONL events; live-tested on 0.159.2.
async fn codex(ctx: &ToolCtx, home: &PathBuf, model: Option<&str>, system: &str, user: &str, session: Option<&str>) -> Result<Reply> {
    let exe = serde_json::to_string(&std::env::current_exe()?)?;
    let home_s = serde_json::to_string(home)?;
    let workdir = home.join("workspace");
    std::fs::create_dir_all(&workdir)?;

    // The bot's instructions (with standing memories) from a private file, not the command line, which any account on
    // the computer can read. They stand in for Codex's own coding-agent instructions: a bot isn't a coding agent.
    let instructions = Private::write(home, system)?;
    let mut cmd = tokio::process::Command::new(std::env::var("MIMIR_CODEX_BIN").unwrap_or_else(|_| "codex".into()));
    cmd.current_dir(&workdir).arg("exec");
    if let Some(s) = session {
        cmd.args(["resume", s]);
    }
    cmd.args(["--json", "--skip-git-repo-check", "--ignore-user-config", "--ignore-rules"]);
    for f in ["shell_tool", "unified_exec", "apps", "browser_use", "browser_use_external", "computer_use", "in_app_browser", "image_generation", "multi_agent", "plugins", "hooks"] {
        cmd.args(["--disable", f]);
    }
    for kv in [
        r#"sandbox_mode="read-only""#.to_string(),
        r#"approval_policy="never""#.to_string(),
        // Its web tool opens any address on OpenAI's side (an "open_page" with data in the query is never seen here),
        // so not after outside content: the conversation has read something that could ask for that.
        format!("web_search={:?}", if ctx.is_tainted() { "disabled" } else { "live" }),
        // A JSON string is also a valid TOML string (Windows backslashes included).
        format!("model_instructions_file={}", serde_json::to_string(&instructions.0)?),
    ] {
        cmd.arg("-c").arg(kv);
    }
    // Tools off: no tools server (--ignore-user-config means none other loads), so the run doesn't pay its startup.
    for kv in if ctx.tools_off { vec![] } else { vec![
        format!("mcp_servers.mimir.command={exe}"),
        // A JSON array of strings is also a valid TOML array.
        format!("mcp_servers.mimir.args={}", serde_json::to_string(&ctx.mcp_args())?),
        format!("mcp_servers.mimir.env={{MIMIR_HOME={home_s}}}"),
        format!("mcp_servers.mimir.tool_timeout_sec={}", TURN_TIMEOUT.as_secs()),
        // Mimir's own gate decides (allow / ask / deny, outside content): Codex hands every call through instead of
        // refusing ones it would otherwise want approved (with approval_policy=never it rejects them outright).
        r#"mcp_servers.mimir.default_tools_approval_mode="approve""#.to_string(),
    ] } {
        cmd.arg("-c").arg(kv);
    }
    if let Some(m) = model {
        cmd.args(["-m", m]);
    }
    for img in &ctx.images {
        cmd.arg("--image").arg(img);
    }
    if ctx.incognito() {
        // Codex otherwise keeps the session's rollout file on disk.
        cmd.arg("--ephemeral");
    }
    cmd.arg("-").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);

    let mut child = cmd.spawn().context("failed to start `codex` — is the Codex CLI installed and logged in with ChatGPT?")?;
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(user.as_bytes()).await?;
    drop(stdin);
    // Read as it comes: the moment it searches the web (pages anyone can write), the run counts as having read
    // outside content, so its tools later in this same turn ask first (the tools server reads the run's mark).
    let mut lines = tokio::io::BufReader::new(child.stdout.take().context("no stdout")?).lines();
    let mut stderr = child.stderr.take().context("no stderr")?;
    let read = async {
        let mut all = String::new();
        while let Some(line) = lines.next_line().await? {
            if serde_json::from_str::<Value>(&line).is_ok_and(|v| v["item"]["type"] == "web_search") {
                ctx.taint();
            }
            all.push_str(&line);
            all.push('\n');
        }
        anyhow::Ok(all)
    };
    let drain = async {
        let mut e = String::new();
        let _ = tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut e).await;
    };
    let (stdout, ()) = tokio::time::timeout(TURN_TIMEOUT, async { tokio::join!(read, drain) }).await.context("codex timed out")?;
    child.wait().await?;
    parse_codex_jsonl(&stdout?)
}

fn parse_codex_jsonl(stdout: &str) -> Result<Reply> {
    let (mut text, mut thread, mut err, mut usage) = (None, None, None, Usage::default());
    for v in stdout.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        match v["type"].as_str().unwrap_or("") {
            "thread.started" => thread = v["thread_id"].as_str().map(String::from),
            "item.completed" if v["item"]["type"] == "agent_message" => text = v["item"]["text"].as_str().map(String::from),
            "turn.completed" => {
                usage.tokens_in += v["usage"]["input_tokens"].as_i64().unwrap_or(0);
                usage.tokens_out += v["usage"]["output_tokens"].as_i64().unwrap_or(0);
                usage.cache_read += v["usage"]["cached_input_tokens"].as_i64().unwrap_or(0);
            }
            "turn.failed" => err = v["error"]["message"].as_str().map(String::from),
            "error" => err = err.or(v["message"].as_str().map(String::from)),
            _ => {}
        }
    }
    match (text, err) {
        (Some(text), _) => Ok(Reply { text, session: thread, usage }),
        (None, Some(e)) => bail!("codex: {e}"),
        (None, None) => bail!("codex produced no reply: {}", tools::truncate(stdout, 300)),
    }
}

/// Accumulates OpenAI-style streamed chunks: text deltas, fragmented tool calls, final usage.
#[derive(Default)]
struct Acc {
    content: String,
    calls: Vec<(String, String, String)>,
    usage: Value,
    model: Option<String>,
}

impl Acc {
    /// Folds one chunk; returns new text to show, if any.
    fn fold(&mut self, c: &Value) -> Option<String> {
        if !c["usage"].is_null() {
            self.usage = c["usage"].clone();
        }
        if let Some(m) = c["model"].as_str() {
            self.model = Some(m.into());
        }
        let d = &c["choices"][0]["delta"];
        for t in d["tool_calls"].as_array().into_iter().flatten() {
            let i = t["index"].as_u64().unwrap_or(0) as usize;
            if self.calls.len() <= i {
                self.calls.resize(i + 1, Default::default());
            }
            let call = &mut self.calls[i];
            if let Some(id) = t["id"].as_str() {
                call.0 = id.into();
            }
            if let Some(n) = t["function"]["name"].as_str() {
                call.1.push_str(n);
            }
            call.2.push_str(t["function"]["arguments"].as_str().unwrap_or(""));
        }
        let text = d["content"].as_str().filter(|s| !s.is_empty())?;
        self.content.push_str(text);
        Some(text.to_string())
    }

    fn into_response(self) -> Value {
        let calls: Vec<Value> = self.calls.into_iter().map(|(id, name, args)| json!({"id": id, "type": "function", "function": {"name": name, "arguments": args}})).collect();
        let mut msg = json!({"role": "assistant", "content": self.content});
        if !calls.is_empty() {
            msg["tool_calls"] = json!(calls);
        }
        json!({"choices": [{"message": msg}], "usage": self.usage, "model": self.model})
    }
}

async fn openai(ctx: &ToolCtx, base: &str, key: &str, model: &str, system: &str, history: &[(String, String)], user: &str) -> Result<Reply> {
    let mut messages = vec![json!({"role":"system","content":system})];
    messages.extend(history.iter().map(|(role, content)| json!({"role":role,"content":content})));
    if ctx.images.is_empty() {
        messages.push(json!({"role":"user","content":user}));
    } else {
        // Vision models take images as data URLs next to the text; big photos are shrunk (requests have size caps,
        // and history resends them).
        let mut parts = vec![json!({"type": "text", "text": user})];
        for (mime, data) in inline_images(&ctx.images, 8_000_000)? {
            parts.push(json!({"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{data}")}}));
        }
        messages.push(json!({"role":"user","content":parts}));
    }
    let mut tool_specs: Vec<Value> = tools::specs_for(ctx)
        .into_iter()
        .map(|(name, description, parameters)| json!({"type":"function","function":{"name":name,"description":description,"parameters":parameters}}))
        .collect();
    let http = reqwest::Client::builder().timeout(TURN_TIMEOUT).build()?;
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));

    // ponytail: no price table; only OpenRouter runs get a cost (it reports one)
    let mut usage = Usage { model: Some(model.to_string()), ..Default::default() };
    // Using the computer takes many small steps (look, click, type, look again).
    let rounds = if tools::specs_for(ctx).iter().any(|s| s.0 == "computer_screenshot") { 40 } else { MAX_TOOL_ROUNDS };
    for _ in 0..rounds {
        // Our own bookkeeping field stays out of the request (strict APIs reject unknown fields).
        let sent: Vec<Value> = messages.iter().map(|m| { let mut m = m.clone(); if let Some(o) = m.as_object_mut() { o.remove("mimir_screenshot"); } m }).collect();
        let mut body = json!({"model":model,"messages":sent});
        // OpenAI rejects an empty tools array.
        if !tool_specs.is_empty() {
            body["tools"] = json!(tool_specs);
        }
        if base.contains("openrouter.ai") {
            body["usage"] = json!({"include": true});
        }
        if ctx.stream {
            body["stream"] = json!(true);
            body["stream_options"] = json!({"include_usage": true});
        }
        let mut req = http.post(&url).json(&body);
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
        let mut resp = req.send().await?;
        let status = resp.status();
        let sse = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("event-stream"));
        let v: Value = if sse && status.is_success() {
            // Streamed: fold SSE chunks into the same shape as a non-streamed response.
            crate::live::reset(ctx.bot_id, ctx.chat_id);
            let mut acc = Acc::default();
            // Bytes until a whole line is in: a character split across two network chunks stays whole.
            let mut bytes: Vec<u8> = vec![];
            while let Some(chunk) = resp.chunk().await? {
                bytes.extend_from_slice(&chunk);
                if bytes.len() > 32 * 1024 * 1024 {
                    bail!("the AI service sent a reply too big to read");
                }
                while let Some(nl) = bytes.iter().position(|b| *b == b'\n') {
                    let line = String::from_utf8_lossy(&bytes[..=nl]).into_owned();
                    bytes.drain(..=nl);
                    if let Some(data) = line.trim().strip_prefix("data:").map(str::trim).filter(|d| *d != "[DONE]")
                        && let Ok(c) = serde_json::from_str::<Value>(data)
                        && let Some(delta) = acc.fold(&c)
                    {
                        crate::live::push(ctx.bot_id, ctx.chat_id, &delta);
                    }
                }
            }
            acc.into_response()
        } else {
            let v: Value = resp.json().await.context("provider returned non-JSON")?;
            if !status.is_success() {
                let msg = v["error"]["message"].as_str().map(String::from).unwrap_or_else(|| v.to_string());
                // Plenty of local (and some OpenRouter) models can't call tools: answer without them, honestly.
                if msg.contains("support tool") && !tool_specs.is_empty() {
                    tool_specs.clear();
                    messages[0]["content"] = json!(format!("{system}\n\nThis model can't use tools: you can't save memories, set routines or open pages. If asked to, say so."));
                    continue;
                }
                if status == reqwest::StatusCode::UNAUTHORIZED {
                    bail!("the API key was rejected ({msg}); check it in Settings → AI keys");
                }
                bail!("provider {status}: {msg}");
            }
            v
        };
        usage.tokens_in += v["usage"]["prompt_tokens"].as_i64().unwrap_or(0);
        usage.tokens_out += v["usage"]["completion_tokens"].as_i64().unwrap_or(0);
        usage.cache_read += v["usage"]["prompt_tokens_details"]["cached_tokens"].as_i64().unwrap_or(0);
        if let Some(c) = v["usage"]["cost"].as_f64() {
            usage.cost_usd = Some(usage.cost_usd.unwrap_or(0.0) + c);
        }
        if let Some(m) = v["model"].as_str() {
            usage.model = Some(m.to_string());
        }
        let msg = v["choices"][0]["message"].clone();
        let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
        if calls.is_empty() {
            return Ok(Reply { text: msg["content"].as_str().unwrap_or("").to_string(), session: None, usage });
        }
        messages.push(msg);
        for c in calls {
            let name = c["function"]["name"].as_str().unwrap_or("");
            let args: Value = serde_json::from_str(c["function"]["arguments"].as_str().unwrap_or("{}")).unwrap_or(json!({}));
            let out = tools::call(ctx, name, &args).await.unwrap_or_else(|e| format!("error: {e}"));
            match crate::computer::tool_image(name, &out, &ctx.db.dir()) {
                (Some(path), text) => {
                    messages.push(json!({"role":"tool","tool_call_id":c["id"],"content":text}));
                    // Tool messages can't carry pictures in this API: the screenshot follows as an image message.
                    // Only the latest two stay in the conversation, so a long session doesn't pile them up.
                    let mut shots: Vec<usize> = messages.iter().enumerate().filter(|(_, m)| m["mimir_screenshot"] == true).map(|(i, _)| i).collect();
                    while shots.len() >= 2 {
                        let i = shots.remove(0);
                        messages[i] = json!({"role":"user","content":"(an earlier screenshot, removed)"});
                    }
                    if let Some(data) = crate::computer::take_image(&path) {
                        messages.push(json!({"role":"user","mimir_screenshot":true,"content":[
                            {"type":"text","text":"The computer's screen:"},
                            {"type":"image_url","image_url":{"url":format!("data:image/jpeg;base64,{data}")}}
                        ]}));
                    }
                }
                (None, text) => messages.push(json!({"role":"tool","tool_call_id":c["id"],"content":text})),
            }
        }
    }
    bail!("gave up after {rounds} tool rounds")
}

/// Tell a CLI model where the attached images are (it opens them with its Read tool).
fn with_image_note(user: &str, images: &[PathBuf]) -> String {
    if images.is_empty() {
        return user.to_string();
    }
    let list: Vec<String> = images.iter().map(|p| p.display().to_string()).collect();
    format!("{user}\n\n[The person attached {} image(s). Look at them with the Read tool before answering: {}]", images.len(), list.join(", "))
}

/// Speech to text for voice notes, with the OpenAI-compatible API key (OpenAI, or OPENAI_BASE_URL).
/// None when no key is set up: the subscriptions (Claude, ChatGPT) don't transcribe audio.
pub async fn transcribe(audio: Vec<u8>, filename: &str) -> Option<Result<String>> {
    let key = crate::host::api_key("openai")?;
    let base = std::env::var("OPENAI_BASE_URL").ok().filter(|b| !b.is_empty()).unwrap_or_else(|| "https://api.openai.com/v1".into());
    Some(
        async {
            let part = reqwest::multipart::Part::bytes(audio).file_name(filename.to_string());
            let form = reqwest::multipart::Form::new().text("model", "whisper-1").part("file", part);
            let v: Value = reqwest::Client::new()
                .post(format!("{}/audio/transcriptions", base.trim_end_matches('/')))
                .bearer_auth(key)
                .multipart(form)
                .timeout(Duration::from_secs(120))
                .send()
                .await?
                .json()
                .await?;
            v["text"].as_str().map(|t| t.trim().to_string()).with_context(|| format!("transcription failed: {}", v["error"]["message"].as_str().unwrap_or("no text")))
        }
        .await,
    )
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_grok_folder_is_trusted_once() {
        let home = std::env::temp_dir().join(format!("mimir-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let dir = home.join("workspace").join("1--100123");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(super::grok_home(&home).join(".grok")).unwrap();
        super::grok_trust(&home, &dir).unwrap();
        super::grok_trust(&home, &dir).unwrap();
        let toml = std::fs::read_to_string(super::grok_home(&home).join(".grok").join("trusted_folders.toml")).unwrap();
        assert_eq!(toml.matches("[folders.").count(), 1, "{toml}");
        assert!(toml.contains("1--100123\"]\ntrusted = true\n"), "{toml}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn forgetting_a_session_removes_only_its_transcript() {
        let home = std::env::temp_dir().join(format!("mimir-forget-{}", std::process::id()));
        let sessions = super::grok_home(&home).join(".grok").join("sessions").join("%2Fworkspace");
        let (mine, other) = (sessions.join("01a11b4b-391e-7141"), sessions.join("99ffffff-0000-1111"));
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        super::forget_session(&home, "grok", "01a11b4b-391e-7141");
        assert!(!mine.exists() && other.exists());
        super::forget_session(&home, "grok", "../..");
        assert!(other.exists(), "nothing path-like is looked for");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn another_model_for_background_work() {
        let p = Provider::Claude { model: Some("sonnet".into()), home: "/h".into() };
        let Provider::Claude { model, home } = p.with_model(Some(" haiku ")) else { panic!() };
        assert_eq!((model.as_deref(), home.to_str()), (Some("haiku"), Some("/h")));
        let Provider::Claude { model, .. } = p.with_model(Some("")) else { panic!() };
        assert_eq!(model.as_deref(), Some("sonnet"), "blank keeps the bot's model");
    }

    #[test]
    fn incognito_grok_sessions_are_removed_whatever_the_path_prefix() {
        let home = std::env::temp_dir().join(format!("mimir-gs-{}", std::process::id()));
        let sessions = super::grok_home(&home).join(".grok").join("sessions");
        for d in ["%2Fprivate%2Ftmp%2Fx%2Fworkspace-incognito/abc", "%5C%5C%3F%5CC%3A%5Cx%5Cworkspace-incognito/def", "%2Ftmp%2Fx%2Fworkspace/keep", "caf%C3%A9-%E2%9C%93/keep2"] {
            std::fs::create_dir_all(sessions.join(d)).unwrap();
        }
        super::forget_incognito_sessions(&home);
        let left: Vec<String> = std::fs::read_dir(&sessions).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left.len(), 2, "{left:?}");
        assert!(left.iter().all(|n| !n.contains("incognito")));
    }

    #[test]
    fn photos_shrink_to_fit_what_a_model_takes_inline() {
        let dir = std::env::temp_dir().join(format!("mimir-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A noisy 1600×1100 photo is megabytes as PNG: far over a Windows command line.
        let big = dir.join("big.png");
        image::RgbImage::from_fn(1600, 1100, |x, y| image::Rgb([((x * 7) ^ (y * 13)) as u8, (x * y) as u8, (x + y * 3) as u8])).save(&big).unwrap();
        let small = dir.join("small.webp");
        std::fs::write(&small, b"RIFF....WEBP").unwrap();
        let out = super::inline_images(std::slice::from_ref(&big), 12_000).unwrap();
        assert_eq!(out[0].0, "image/jpeg");
        assert!(out[0].1.len() <= 12_000, "{}", out[0].1.len());
        let both = super::inline_images(&[big, small], 2_000_000).unwrap();
        assert_eq!((both[1].0.as_str(), both[1].1.as_str()), ("image/webp", "UklGRi4uLi5XRUJQ"), "small files go as they are, with the right type");
        assert!(super::inline_images(&[], 10).unwrap().is_empty());
    }

    use super::*;

    #[test]
    fn api_accounts() {
        let home = PathBuf::from("/tmp");
        let new = |kind, model: Option<&str>, base: Option<&str>, key: Option<&str>| {
            Provider::new(kind, model.map(String::from), base.map(String::from), key.map(String::from), home.clone())
        };
        assert_eq!(new("openrouter", None, None, Some("k")).unwrap().name(), "openrouter/auto @ https://openrouter.ai/api/v1");
        assert!(new("openrouter", None, None, None).is_err(), "OpenRouter needs a key");
        assert!(new("xai", None, None, None).is_err(), "xAI needs a key");
        assert_eq!(new("grok", None, None, None).unwrap().name(), "grok -p (default model)");
        assert_eq!(new("xai", None, None, Some("k")).unwrap().name(), "grok-4.7 @ https://api.x.ai/v1");
        assert!(new("local", None, None, None).is_err(), "a local bot needs a model");
        assert_eq!(new("local", Some("qwen3:4b"), None, None).unwrap().name(), "qwen3:4b @ http://localhost:11434/v1");
        assert_eq!(new("local", Some("m"), Some("http://localhost:1234/v1"), None).unwrap().name(), "m @ http://localhost:1234/v1");
        assert!(new("nope", None, None, None).is_err());
    }

    #[test]
    fn openai_stream_fold() {
        let mut a = Acc::default();
        let chunks = [
            json!({"model":"gpt-x","choices":[{"delta":{"content":"Hel"}}]}),
            json!({"choices":[{"delta":{"content":"lo"}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"memory_search","arguments":"{\"qu"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ery\":\"rust\"}"}}]}}]}),
            json!({"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":3}}),
        ];
        let shown: Vec<String> = chunks.iter().filter_map(|c| a.fold(c)).collect();
        assert_eq!(shown, ["Hel", "lo"]);
        let v = a.into_response();
        assert_eq!(v["choices"][0]["message"]["content"], "Hello");
        assert_eq!(v["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"], "{\"query\":\"rust\"}");
        assert_eq!((v["usage"]["prompt_tokens"].as_i64(), v["model"].as_str()), (Some(12), Some("gpt-x")));
    }

    #[test]
    fn claude_stream_events() {
        on_claude_event(42, 9, &json!({"type":"message_start"}));
        on_claude_event(42, 9, &json!({"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hmm"}}));
        on_claude_event(42, 9, &json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi "}}));
        on_claude_event(42, 9, &json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"there"}}));
        assert_eq!(crate::live::get(42, 9).as_deref(), Some("Hi there"), "thinking isn't shown");
        on_claude_event(42, 9, &json!({"type":"message_start"}));
        assert_eq!(crate::live::get(42, 9).as_deref(), Some(""), "a new message replaces the old text");
        crate::live::end(42, 9);
    }

    #[test]
    fn codex_events() {
        let ok = r#"{"type":"thread.started","thread_id":"t1"}
{"type":"item.completed","item":{"id":"item_0","type":"error","message":"noise"}}
{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"hello"}}
{"type":"turn.completed","usage":{"input_tokens":5}}"#;
        let r = parse_codex_jsonl(ok).unwrap();
        assert_eq!((r.text.as_str(), r.session.as_deref()), ("hello", Some("t1")));
        assert_eq!((r.usage.tokens_in, r.usage.tokens_out), (5, 0));
        let c = claude_usage(&serde_json::json!({"total_cost_usd":0.02,"usage":{"input_tokens":2,"cache_read_input_tokens":10,"cache_creation_input_tokens":5,"output_tokens":4},"modelUsage":{"claude-sonnet-5":{}}}));
        assert_eq!((c.model.as_deref(), c.tokens_in, c.tokens_out, c.cost_usd), (Some("claude-sonnet-5"), 17, 4, Some(0.02)));

        let limited = r#"{"type":"thread.started","thread_id":"t2"}
{"type":"error","message":"You've hit your usage limit."}
{"type":"turn.failed","error":{"message":"You've hit your usage limit."}}"#;
        let e = parse_codex_jsonl(limited).unwrap_err();
        assert!(is_usage_limit(&e), "{e}");
        // When it resets: what Claude Code reported, else the time in the error.
        let p = std::env::temp_dir().join(format!("mimir-reset-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = crate::db::Db::open(&p).unwrap();
        assert_eq!(quota_reset(&db, "claude", "Claude AI usage limit reached|1791650400", 1_791_600_000), Some(1_791_650_400));
        assert_eq!(quota_reset(&db, "claude", "usage limit", 1_791_600_000), None);
        let info = serde_json::json!({"at": 1, "info": {"status": "rejected", "resetsAt": 1_791_700_000, "unifiedWindows": {"five_hour": {"utilization": 1.0, "resetsAt": 1_791_700_000}, "seven_day": {"utilization": 0.4, "resetsAt": 1_792_000_000}}}});
        db.set_setting(WINDOWS, &info.to_string()).unwrap();
        assert_eq!(quota_reset(&db, "claude", "usage limit", 1_791_600_000), Some(1_791_700_000));
        assert_eq!(quota_reset(&db, "codex", "usage limit", 1_791_600_000), None, "Claude's windows aren't another account's");
        assert_eq!(windows(&db).unwrap()["windows"].as_array().unwrap().len(), 2);
    }
}
