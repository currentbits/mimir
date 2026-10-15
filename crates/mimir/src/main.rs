use anyhow::{Context, Result};
use mimir_core::{Trigger, db::Db, mcp, provider::Provider, runtime, tools::ToolCtx, turn};
use std::path::PathBuf;

mod server;
use std::sync::Arc;

const USAGE: &str = "mimir — always-on personal assistants

USAGE:
  mimir server [--listen 127.0.0.1:8080]   self-hosted mode: all bots 24/7 + web UI + API
  mimir admin-token [--new] print the server's admin token (--new replaces it; restart the server)
  mimir computer [--listen 0.0.0.0:7300]   the bots' computer as its own service (docker compose)
  mimir serve               run one bot from env vars (no UI) on Telegram/Slack/Discord
  mimir chat <message>      one local turn as chat 0 (no Telegram needed)
  mimir mcp --chat <id>     MCP stdio server (spawned by `claude -p`)
  mimir transcribe <file>   speech to text for a recording (whisper.cpp here, or your OpenAI-compatible key)
  mimir runs                recent runs with tokens, cost and tool calls
  mimir approvals           list actions waiting for your approval
  mimir approve <id>        approve one (deny <id> to refuse)
  mimir doctor              check AI logins, channels, the computer, disk, database, backups and security

ENV:
  TELEGRAM_BOT_TOKEN         from @BotFather (serve)
  SLACK_BOT_TOKEN, SLACK_APP_TOKEN   xoxb-… and xapp-… (Socket Mode) (serve)
  DISCORD_BOT_TOKEN          from the Discord Developer Portal (serve)
  MIMIR_PROVIDER            claude (default, your `claude` login) | codex (your ChatGPT login)
                             | grok (your SuperGrok login via Grok Build) | openrouter | xai (Grok API key)
                             | local (Ollama / LM Studio) | openai
  MIMIR_MODEL               model override (required for local, e.g. qwen3:4b)
  OPENROUTER_API_KEY         for provider=openrouter
  XAI_API_KEY                for provider=xai
  OPENAI_API_KEY             for provider=openai
  OPENAI_BASE_URL            endpoint for openai/local (local default http://localhost:11434/v1)
  MIMIR_HOME                data dir (default ~/.mimir)
  MIMIR_BOT                 which bot id the CLI acts as (default 1; manage bots in the app)
  MIMIR_ADMIN_TOKEN         server: fixed admin token (else generated on first run)
  MIMIR_UI_DIR              server: web UI directory (default ui/dist or /app/ui)
  MIMIR_COMPUTER            server: the computer service's address (computer:7300); unset, it runs in-process
  MIMIR_COMPUTER_TOKEN      server + computer: the control channel's shared secret (24+ characters)";

/// The CLI keeps its env-var provider config and acts as one bot (memory/routines/runs land on it).
fn cli_bot() -> i64 {
    std::env::var("MIMIR_BOT").ok().and_then(|b| b.parse().ok()).unwrap_or(1)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The PDF helper (a separate process for reading one PDF): nothing else, no data folder.
    if args.first().map(String::as_str) == Some("read-pdf") {
        mimir_core::pdf_helper_main();
    }
    mimir_core::use_pdf_helper();
    let listen = args.iter().position(|a| a == "--listen").and_then(|i| args.get(i + 1)).map(String::as_str);
    // The bots' computer as its own service: no data folder, no database.
    if args.first().map(String::as_str) == Some("computer") {
        return tokio::task::spawn_blocking({
            let listen = listen.unwrap_or("0.0.0.0:7300").to_string();
            move || mimir_core::computer::serve(&listen)
        })
        .await?;
    }
    let home = mimir_core::data_dir()?;
    // Before opening the database: a damaged or too-new one is one of the lines.
    if args.first().map(String::as_str) == Some("doctor") {
        let checks = mimir_core::doctor::run(&home).await;
        checks.iter().for_each(|c| println!("{c}"));
        std::process::exit(if mimir_core::doctor::passed(&checks) { 0 } else { 1 });
    }
    let db = Db::open(&home.join("mimir.db"))?;
    match args.first().map(String::as_str) {
        Some("serve") => serve(db, home).await,
        Some("transcribe") => {
            let file = args.get(1).context("usage: mimir transcribe <audio or video file>")?;
            let model = mimir_core::transcribe::model_name(&db);
            println!("{}", mimir_core::transcribe::transcribe_file(&home, &model, std::path::Path::new(file)).await?);
            Ok(())
        }
        Some("server") => {
            server::run(db, home, listen.unwrap_or("127.0.0.1:8080")).await
        }
        Some("admin-token") => {
            if args.get(1).is_some_and(|a| a == "--new") {
                if std::env::var("MIMIR_ADMIN_TOKEN").is_ok_and(|t| !t.trim().is_empty()) {
                    anyhow::bail!("the token comes from MIMIR_ADMIN_TOKEN: change it there");
                }
                // Signed-in browsers and connected apps need the new one; the running server picks it up on restart.
                db.set_setting("admin_token", &mimir_core::random_hex(24)?)?;
            }
            println!("{}", server::admin_token(&db)?.0);
            Ok(())
        }
        Some("chat") if args.len() > 1 => {
            let provider = Provider::from_env(home)?;
            db.pair(0, "local")?;
            println!("{}", turn(&provider, &ToolCtx::new(db, cli_bot(), 0), Trigger::Message(&args[1..].join(" "))).await?);
            Ok(())
        }
        Some("runs") => {
            let t = db.run_totals(None, mimir_core::db::now() - 30 * 86400)?;
            println!("last 30 days: {} runs, {} errors, {} tokens in / {} out, ${:.2} API-equivalent\n", t.runs, t.errors, t.tokens_in, t.tokens_out, t.cost_usd);
            for r in db.runs(None, 20)? {
                let cost = r.cost_usd.map(|c| format!("${c:.4}")).unwrap_or_else(|| "-".into());
                println!("#{} {} chat {} {} [{}] {} {}→{} tok {} {} tools{}", r.id, mimir_core::tools::fmt_ts(r.started_at), r.chat_id, r.trigger,
                    r.model.as_deref().unwrap_or(&r.provider), r.status, r.tokens_in, r.tokens_out, cost, r.tool_calls,
                    r.error.map(|e| format!("  ! {}", mimir_core::tools::truncate(&e, 120))).unwrap_or_default());
                for c in db.tool_calls(r.id)? {
                    println!("    {} {} [{}] {}", if c.ok { "✓" } else { "✗" }, c.summary, c.decision, mimir_core::tools::truncate(&c.result, 80));
                }
            }
            Ok(())
        }
        Some("approvals") => {
            for a in db.approvals_pending()? {
                println!("#{}  chat {}  {}  (expires {})", a.id, a.chat_id, a.summary, mimir_core::tools::fmt_ts(a.expires_at));
            }
            Ok(())
        }
        Some(cmd @ ("approve" | "deny")) => {
            let id: i64 = args.get(1).and_then(|s| s.parse().ok()).context("usage: mimir approve|deny <id>")?;
            match db.approval_decide(id, cmd == "approve", None)? {
                Some(a) => println!("{}: {}", a.status, a.summary),
                None => println!("#{id} is not pending (already decided or expired)"),
            }
            Ok(())
        }
        Some("mcp") => {
            // ask_bot may start another bot from inside this subprocess; server-stored API key (else env).
            mimir_core::host::install_secrets(std::sync::Arc::new(mimir_core::host::DbSecrets(db.clone())));
            mcp::serve_stdio(mcp::ctx_from_args(db, &args)?).await
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

async fn serve(db: Db, home: PathBuf) -> Result<()> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let channels = runtime::Channels {
        telegram: var("TELEGRAM_BOT_TOKEN"),
        slack: var("SLACK_BOT_TOKEN").zip(var("SLACK_APP_TOKEN")),
        discord: var("DISCORD_BOT_TOKEN"),
    };
    if channels.telegram.is_none() && channels.slack.is_none() && channels.discord.is_none() {
        eprintln!("no channel configured (TELEGRAM_BOT_TOKEN / SLACK_BOT_TOKEN+SLACK_APP_TOKEN / DISCORD_BOT_TOKEN); running routines only");
    }
    db.close_interrupted()?;
    let provider = Arc::new(Provider::from_env(home)?);
    mimir_core::say::load(&db);
    runtime::serve(db, cli_bot(), provider, channels, Default::default()).await
}
