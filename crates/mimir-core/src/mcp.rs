//! Minimal MCP stdio server (newline-delimited JSON-RPC) exposing `tools::specs`.
// ponytail: hand-rolled initialize/tools.list/tools.call only; move to rmcp if we need resources/prompts/sampling

use crate::tools::{self, ToolCtx};
use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Builds the context from `mcp [--bot <id>] --chat <id> [--run <id>]` argv (shared by CLI and desktop binaries).
pub fn ctx_from_args(db: crate::db::Db, args: &[String]) -> Result<ToolCtx> {
    // `mcp --from-env`: grok's config is shared by every run, so its per-run arguments come in MIMIR_MCP_ARGS.
    // Only that form reads them, so a server started further down (say, for a Claude bot it asked) can't inherit them.
    let from_env = run_args(args, std::env::var("MIMIR_MCP_ARGS").ok().as_deref());
    if args.iter().any(|a| a == "--from-env") && from_env.is_none() {
        anyhow::bail!("started with --from-env but MIMIR_MCP_ARGS is missing or invalid");
    }
    let args = from_env.as_deref().unwrap_or(args);
    let flag = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).and_then(|s| s.parse::<i64>().ok());
    let chat = flag("--chat").ok_or_else(|| anyhow::anyhow!("--chat <id> required"))?;
    let mut ctx = ToolCtx::new(db, flag("--bot").unwrap_or(1), chat).with_run(flag("--run"));
    ctx.conv = flag("--conv").unwrap_or(chat);
    ctx.ask_chat = flag("--ask-chat").unwrap_or(chat);
    ctx.tools_off = args.iter().any(|a| a == "--no-tools");
    ctx.read_only = args.iter().any(|a| a == "--read-only");
    ctx.depth = flag("--depth").unwrap_or(0).clamp(0, 9) as u8;
    ctx.via_bot = flag("--via");
    ctx.sticky = args.iter().any(|a| a == "--sticky");
    ctx.unattended = args.iter().any(|a| a == "--unattended");
    ctx.rehearsal = args.iter().any(|a| a == "--rehearsal");
    // The flag from the parent, or the run itself: a server started again mid-run (grok resuming a session, a CLI
    // restarting its servers) must not forget that this run already read untrusted content.
    if args.iter().any(|a| a == "--tainted")
        || ctx.run_id.is_some_and(|r| ctx.db.run_tainted(r).unwrap_or(true))
        || (ctx.sticky && ctx.db.conv_tainted(ctx.bot_id, ctx.conv).unwrap_or(true))
    {
        ctx.pre_taint();
    }
    if args.iter().any(|a| a == "--private") {
        ctx.pre_private();
    }
    if let Some(s) = args.iter().position(|a| a == "--sensitive").and_then(|i| args.get(i + 1)) {
        ctx.pre_sensitive(s);
    }
    if ctx.sticky
        && let Some(kept) = ctx.db.setting(&crate::tools::private_key(ctx.bot_id, ctx.conv)).ok().flatten()
    {
        ctx.restore_private(&kept);
    }
    Ok(ctx)
}

fn run_args(args: &[String], env: Option<&str>) -> Option<Vec<String>> {
    if args.iter().any(|a| a == "--from-env") { serde_json::from_str(env?).ok() } else { None }
}

pub async fn serve_stdio(ctx: ToolCtx) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let Some(resp) = handle(&ctx, &line).await else { continue };
        out.write_all(format!("{resp}\n").as_bytes()).await?;
        out.flush().await?;
    }
    Ok(())
}

/// Returns None for notifications (no id).
pub async fn handle(ctx: &ToolCtx, line: &str) -> Option<Value> {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":e.to_string()}})),
    };
    let id = req.get("id").cloned()?;
    let method = req["method"].as_str().unwrap_or("");
    let params = &req["params"];
    let result = match method {
        "initialize" => json!({
            // The version asked for if it's one this server speaks, else its own: a 2026-07-28 client (stateless, no
            // initialize) falls back to it. ponytail: 2026-07-28 itself waits until MCP Events need it.
            "protocolVersion": params["protocolVersion"].as_str().filter(|v| ["2025-06-18", "2025-03-26", "2024-11-05"].contains(v)).unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "mimir", "version": env!("CARGO_PKG_VERSION")}
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools::specs_for(ctx).into_iter()
            .map(|(name, description, schema)| json!({"name": name, "description": description, "inputSchema": schema}))
            .collect::<Vec<_>>()}),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match tools::call(ctx, name, &args).await {
                // A screenshot goes to the model as an image, next to the text.
                Ok(out) => match crate::computer::tool_image(name, &out, &ctx.db.dir()) {
                    (Some(path), text) => match crate::computer::take_image(&path) {
                        Some(data) => json!({"content":[{"type":"image","data":data,"mimeType":"image/jpeg"},{"type":"text","text":text}]}),
                        None => json!({"content":[{"type":"text","text":text}]}),
                    },
                    (None, text) => json!({"content":[{"type":"text","text":text}]}),
                },
                Err(e) => json!({"content":[{"type":"text","text":format!("error: {e}")}],"isError":true}),
            }
        }
        _ => return Some(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("unknown method {method}")}})),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_server_started_mid_run_remembers_the_run_read_outside_content() {
        let p = std::env::temp_dir().join(format!("mimir-mcpt-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = crate::db::Db::open(&p).unwrap();
        let run = db.run_start(1, 0, "message", None, "test").unwrap();
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let args = a(&["mcp", "--bot", "1", "--chat", "0", "--run", &run.to_string()]);
        assert!(!super::ctx_from_args(db.clone(), &args).unwrap().is_tainted());
        db.run_taint(run).unwrap();
        assert!(super::ctx_from_args(db.clone(), &args).unwrap().is_tainted(), "no --tainted flag, but the run says so");
        assert!(super::ctx_from_args(db, &a(&["mcp", "--from-env"])).is_err(), "missing MIMIR_MCP_ARGS fails loudly");
    }

    #[test]
    fn grok_args_come_from_the_environment_only_when_asked() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let env = r#"["mcp","--bot","3","--chat","9","--tainted"]"#;
        assert_eq!(super::run_args(&a(&["mcp", "--from-env"]), Some(env)), Some(a(&["mcp", "--bot", "3", "--chat", "9", "--tainted"])));
        assert_eq!(super::run_args(&a(&["mcp", "--chat", "1"]), Some(env)), None, "a Claude/Codex server ignores a leftover variable");
        assert_eq!(super::run_args(&a(&["mcp", "--from-env"]), None), None);
    }
}
