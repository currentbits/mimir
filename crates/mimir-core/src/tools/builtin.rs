//! The built-in tools themselves.

use super::*;

/// Where ask_choice leaves its question for the reply being written in this chat.
pub(crate) fn choice_key(bot: i64, chat: i64) -> String {
    format!("choice:{bot}:{chat}")
}

/// Where progress_set keeps the checklist of the reply being written in this chat (cleared when the run ends).
pub(crate) fn progress_key(bot: i64, chat: i64) -> String {
    format!("progress:{bot}:{chat}")
}

/// The checklist of the reply being written in this chat, as (step, done).
pub fn progress(db: &Db, bot: i64, chat: i64) -> Vec<(String, bool)> {
    let v: Value = db.setting(&progress_key(bot, chat)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    v.as_array().into_iter().flatten().map(|s| (s["step"].as_str().unwrap_or_default().to_string(), s["done"].as_bool().unwrap_or(false))).collect()
}

/// The checklist as lines, for a messaging app's draft: "☑ Find flights" / "☐ Book".
pub(crate) fn checklist(steps: &[(String, bool)]) -> String {
    steps.iter().map(|(s, done)| format!("{} {s}", if *done { "☑" } else { "☐" })).collect::<Vec<_>>().join("\n")
}

/// How long a bot waits for the person to take over its computer.
const HANDOFF_SECS: i64 = 600;

/// The setting (Settings → Memory) that saves an instruction a bot learns right away, instead of suggesting it.
pub const ACCEPT_INSTRUCTIONS: &str = "accept_instructions";

/// chat_search results for the model: "[Mon 5 Oct 2026 14:02] You: …" / "Ada: …", flagging replies that came from
/// a conversation that had read outside content.
fn chat_hits(db: &Db, hits: &[crate::db::ChatHit]) -> String {
    use chrono::TimeZone;
    if hits.is_empty() {
        return "nothing in earlier conversations matches".into();
    }
    hits.iter()
        .map(|h| {
            let when = chrono::Local.timestamp_opt(h.ts, 0).single().map(|d| d.format("%a %-d %b %Y %H:%M").to_string()).unwrap_or_default();
            let who = if h.routine {
                "A routine asked".to_string()
            } else if h.role == "user" {
                "They said".to_string()
            } else {
                db.bot(h.bot_id).map(|b| b.name).unwrap_or_else(|_| "A bot".into())
            };
            let flag = if h.tainted { " (this reply came from a conversation that had read outside content; not their words)" } else { "" };
            format!("[{when}] {who}: {}{flag}", h.excerpt.replace('\n', " "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What a bot is asked when a meeting ends and the person wants notes. It reads the transcript itself (meeting_read),
/// so what others said arrives as outside content, not as the person's own message.
pub fn meeting_notes_prompt(db: &Db, id: i64) -> Result<String> {
    // The title isn't repeated here: it comes from the calendar invite (anyone can send one) or the call app, so it's
    // outside content; the bot sees it in meeting_read, which marks the reply as having read outside content.
    db.meetings()?.into_iter().find(|m| m["id"] == id).context("no such meeting")?;
    Ok(format!(
        "My meeting just ended (meeting #{id}). Read it with meeting_read and write my notes: a two-line summary, decisions, \
         action items with who owns each, and anything I promised. If I jotted notes during the call, they're what mattered \
         to me: build around them and fill them out from the transcript. If the invite list and what was said make clear \
         who a 'Speaker N' is, use their name (say it's a guess if it is). Short and scannable."
    ))
}

/// meeting_search / meeting_read, as text for the model.
fn meetings_tool(db: &Db, name: &str, args: &Value) -> Result<String> {
    use chrono::TimeZone;
    let when = |t: i64| chrono::Local.timestamp_opt(t, 0).single().map(|d| d.format("%a %-d %b %Y %H:%M").to_string()).unwrap_or_default();
    let title = |t: &str, started: i64| if t.is_empty() { format!("Meeting {}", when(started)) } else { t.to_string() };
    // "Me", a speaker's name if the person gave one, "Speaker 2" once voices are told apart, else "Others".
    let who = |w: &str, names: &Value| match w {
        "you" => "Me".to_string(),
        w if w.starts_with("others:") => names[w].as_str().map(str::to_string).unwrap_or_else(|| format!("Speaker {}", &w[7..])),
        _ => "Others".to_string(),
    };
    let all = db.meetings()?;
    let names_of = |id: i64| all.iter().find(|m| m["id"] == id).map(|m| m["speakers"].clone()).unwrap_or_default();
    if name == "meeting_read" {
        let id = i(args, "id")?;
        let m = all.iter().find(|m| m["id"] == id).context("no such meeting")?;
        let names = names_of(id);
        let started = m["started_at"].as_i64().unwrap_or(0);
        let lines: Vec<String> = db.meeting_lines(id)?.into_iter().map(|(ts, w, text)| format!("[{}:{:02}] {}: {text}", (ts - started) / 60, (ts - started) % 60, who(&w, &names))).collect();
        let head = format!("#{id} {} ({}, {} min)", title(m["title"].as_str().unwrap_or(""), started), when(started), (m["ended_at"].as_i64().unwrap_or(started) - started) / 60);
        // The person's own notes first: what mattered to them, in their words.
        let invited = m["attendees"].as_array().filter(|a| !a.is_empty()).map(|a| format!("Invited: {}\n", a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))).unwrap_or_default();
        let notes = m["notes"].as_str().map(str::trim).filter(|n| !n.is_empty()).map(|n| format!("My notes during the call:\n{n}\n\nTranscript:\n")).unwrap_or_default();
        // A long meeting keeps its start and its end (where decisions tend to land); the middle is cut, and says so.
        let mut body = lines.join("\n");
        if body.len() > 60_000 {
            let (a, b) = (body.floor_char_boundary(20_000), body.floor_char_boundary(body.len() - 38_000));
            body = format!("{}\n[… the middle of this long meeting is cut here; use meeting_search to find a topic in it …]\n{}", &body[..a], &body[b..]);
        }
        let summary = m["summary"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("Notes written after the call:\n{s}\n\n")).unwrap_or_default();
        let transcript = if notes.is_empty() && !summary.is_empty() { "Transcript:\n" } else { "" };
        return Ok(format!("{head}\n{invited}{summary}{notes}{transcript}{body}"));
    }
    match args["query"].as_str().map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) => {
            let hits = db.meeting_search(q, 30)?;
            Ok(if hits.is_empty() {
                "no meeting mentions that".into()
            } else {
                hits.iter().map(|(id, t, started, _, w, text)| format!("#{id} {}: {}: {text}", title(t, *started), who(w, &names_of(*id)))).collect::<Vec<_>>().join("\n")
            })
        }
        None => {
            let v = all;
            Ok(if v.is_empty() {
                "no meetings yet".into()
            } else {
                v.iter().take(15).map(|m| {
                    let started = m["started_at"].as_i64().unwrap_or(0);
                    format!("#{} {} ({}, {} min, {} words)", m["id"], title(m["title"].as_str().unwrap_or(""), started), when(started), (m["ended_at"].as_i64().unwrap_or(started) - started) / 60, m["words"])
                }).collect::<Vec<_>>().join("\n")
            })
        }
    }
}

/// Limits on what a bot may ask for, checked before any approval card.
/// Where web_search goes: Brave Search with its key, else a SearXNG address; None when neither is set up.
pub(crate) fn search_service(db: &Db) -> Option<(&'static str, String)> {
    match crate::host::api_key("brave") {
        Some(k) => Some(("brave", k)),
        None => db.setting("searxng_url").ok().flatten().filter(|u| !u.is_empty()).map(|u| ("searxng", u)),
    }
}

/// (title, url, snippet) for a query, from the search service.
async fn web_search(db: &Db, q: &str, n: usize) -> Result<Vec<(String, String, String)>> {
    let (kind, v) = search_service(db).context("no web search set up (Settings → Web search)")?;
    let http = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
    let (body, list, snippet) = match kind {
        "brave" => {
            let url = url::Url::parse_with_params("https://api.search.brave.com/res/v1/web/search", &[("q", q), ("count", &n.to_string())])?;
            (http.get(url).header("X-Subscription-Token", v).header("Accept", "application/json").send().await?.error_for_status()?.json::<Value>().await?, "/web/results", "description")
        }
        _ => {
            let url = url::Url::parse_with_params(&format!("{v}/search"), &[("q", q), ("format", "json")])?;
            (http.get(url).send().await?.error_for_status()?.json::<Value>().await?, "/results", "content")
        }
    };
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    Ok(body.pointer(list).and_then(Value::as_array).into_iter().flatten().take(n).map(|r| (s(&r["title"]), s(&r["url"]), s(&r[snippet]))).filter(|h| !h.1.is_empty()).collect())
}

/// Recent posts on X (the last 7 days) for a query, newest first: (author, link, date, text). Needs an X API bearer token.
async fn x_search(q: &str, n: usize) -> Result<Vec<(String, String, String, String)>> {
    let token = crate::host::api_key("x").context("no X API token set up (Settings → Web search → X)")?;
    // ponytail: recent search only (7 days); full-archive search is a pricier X plan, add when someone asks
    let url = url::Url::parse_with_params(
        "https://api.x.com/2/tweets/search/recent",
        &[("query", q), ("max_results", &n.max(10).to_string()), ("sort_order", "recency"), ("tweet.fields", "created_at,author_id"), ("expansions", "author_id"), ("user.fields", "username")],
    )?;
    let http = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
    let r = http.get(url).bearer_auth(token).send().await?;
    let status = r.status();
    let body: Value = r.json().await.unwrap_or_default();
    if !status.is_success() {
        // X says why (no credits, wrong plan, a bad query) in `detail` or `errors[].message`.
        let why = body["detail"].as_str().or(body["errors"][0]["message"].as_str()).or(body["title"].as_str()).unwrap_or("");
        bail!("X said {status}: {}", truncate(why, 300));
    }
    Ok(x_posts(&body, n))
}

/// X's recent-search answer as (author, link, date, text); authors come from `includes.users`.
pub(super) fn x_posts(body: &Value, n: usize) -> Vec<(String, String, String, String)> {
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let users = body["includes"]["users"].as_array().cloned().unwrap_or_default();
    body["data"].as_array().into_iter().flatten().take(n).map(|t| {
        let who = users.iter().find(|u| u["id"] == t["author_id"]).map(|u| s(&u["username"])).unwrap_or_default();
        // x.com/i/status/<id> opens a post without knowing its author.
        let link = format!("https://x.com/{}/status/{}", if who.is_empty() { "i" } else { &who }, s(&t["id"]));
        (who, link, s(&t["created_at"]), s(&t["text"]))
    }).collect()
}

/// The household member a routine is for (`for`): their direct chat and its title. None: this chat.
pub(super) fn remind_target(ctx: &ToolCtx, args: &Value) -> Result<Option<(i64, String)>> {
    match args["for"].as_str().map(str::trim).filter(|f| !f.is_empty()) {
        Some(name) => Ok(Some(ctx.db.household_member(name, ctx.chat_id)?)),
        None => Ok(None),
    }
}

/// You said yes to a reminder for this person's chat once already.
pub(super) fn reminds_ok(db: &Db, chat: i64) -> bool {
    db.setting(&format!("remind_ok:{chat}")).ok().flatten().is_some()
}

/// "Add to calendar" links for Google Calendar and Outlook (the person clicks, checks and saves: nothing is written for
/// them). `start` is local time ("2026-10-12 15:00"), or a date for an all-day event.
pub(super) fn calendar_links(title: &str, start: &str, minutes: i64, location: &str, notes: &str) -> Result<String> {
    if title.trim().is_empty() || title.chars().count() > 200 || notes.chars().count() > 2000 || location.chars().count() > 300 {
        bail!("a title (up to 200 characters), a place and notes of reasonable length");
    }
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    let (google_dates, outlook) = match chrono::NaiveDate::parse_from_str(start.trim(), "%Y-%m-%d") {
        Ok(day) => {
            let next = day + chrono::Duration::days(1);
            (format!("{}/{}", day.format("%Y%m%d"), next.format("%Y%m%d")), format!("startdt={}&enddt={}&allday=true", day, next))
        }
        Err(_) => {
            let from = local_time(start)?;
            let to = from + minutes.clamp(5, 24 * 60) * 60;
            let utc = |t: i64| chrono::DateTime::from_timestamp(t, 0).map(|d| d.format("%Y%m%dT%H%M%SZ").to_string()).unwrap_or_default();
            let iso = |t: i64| chrono::DateTime::from_timestamp(t, 0).map(|d| d.to_rfc3339()).unwrap_or_default();
            (format!("{}/{}", utc(from), utc(to)), format!("startdt={}&enddt={}", enc(&iso(from)), enc(&iso(to))))
        }
    };
    let google = format!(
        "https://calendar.google.com/calendar/render?action=TEMPLATE&text={}&dates={google_dates}&location={}&details={}",
        enc(title.trim()),
        enc(location),
        enc(notes)
    );
    let outlook = format!(
        "https://outlook.live.com/calendar/0/deeplink/compose?subject={}&{outlook}&location={}&body={}",
        enc(title.trim()),
        enc(location),
        enc(notes)
    );
    Ok(format!("Google Calendar: {google}\nOutlook: {outlook}\n(Nothing is added until the user opens one and saves it.)"))
}

/// A routine timed off the calendar needs one linked; one started by mail, a mailbox. A webhook's secret is the
/// person's to hand out, so they make those in the app.
pub(super) fn calendar_linked(ctx: &ToolCtx, cron: &str) -> Result<()> {
    if crate::calendar::trigger(cron)?.is_some() && crate::host::secret_get(crate::calendar::SECRET).is_none() {
        bail!("no calendar is linked yet: the person adds its link in Mimir's Meetings → Calendar first");
    }
    if crate::events::is_hook(cron) {
        bail!("only the person can set up a webhook, in Mimir's Routines tab (When called)");
    }
    crate::events::ready(&ctx.db, ctx.chat_id, cron)
}

/// When a routine runs next, for the bots: a calendar one may have no matching event coming up.
fn next_run(r: &crate::db::Routine) -> String {
    match crate::calendar::trigger(&r.cron) {
        Ok(Some(t)) if r.next_fire >= crate::db::IDLE => format!("({}; no such event in the next week)", t.words()),
        Ok(Some(t)) => format!("{} ({})", fmt_ts(r.next_fire), t.words()),
        _ => match crate::events::on(&r.cron) {
            Ok(Some(on)) => format!("on {}{}", on.words(), if r.once { ", then it's gone" } else { "" }),
            _ => fmt_ts(r.next_fire),
        },
    }
}

/// Calendar events as the bots read them: one line each, in order.
pub(crate) fn calendar_lines(events: &[crate::calendar::Event]) -> String {
    if events.is_empty() {
        return "Nothing on the calendar then.".into();
    }
    let fmt = |t: i64| chrono::DateTime::from_timestamp(t, 0).map(|d| d.with_timezone(&chrono::Local).format("%a %-d %b %H:%M").to_string()).unwrap_or_default();
    events
        .iter()
        .take(200)
        .map(|e| {
            let with = if e.attendees.is_empty() { String::new() } else { format!(" (with {})", e.attendees.iter().take(8).cloned().collect::<Vec<_>>().join(", ")) };
            format!("- {}–{} {}{with}", fmt(e.start), crate::calendar::clock(e.end), e.title)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The bot a bot_create call would make, or why it can't: checked before anyone is asked, and again when it runs.
pub(super) fn new_bot(ctx: &ToolCtx, args: &Value) -> Result<crate::db::Bot> {
    let (name, role) = (s(args, "name")?.trim(), s(args, "role")?.trim());
    if name.is_empty() || name.chars().count() > 40 {
        bail!("a bot's name is 1 to 40 characters");
    }
    // The whole role goes on the approval card (it becomes the bot's standing instructions): nothing can hide past its end.
    if role.is_empty() || role.chars().count() > 1000 {
        bail!("a role can be up to 1000 characters, so the person can read all of it before approving: keep it to what the bot is for and how it works");
    }
    if let Some(why) = looks_secret(&ctx.db, role) {
        bail!("not created: the role contains {why}; secrets go in the app (saved logins, Settings), not in a bot's instructions");
    }
    if ctx.db.bots()?.iter().any(|b| b.name.eq_ignore_ascii_case(name)) {
        bail!("there's already a bot called {name}; pick another name or hand it the task with ask_bot");
    }
    let me = ctx.db.bot(ctx.bot_id)?;
    // Never more than this bot may use itself, and no bot-making of its own.
    let all: Vec<String> = all_specs(&ctx.db).into_iter().map(|t| t.0).filter(|t| me.allows(t) && t != "bot_create").collect();
    let tools: Vec<String> = match args.get("tools").and_then(Value::as_array) {
        Some(asked) if !asked.is_empty() => {
            let asked: Vec<String> = asked.iter().filter_map(Value::as_str).map(|t| t.trim().to_string()).collect();
            let not: Vec<&str> = asked.iter().filter(|t| !all.contains(t)).map(String::as_str).collect();
            if !not.is_empty() {
                bail!("can't give it {}: only tools you have yourself ({})", not.join(", "), all.join(", "));
            }
            asked
        }
        _ => all,
    };
    Ok(crate::db::Bot { id: 0, name: name.into(), persona: role.into(), provider: me.provider, model: me.model, base_url: me.base_url, tools: tools.join(","), look: String::new() })
}

/// An event's time from a tool's arguments (a whole day, or a start and how many minutes) and how a card says it.
pub(super) fn event_when(start: &str, minutes: i64) -> Result<(crate::caldav::When, String)> {
    use chrono::TimeZone;
    if let Ok(day) = chrono::NaiveDate::parse_from_str(start.trim(), "%Y-%m-%d") {
        return Ok((crate::caldav::When::Day(day), format!("{} (all day)", day.format("%a %-d %b %Y"))));
    }
    let from = local_time(start)?;
    let to = from + minutes.clamp(5, 24 * 60) * 60;
    let at = |t: i64, f: &str| chrono::Local.timestamp_opt(t, 0).single().map(|d| d.format(f).to_string()).unwrap_or_default();
    Ok((crate::caldav::When::At(from, to), format!("{}–{}", at(from, "%a %-d %b %Y %H:%M"), at(to, "%H:%M"))))
}

/// Memories from a scope marked sensitive were read (searched, or in standing memory): what goes out asks from here on.
pub(crate) fn mark_sensitive_memory(ctx: &ToolCtx, scope: &str, read: &[Entry]) {
    if !read.is_empty() && is_sensitive(&ctx.db, scope) {
        let title = scope.trim_start_matches("chat:").parse().ok().and_then(|c| ctx.db.chat_title(c).ok()).filter(|t| !t.is_empty());
        ctx.mark_sensitive(&format!("the memory of {}", title.as_deref().unwrap_or(scope)));
    }
}

pub(super) async fn exec(ctx: &ToolCtx, name: &str, args: &Value) -> Result<String> {
    if crate::ext::find(&ctx.db, name).is_some() {
        // Pages, files, issues, mail: whatever a tool server returns is untrusted content.
        ctx.taint();
        // A server (or tool) you marked sensitive: from here on, what goes out asks.
        if is_sensitive(&ctx.db, name) {
            ctx.mark_sensitive(crate::ext::split(name).map_or(name, |(server, _)| server));
        }
        return crate::ext::call(ctx, name, args).await;
    }
    let scope = ctx.scope();
    Ok(match name {
        "memory_store" => {
            let src = args.get("source").and_then(Value::as_str).unwrap_or("");
            let sup = args.get("supersedes").and_then(Value::as_i64);
            let (title, body) = (s(args, "title")?, s(args, "body")?);
            if let Some(why) = looks_secret(&ctx.db, &format!("{title}\n{body}")) {
                bail!("not saved: that contains {why}. Memory is shown in prompts and the app, so secrets don't belong there; saved logins (Settings → Browser) keep them safe.");
            }
            let kind = s(args, "type")?;
            // The household's memories, or this chat's own.
            let household = args["household"].as_bool().unwrap_or(false);
            if household && !ctx.db.in_household(ctx.chat_id) {
                bail!("this chat isn't in the household; save it here instead (without household)");
            }
            let scope = if household { ctx.db.household_ensure()?; crate::HOUSEHOLD.to_string() } else { scope };
            // None: leave it (a new version keeps the old end date); Some(None): no end date.
            let until = match args.get("until").and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty()) {
                Some("none") => Some(None),
                Some(u) => Some(Some(end_of_day(u)?)),
                None => None,
            };
            // A convention steers every future reply: only the person's own words set one directly. One a bot
            // inferred (or picked up after reading outside content) waits for them to accept it.
            // The person's own message must state a rule, and be about this one (shares a word with it).
            // A chat API token's chat says nothing in your words: anyone with the token (or at the speaker using it) types there.
            let asked = ctx.run_id.filter(|_| !crate::host::is_api_chat(ctx.chat_id)).map(|r| ctx.db.asked(r)).transpose()?.flatten();
            let own_words = !ctx.is_tainted() && asked.as_deref().is_some_and(|t| crate::states_rule(t) && crate::shares_words(t, &format!("{title} {body}")));
            // What the person's message says in their own words is theirs, whatever else the conversation read (a page
            // can't put words in their message): it isn't marked unconfirmed for the conversation's outside content.
            let theirs = asked.as_deref().is_some_and(|t| crate::own_words(t) && crate::db::alike(&format!("{title} {body}"), t));
            let tainted = ctx.is_tainted() && !theirs;
            // A guest (or someone in a family group) adding to the household: it's a suggestion for the person who runs
            // the bots, in their Needs you.
            if household && ctx.db.chat_guest(ctx.chat_id) {
                // In a family group, whoever wrote the message ("Sam: …", put there by Mimir); else the guest's chat.
                let who = match asked.as_deref().and_then(|t| t.split_once(": ")).filter(|_| ctx.db.chat_family(ctx.chat_id)) {
                    Some((name, _)) => format!("{name} (in the family group)"),
                    None => format!("{} (a guest)", ctx.db.chat_title(ctx.chat_id)?),
                };
                let sug = crate::db::Suggestion {
                    id: 0,
                    action: if sup.is_some() { "update" } else { "add" }.into(),
                    entry_id: sup,
                    kind: kind.into(),
                    title: title.into(),
                    body: body.into(),
                    reason: format!("{who} wants to add this to the household{}.", if src.is_empty() { String::new() } else { format!(" ({src})") }),
                    check: None,
                    tainted,
                };
                return Ok(match ctx.db.suggestion_add(&scope, &sug)? {
                    Some(_) => "Sent to the person who runs you to accept: the household changes once they do. Say so.".into(),
                    None => "Not sent: this was already suggested (or turned down recently).".into(),
                });
            }
            // Replacing one of the person's conventions counts too, whatever type the new entry claims to be.
            let replaces_convention = sup.is_some_and(|id| ctx.db.entries(&scope).is_ok_and(|v| v.iter().any(|e| e.id == id && e.kind == "convention")));
            // Settings → Memory → Always accept instructions, or a bot set to "Don't ask me first": saved straight away
            // (one picked up from outside content is still kept unconfirmed, like any memory: out of the standing
            // instructions until you check it).
            let accept = crate::tools::hands_off(ctx) || ctx.db.setting(ACCEPT_INSTRUCTIONS)?.as_deref() == Some("on");
            if (kind == "convention" || replaces_convention) && !own_words && !accept {
                let why = if src.is_empty() { String::new() } else { format!(" ({src})") };
                let sug = crate::db::Suggestion {
                    id: 0,
                    action: if sup.is_some() { "update" } else { "add" }.into(),
                    entry_id: sup,
                    kind: kind.into(),
                    title: title.into(),
                    body: body.into(),
                    // ponytail: a suggestion has no end date of its own; the card says it, you set it after accepting
                    reason: match args.get("until").and_then(Value::as_str).filter(|_| matches!(until, Some(Some(_)))) {
                        Some(day) => format!("{}{why}. Meant to end {}: set that on the memory after accepting.", crate::db::BOT_PROPOSED, day.trim()),
                        None => format!("{}{why}.", crate::db::BOT_PROPOSED),
                    },
                    check: None,
                    tainted,
                };
                return Ok(match ctx.db.suggestion_add(&scope, &sug)? {
                    Some(_) => "Not saved yet: the user didn't state this rule themselves, so it's proposed to them in Memory to accept. Tell them it's suggested, not saved.".into(),
                    None => "Not saved: this was already proposed to the user (or they turned it down recently).".into(),
                });
            }
            let prov = crate::db::Prov { origin: "bot", bot: Some(ctx.bot_id), run: ctx.run_id, tainted };
            // Saving the exact text of a memory you already have returns that one; after outside content, that must not
            // let a page give your memory an end date (and push it out of standing memory).
            let existing = ctx.is_tainted() && ctx.db.entries(&scope)?.iter().any(|e| e.status == "active" && e.title == title && e.body == body);
            let until = until.filter(|_| !existing);
            let (id, new) = ctx.db.store_by(&scope, kind, title, body, src, sup, &prov)?;
            if let Some(until) = until {
                ctx.db.set_until(&scope, id, until)?;
            }
            // Only a new row: "just for you" on one every bot already has would take it away from the others.
            if new && args["only_me"].as_bool() == Some(true) {
                ctx.db.set_for_bot(&scope, id, ctx.bot_id)?;
            }
            if !new {
                let shared = ctx.db.entries(&scope)?.iter().any(|e| e.id == id && e.for_bot == 0);
                return Ok(format!("already known as #{id} ({})", if shared { "shared" } else { "just for you" }));
            }
            format!("stored #{id}{}", if household { " in the household" } else { "" })
        }
        "memory_search" => {
            let q = s(args, "query")?;
            let hits = ctx.db.search(&scope, ctx.bot_id, q, 8)?;
            let shared = if ctx.db.in_household(ctx.chat_id) { ctx.db.search(crate::HOUSEHOLD, ctx.bot_id, q, 8)? } else { vec![] };
            // An unconfirmed memory's text came from outside in the first place.
            if hits.iter().chain(&shared).any(|e| e.tainted) {
                ctx.taint();
            }
            mark_sensitive_memory(ctx, &scope, &hits);
            mark_sensitive_memory(ctx, crate::HOUSEHOLD, &shared);
            match (hits.is_empty(), shared.is_empty()) {
                (_, true) => lines(&hits),
                (true, false) => format!("household:\n{}", lines(&shared)),
                (false, false) => format!("{}\nhousehold:\n{}", lines(&hits), lines(&shared)),
            }
        }
        "chat_search" => {
            let day = |k: &str| args.get(k).and_then(Value::as_str).filter(|d| !d.trim().is_empty()).map(day_bounds).transpose();
            let range = (day("since")?.map_or(0, |d| d.0), day("until")?.map_or(i64::MAX, |d| d.1));
            let hits = ctx.db.chat_search(ctx.chat_id, s(args, "query")?, ctx.run_id, range, 12)?;
            // The person's own words are theirs; a bot reply written after reading outside content isn't, so finding one
            // counts as reading outside content again (a planted instruction can't come back as a memory of the chat).
            if hits.iter().any(|h| h.tainted) {
                ctx.taint();
            }
            chat_hits(&ctx.db, &hits)
        }
        "email_search" => {
            // Mail is other people's words, and it's private.
            ctx.taint();
            ctx.mark_private();
            let all = crate::email::accounts(&ctx.db);
            let boxes = match args["account"].as_str().filter(|a| !a.trim().is_empty()) {
                Some(u) => vec![crate::email::find(&ctx.db, Some(u))?],
                None if all.is_empty() => bail!("no email account set up (Settings → Email)"),
                None => all.clone(),
            };
            let since = args["since"].as_str().filter(|d| !d.trim().is_empty()).map(|d| chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d")).transpose().map_err(|_| anyhow::anyhow!("since is a date like 2026-10-05"))?;
            let q = crate::email::criteria(args["query"].as_str(), args["from"].as_str(), since, args["unread"].as_bool().unwrap_or(false))?;
            let folder = args["folder"].as_str().filter(|f| !f.trim().is_empty()).unwrap_or("INBOX");
            let limit = args["limit"].as_u64().unwrap_or(10).clamp(1, 25) as usize;
            // Each mailbox its own block, named when there are several: ids are only unique within one.
            let mut out = vec![];
            for a in &boxes {
                let lines = match crate::email::search(a, folder, &q, limit).await {
                    Ok(found) if found.is_empty() => "No messages match.".into(),
                    Ok(found) => found.iter().map(|m| format!("#{} · {} · {} · {}{}", m.uid, m.date, m.from, m.subject, if m.unread { " (unread)" } else { "" })).collect::<Vec<_>>().join("\n"),
                    Err(e) if boxes.len() > 1 => format!("error: {e:#}"),
                    Err(e) => return Err(e),
                };
                out.push(if all.len() > 1 { format!("{}:\n{lines}", a.user) } else { lines });
            }
            out.join("\n\n")
        }
        "file_list" => {
            let v = crate::files::list(&ctx.db, ctx.chat_id, args["dir"].as_str().unwrap_or(""))?;
            if v.is_empty() { "The workspace is empty.".into() } else { v.iter().map(|(p, n)| format!("{p} ({n} bytes)")).collect::<Vec<_>>().join("\n") }
        }
        "file_read" => {
            let (text, outside) = crate::files::read(&ctx.db, ctx.chat_id, s(args, "path")?)?;
            if outside {
                ctx.taint();
            }
            truncate(&text, 60_000)
        }
        "file_write" => crate::files::write(&ctx.db, ctx.chat_id, s(args, "path")?, s(args, "content")?, s(args, "mode")?, ctx.is_tainted())?,
        "file_delete" => {
            let p = s(args, "path")?;
            crate::files::delete(&ctx.db, ctx.chat_id, p)?;
            format!("deleted {p}")
        }
        "send_file" => {
            let run = ctx.run_id.context("files go with a reply; there's none to send this with")?;
            let (name, bytes) = if args["screenshot"].as_bool() == Some(true) {
                if !(usable(ctx, "computer_screenshot") && ctx.db.bot(ctx.bot_id)?.allows("computer_screenshot")) {
                    bail!("you have no computer to take a screenshot of");
                }
                ("screen.jpg".to_string(), tokio::task::spawn_blocking(crate::computer::screenshot_jpeg).await??)
            } else {
                let p = s(args, "path").map_err(|_| anyhow::anyhow!("give a workspace path, or screenshot=true"))?;
                let (text, _) = crate::files::read(&ctx.db, ctx.chat_id, p)?;
                match args["format"].as_str() {
                    Some(f) => crate::office::make(f, p, &text)?,
                    None => (p.rsplit('/').next().unwrap_or(p).to_string(), text.into_bytes()),
                }
            };
            let name = crate::files::send(&ctx.db, ctx.chat_id, run, &name, &bytes)?;
            format!("{name} will be sent to the chat with your reply. Don't paste its contents too.")
        }
        "email_send" => {
            let a = crate::email::find(&ctx.db, args["account"].as_str())?;
            let reply_to = args["reply_to"].as_i64().map(u32::try_from).transpose()?;
            let to = s(args, "to")?;
            let disclose = ctx.db.setting("email_disclose")?;
            let copy = crate::email::send(&a, &crate::email::Draft { to, cc: args["cc"].as_str().unwrap_or(""), subject: s(args, "subject")?, body: s(args, "body")?, reply_to }, disclose.as_deref()).await?;
            format!("Sent to {to}.{copy}")
        }
        "email_hand_over" => {
            let line = |k: &str| s(args, k).map(|v| truncate(v.trim(), 200));
            let h = crate::email::HandedOver {
                id: 0,
                bot_id: ctx.bot_id,
                chat_id: ctx.chat_id,
                account: crate::email::find(&ctx.db, args["account"].as_str())?.user,
                from: line("from")?,
                subject: line("subject")?,
                summary: line("summary")?,
                at: crate::db::now(),
            };
            crate::email::hand_over(&ctx.db, h)?;
            "It's in the user's Needs you now. Don't answer it or draft a reply; tell them in one line.".into()
        }
        "email_draft" => {
            let a = crate::email::find(&ctx.db, args["account"].as_str())?;
            let reply_to = args["reply_to"].as_i64().map(u32::try_from).transpose()?;
            let d = crate::email::Draft { to: s(args, "to")?, cc: args["cc"].as_str().unwrap_or(""), subject: s(args, "subject")?, body: s(args, "body")?, reply_to };
            let folder = crate::email::draft(&a, &d).await?;
            format!("Saved as a draft in {folder}: nothing was sent. Tell the user it's waiting there for them to read and send.")
        }
        "email_read" => {
            ctx.taint();
            ctx.mark_private();
            let a = crate::email::find(&ctx.db, args["account"].as_str())?;
            let uid = u32::try_from(args["id"].as_i64().context("id is the number email_search gave")?)?;
            crate::email::read(&a, args["folder"].as_str().filter(|f| !f.trim().is_empty()).unwrap_or("INBOX"), uid).await?
        }
        "web_search" => {
            ctx.taint();
            let q = s(args, "query")?.trim();
            if q.is_empty() || q.chars().count() > 400 {
                bail!("a search is 1 to 400 characters");
            }
            let n = args["count"].as_u64().unwrap_or(6).clamp(1, 10) as usize;
            let hits = web_search(&ctx.db, q, n).await?;
            if hits.is_empty() {
                "No results.".into()
            } else {
                hits.iter().enumerate().map(|(i, (t, u, d))| format!("{}. {t}\n   {u}\n   {}", i + 1, truncate(d, 300))).collect::<Vec<_>>().join("\n")
            }
        }
        "x_search" => {
            ctx.taint();
            let q = s(args, "query")?.trim();
            // X caps a recent-search query at 512 characters.
            if q.is_empty() || q.chars().count() > 400 {
                bail!("a search is 1 to 400 characters");
            }
            let n = args["count"].as_u64().unwrap_or(10).clamp(1, 25) as usize;
            let posts = x_search(q, n).await?;
            if posts.is_empty() {
                "No posts in the last 7 days.".into()
            } else {
                posts.iter().enumerate().map(|(i, (who, u, at, text))| format!("{}. @{who} · {at}\n   {u}\n   {}", i + 1, truncate(&text.replace('\n', " "), 400))).collect::<Vec<_>>().join("\n")
            }
        }
        "calendar_add" => calendar_links(
            s(args, "title")?,
            s(args, "start")?,
            args["minutes"].as_i64().unwrap_or(60),
            args["location"].as_str().unwrap_or(""),
            args["notes"].as_str().unwrap_or(""),
        )?,
        "calendar_create" => {
            let cal = crate::caldav::account(&ctx.db).context("no calendar connected for adding events (Settings → Calendar)")?;
            let (when, said) = event_when(s(args, "start")?, args["minutes"].as_i64().unwrap_or(60))?;
            let title = s(args, "title")?;
            crate::caldav::add(&cal, &crate::caldav::NewEvent { title, when, location: args["location"].as_str().unwrap_or(""), notes: args["notes"].as_str().unwrap_or("") }).await?;
            format!("Added \"{title}\" to {}, {said}.", cal.name)
        }
        "calendar_list" => {
            // Invites come from anyone: their titles are outside content.
            ctx.taint();
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let from = day_bounds(args["from"].as_str().filter(|d| !d.trim().is_empty()).unwrap_or(&today))?.0;
            let to = match args["to"].as_str().filter(|d| !d.trim().is_empty()) {
                Some(d) => day_bounds(d)?.1 + 1,
                None => from + 7 * 86400,
            };
            if to <= from || to - from > 62 * 86400 {
                bail!("pick a range of up to two months, 'to' after 'from'");
            }
            calendar_lines(&crate::calendar::events(from, to).await?)
        }
        "meeting_search" | "meeting_read" => {
            // What other people said on a call is outside content, like a web page, and private.
            ctx.taint();
            ctx.mark_private();
            meetings_tool(&ctx.db, name, args)?
        }
        "plan_propose" => propose_plan(ctx, args).await?,
        "ask_choice" => {
            let question = truncate(s(args, "question")?.trim(), 300);
            let options: Vec<String> = args["options"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|o| truncate(o.trim(), 60)).filter(|o| !o.is_empty()).collect();
            if question.is_empty() || !(2..=4).contains(&options.len()) {
                bail!("give a question and 2 to 4 short options");
            }
            // The reply's channel shows it once the reply is written (runtime::Core::choice_rows), whichever process this runs in.
            ctx.db.set_setting(&choice_key(ctx.bot_id, ctx.chat_id), &json!({"question": question, "options": options}).to_string())?;
            "Shown under your reply as buttons. End your reply now; their answer comes as their next message.".into()
        }
        "progress_set" => {
            let steps: Vec<Value> = args["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| json!({"step": truncate(s["step"].as_str().unwrap_or("").trim(), 100), "done": s["done"].as_bool().unwrap_or(false)}))
                .filter(|s| s["step"] != "")
                .collect();
            if steps.is_empty() || steps.len() > 50 {
                bail!("give 1 to 50 steps");
            }
            let open = steps.iter().filter(|s| s["done"] == false).count();
            // The app and the chat's draft show it while the reply is written (runtime's Follower), whichever process this runs in.
            ctx.db.set_setting(&progress_key(ctx.bot_id, ctx.chat_id), &Value::from(steps).to_string())?;
            if open == 0 { "All steps ticked.".into() } else { format!("Saved; {open} still open.") }
        }
        "computer_handoff" => {
            let reason = s(args, "reason")?.trim().to_string();
            // Their sign-in goes where this bot will look: its own browser profile.
            if crate::computer::usable(&ctx.db) {
                let (home, bot) = (ctx.db.dir(), ctx.bot_id);
                tokio::task::spawn_blocking(move || crate::computer::start(&home, Some(bot))).await??;
            }
            // Let go of the computer first, so the person's clicks make it theirs.
            if let Some(r) = ctx.run_id {
                ctx.db.computer_release(&format!("run:{r}"))?;
            }
            let link = match ctx.db.setting("public_url")? {
                Some(url) => format!("Open {}/#computer", url.trim_end_matches('/')),
                None => "Open Mimir and go to Computer".into(),
            };
            // A routine has nobody watching it: ask, end this run, and run the routine again once they're done (the
            // computer keeps what they did, like a sign-in), instead of holding the bot for ten minutes.
            if ctx.unattended {
                let routine = ctx.run_id.and_then(|r| ctx.db.run_routine_id(r).ok().flatten()).and_then(|id| ctx.db.routine(id).ok().flatten());
                let summary = format!("{reason}\n\n{link}, do it on the screen, then press Done here: the routine runs again.");
                let id = ctx.db.approval_create(ctx.via_bot.unwrap_or(ctx.bot_id), ctx.ask_chat, "computer_handoff", &summary, PARKED_TTL_SECS, "takeover", None)?;
                let again = json!({"reason": reason, "routine": routine.as_ref().map(|r| r.id), "prompt": routine.as_ref().map(|r| r.prompt.clone()), "quiet": routine.as_ref().is_some_and(|r| r.quiet)});
                ctx.db.approval_park(id, &again.to_string(), ctx.is_tainted())?;
                return Ok("The person has been asked to take over for this step. When they're done, this routine runs again from the start. \
                           Stop here, and say briefly what's waiting for them."
                    .into());
            }
            let summary = format!("{reason}\n\n{link}, do it on the screen, then press Give back.");
            let id = ctx.db.approval_create(ctx.via_bot.unwrap_or(ctx.bot_id), ctx.ask_chat, "computer_handoff", &summary, HANDOFF_SECS, "takeover", None)?;
            // Done when they give it back after taking over, or say so on the card; "Can't now" or nobody in time, stop.
            let mut took = false;
            loop {
                match ctx.db.approval_status(id)?.as_str() {
                    "approved" => break,
                    "denied" => bail!("the person can't do it now ({reason}). Stop here and tell them what's left to do."),
                    "expired" => bail!("nobody took over in 10 minutes ({reason}). Stop here and tell them what's needed."),
                    _ => {}
                }
                // Theirs while the lease is (even after a quiet minute waiting for a text message), until Give back.
                let theirs = ctx.db.setting("computer_lease")?.is_some_and(|v| v.contains("\"who\":\"you\""));
                if took && !theirs {
                    ctx.db.approval_decide(id, true, None)?;
                    break;
                }
                took |= theirs;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            ctx.taint();
            "The person did it and gave the computer back. Take a screenshot to see where things are now, then carry on.".into()
        }
        "computer_signin" => computer_signin(ctx).await?,
        n if n.starts_with("computer_") => {
            hold_computer(ctx).await?;
            // What's on the screen is outside content, like a web page.
            ctx.taint();
            let (home, bot) = (ctx.db.dir(), ctx.bot_id);
            let (n, args) = (n.to_string(), args.clone());
            // xdotool, scrot and the screenshot encoding block: off the async runtime.
            tokio::task::spawn_blocking(move || computer_action(&home, bot, &n, &args)).await??
        }
        "skill_save" => {
            let (title, description, body) = (s(args, "title")?, s(args, "description")?, s(args, "body")?);
            // Only from what this conversation did: with nothing done here, there's nothing proven to save.
            let steps = ctx.db.recent_steps(ctx.bot_id, ctx.chat_id, 4)?;
            if steps.is_empty() {
                bail!("nothing has been done in this conversation yet to save as a skill; do the task first (or teach it from a link in Memory → Skills)");
            }
            if let Some(why) = looks_secret(&ctx.db, &format!("{title}\n{description}\n{body}")) {
                bail!("not saved: that contains {why}");
            }
            let mut reason = format!("You asked to save this from the conversation. What it did: {}.", truncate(&steps.join(" → "), 400));
            if ctx.is_tainted() {
                reason.push_str(" The conversation had read outside content: check the steps.");
            }
            match ctx.db.skill_saved_propose(&scope, ctx.bot_id, title, description, body, &reason, ctx.run_id)? {
                // A bot set to "Don't ask me first" keeps it straight away (its reason, with the steps, stays on it).
                Some(id) if crate::tools::hands_off(ctx) => {
                    ctx.db.skill_decide(&scope, id, true)?;
                    format!("Saved skill #{id} \"{}\".", title.trim())
                }
                Some(id) => format!("Proposed skill #{id} \"{}\". It's waiting for the person to press Save (here in the chat, or in Memory → Skills); tell them.", title.trim()),
                None => "A version of that skill is already waiting for the person, or they turned it down recently.".into(),
            }
        }
        "skill_view" => {
            let name = s(args, "name")?;
            match ctx.db.skills_active(&scope, ctx.bot_id)?.into_iter().find(|k| k.name.eq_ignore_ascii_case(name.trim())) {
                Some(k) => {
                    // Each use is counted: how skills earn their keep, and which run to look at when one goes wrong.
                    ctx.db.skill_used(k.id, ctx.run_id)?;
                    // Taught from a link, a file or a SKILL.md: its words came from outside, even though you accepted it.
                    if k.kind == "taught" {
                        ctx.taint();
                    }
                    format!("{}\n{}", k.name, skill_part(&k.body, args["section"].as_str()))
                }
                None => "no skill by that name".into(),
            }
        }
        "memory_forget" => {
            if ctx.db.forget(&scope, i(args, "id")?)? { "forgotten".into() } else { "no such memory".into() }
        }
        "routine_create" => {
            let quiet = args.get("quiet").and_then(Value::as_bool).unwrap_or(false);
            // For someone else in the household: in their own chat. (Getting here means you were asked, or had been.)
            let to = remind_target(ctx, args)?;
            let chat = to.as_ref().map_or(ctx.chat_id, |t| t.0);
            if let Some((to, _)) = &to {
                ctx.db.set_setting(&format!("remind_ok:{to}"), "1")?;
            }
            let r = match args["at"].as_str().filter(|a| !a.trim().is_empty()) {
                Some(at) => ctx.db.routine_once(ctx.bot_id, chat, local_time(at)?, s(args, "prompt")?, quiet)?,
                None => ctx.db.routine_create(ctx.bot_id, chat, s(args, "cron")?, s(args, "prompt")?, quiet)?,
            };
            if ctx.is_tainted() {
                ctx.db.routine_taint(r.id)?;
            }
            ctx.mark_routine(r.id)?;
            if crate::events::on(&r.cron)?.is_some() && !args["keep"].as_bool().unwrap_or(false) {
                ctx.db.routine_set_once(r.id)?;
            }
            if let Some(w) = args["watch"].as_str().filter(|w| quiet && !w.trim().is_empty()) {
                ctx.db.routine_set_watch(ctx.chat_id, r.id, w)?;
            }
            // An end it can't have (its first run would be after it): no routine, rather than one that runs on forever.
            if let Some(u) = args["until"].as_str().filter(|u| !r.once && !u.trim().is_empty())
                && let Err(e) = until_time(u).and_then(|t| ctx.db.routine_set_ends(chat, r.id, t))
            {
                ctx.db.routine_delete(chat, r.id)?;
                return Err(e);
            }
            let followup = r.once && args["followup"].as_bool() == Some(true);
            if followup {
                ctx.db.routine_set_followup(r.id)?;
            }
            if to.is_none() && followup {
                ctx.db.goal_claim_routine(r.id, chat)?;
            }
            let for_goal = if ctx.db.routine(r.id)?.is_some_and(|r| r.for_goal) { " (for the goal)" } else { "" };
            if let Some((_, name)) = &to {
                return Ok(format!("#{} set for {name}: it runs in their own chat, {}", r.id, if r.once { format!("once, {}", next_run(&r)) } else { format!("next {}", next_run(&r)) }));
            }
            if followup {
                return Ok(format!("follow-up #{}{for_goal} booked; runs once, {}. Tell the person when you'll check back, and that they can cancel it.", r.id, next_run(&r)));
            }
            let until = args["until"].as_str().filter(|u| !r.once && !u.trim().is_empty()).map(|u| format!(", until {u}")).unwrap_or_default();
            format!("{} #{} created; {} {}{until}", if quiet { "quiet check" } else if r.once { "one-off" } else { "routine" }, r.id, if r.once { "runs once," } else { "next run" }, next_run(&r))
        }
        "routine_list" => {
            let v = ctx.db.routines(ctx.chat_id)?;
            if v.is_empty() {
                "no routines".into()
            } else {
                v.iter()
                    .map(|r| format!("#{}{} [{}]{}{}{}{} — {}", r.id, if r.for_goal { " (for the goal)" } else { "" }, if r.followup { "follow-up" } else if r.once { "once" } else { &r.cron }, if r.ends > 0 { format!(" until {}", fmt_ts(r.ends)) } else { String::new() }, if r.quiet { " quiet" } else { "" }, if r.watch.is_empty() { String::new() } else { format!(" watching {}", r.watch) }, if r.paused || ctx.db.paused(r.bot_id, crate::db::now()).ok().flatten().is_some() { " paused".into() } else if r.goal_held { " paused (goal)".into() } else { format!(" next {}", next_run(r)) }, r.prompt))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        "card_read" => {
            let v = ctx.db.cards(ctx.chat_id)?;
            // A card a page-reading run wrote holds someone else's words.
            if v.iter().any(|k| k.tainted) {
                ctx.taint();
            }
            if v.is_empty() {
                "no cards".into()
            } else {
                v.iter().map(|k| format!("## {}\n({}, {})\n{}", k.title, if k.by_you { "last changed by the person" } else { "last written by a bot" }, fmt_ts(k.updated_at), k.body)).collect::<Vec<_>>().join("\n\n")
            }
        }
        "goal_set" => {
            let budget = args["budget"].as_i64().unwrap_or(0);
            let g = ctx.db.goal_set(ctx.bot_id, ctx.chat_id, s(args, "goal")?, s(args, "done_when")?, budget, ctx.is_tainted())?;
            format!("Goal set: {} (done when {}). The person sees it pinned in this chat. Keep at it: book follow-ups for what you're waiting on, and goal_update as it moves.", g.text, g.done_when)
        }
        "goal_update" => {
            let progress = args["progress"].as_str().unwrap_or("");
            match args["status"].as_str() {
                Some("done") => {
                    let g = ctx.db.goal(ctx.chat_id)?.context("this chat has no goal")?;
                    if !["active", "blocked"].contains(&g.status.as_str()) {
                        bail!("the goal is {}; only the person can change it now", g.status);
                    }
                    // The log is the truth: done only on something it shows was done since the goal was set.
                    let backed = crate::receipts::backing(&ctx.db.tool_calls_since(ctx.chat_id, g.created_at)?);
                    if backed.is_empty() {
                        bail!("not marked done: this chat's run log shows nothing done towards it yet (no email sent, nothing added to the calendar). Say what's left, or set it blocked and ask the person; they can mark it done themselves");
                    }
                    ctx.db.goal_progress(ctx.chat_id, &truncate(format!("{} ✓ {}", progress.trim(), backed.join("; ")).trim(), 300), None, "", ctx.is_tainted())?;
                    crate::runtime::stop_handed(&ctx.db.goal_status(ctx.chat_id, "done", "")?);
                    format!("Goal done, on: {}. Its follow-ups and routines are removed. Tell the person.", backed.join("; "))
                }
                status => {
                    let g = ctx.db.goal_progress(ctx.chat_id, progress, status, args["why"].as_str().unwrap_or(""), ctx.is_tainted())?;
                    format!("Goal {}: {}", g.status, g.progress)
                }
            }
        }
        "card_write" => {
            let k = ctx.db.card_write(ctx.bot_id, ctx.chat_id, s(args, "title")?, s(args, "body")?, ctx.is_tainted())?;
            format!("card \"{}\" updated; the person sees it pinned in this chat", k.title)
        }
        "routine_delete" => {
            if ctx.db.routine_delete(ctx.chat_id, i(args, "id")?)? { "deleted".into() } else { "no such routine".into() }
        }
        "routine_update" => {
            let id = i(args, "id")?;
            let r = ctx.db.routines(ctx.chat_id)?.into_iter().find(|r| r.id == id).context("no such routine in this chat")?;
            let quiet = args["quiet"].as_bool().unwrap_or(r.quiet);
            let prompt = args["prompt"].as_str().unwrap_or(&r.prompt);
            if r.once && args["cron"].is_string() {
                bail!("a one-off has a time, not a schedule: delete it and create a new one");
            }
            // The end with the schedule: checked together, so a refused change leaves both as they were.
            let ends = match args["until"].as_str().map(str::trim) {
                Some("") => Some(0),
                Some(u) => Some(until_time(u)?),
                None => None,
            };
            ctx.db.routine_update(ctx.chat_id, id, args["cron"].as_str().unwrap_or(&r.cron), prompt, quiet, r.paused, ends)?;
            // Watching only makes sense for a quiet check: one that stops being quiet stops watching.
            let watch = if quiet { args["watch"].as_str().unwrap_or(&r.watch) } else { "" };
            ctx.db.routine_set_watch(ctx.chat_id, id, watch)?;
            // Rewritten after reading outside content: its words may be someone else's, like a routine made then.
            if ctx.is_tainted() {
                ctx.db.routine_taint(id)?;
            }
            ctx.mark_routine(id)?;
            let r = ctx.db.routines(ctx.chat_id)?.into_iter().find(|r| r.id == id).context("no such routine")?;
            format!("routine #{id} updated; {} {}: {}", if r.once { "runs once," } else { "next run" }, next_run(&r), r.prompt)
        }
        "web_fetch" => {
            let url = s(args, "url")?;
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                bail!("only http(s) urls");
            }
            if is_private_host(url) {
                bail!("refusing to fetch local/private addresses");
            }
            ctx.taint();
            let body = super::capped_text(ctx.http.get(url).send().await?.error_for_status()?, super::PAGE_MAX).await?;
            truncate(&strip_html(&body), 15_000)
        }
        "bot_create" => {
            let bot = new_bot(ctx, args)?;
            let id = ctx.db.bot_save(&bot)?;
            let n = bot.tools.split(',').filter(|t| !t.is_empty()).count();
            format!("Created {} (bot #{id}) on your AI account, with {n} tools. Hand it tasks with ask_bot; the user can chat with it in the app and connect a messaging app there.", bot.name)
        }
        "ask_bot" => {
            let (who, task) = (s(args, "bot")?.trim(), s(args, "task")?);
            let bots = ctx.db.bots()?;
            let target = bots.iter().find(|b| b.name.eq_ignore_ascii_case(who) || b.id.to_string() == who).with_context(|| {
                format!("no bot called {who}; the team is: {}", bots.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join(", "))
            })?;
            if target.id == ctx.bot_id {
                bail!("{who} is you; do the task yourself");
            }
            if ctx.db.paused(target.id, crate::db::now())?.is_some() {
                bail!("{} is paused by the user; do the task yourself or say it has to wait", target.name);
            }
            // Its own chat is the person's app chat: what it reads and writes there may only flow back to another chat of
            // theirs alone. A rehearsal posts nothing, so it runs the usual way.
            let own = args["in_own_chat"].as_bool() == Some(true) && !ctx.rehearsing();
            if own && ctx.chat_id != crate::host::LOCAL_CHAT && !crate::runtime::decisions_ok(&ctx.db, ctx.chat_id) {
                bail!("in_own_chat only works from the person's own private chat; hand it over without it");
            }
            // Later: it runs on its own (the asking bot's scheduler starts it) and reports back. A rehearsal waits for it,
            // so nothing outlives the rehearsal.
            if args["later"].as_bool() == Some(true) && !ctx.rehearsing() {
                let d = crate::db::Delegation {
                    bot_id: ctx.bot_id,
                    to_bot: target.id,
                    chat_id: ctx.chat_id,
                    conv: ctx.conv,
                    run_id: ctx.run_id,
                    task: task.to_string(),
                    unattended: ctx.unattended,
                    tainted: ctx.is_tainted(),
                    ..Default::default()
                };
                let id = ctx.db.delegation_add(&d)?;
                ctx.db.goal_claim_delegation(id, ctx.chat_id)?;
                // Read your email or a meeting: the helper's web addresses ask too, though it runs later.
                if ctx.saw_private() {
                    ctx.db.set_setting(&delegation_private_key(id), &ctx.private_value())?;
                }
                if own {
                    ctx.db.set_setting(&delegation_own_key(id), "1")?;
                    return Ok(format!("Handed to {}; it works in its own chat. A link to its answer comes back to you here when it's done; tell the person then.", target.name));
                }
                return Ok(format!("Handed to {}. Its result comes back to you in this chat when it's done; tell the person then.", target.name));
            }
            let reply = delegate(ctx, target, task, own).await?;
            if own {
                return own_chat_answer(&ctx.db, target, &reply);
            }
            format!("{} replied:\n{}", target.name, truncate(&reply, 6000))
        }
        _ => bail!("unknown tool {name}"),
    })
}

/// `target`'s turn on a task from the bot of `ctx`, one level down (no chains), under the asking run.
/// `own`: in the target's own app chat, where the task and its answer are kept and its files go, instead of the asker's.
pub(crate) async fn delegate(ctx: &ToolCtx, target: &crate::db::Bot, task: &str, own: bool) -> Result<String> {
    let from = ctx.db.bot(ctx.bot_id)?.name;
    let provider = crate::provider::Provider::for_bot(target, ctx.db.dir())?;
    let chat = if own { crate::host::LOCAL_CHAT } else { ctx.chat_id };
    // Carries the asking run's id into turn(), which links the delegate's run to it (then replaces it).
    let mut sub = ToolCtx::new(ctx.db.clone(), target.id, chat).with_run(ctx.run_id);
    // Taint flows both ways: a tainted caller hands it on, and a delegate that read the web taints the caller.
    sub.tainted = ctx.tainted.clone();
    sub.private = ctx.private.clone();
    sub.sensitive = ctx.sensitive.clone();
    sub.depth = ctx.depth + 1;
    // Inside a routine, the helper's asks wait in the inbox too, rather than sending a live card.
    sub.unattended = ctx.unattended;
    // A rehearsal stays one in the helper.
    sub.rehearsal = ctx.rehearsing();
    // Its asks come from the bot the person is talking to, in the chat they asked from (even when it works in its own
    // chat: the person may be on Telegram), naming the helper (gate.rs, for_helper).
    sub.via_bot = Some(ctx.via_bot.unwrap_or(ctx.bot_id));
    sub.ask_chat = ctx.ask_chat;
    // Boxed: the handed-over turn may itself call tools::call (async recursion).
    let reply = Box::pin(crate::turn(&provider, &sub, crate::Trigger::Delegated { from: &from, task, own })).await?;
    if ctx.db.delegated_tainted(target.id, chat, ctx.run_id).unwrap_or(false) {
        ctx.taint(); // the delegate ran in its own process (CLI providers): its taint is in the run log
    }
    Ok(reply)
}

/// What the asking bot hears of a task done in the helper's own chat: a link to it there, and how it starts.
pub(crate) fn own_chat_answer(db: &Db, target: &crate::db::Bot, reply: &str) -> Result<String> {
    let link = match db.setting("public_url")? {
        Some(u) => format!("{}/#bot-{}", u.trim_end_matches('/'), target.id),
        None => format!("in Mimir, open {}", target.name),
    };
    Ok(format!(
        "{} did it in its own chat, where its whole answer is kept: {link}\nTell the person in one line, with that link; don't repeat its answer. It starts:\n{}",
        target.name,
        truncate(reply, 600)
    ))
}

/// One computer action, then a screenshot of what it led to.
/// One user of the computer at a time: this run holds it while it works; another bot waits a little; the person
/// taking over in the app always wins (bots wait until they give it back).
/// The bot may have been sent to the page it's on by someone other than the person: the conversation read outside
/// content, or the run is on words that aren't theirs (another bot's task, a document).
pub(super) fn steered(ctx: &ToolCtx) -> bool {
    ctx.is_tainted() || ctx.run_id.and_then(|r| ctx.db.brief(r).ok().flatten()).is_some_and(|b| !crate::own_words(&b))
}

/// Sign in to the page on the computer's screen without the AI seeing the login: a login this bot saved for the site
/// is typed by Mimir (after an OK, like every saved login); otherwise the person gets a card with a link to a masked
/// form in Mimir, and Mimir types what they enter there and keeps it as this bot's login for the site. The model only
/// ever hears "signed in" or "failed".
async fn computer_signin(ctx: &ToolCtx) -> Result<String> {
    hold_computer(ctx).await?;
    let (home, bot) = (ctx.db.dir(), ctx.bot_id);
    tokio::task::spawn_blocking(move || crate::computer::start(&home, Some(bot))).await??;
    let form = crate::computer::sign_in_form().await.context("couldn't read the page on the computer; take a screenshot and try again")?;
    let url = form["url"].as_str().unwrap_or("").to_string();
    if !(form["password"] == true || form["code"] == true || form["user"] == true) {
        bail!("there's no sign-in form on the screen; open the site's sign-in page first (or use computer_handoff)");
    }
    let site = crate::ext::site_of(&url).context("this page has no website address to keep a login for; use computer_handoff")?;
    // A saved login the site turned down last time isn't tried again: the person gets a card, and what they sign in
    // with there replaces it.
    let stale = crate::ext::vault_for(&ctx.db, ctx.bot_id).into_iter().find(|e| e.bot == Some(ctx.bot_id) && crate::ext::host_allowed(&url, &e.domains) && crate::ext::login_failed(&ctx.db, &e.name));
    if let Some(e) = crate::ext::signin_login(&ctx.db, ctx.bot_id, &url) {
        // Always on the card: this site's saved login is typed without asking from then on.
        if !crate::tools::trusted(&ctx.db, "computer_signin", Some(&site)) {
            crate::tools::wait_for_approval_with(ctx, "computer_signin", &Value::Null, &format!("Use saved login {} on {url}", e.name), "login", Some(&site)).await?;
        }
        // The OK can take longer than the computer's lease: take it again, and type only into this bot's own browser
        // (sign_in checks), so another bot that used the computer meanwhile never gets this login.
        hold_computer(ctx).await?;
        let get = |k: &str| crate::host::secret_get(&format!("vault:{}{k}", e.name)).unwrap_or_default();
        let login = crate::computer::Login { user: get(":user"), password: get(""), code: String::new(), totp: crate::totp::parse(&get(":totp")).ok() };
        if crate::computer::sign_in(&ctx.db.dir(), ctx.bot_id, &login, &e.domains).await? {
            return Ok(format!("Signed in to {site} with your saved login. Take a screenshot to see the page, then carry on."));
        }
        crate::ext::login_failed_set(&ctx.db, &e.name)?;
        return Ok(format!("Signing in to {site} with the saved login failed (it may be out of date). Call computer_signin again: the person gets a sign-in card this time."));
    }
    if ctx.unattended {
        bail!("there's no saved login for {site} yet, and nobody is watching this run to sign in; use computer_handoff");
    }
    // A card comes from the bot they talk to, but the login is this one's (its browser profile): not mixed up.
    // ponytail: a delegated task asks to take over instead; keep the computer's bot on the card if that's wanted.
    if ctx.via_bot.is_some_and(|v| v != ctx.bot_id) {
        bail!("there's no saved login for {site} yet; in a task handed to you, ask the person to take over with computer_handoff");
    }
    let link = match ctx.db.setting("public_url")? {
        Some(u) => format!("{}/#signin", u.trim_end_matches('/')),
        None => "Open Mimir, Needs you".into(),
    };
    // Phishing: a bot steered by a page or a message can open a look-alike sign-in. The card says so, with the full
    // address, before anyone types a password.
    let steered = steered(ctx);
    let card = crate::ext::signin_warnings(&ctx.db, &url, &site, steered);
    let host = card["host"].as_str().unwrap_or(&site).to_string();
    let like = card["lookalike"].as_str().map(|k| format!(", made to look like {k}")).unwrap_or_default();
    let check = match (steered, card["lookalike"].as_str()) {
        _ if card["foreign"] == true => format!("\n\n⚠️ This address uses letters from another alphabet{like}: {host}"),
        (_, Some(k)) => format!("\n\n⚠️ This isn't {k}, where you've signed in before. Check the address: {host}"),
        (true, None) => format!("\n\n⚠️ I came to this site after reading a web page or a message. Check the address: {host}"),
        _ => String::new(),
    };
    let again = if stale.is_some() { "\n\nThe login saved for it didn't work last time; this one replaces it." } else { "" };
    let summary = format!("Sign in to {site} ({host}){check}{again}\n\n{link}: type your login there, never in this chat. Mimir types it into the page and keeps it for this bot on {site}.");
    let id = ctx.db.approval_create(ctx.bot_id, ctx.chat_id, "computer_signin", &summary, HANDOFF_SECS, "signin", Some(&site))?;
    ctx.db.signin_card_set(id, &card)?;
    // Filled in from the form (host.rs, signin_submit): approved = signed in. Held meanwhile, so no other bot's profile
    // takes the screen; and a fill that never reports back still ends.
    let who = ctx.run_id.map(|r| format!("run:{r}")).unwrap_or_else(|| format!("bot:{}", ctx.bot_id));
    let until = crate::db::now() + HANDOFF_SECS + 120;
    loop {
        match ctx.db.approval_status(id)?.as_str() {
            "approved" => return Ok(format!("Signed in to {site}. Take a screenshot to see the page, then carry on.")),
            "denied" => bail!("the sign-in to {site} failed, or they couldn't do it now. Stop here and tell them; computer_handoff lets them do it on the screen."),
            "expired" => bail!("nobody signed in to {site} in 10 minutes. Stop here and tell them what's needed."),
            _ if crate::db::now() > until => bail!("the sign-in to {site} didn't finish"),
            _ => {}
        }
        let _ = ctx.db.computer_lease(&who, "A bot signing in", 120)?;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

pub(crate) async fn hold_computer(ctx: &ToolCtx) -> Result<()> {
    let who = ctx.run_id.map(|r| format!("run:{r}")).unwrap_or_else(|| format!("bot:{}", ctx.bot_id));
    let name = ctx.db.bot(ctx.bot_id).map(|b| b.name).unwrap_or_else(|_| "A bot".into());
    let mut waited = 0;
    while let Some((holder, holder_name)) = ctx.db.computer_lease(&who, &name, 120)? {
        if holder == "you" {
            bail!("the person has taken over the computer right now; wait, or ask them to give it back (Computer → Give back)");
        }
        if waited >= 30 {
            bail!("the computer is busy: {holder_name} is using it; try again in a minute");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        waited += 2;
    }
    Ok(())
}

fn computer_action(home: &std::path::Path, bot: i64, name: &str, args: &Value) -> Result<String> {
    use crate::computer as c;
    c::start(home, Some(bot))?;
    let int = |k: &str| args.get(k).and_then(Value::as_i64).with_context(|| format!("missing number '{k}'"));
    let done = match name {
        "computer_screenshot" => "The screen right now.".to_string(),
        "computer_click" => {
            c::click(int("x")?, int("y")?, args["button"].as_str().unwrap_or("left"), args["double"].as_bool().unwrap_or(false))?;
            "Clicked. The screen now:".into()
        }
        "computer_type" => {
            c::type_text(s(args, "text")?)?;
            "Typed. The screen now:".into()
        }
        "computer_key" => {
            c::key(s(args, "keys")?)?;
            "Pressed. The screen now:".into()
        }
        "computer_scroll" => {
            c::scroll(int("x")?, int("y")?, args["direction"].as_str() == Some("down"), args.get("amount").and_then(Value::as_i64).unwrap_or(5))?;
            "Scrolled. The screen now:".into()
        }
        "computer_open" => {
            c::open_url(home, bot, s(args, "url")?)?;
            std::thread::sleep(Duration::from_millis(1500));
            "Opened. The screen now:".into()
        }
        _ => bail!("unknown tool {name}"),
    };
    c::with_screenshot(home, &done)
}

/// A skill's steps, or one of its reference sections ("## Title" blocks after the steps). Without a section: the steps
/// and the titles of the sections, so a long skill taught from a document costs only what's needed.
pub fn skill_part(body: &str, section: Option<&str>) -> String {
    let mut parts = body.split("\n## ");
    let steps = parts.next().unwrap_or("").trim_end();
    let sections: Vec<(&str, &str)> = parts.map(|p| p.split_once('\n').unwrap_or((p, ""))).collect();
    match section.map(str::trim).filter(|s| !s.is_empty()) {
        Some(want) => match sections.iter().find(|(t, _)| t.trim().to_lowercase().contains(&want.to_lowercase())) {
            Some((t, text)) => format!("## {}\n{}", t.trim(), text.trim()),
            None => format!("no section like \"{want}\"; sections: {}", sections.iter().map(|s| s.0.trim()).collect::<Vec<_>>().join(", ")),
        },
        None if sections.is_empty() => steps.to_string(),
        None => format!("{steps}\n\nReference sections (read one with section): {}", sections.iter().map(|s| s.0.trim()).collect::<Vec<_>>().join(", ")),
    }
}

/// Text in something taught from outside that reads like an instruction to the assistant (the place a planted
/// prompt would hide): the phrase that matched, for the card.
pub fn instruction_like(text: &str) -> Option<String> {
    // Spacing, line breaks and invisible characters don't hide a phrase.
    let l = text
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '\u{00ad}' | '\u{200b}'..='\u{200f}' | '\u{2060}' | '\u{feff}'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    const PHRASES: [&str; 12] = [
        "ignore previous", "ignore all previous", "ignore your", "disregard", "system prompt", "you are now", "new instructions",
        "do not tell", "don't tell the user", "send it to", "forward all", "exfiltrat",
    ];
    PHRASES.iter().find(|p| l.contains(*p)).map(|p| p.to_string())
}
