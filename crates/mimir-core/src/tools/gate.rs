//! Policy: which tools a bot may use here, and which calls ask the user first (approvals, plans, loops, parked calls).

use super::*;

/// The bot's tools that ask the user first under their rules (not counting per-site rules or outside content),
/// so it knows when a plan saves the person several asks.
pub fn asking_tools(ctx: &ToolCtx) -> Vec<String> {
    specs_for(ctx).into_iter().map(|s| s.0).filter(|n| n != "plan_propose" && gate(&ctx.db, n, None, false).is_ok_and(|g| g.0 == "ask")).collect()
}

/// Tools that only read: built-in READ_ONLY, or external tools their server marks read-only.
pub(super) fn is_read_only(db: &Db, tool: &str) -> bool {
    READ_ONLY.contains(&tool) || tool == "computer_screenshot" || crate::ext::find(db, tool).is_some_and(|t| t.read_only)
}

/// Mode limits on top of the bot's allowlist.
/// What a bot may do in a guest's chat: their own memory, routines, files and skills, the web, and calendar links.
/// Nothing of yours (email, calendar, meetings, saved logins and the browser, tool servers, the computer), and no new
/// bots in your app.
pub(super) const GUEST_TOOLS: [&str; 25] = [
    "memory_store", "memory_search", "memory_forget", "chat_search", "routine_create", "routine_update", "routine_list", "routine_delete", "goal_set", "goal_update", "web_fetch", "web_search",
    "file_list", "file_read", "file_write", "file_delete", "skill_view", "skill_save", "calendar_add", "plan_propose", "ask_bot", "ask_choice", "send_file", "card_read", "card_write",
];

pub(super) fn usable(ctx: &ToolCtx, tool: &str) -> bool {
    !ctx.tools_off
        && (GUEST_TOOLS.contains(&tool) || !ctx.db.chat_guest(ctx.chat_id))
        // The computer only where it's switched on and can run (a self-hosted Linux server); never incognito
        // (its browser keeps history and cookies), and like email only in a chat with you alone.
        && (!tool.starts_with("computer_") || (crate::computer::usable(&ctx.db) && !ctx.incognito() && ctx.db.chat_private(ctx.chat_id)))
        && (!ctx.read_only || is_read_only(&ctx.db, tool))
        && (tool != "ask_bot" || ctx.depth == 0)
        // A goal is the chat's: kept by the bot the person talks to, never incognito (nothing there lasts).
        && (!tool.starts_with("goal_") || (ctx.depth == 0 && !ctx.incognito()))
        // A file goes with the reply to the person: not from a task handed over by another bot, nor incognito (it's kept
        // on disk to show in the chat).
        && (tool != "send_file" || (ctx.depth == 0 && !ctx.incognito()))
        // Bots are made only in a conversation with the person (not by a bot working on another's task, nor incognito,
        // where nothing it makes may last).
        && (tool != "bot_create" || (ctx.depth == 0 && !ctx.incognito()))
        // Buttons need someone to tap them in a messaging app: not a routine, not another bot's task, not the app's chat.
        && (tool != "ask_choice" || (!ctx.unattended && ctx.depth == 0 && ctx.db.chat_address(ctx.chat_id).ok().flatten().is_some()))
        // Meetings only once there are some (this computer's, or the server's).
        // Meetings are private to you: not in a group, channel or server, where the reply goes to everyone there.
        && (!tool.starts_with("meeting_") || (ctx.db.has_meetings() && ctx.db.chat_private(ctx.chat_id)))
        // Your email, once an account is set up, and only in a conversation with you alone.
        && (!tool.starts_with("email_") || (crate::email::account(&ctx.db).is_some() && ctx.db.chat_private(ctx.chat_id)))
        // The workspace keeps files on disk: not in incognito.
        && !(ctx.incognito() && tool.starts_with("file_"))
        // Web search once a search service is set up.
        && (tool != "web_search" || search_service(&ctx.db).is_some())
        // Your calendar, once one is linked, and like meetings only in a conversation with you alone.
        && (tool != "calendar_list" || (crate::host::secret_get(crate::calendar::SECRET).is_some() && ctx.db.chat_private(ctx.chat_id)))
        // Adding to it, once one is connected for that (CalDAV), likewise.
        && (tool != "calendar_create" || (crate::caldav::account(&ctx.db).is_some() && ctx.db.chat_private(ctx.chat_id)))
        // The browser keeps a profile (history, cookies) on disk, so it's out too.
        && !(ctx.incognito() && (tool.starts_with("memory_") || tool.starts_with("meeting_") || tool.starts_with("email_") || tool.starts_with("calendar_create") || tool == "calendar_list" || tool == "chat_search" || tool == "skill_save" || tool.starts_with("routine_") || tool.starts_with("card_") || tool == "ask_bot" || tool.starts_with("browser__")))
}

/// Tools that only read; they never need approval, even after untrusted content.
pub const READ_ONLY: [&str; 15] = [
    "memory_search", "chat_search", "meeting_search", "meeting_read", "calendar_list", "calendar_add", "email_search", "email_read", "file_list", "file_read",
    "routine_list", "card_read", "skill_view", "web_fetch", "web_search",
];

/// Built-in rule when the user hasn't set one: destructive actions ask, everything else is allowed.
pub fn default_action(tool: &str) -> &'static str {
    match tool {
        // Opening a site asks, like the browser's (with "always on this site"); looking, clicking and typing don't.
        "memory_forget" | "routine_delete" | "computer_open" | "bot_create" | "file_delete" | "email_send" | "calendar_create" => "ask",
        _ => "allow",
    }
}

/// Default for any tool, external ones included.
pub fn default_for(db: &Db, tool: &str) -> &'static str {
    match crate::ext::find(db, tool) {
        Some(t) => crate::ext::default_action(tool, t.read_only),
        None => default_action(tool),
    }
}

/// allow | ask | deny for this call. Untrusted content bumps "allow" to "ask" for anything that isn't
/// read-only, except the browser's own actions (reading pages is what the browser is for; its
/// navigation and page scripting are gated separately).
pub fn decide(db: &Db, tool: &str, tainted: bool) -> Result<String> {
    Ok(gate(db, tool, None, tainted)?.0)
}

/// The decision plus why an "ask" asks: `rule` (the rules say so) or `guarded` (allowed, but this
/// conversation read untrusted content). A per-site rule beats the tool's rule; guarding still applies.
pub(super) fn gate(db: &Db, tool: &str, site: Option<&str>, tainted: bool) -> Result<(String, &'static str)> {
    let by_site = match site {
        Some(h) => db.site_rule(tool, h)?,
        None => None,
    };
    let action = match by_site {
        Some(a) => a,
        None => db.rule(tool)?.unwrap_or_else(|| default_for(db, tool).to_string()),
    };
    // "Always" pressed on a card that asked after outside content (or for another reason): never asks, not even then.
    if action == "always" {
        return Ok(("allow".into(), "rule"));
    }
    // The browser's and the computer's own actions read pages by nature: they're gated per site, not guarded.
    let browser = crate::ext::split(tool).is_some_and(|(s, _)| s == crate::ext::BROWSER) || tool.starts_with("computer_");
    // plan_propose only ever asks (and refuses on its own after outside content), so asking first would be a double ask.
    // ask_choice only shows the person buttons; what they tap is theirs to pick. send_file goes to them, like the reply.
    // An ask after outside content is guarded too, so it never offers "always" (a page could be asking).
    let exempt = is_read_only(db, tool) || matches!(tool, "plan_propose" | "skill_save" | "ask_choice" | "send_file");
    Ok(if tainted && !exempt && ((action == "allow" && !browser) || action == "ask") { ("ask".into(), "guarded") } else { (action, "rule") })
}

/// After outside content, a few calls that only look like reading can carry your data out; those ask:
/// - a web address with data in it (a query string, a long path or subdomain, a user name): `?d=<your memories>`;
/// - on the bots' computer, keys that leave the page for the address bar or a new tab, or a click on the browser's
///   own bar (then typing an address would open any site without the "open a site" ask);
/// - typing something from your memory into a page (the computer's or the browser's).
// ponytail: heuristics; a model can still spell data into a short path, one fetch at a time (each one is logged)
pub(super) fn leaks_out(ctx: &ToolCtx, name: &str, args: &Value) -> bool {
    match name {
        // After reading your email or a meeting, any address could carry a code or a name out: ask.
        "web_fetch" | "computer_open" | "browser__browser_navigate" => ctx.saw_private() || url_carries_data(args["url"].as_str().unwrap_or("")),
        // A query goes to the search service: one long enough to carry text, or holding a memory, could be your data.
        "web_search" => {
            let q = args["query"].as_str().unwrap_or("");
            q.chars().count() > 200 || types_memory(ctx, q)
        }
        "computer_key" => args["keys"].as_str().unwrap_or("").split_whitespace().any(|k| {
            // xdotool takes several spellings of the same combo ("control+L", "Ctrl_L+l").
            let mut parts: Vec<String> = k
                .to_ascii_lowercase()
                .split('+')
                .map(|p| match p.trim_end_matches("_l").trim_end_matches("_r") {
                    "control" | "ctl" => "ctrl".to_string(),
                    "meta" | "cmd" | "command" => "super".to_string(),
                    p => p.to_string(),
                })
                .collect();
            let key = parts.pop().unwrap_or_default();
            parts.sort();
            let combo = if parts.is_empty() { key } else { format!("{}+{key}", parts.join("+")) };
            ["ctrl+l", "alt+d", "f6", "ctrl+t", "ctrl+n", "ctrl+shift+n", "ctrl+k", "ctrl+e", "ctrl+o", "super+l"].contains(&combo.as_str())
        }),
        // The browser's tab strip and address bar are along the top of the screen.
        "computer_click" => args["y"].as_i64().is_some_and(|y| y < 90),
        "computer_type" => types_memory(ctx, args["text"].as_str().unwrap_or("")),
        n if crate::ext::split(n).is_some_and(|(s, _)| s == crate::ext::BROWSER) => {
            let typed: Vec<String> = match n.rsplit("__").next().unwrap_or("") {
                "browser_type" => vec![args["text"].as_str().unwrap_or("").to_string()],
                "browser_fill_form" => args["fields"].as_array().into_iter().flatten().filter_map(|f| f["value"].as_str().map(String::from)).collect(),
                _ => vec![],
            };
            typed.iter().any(|t| types_memory(ctx, t))
        }
        // A tool server's tool that only reads but reaches the outside (a search, a lookup) can carry data out in what
        // it's asked: long text, an address with data in it, or one of your memories.
        n if crate::ext::find(&ctx.db, n).is_some_and(|t| t.read_only && t.open_world) => {
            let mut all = vec![];
            crate::ext::strings(args, &mut all);
            ctx.saw_private() || all.iter().any(|s| s.chars().count() > 200 || (s.starts_with("http") && url_carries_data(s)) || types_memory(ctx, s))
        }
        _ => false,
    }
}

/// A URL that could be carrying data rather than just naming a page.
pub(super) fn url_carries_data(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return true };
    let host = u.host_str().unwrap_or("");
    let labels: Vec<&str> = host.split('.').collect();
    // Tracking tags on a link someone shared aren't your data.
    let query = u.query_pairs().filter(|(k, _)| !(k.starts_with("utm_") || k == "fbclid" || k == "gclid")).count();
    query > 0
        || !u.username().is_empty()
        || u.password().is_some()
        || host.len() > 60
        || u.path().len() > 80
        // A long id (a shared doc's) is fine; a segment long enough to carry text isn't.
        || u.path_segments().into_iter().flatten().any(|s| s.len() > 64)
        || labels.iter().rev().skip(2).any(|l| l.len() > 24)
}

/// Text to be typed that holds one of your memories (or most of one).
fn types_memory(ctx: &ToolCtx, text: &str) -> bool {
    let t = text.to_lowercase();
    if t.trim().len() < 12 {
        return false;
    }
    ctx.db.entries(&ctx.scope()).unwrap_or_default().iter().filter(|e| e.status == "active").any(|e| {
        let b = e.body.to_lowercase();
        (b.len() >= 12 && t.contains(&b)) || (t.len() >= 16 && b.contains(t.trim()))
    })
}

/// The request-match check (setting ask_check=on): a send, delete or edit whose target isn't in what the person asked
/// (their own words) or the routine's prompt. No brief (a task from another bot, words not theirs): no check, as before.
/// The browser and the computer act step by step on pages, gated per site; they aren't checked.
pub(super) fn off_brief(ctx: &ToolCtx, name: &str, args: &Value) -> bool {
    if ctx.db.setting("ask_check").ok().flatten().as_deref() != Some("on") {
        return false;
    }
    let external = crate::ext::split(name).is_some_and(|(s, _)| s != crate::ext::BROWSER) && !is_read_only(&ctx.db, name);
    if !external && !["email_send", "calendar_create", "memory_forget", "routine_delete", "file_delete"].contains(&name) {
        return false;
    }
    let Some(brief) = ctx.run_id.and_then(|r| ctx.db.brief(r).ok().flatten()).filter(|b| crate::own_words(b)) else { return false };
    let id = args["id"].as_i64().unwrap_or(0);
    // What it acts on: who an email goes to (not its text, which the bot writes from the request), which memory or
    // routine, which file; for a tool server's tool, everything it's given.
    let target = match name {
        "email_send" => format!("{} {}", args["to"].as_str().unwrap_or(""), args["cc"].as_str().unwrap_or("")),
        "memory_forget" => ctx.db.entries(&ctx.scope()).ok().and_then(|v| v.into_iter().find(|e| e.id == id)).map(|e| format!("{id} {} {}", e.title, e.body)).unwrap_or(id.to_string()),
        "routine_delete" => ctx.db.routine(id).ok().flatten().map(|r| format!("{id} {}", r.prompt)).unwrap_or(id.to_string()),
        _ => {
            let mut all = vec![];
            crate::ext::strings(args, &mut all);
            all.join(" ")
        }
    };
    !crate::fits_ask(&brief, &target)
}

/// What the person marked sensitive (Settings → Sensitive sources): tool servers by name, single tools
/// (`bank__balance`), memory scopes (`chat:7`). Never set by Mimir itself.
pub fn sensitive_list(db: &Db) -> Vec<String> {
    db.setting("sensitive").ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

/// Whether this server, tool or memory scope is marked sensitive (a tool also through its server).
pub(crate) fn is_sensitive(db: &Db, name: &str) -> bool {
    let list = sensitive_list(db);
    list.iter().any(|s| s == name || crate::ext::split(name).is_some_and(|(server, _)| server == s))
}

/// A server whose name looks like banking, money or health: suggested (never set) as sensitive.
pub fn looks_sensitive(name: &str) -> bool {
    const WORDS: [&str; 22] = [
        "bank", "finance", "money", "broker", "invest", "trading", "stripe", "paypal", "plaid", "wise", "revolut", "monzo", "ynab", "tax", "health", "medical",
        "doctor", "patient", "pharma", "fitbit", "oura", "clinic",
    ];
    let n = name.to_ascii_lowercase();
    WORDS.iter().any(|w| n.contains(w))
}

/// After a run read a sensitive source, these are the ways data leaves: an email, a web request (fetch, search, the
/// computer's browser), a hand-off to another bot, a message to another chat, a memory the household's guests read,
/// and any tool server's write (or its
/// read that reaches the internet, carrying what it's asked).
pub(super) fn sends_out(ctx: &ToolCtx, name: &str, args: &Value) -> bool {
    match name {
        // A sign-in types a login into a site, like typing does.
        "email_send" | "web_fetch" | "web_search" | "ask_bot" | "computer_open" | "computer_type" | "computer_signin" => true,
        "routine_create" => args["for"].as_str().is_some_and(|f| !f.trim().is_empty()),
        "memory_store" => args["household"].as_bool() == Some(true),
        "send_file" => ctx.db.chat_guest(ctx.chat_id) || !ctx.db.chat_private(ctx.chat_id),
        n => crate::ext::find(&ctx.db, n).is_some_and(|t| {
            // Reading the sensitive server again, or the browser looking at its page, sends nothing new out.
            let server = crate::ext::split(n).map(|(s, _)| s).unwrap_or("");
            !t.read_only || (t.open_world && server != crate::ext::BROWSER && !is_sensitive(&ctx.db, n))
        }),
    }
}

/// The setting that says a conversation read the person's private data (cleared with its taint).
pub(crate) fn private_key(bot: i64, conv: i64) -> String {
    format!("private:{bot}:{conv}")
}

/// The setting that says a routine was made or rewritten after reading a sensitive source (removed with it).
pub(crate) fn routine_private_key(id: i64) -> String {
    format!("routine-private:{id}")
}

/// The setting that says a hand-off (or its helper) read the person's private data: for the helper running later, and
/// the turn that reports back.
pub(crate) fn delegation_private_key(id: i64) -> String {
    format!("private-delegation:{id}")
}

/// Set on a task handed over with `later` and `in_own_chat`: the helper does it in its own chat.
pub(crate) fn delegation_own_key(id: i64) -> String {
    format!("own-delegation:{id}")
}

/// The website a call acts on, for per-site rules ("always allow opening github.com").
pub(super) fn site_of(name: &str, args: &Value) -> Option<String> {
    if name != "web_fetch" && name != "browser__browser_navigate" && name != "computer_open" {
        return None;
    }
    // Parsed like the browser will, so "github.com:x@evil.example" is evil.example; one with a user name or password
    // gets no site (no per-site "always" can cover it).
    let u = url::Url::parse(args.get("url")?.as_str()?).ok()?;
    if !u.username().is_empty() || u.password().is_some() {
        return None;
    }
    let host = u.host_str()?.to_lowercase();
    let host = host.trim_start_matches("www.").to_string();
    (!host.is_empty()).then_some(host)
}

/// Human-readable one-liner for an approval card.
pub(super) fn describe(ctx: &ToolCtx, name: &str, args: &Value) -> String {
    let id = args.get("id").and_then(Value::as_i64).unwrap_or(0);
    match name {
        "memory_forget" => {
            let title = ctx.db.entries(&ctx.scope()).ok().and_then(|v| v.into_iter().find(|e| e.id == id)).map(|e| e.title).unwrap_or_default();
            format!("Forget memory #{id} \"{title}\"")
        }
        "routine_delete" => {
            let r = ctx.db.routines(ctx.chat_id).ok().and_then(|v| v.into_iter().find(|r| r.id == id));
            format!("Delete routine #{id}{}", r.map(|r| format!(" ({}: {})", r.cron, r.prompt)).unwrap_or_default())
        }
        "memory_store" => format!("Remember \"{}\": {}", args["title"].as_str().unwrap_or(""), args["body"].as_str().unwrap_or("")),
        "routine_update" => format!(
            "Change routine #{id}:{}{}",
            args["cron"].as_str().map(|c| format!(" [{c}]")).unwrap_or_default(),
            args["prompt"].as_str().map(|p| format!(" {p}")).unwrap_or_default()
        ),
        "goal_set" => format!("Set the goal: {} (done when {})", args["goal"].as_str().unwrap_or(""), args["done_when"].as_str().unwrap_or("")),
        // Done takes its follow-ups with it: the card says how many.
        "goal_update" if args["status"].as_str() == Some("done") => {
            let n = ctx.db.goal_routines(ctx.chat_id).map(|v| v.len()).unwrap_or(0);
            let gone = match n {
                0 => String::new(),
                1 => "; this removes 1 follow-up".into(),
                n => format!("; this removes {n} follow-ups"),
            };
            format!("Mark the goal done{gone}: {}", args["progress"].as_str().unwrap_or(""))
        }
        "goal_update" => format!("Goal{}: {}", args["status"].as_str().map(|s| format!(" {s}")).unwrap_or_default(), args["progress"].as_str().unwrap_or("")),
        "routine_create" => {
            let to = args["for"].as_str().filter(|f| !f.trim().is_empty()).map(|f| format!(" in {f}'s own chat")).unwrap_or_default();
            match args["at"].as_str().filter(|a| !a.trim().is_empty()) {
                Some(at) if args["followup"].as_bool() == Some(true) => format!("Follow-up at {at}{to}: {}", args["prompt"].as_str().unwrap_or("")),
                Some(at) => format!("Once at {at}{to}: {}", args["prompt"].as_str().unwrap_or("")),
                None => format!(
                    "Create routine [{}]{to}{} {}",
                    args["cron"].as_str().unwrap_or(""),
                    args["until"].as_str().filter(|u| !u.trim().is_empty()).map(|u| format!(" until {u}")).unwrap_or_default(),
                    args["prompt"].as_str().unwrap_or("")
                ),
            }
        }
        "memory_search" => format!("Search memory: {}", args["query"].as_str().unwrap_or("")),
        "web_fetch" => format!("Read {}", args["url"].as_str().unwrap_or("")),
        "web_search" => format!("Search the web: {}", args["query"].as_str().unwrap_or("")),
        "email_search" => format!("Look through your email{}", args["query"].as_str().or(args["from"].as_str()).map(|q| format!(": {q}")).unwrap_or_default()),
        "email_read" => format!("Read email #{}", args["id"]),
        "file_list" => "List the workspace's files".into(),
        "file_read" => format!("Read {}", args["path"].as_str().unwrap_or("?")),
        "file_write" => format!("{} {}: {}", match args["mode"].as_str() { Some("append") => "Add to", Some("replace") => "Rewrite", _ => "Create" }, args["path"].as_str().unwrap_or("?"), truncate(args["content"].as_str().unwrap_or(""), 300)),
        "file_delete" => format!("Delete the file {}", args["path"].as_str().unwrap_or("?")),
        "send_file" => match args["path"].as_str().filter(|_| args["screenshot"].as_bool() != Some(true)) {
            Some(p) => format!("Send the file {p} to this chat"),
            None => "Send a screenshot of the computer's screen to this chat".into(),
        },
        // Who it's to first, then the text (an email to send, all of it: what you approve is what goes).
        "email_send" | "email_draft" => describe_email(name, args),
        "chat_search" => format!("Search earlier chats: {}", args["query"].as_str().unwrap_or("")),
        "meeting_search" => match args["query"].as_str() {
            Some(q) if !q.trim().is_empty() => format!("Search meetings: {q}"),
            _ => "List recent meetings".into(),
        },
        "meeting_read" => format!("Read meeting #{}", args["id"]),
        "calendar_list" => format!("Look at your calendar{}", args["from"].as_str().map(|f| format!(" from {f}")).unwrap_or_default()),
        "calendar_add" => format!("Calendar link: {} at {}", args["title"].as_str().unwrap_or("?"), args["start"].as_str().unwrap_or("?")),
        // When first, then what and where: the whole event is on the card.
        "calendar_create" => format!(
            "Add to {}: {}, \"{}\"{}{}",
            crate::caldav::account(&ctx.db).map(|c| c.name).unwrap_or_else(|| "your calendar".into()),
            event_when(args["start"].as_str().unwrap_or(""), args["minutes"].as_i64().unwrap_or(60)).map(|w| w.1).unwrap_or_else(|_| args["start"].as_str().unwrap_or("?").to_string()),
            args["title"].as_str().unwrap_or("?"),
            args["location"].as_str().filter(|l| !l.trim().is_empty()).map(|l| format!(" at {l}")).unwrap_or_default(),
            args["notes"].as_str().filter(|n| !n.trim().is_empty()).map(|n| format!(" — {n}")).unwrap_or_default()
        ),
        "routine_list" => "List routines".into(),
        "card_read" => "Read the pinned cards".into(),
        "card_write" => format!("Update the card \"{}\"", args["title"].as_str().unwrap_or("")),
        "ask_choice" => format!("Ask \"{}\" with buttons", truncate(args["question"].as_str().unwrap_or(""), 120)),
        "skill_view" => format!("Read skill \"{}\"", args["name"].as_str().unwrap_or("")),
        "skill_save" => format!("Save as a skill: {}", args["title"].as_str().unwrap_or("")),
        "computer_screenshot" => "Look at the computer's screen".into(),
        "computer_handoff" => format!("Ask you to take over the computer: {}", args["reason"].as_str().unwrap_or("")),
        "computer_signin" => "Sign in to the site on the computer".into(),
        "computer_click" => format!("Click the computer at {},{}", args["x"], args["y"]),
        "computer_type" => format!("Type on the computer: {}", args["text"].as_str().unwrap_or("")),
        "computer_key" => format!("Press {} on the computer", args["keys"].as_str().unwrap_or("")),
        "computer_scroll" => format!("Scroll {} on the computer", args["direction"].as_str().unwrap_or("")),
        "computer_open" => format!("Open {} on the computer", args["url"].as_str().unwrap_or("")),
        "ask_bot" => format!("Ask {}: {}", args["bot"].as_str().unwrap_or("?"), args["task"].as_str().unwrap_or("")),
        // The whole role: it becomes the new bot's standing instructions, so the card must show what you approve.
        "bot_create" => format!(
            "Create a bot \"{}\" with this role: {} (tools: {})",
            args["name"].as_str().unwrap_or("?"),
            args["role"].as_str().unwrap_or(""),
            match args["tools"].as_array() {
                Some(t) if !t.is_empty() => t.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
                _ => "the same as mine".into(),
            }
        ),
        _ => describe_external(name, args).unwrap_or_else(|| format!("{name} {args}")),
    }
}

/// An email on its card: who it's to first, never cut short (card_fields' rule), then the subject and text. An email to
/// send shows all of it; a draft, the start.
pub fn describe_email(name: &str, args: &Value) -> String {
    let body = args["body"].as_str().unwrap_or("");
    format!(
        "{} an email to {}{}{}: \"{}\" — {}",
        if name == "email_send" { "Send" } else { "Draft" },
        crate::email::addresses_shown(args["to"].as_str().unwrap_or("?")),
        args["cc"].as_str().filter(|c| !c.trim().is_empty()).map(|c| format!(" (cc {})", crate::email::addresses_shown(c))).unwrap_or_default(),
        args["account"].as_str().filter(|a| !a.trim().is_empty()).map(|a| format!(" from {a}")).unwrap_or_default(),
        args["subject"].as_str().unwrap_or(""),
        if name == "email_send" { body.to_string() } else { truncate(body, 400) }
    )
}

/// The checks on an email's text, before anyone is asked and again after you edit it: a one-line subject, and an email
/// to send whole on the card.
fn email_fits(name: &str, args: &Value) -> Result<()> {
    crate::email::header_text(args["subject"].as_str().unwrap_or(""))?;
    if name == "email_send" && describe_email(name, args).chars().count() > 1150 {
        bail!("too long to show whole on the approval card: save it with email_draft for the user to send");
    }
    if args["body"].as_str().map_or(0, |b| b.chars().count()) > 20_000 {
        bail!("an email body can be up to 20,000 characters");
    }
    Ok(())
}

/// An external tool call in words: "notes: Edit file (path: /x.md; edits: […])".
pub(super) fn describe_external(name: &str, args: &Value) -> Option<String> {
    let (server, tool) = crate::ext::split(name)?;
    let mut words = tool.trim_start_matches("browser_").split('_').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ").chars().collect::<Vec<_>>();
    if let Some(c) = words.first_mut() {
        *c = c.to_ascii_uppercase();
    }
    let what: String = words.into_iter().collect();
    let args = card_fields(args);
    Some(if args.is_empty() { format!("{server}: {what}") } else { format!("{server}: {what} ({args})") })
}

/// Arguments in words, for a card: where it goes first (at any depth) and never cut short; other values cut on their
/// own, so a long body can't push the recipient off the card.
fn card_fields(v: &Value) -> String {
    const WHERE: [&str; 13] = ["to", "recipient", "recipients", "email", "cc", "bcc", "url", "address", "channel", "path", "repo", "account", "chat_id"];
    fn is_where(k: &str) -> bool {
        WHERE.contains(&k.to_ascii_lowercase().as_str())
    }
    // Whether a destination is somewhere inside: such a field comes before the others.
    fn has_where(v: &Value) -> bool {
        match v {
            Value::Object(m) => m.iter().any(|(k, v)| is_where(k) || has_where(v)),
            Value::Array(a) => a.iter().any(has_where),
            _ => false,
        }
    }
    let Some(m) = v.as_object() else { return String::new() };
    let mut fields: Vec<(&String, &Value)> = m.iter().collect();
    fields.sort_by_key(|(k, v)| !is_where(k) && !has_where(v));
    fields
        .iter()
        .map(|(k, v)| {
            let shown = match v {
                Value::Object(_) => format!("{{{}}}", card_fields(v)),
                Value::Array(a) if has_where(v) => format!("[{}]", a.iter().map(|i| if i.is_object() { format!("{{{}}}", card_fields(i)) } else { truncate(&i.to_string(), 120) }).collect::<Vec<_>>().join(", ")),
                Value::String(s) if WHERE.contains(&k.to_ascii_lowercase().as_str()) => truncate(s, 300),
                Value::String(s) => truncate(s, 120),
                other if WHERE.contains(&k.to_ascii_lowercase().as_str()) => truncate(&other.to_string(), 300),
                other => truncate(&other.to_string(), 120),
            };
            format!("{k}: {shown}")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// An approved plan's actions for this run: (tool, site). A site-less entry covers every call of that tool.
pub(super) fn plan_of(ctx: &ToolCtx) -> Vec<(String, Option<String>)> {
    let Some(run) = ctx.run_id else { return vec![] };
    ctx.db.setting(&format!("plan:{run}")).ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

/// Does the approved plan cover this call?
pub(super) fn planned(plan: &[(String, Option<String>)], name: &str, site: Option<&str>) -> bool {
    plan.iter().any(|(t, s)| t == name && (s.is_none() || s.as_deref() == site))
}

/// plan_propose: one approval for a multi-step job. Only tools that exist for this bot are listed; the card shows
/// the steps and what will run, and approving stores the plan for this run (cleared when the run ends).
pub(super) async fn propose_plan(ctx: &ToolCtx, args: &Value) -> Result<String> {
    if ctx.unattended {
        bail!("plans need the person there to approve them; in a routine, just do the steps (each ask waits in their inbox).");
    }
    let run = ctx.run_id.context("no run to attach the plan to")?;
    if ctx.is_tainted() {
        bail!("this conversation read outside content, so each action asks on its own; just go ahead step by step.");
    }
    // One line per step: a step can't add lines that look like more of the card.
    let steps: Vec<String> = args["steps"].as_array().into_iter().flatten().filter_map(|s| s.as_str()).map(|s| truncate(&s.split_whitespace().collect::<Vec<_>>().join(" "), 200)).take(12).collect();
    let bot = ctx.db.bot(ctx.bot_id)?;
    let mut actions: Vec<(String, Option<String>)> = vec![];
    for a in args["actions"].as_array().into_iter().flatten() {
        // Claude and Codex know our tools by their MCP names (mcp__mimir__memory_forget); grok as mimir__….
        let Some(tool) = a["tool"].as_str().map(|t| t.trim_start_matches("mcp__").trim_start_matches("mimir__")) else { continue };
        let exists = specs().iter().any(|(n, ..)| *n == tool) || crate::ext::find(&ctx.db, tool).is_some();
        if !exists || tool == "plan_propose" || !usable(ctx, tool) || !bot.allows(tool) {
            continue;
        }
        let site = a["site"].as_str().map(|s| s.trim().trim_start_matches("www.").to_lowercase()).filter(|s| !s.is_empty());
        if !actions.contains(&(tool.to_string(), site.clone())) {
            actions.push((tool.to_string(), site));
        }
    }
    if steps.is_empty() || actions.is_empty() {
        bail!("a plan needs its steps and at least one of your tools in actions (by name, e.g. memory_forget or browser__browser_navigate)");
    }
    let uses: Vec<String> = actions.iter().map(|(t, s)| format!("{}{}", describe_tool(t), s.as_ref().map(|s| format!(" on {s}")).unwrap_or_default())).collect();
    // What it pre-approves comes first: long steps can't push it off the card.
    let summary = format!(
        "Uses: {}\nPlan:\n{}",
        uses.join(", "),
        steps.iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect::<Vec<_>>().join("\n")
    );
    if !trusted(&ctx.db, "plan_propose", None) {
        wait_for_approval_with(ctx, "plan_propose", args, &summary, "plan", None).await?;
    }
    // Add to what's already allowed in this reply ("all like this" earlier), don't replace it.
    let mut plan = plan_of(ctx);
    plan.extend(actions.into_iter().filter(|a| !plan_of(ctx).contains(a)));
    ctx.db.set_setting(&format!("plan:{run}"), &serde_json::to_string(&plan)?)?;
    Ok("The user approved the plan. Go ahead: those actions won't ask again during this reply.".into())
}

/// Making a bot and sending an email ask every time, even with an "allow" rule set by hand, as their own kinds (so never
/// "always" or "all like this"); after outside content they're marked as such. Other tools keep their decision.
pub(super) fn always_ask(name: &str, decision: String, why: &'static str, tainted: bool) -> (String, &'static str) {
    if !matches!(name, "bot_create" | "email_send") || decision == "deny" {
        return (decision, why);
    }
    let kind = if tainted { "guarded" } else if name == "bot_create" { "bot" } else { "send" };
    ("ask".into(), kind)
}

pub(super) fn precheck(ctx: &ToolCtx, name: &str, args: &Value) -> Result<()> {
    let len = |k: &str| args[k].as_str().map_or(0, |s| s.chars().count());
    match name {
        // Memories are short facts; a long one would also outgrow its approval card.
        "memory_store" if len("title") > 120 || len("body") > 1000 => {
            bail!("too long for a memory (title up to 120 characters, body up to 1000): keep it to the fact, or split it")
        }
        "computer_handoff" if args["reason"].as_str().is_none_or(|r| r.trim().is_empty()) || len("reason") > 300 => {
            bail!("say briefly what the person needs to do (up to 300 characters)")
        }
        "routine_update" => {
            if len("prompt") > 1000 || len("cron") > 100 {
                bail!("a routine's prompt can be up to 1000 characters: say what to do, briefly");
            }
            if args["prompt"].as_str().is_some_and(|p| p.trim().is_empty()) {
                bail!("a routine needs a prompt");
            }
            // Bots never change a schedule on their own: in a routine's run (nobody there), its times and end stay.
            if ctx.unattended && (args["cron"].is_string() || args["until"].is_string()) {
                bail!("a routine can't change when it runs or ends on its own; say what you'd change in your report and the person can");
            }
            if let Some(cron) = args["cron"].as_str()
                && crate::db::min_gap_minutes(cron)? < 15
            {
                bail!("routines a bot sets run at most every 15 minutes; pick a slower schedule");
            }
            if let Some(cron) = args["cron"].as_str() {
                calendar_linked(ctx, cron)?;
            }
            if let Some(w) = args["watch"].as_str() {
                crate::watch::valid(w)?;
            }
            if let Some(u) = args["until"].as_str().filter(|u| !u.trim().is_empty()) {
                until_time(u)?;
            }
        }
        "routine_create" => {
            let mut chat = ctx.chat_id;
            if args["for"].as_str().is_some_and(|f| !f.trim().is_empty()) {
                // Only you send things to other people's chats: a guest (or a family group) can't.
                if ctx.db.chat_guest(ctx.chat_id) {
                    bail!("only the person who runs you can set reminders for others");
                }
                // A follow-up is the bot checking back in this chat, not a reminder for someone else.
                if args["followup"].as_bool() == Some(true) {
                    bail!("a follow-up is for this chat; to remind someone else, leave out followup");
                }
                chat = remind_target(ctx, args)?.map_or(chat, |t| t.0);
            }
            if len("prompt") > 1000 || len("cron") > 100 {
                bail!("a routine's prompt can be up to 1000 characters: say what to do, briefly");
            }
            // A bot can't set up something that burns through the person's AI account: at most every 15 minutes, and
            // a limited number per chat. (Routines you make yourself in the app aren't limited.)
            match args["at"].as_str().filter(|a| !a.trim().is_empty()) {
                Some(at) => {
                    let t = local_time(at)?;
                    if t <= crate::db::now() || t > crate::db::now() + 366 * 86400 {
                        bail!("'at' must be in the future, within a year");
                    }
                    // A run booking the next one-off would otherwise repeat every minute.
                    if ctx.unattended && t < crate::db::now() + 15 * 60 {
                        bail!("from a routine, a one-off runs at least 15 minutes from now");
                    }
                }
                None if crate::db::min_gap_minutes(s(args, "cron").map_err(|_| anyhow::anyhow!("give cron for a recurring routine, or at for a one-off"))?)? < 15 => {
                    bail!("routines a bot creates run at most every 15 minutes; pick a slower schedule")
                }
                None => calendar_linked(ctx, s(args, "cron")?)?,
            }
            if let Some(w) = args["watch"].as_str().filter(|w| !w.trim().is_empty()) {
                crate::watch::valid(w)?;
                if !args["quiet"].as_bool().unwrap_or(false) {
                    bail!("watch is for quiet checks: set quiet=true too");
                }
            }
            let at = args["at"].as_str().is_some_and(|a| !a.trim().is_empty());
            if args["followup"].as_bool() == Some(true) && !at {
                bail!("a follow-up is a one-off: give at, when you'll check back");
            }
            if let Some(u) = args["until"].as_str().filter(|u| !u.trim().is_empty()) {
                if at {
                    bail!("until is for a repeating routine; a one-off runs once at its time");
                }
                if until_time(u)? <= crate::db::now() {
                    bail!("until must be in the future");
                }
            }
            if ctx.db.routines(chat)?.len() >= 25 {
                bail!("this chat already has 25 routines; ask the person to remove some first");
            }
        }
        "bot_create" => {
            new_bot(ctx, args)?;
            if describe(ctx, name, args).chars().count() > 1200 {
                bail!("too long to show whole on the approval card: shorten the role or list fewer tools");
            }
        }
        // Checked before anyone is asked: the addresses, the subject, that the whole email fits on the approval card,
        // and at most 20 sent a day from a chat (a loop, or a page talking the bot into it, can't spam from your account).
        "email_send" => {
            if crate::email::addresses(args["to"].as_str().unwrap_or(""))?.is_empty() {
                bail!("who is it to?");
            }
            crate::email::addresses(args["cc"].as_str().unwrap_or(""))?;
            email_fits(name, args)?;
            if ctx.db.sent_today(ctx.chat_id, "email_send")? >= 20 {
                bail!("20 emails were sent from this chat in the last day; draft this one instead");
            }
        }
        // Before anyone is asked: a real time, and an event short enough to be read whole on the card.
        "calendar_create" => {
            if len("title") == 0 || len("title") > 200 || len("location") > 300 || len("notes") > 800 {
                bail!("a title (up to 200 characters), a place (300) and notes (800)");
            }
            let (when, _) = event_when(s(args, "start")?, args["minutes"].as_i64().unwrap_or(60))?;
            if let crate::caldav::When::At(from, _) = when
                && from < crate::db::now() - 86400
            {
                bail!("that's in the past");
            }
        }
        "email_draft" => {
            if crate::email::addresses(args["to"].as_str().unwrap_or(""))?.is_empty() {
                bail!("who is it to?");
            }
            crate::email::addresses(args["cc"].as_str().unwrap_or(""))?;
            email_fits(name, args)?;
        }
        "ask_bot" if len("task") > 1000 => bail!("a task for another bot can be up to 1000 characters: say what you need, briefly"),
        "computer_type" if len("text") > 1000 => bail!("type at most 1000 characters at a time"),
        // Like web_fetch: the browser doesn't go to this computer or the local network (a page could ask for your router).
        "browser__browser_navigate" | "browser__browser_tabs" | "computer_open" if args["url"].as_str().is_some_and(is_private_host) => {
            bail!("refusing to open local/private addresses")
        }
        // A new tab with an address would open the page past browser_navigate's asks and checks.
        "browser__browser_tabs" if args.get("url").is_some_and(|u| !u.is_null()) => {
            bail!("open the tab without a url, then go to the page with browser_navigate")
        }
        _ => {}
    }
    Ok(())
}

/// What an approval card shows: all of it up to a size every chat app can display, and if it's longer, a plain note
/// that it was cut (memories, routine prompts and hand-offs are capped below that, so theirs is always whole).
fn card_text(summary: &str) -> String {
    const MAX: usize = 1200;
    // Characters that silently reverse or reorder the text after them show as what they are, so the card reads the
    // way it will act.
    let summary: String = summary.chars().map(|c| if matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}') { '⁇' } else { c }).collect();
    let summary = summary.as_str();
    let n = summary.chars().count();
    if n <= MAX { summary.to_string() } else { format!("{}… (+{} more characters not shown)", summary.chars().take(MAX).collect::<String>(), n - MAX) }
}

/// A card for a task handed to another bot comes from the bot the person talks to, but the helper acts: it says so.
pub(super) fn for_helper(ctx: &ToolCtx, summary: String) -> String {
    match ctx.via_bot.filter(|v| *v != ctx.bot_id) {
        Some(v) => {
            let name = |id| ctx.db.bot(id).map(|b| b.name).unwrap_or_else(|_| "Another bot".into());
            format!("{}, on a task from {}: {summary}", name(ctx.bot_id), name(v))
        }
        None => summary,
    }
}

/// A tool's name in words, for a plan card: "browser: navigate", "Forget a memory".
pub(crate) fn describe_tool(name: &str) -> String {
    match crate::ext::split(name) {
        Some((server, tool)) => format!("{server}: {}", tool.trim_start_matches("browser_").replace('_', " ")),
        // In words, like Settings → Permissions in the app (its TOOL_LABEL).
        None => match name {
        "ask_bot" => "hand tasks to other bots",
        "ask_choice" => "ask with buttons",
        "bot_create" => "make new bots",
        "calendar_add" => "give calendar links",
        "calendar_create" => "add calendar events",
        "calendar_list" => "look at your calendar",
        "card_write" => "update pinned cards",
        "chat_search" => "search earlier conversations",
        "computer_click" => "click on the computer",
        "computer_handoff" => "ask you to take over the computer",
        "computer_key" => "press keys on the computer",
        "computer_open" => "open sites on the computer",
        "computer_screenshot" => "look at the computer's screen",
        "computer_scroll" => "scroll on the computer",
        "computer_signin" => "use saved logins",
        "computer_type" => "type on the computer",
        "email_draft" => "draft emails",
        "email_read" => "read your email",
        "email_search" => "search your email",
        "email_send" => "send emails",
        "file_delete" => "delete files",
        "file_list" => "list files",
        "file_read" => "read files",
        "file_write" => "write files",
        "goal_set" => "set goals",
        "goal_update" => "update goals",
        "meeting_read" => "read a meeting transcript",
        "meeting_search" => "search your meetings",
        "memory_forget" => "forget a memory",
        "memory_search" => "look up memories",
        "memory_store" => "remember things",
        "plan_propose" => "propose plans",
        "routine_create" => "create routines",
        "routine_delete" => "delete a routine",
        "routine_list" => "list routines",
        "routine_update" => "change routines",
        "send_file" => "send files",
        "skill_save" => "save skills",
        "skill_view" => "read its skills",
        "web_fetch" => "read web pages",
        "web_search" => "search the web",
            _ => return name.replace('_', " "),
        }
        .into(),
    }
}

/// The fields of a call you may edit on its card before it goes: an email's subject and body. Never who it's to.
pub fn editable(tool: &str) -> &'static [&'static str] {
    match tool {
        "email_send" | "email_draft" => &["subject", "body"],
        _ => &[],
    }
}

/// What a card you can edit holds, as it starts out (None for a tool without any): the editable fields, and who it's to
/// and from, to show (never taken back from it).
fn draft_of(name: &str, args: &Value) -> Option<Value> {
    (!editable(name).is_empty()).then(|| ["to", "cc", "account", "subject", "body"].iter().map(|f| (f.to_string(), json!(args[*f].as_str().unwrap_or("")))).collect::<serde_json::Map<_, _>>().into())
}

/// The call with the card's edited text in it. Only the editable fields are taken, as text; everything else (who it's
/// to above all) stays as the bot asked, even if the stored draft says otherwise. Edited, it's checked again as the bot's
/// own call was.
pub fn with_draft(ctx: &ToolCtx, name: &str, args: &Value, draft: Option<&Value>) -> Result<Value> {
    let mut out = args.clone();
    for f in editable(name) {
        if let Some(v) = draft.and_then(|d| d[*f].as_str()) {
            out[*f] = json!(v);
        }
    }
    if out != *args {
        precheck(ctx, name, &out).map_err(|e| anyhow::anyhow!("your edit couldn't go as it is ({e:#}); nothing was done"))?;
    }
    Ok(out)
}

/// Your edits to a pending card, checked and kept: only its editable fields, as text, and still fitting on the card. The
/// card then says what will go.
pub fn edit_card(db: &Db, a: &crate::db::Approval, edits: &Value) -> Result<()> {
    let (Some(draft), Some(edits)) = (a.draft.as_ref(), edits.as_object()) else { bail!("this request can't be edited") };
    let mut out = draft.clone();
    for (k, v) in edits {
        let Some(v) = v.as_str().filter(|_| editable(&a.tool).contains(&k.as_str())) else { bail!("only the text can be edited, not {k}") };
        out[k] = json!(v);
    }
    email_fits(&a.tool, &out)?;
    // Only the email's part of the card changes: a note before it (a sensitive source read) stays.
    let (was, now) = (card_text(&describe_email(&a.tool, draft)), card_text(&describe_email(&a.tool, &out)));
    let summary = if a.summary.contains(&was) { a.summary.replacen(&was, &now, 1) } else { now };
    if !db.approval_edit(a.id, &out, &summary)? {
        bail!("this request is no longer pending");
    }
    Ok(())
}

/// Block until the user approves, denies, or the request expires. Works across processes via SQLite.
/// `kind` says why it asks (rule | guarded | login); `site` is the website, for a per-site "always".
/// Approved, it gives back the call's arguments with any text you edited on the card (`args` itself otherwise).
pub(crate) async fn wait_for_approval_with(ctx: &ToolCtx, name: &str, args: &Value, summary: &str, kind: &str, site: Option<&str>) -> Result<Value> {
    let summary = for_helper(ctx, card_text(summary));
    let kind = guest_kind(ctx, kind);
    let id = ctx.db.approval_create(ctx.via_bot.unwrap_or(ctx.bot_id), ctx.ask_chat, name, &summary, APPROVAL_TTL_SECS, kind, site)?;
    if let Some(d) = draft_of(name, args) {
        ctx.db.approval_set_draft(id, &d)?;
    }
    // Only clean asks can become "all like this": none after the conversation read outside content.
    if let Some(run) = ctx.run_id.filter(|_| kind == "rule" && !ctx.is_tainted()) {
        ctx.db.approval_set_run(id, run)?;
    }
    loop {
        match ctx.db.approval_status(id)?.as_str() {
            "approved" => {
                let a = ctx.db.approval(id)?;
                return with_draft(ctx, name, args, a.as_ref().and_then(|a| a.draft.as_ref()));
            }
            "denied" => {
                let reason = ctx.db.approval(id)?.and_then(|a| a.reason);
                // Saying no is a strong hint for the tidy-up (an action that merely failed is not).
                if !ctx.incognito() {
                    let said = reason.as_ref().map(|r| format!("{summary} (Denied: {r})"));
                    ctx.db.signal_add(ctx.bot_id, ctx.chat_id, "denied", said.as_deref().unwrap_or(&summary))?;
                }
                match reason {
                    // Their own words: the bot changes course instead of only giving up.
                    Some(r) => bail!("the user denied this action ({summary}) and said why, in their own words: \"Denied: {r}\". Don't retry it as it was: do what they said instead, or ask them if it isn't clear."),
                    None => bail!("the user denied this action ({summary}). Don't retry it; tell the user it was not done."),
                }
            }
            "expired" => bail!("approval request timed out ({summary}); the action was not done."),
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

/// What a loop-breaker card says before the step itself.
pub(super) const LOOP_CARD: &str = "The bot keeps repeating this step. Let it go on? ";

/// Whether this call would repeat the run's last one, two or three calls over and over (since the last loop ask).
/// A call is its tool and its card text: the same arguments as far as the person can see.
pub(super) fn looping(ctx: &ToolCtx, name: &str, summary: &str) -> bool {
    let Some(run) = ctx.run_id else { return false };
    let calls = ctx.db.tool_calls(run).unwrap_or_default();
    let from = calls.iter().rposition(|c| c.decision == "loop").map_or(0, |i| i + 1);
    let past: Vec<(&str, &str)> = calls[from..].iter().map(|c| (c.tool.as_str(), c.summary.as_str())).collect();
    // Looking again is often fine (a screenshot while a page loads, scrolling down a long one); doing the same thing
    // again rarely is.
    let times = if is_read_only(&ctx.db, name) || name == "computer_scroll" { LOOP_READS } else { LOOP_ACTIONS };
    loops(&past, (name, &truncate(summary, 300)), times)
}

/// Rounds of the same steps before a run is stopped and asked: actions, and calls that only read.
const LOOP_ACTIONS: usize = 3;
pub(super) const LOOP_READS: usize = 5;

/// True when `next` completes `times` rounds in a row of the same cycle of one to three calls.
pub(super) fn loops(past: &[(&str, &str)], next: (&str, &str), times: usize) -> bool {
    let mut seq = past.to_vec();
    seq.push(next);
    (1..=3).any(|n| seq.len() >= n * times && {
        let tail = &seq[seq.len() - n * times..];
        tail.iter().zip(&tail[n..]).all(|(a, b)| a == b)
    })
}

/// How long a parked approval waits in the inbox.
pub(super) const PARKED_TTL_SECS: i64 = 24 * 3600;

/// An unattended run can't wait for a person: park the exact call in the inbox and carry on.
/// A plain ask in a guest's chat is their OK for this once: "always" would change your rules for every chat.
fn guest_kind<'a>(ctx: &ToolCtx, kind: &'a str) -> &'a str {
    if kind == "rule" && ctx.db.chat_guest(ctx.ask_chat) { "guest" } else { kind }
}

pub(super) fn park(ctx: &ToolCtx, name: &str, args: &Value, summary: &str, why: &str, site: Option<&str>) -> Result<String> {
    let summary = for_helper(ctx, card_text(summary));
    let id = ctx.db.approval_create(ctx.via_bot.unwrap_or(ctx.bot_id), ctx.ask_chat, name, &summary, PARKED_TTL_SECS, guest_kind(ctx, why), site)?;
    ctx.db.approval_park(id, &args.to_string(), ctx.is_tainted())?;
    if let Some(d) = draft_of(name, args) {
        ctx.db.approval_set_draft(id, &d)?;
    }
    Ok(format!("Not done yet: this needs the user's OK, so it is waiting in their Mimir inbox and will run as soon as they approve ({summary}). Mention briefly that it's waiting."))
}

/// After a parked approval is approved: run the stored call and say so in the chat.
pub async fn run_parked(db: Db, a: crate::db::Approval) {
    let Ok(Some((raw, tainted))) = db.approval_args(a.id) else { return };
    let args: Value = serde_json::from_str(&raw).unwrap_or(json!({}));
    // A routine asked the person to take over its computer, and they're done: the routine runs again from the start.
    if a.tool == "computer_handoff" {
        let routine = args["routine"].as_i64().filter(|id| db.routine(*id).ok().flatten().is_some());
        let again = match (routine, args["prompt"].as_str()) {
            (Some(id), _) => db.routine_run_soon(id).map(|_| id),
            // A one-off that already finished: once more, as a new one-off.
            (None, Some(prompt)) => db.routine_once(a.bot_id, a.chat_id, crate::db::now() + 60, prompt, args["quiet"].as_bool().unwrap_or(false)).map(|r| r.id),
            (None, None) => Err(anyhow::anyhow!("the routine is gone")),
        };
        let note = match again {
            Ok(_) => "Thanks. I'll run that routine again now, from where you left the computer.".to_string(),
            Err(e) => format!("Thanks. I couldn't run the routine again ({e:#}); ask me to, and I'll carry on."),
        };
        let _ = db.log(a.bot_id, a.chat_id, "assistant", &note, None);
        return;
    }
    // It runs as the routine that asked would have: unattended, and still marked if that run had read outside
    // content (so a memory it saves stays unconfirmed, a routine it makes stays marked).
    let mut ctx = ToolCtx::new(db.clone(), a.bot_id, a.chat_id);
    ctx.unattended = true;
    if tainted {
        ctx.pre_taint();
    }
    let out = match with_draft(&ctx, &a.tool, &args, a.draft.as_ref()) {
        Ok(args) => exec(&ctx, &a.tool, &args).await,
        Err(e) => Err(e),
    };
    // A screenshot taken for a model that isn't there to look at it: not kept.
    if let Ok(r) = &out
        && let (Some(img), _) = crate::computer::tool_image(&a.tool, r, &db.dir())
    {
        let _ = std::fs::remove_file(img);
    }
    let result = match &out {
        Ok(r) => truncate(&crate::computer::tool_image(&a.tool, r, &db.dir()).1, 400),
        Err(e) => truncate(&format!("{e:#}"), 400),
    };
    let _ = db.tool_call_log(None, a.chat_id, &a.tool, &a.summary, "approved later", out.is_ok(), &result);
    let note = if out.is_ok() { format!("Done, as you approved: {}", a.summary) } else { format!("I tried what you approved ({}), but it failed: {result}", a.summary) };
    let _ = db.log(a.bot_id, a.chat_id, "assistant", &note, None);
}

/// Call after any approval decision (app or messaging button): an approved parked call runs now.
/// "Always" on an approval: from now on this tool (or this tool on this website) runs without asking. Only plain rule
/// asks qualify: ones after outside content, saved logins and plans must keep asking.
pub fn always_allow(db: &Db, a: &crate::db::Approval) -> Result<()> {
    if a.status != "approved" {
        return Ok(());
    }
    // Signing in and taking over are the person's own steps: there's nothing to allow.
    if matches!(a.kind.as_str(), "signin" | "takeover") {
        bail!("approved once; this needs you every time");
    }
    // A plain ask: allowed, still asking after outside content. Any other (after outside content, a sensitive source,
    // an email, a new bot, a saved login, a plan, a loop…): the person chose never to be asked before it again.
    // A guest's card was a plain ask in their chat.
    let action = if matches!(a.kind.as_str(), "rule" | "guest") { "allow" } else { "always" };
    match a.site.as_deref() {
        Some(site) => db.set_site_rule(&a.tool, site, action),
        None => db.set_rule(&a.tool, action),
    }
}

/// The person pressed Always on a card that asked for more than its rule: this tool (on this site) never asks again.
pub(crate) fn trusted(db: &Db, tool: &str, site: Option<&str>) -> bool {
    let by_site = site.and_then(|h| db.site_rule(tool, h).ok().flatten());
    by_site.or_else(|| db.rule(tool).ok().flatten()).as_deref() == Some("always")
}

/// The button text for "always" on a chat-app card: the whole tool, or the tool on one site.
pub fn always_label(a: &crate::db::Approval) -> String {
    let s = match a.site.as_deref() {
        Some(site) => crate::say::trf("Always on {site}", &[("site", site)]),
        None if a.kind == "plan" => crate::say::tr("Always approve plans").into(),
        None => crate::say::trf("Always: {tool}", &[("tool", &describe_tool(&a.tool))]),
    };
    // Buttons are short on every app (Slack allows 75 characters).
    truncate(&s, 60)
}

pub fn after_decision(db: &Db, a: &crate::db::Approval) {
    if a.parked && a.status == "approved" {
        tokio::spawn(run_parked(db.clone(), a.clone()));
    }
    // A routine's ask said no to with a reason: its run has moved on, so the reason goes into the chat's conversation,
    // in the person's words, for the bot's next turn there (and to the tidy-up, like a live no).
    if let Some(why) = a.reason.as_deref().filter(|_| a.parked && a.status == "denied") {
        let said = denied_note(&a.summary, why);
        let _ = db.log(a.bot_id, a.chat_id, "user", &said, None);
        let _ = db.signal_add(a.bot_id, a.chat_id, "denied", &format!("{} (Denied: {why})", a.summary));
    }
}

/// What a no with a reason to a routine's ask says in the chat: what it was about, then the person's words.
pub fn denied_note(summary: &str, why: &str) -> String {
    format!("About what a routine asked to do ({}): Denied: {why}", truncate(summary, 300))
}
