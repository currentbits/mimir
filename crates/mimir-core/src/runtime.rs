//! The always-on runtime for one bot: channel listeners (Telegram, Slack, Discord, Signal) + routine
//! scheduler + approval cards. Shared by the CLI (`mimir serve`) and the desktop app, which
//! aborts/respawns it on settings change.

use crate::db::{Approval, Db};
use crate::say::{tr, trf};
use crate::{Trigger, discord::Discord, provider::Provider, signal::Signal, slack::Slack, telegram::Tg, tools::ToolCtx, turn};
use anyhow::Result;
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use tokio::task::{JoinHandle, JoinSet};

#[derive(Debug, Clone, Default, Serialize)]
pub struct ChannelStatus {
    pub kind: String,
    pub connected: bool,
    /// e.g. "@my_bot" or "mimir in Acme".
    pub handle: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub bot_id: i64,
    pub provider: String,
    pub pair_code: Option<String>,
    pub channels: Vec<ChannelStatus>,
    pub error: Option<String>,
}

pub type SharedStatus = Arc<std::sync::Mutex<Status>>;

/// Channel credentials for one bot. None = channel not used.
#[derive(Debug, Clone, Default)]
pub struct Channels {
    pub telegram: Option<String>,
    /// (bot token xoxb-…, app-level token xapp-… for Socket Mode)
    pub slack: Option<(String, String)>,
    pub discord: Option<String>,
    /// (signal-cli-rest-api address, "" for the default; the bot's own number)
    pub signal: Option<(String, String)>,
}

/// A connected messaging channel. Enum dispatch keeps the adapters plain structs.
#[derive(Clone)]
pub(crate) enum Chan {
    Telegram(Tg),
    Slack(Slack),
    Discord(Discord),
    Signal(Signal),
}

/// Telegram allows about one edit per second per chat, Discord five per five seconds: 1.5s suits all.
const EDIT_EVERY: Duration = Duration::from_millis(1500);
/// Drafts are a preview; Discord's 2000-char cap is the tightest.
const DRAFT_MAX: usize = 1800;

/// A reply streaming into one message while it's being written.
struct Follower {
    stop: Arc<Notify>,
    task: JoinHandle<Option<String>>,
}

impl Follower {
    /// Stop following; returns the draft message to replace, if one was posted.
    async fn finish(self) -> Option<String> {
        self.stop.notify_one();
        self.task.await.ok().flatten()
    }
}

impl Chan {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Chan::Telegram(_) => "telegram",
            Chan::Slack(_) => "slack",
            Chan::Discord(_) => "discord",
            Chan::Signal(_) => "signal",
        }
    }

    /// A passing message (a command's answer, a note): not kept if it doesn't go. What matters goes through `Core::say`.
    pub(crate) async fn send(&self, ext: &str, text: &str) {
        if let Err(e) = self.post(ext, None, text, &[]).await {
            eprintln!("{} send to {ext} failed: {e:#}", self.kind());
        }
    }

    /// A message, with buttons under it if any; replaces the streamed draft if there is one.
    async fn post(&self, ext: &str, draft: Option<&str>, text: &str, rows: &[Row]) -> Result<()> {
        let text = if text.trim().is_empty() { "(no reply)" } else { text };
        match self {
            Chan::Telegram(c) => c.finish(ext, draft, text, rows).await,
            Chan::Slack(c) => c.finish(ext, draft, text, rows).await,
            Chan::Discord(c) => c.finish(ext, draft, text, rows).await,
            Chan::Signal(c) => c.finish(ext, draft, text, rows).await,
        }
    }

    /// The files a reply named (`[[file:…]]` lines), uploaded after its text; one that fails is said in the chat.
    async fn attach(&self, ext: &str, db: &Db, chat: i64, files: &[String]) {
        for f in files {
            let Some((path, name)) = crate::files::sent_path(&db.dir(), chat, f) else { continue };
            let r = match std::fs::read(&path) {
                Ok(bytes) => match self {
                    Chan::Telegram(c) => c.upload(ext, &name, bytes).await,
                    Chan::Slack(c) => c.upload(ext, &name, bytes).await,
                    Chan::Discord(c) => c.upload(ext, &name, bytes).await,
                    Chan::Signal(c) => c.upload(ext, &name, bytes).await,
                },
                Err(e) => Err(e.into()),
            };
            if let Err(e) = r {
                eprintln!("{} upload to {ext} failed: {e:#}", self.kind());
                self.send(ext, &format!("I couldn't attach {name} ({}).", crate::tools::truncate(&format!("{e:#}"), 200))).await;
            }
        }
    }

    /// Stream a reply into one message: post the draft once text arrives, then edit it as it grows.
    fn follow(&self, bot: i64, chat_id: i64, ext: &str) -> Follower {
        let (chan, ext, stop) = (self.clone(), ext.to_string(), Arc::new(Notify::new()));
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            let (mut id, mut shown) = (None::<String>, String::new());
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(EDIT_EVERY) => {}
                    _ = stopped.notified() => return id,
                }
                let Some(draft) = crate::live::get(bot, chat_id).filter(|d| !d.trim().is_empty()) else { continue };
                // A long reply's draft stops growing at DRAFT_MAX; the final message has all of it.
                let head: String = draft.chars().take(DRAFT_MAX).collect();
                let text = if head.len() < draft.len() { format!("{}… ▍", head.trim_end()) } else { format!("{head} ▍") };
                if text == shown {
                    continue;
                }
                match &id {
                    None => id = chan.draft_post(&ext, &text).await,
                    Some(m) => chan.draft_edit(&ext, m, &text).await,
                }
                shown = text;
            }
        });
        Follower { stop, task }
    }

    async fn draft_post(&self, ext: &str, text: &str) -> Option<String> {
        match self {
            Chan::Telegram(c) => c.draft_post(ext, text).await,
            Chan::Slack(c) => c.draft_post(ext, text).await,
            Chan::Discord(c) => c.draft_post(ext, text).await,
            Chan::Signal(c) => c.draft_post(ext, text).await,
        }
    }

    async fn draft_edit(&self, ext: &str, id: &str, text: &str) {
        match self {
            Chan::Telegram(c) => c.draft_edit(ext, id, text).await,
            Chan::Slack(c) => c.draft_edit(ext, id, text).await,
            Chan::Discord(c) => c.draft_edit(ext, id, text).await,
            Chan::Signal(c) => c.draft_edit(ext, id, text).await,
        }
    }

    /// "typing…" indicator until the handle is aborted (Slack bots have none).
    fn typing(&self, ext: &str) -> Option<JoinHandle<()>> {
        match self {
            Chan::Telegram(c) => Some(c.typing(ext)),
            Chan::Discord(c) => Some(c.typing(ext)),
            Chan::Signal(c) => Some(c.typing(ext)),
            Chan::Slack(_) => None,
        }
    }

    async fn card(&self, ext: &str, a: &Approval) -> Result<Option<String>> {
        match self {
            Chan::Telegram(c) => c.card(ext, a).await,
            Chan::Slack(c) => c.card(ext, a).await,
            Chan::Discord(c) => c.card(ext, a).await,
            Chan::Signal(c) => c.card(ext, a).await,
        }
    }

    async fn listen(&self, core: Arc<Core>, slot: Slot) -> Result<()> {
        match self {
            Chan::Telegram(c) => c.listen(self, core, slot).await,
            Chan::Slack(c) => c.listen(self, core, slot).await,
            Chan::Discord(c) => c.listen(self, core, slot).await,
            Chan::Signal(c) => c.listen(self, core, slot).await,
        }
    }
}

/// A channel's entry in the shared status.
#[derive(Clone)]
pub(crate) struct Slot {
    status: SharedStatus,
    kind: &'static str,
}

impl Slot {
    fn update(&self, f: impl FnOnce(&mut ChannelStatus)) {
        let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = s.channels.iter_mut().find(|c| c.kind == self.kind) {
            f(c);
        }
    }
    pub(crate) fn connected(&self, handle: String) {
        eprintln!("{} connected as {handle}", self.kind);
        self.update(|c| {
            c.connected = true;
            c.handle = Some(handle);
            c.error = None;
        });
    }
    fn failed(&self, e: &anyhow::Error) {
        self.update(|c| {
            c.connected = false;
            c.error = Some(format!("{e:#}"));
        });
    }
}

/// What every channel listener shares.
pub(crate) struct Core {
    db: Db,
    bot: i64,
    provider: Arc<Provider>,
    /// The pairing code: good for one chat, then a new one is made (shown on the bot's page).
    code: std::sync::Mutex<String>,
    status: SharedStatus,
    /// Wrong pairing codes per chat in the last hour; after a few, that chat's tries are ignored until the hour is up.
    misses: std::sync::Mutex<std::collections::HashMap<String, (u32, i64)>>,
    /// One reply or routine at a time per chat (keeps its CLI --resume session serial); other chats don't wait.
    /// Telegram topics share their chat's lock, which is why live drafts, TURNS and runs stay keyed by chat while a
    /// topic has its own conversation.
    locks: std::sync::Mutex<std::collections::HashMap<i64, Arc<Mutex<()>>>>,
    /// Unpaired chats already told how to pair (once each per run, so a stranger can't make it chatty).
    greeted: std::sync::Mutex<std::collections::HashSet<String>>,
}

/// The replies being written in messaging-app chats, by (bot, chat): /stop, or Stop in the app's Runs list, cuts one off.
pub(crate) static TURNS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(i64, i64), tokio::task::AbortHandle>>> =
    std::sync::LazyLock::new(Default::default);

pub(crate) enum Command<'a> {
    Pair(&'a str),
    Start,
    Memory,
    /// "/why": what the last reply drew on.
    Why,
    Routines,
    New,
    /// "/stop", or a bare "stop" while a reply is being written.
    Stop(bool),
    /// "/undo N": take back the Nth memory the weekly review listed.
    Undo(usize),
    Chat,
}

impl Core {
    fn chat_lock(&self, chat: i64) -> Arc<Mutex<()>> {
        self.locks.lock().unwrap().entry(chat).or_default().clone()
    }

    /// This bot's skills per Telegram chat, as menu commands (name, description), for the "/" menu.
    pub(crate) fn skill_menus(&self) -> Vec<(i64, Vec<(String, String)>)> {
        self.db
            .skill_menus(self.bot)
            .unwrap_or_default()
            .into_iter()
            .map(|(chat, skills)| {
                let cmds = skills
                    .into_iter()
                    // A description up to 256 characters.
                    .filter_map(|(name, desc)| Some((crate::skill_menu_name(&name)?, format!("Skill: {desc}").chars().take(240).collect::<String>())))
                    // Telegram refuses a list with the same command twice ("Café" and "Cafe" both become /caf…).
                    .fold(Vec::<(String, String)>::new(), |mut v, c| {
                        if !v.iter().any(|(n, _)| *n == c.0) {
                            v.push(c);
                        }
                        v
                    });
                (chat, cmds)
            })
            .collect()
    }
}

/// Who sent a message or pressed a button: their id in that app, and whether it's a direct chat with the bot.
#[derive(Clone, Copy)]
pub(crate) struct Sender<'a> {
    pub id: &'a str,
    pub dm: bool,
}

/// `/why`: the memories, skills and pages the last reply used, with how to fix a memory that steered it wrong.
fn why(used: Option<crate::db::Used>) -> String {
    let Some(u) = used.filter(|u| !u.is_empty()) else {
        return tr("My last reply didn't use any memories, skills or pages.").into();
    };
    let mut out = vec![tr("My last reply used:").to_string()];
    for m in &u.memories {
        let gone = match m.status { "forgotten" => tr(" (forgotten since)"), "superseded" => tr(" (corrected since)"), _ => "" };
        out.push(format!("• #{} {}: {}{gone}", m.id, m.title, crate::tools::truncate(&m.body, 200)));
    }
    for (_, name) in &u.skills {
        out.push(trf("• skill: {name}", &[("name", name)]));
    }
    for p in &u.pages {
        out.push(format!("• {}", crate::tools::truncate(p, 200)));
    }
    if let Some(m) = u.memories.iter().find(|m| m.status == "active") {
        out.push(trf("\nIf a memory is wrong, tell me (\"forget #{id}\", or what's true now), or fix it in the app under Memory.", &[("id", &m.id.to_string())]));
    }
    out.join("\n")
}

/// Pairing works as "/pair CODE" or "pair CODE" (Slack reserves leading slashes).
pub(crate) fn parse_command(text: &str) -> Command<'_> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("/pair") || lower.starts_with("pair ") {
        return Command::Pair(t.trim_start_matches('/')[4..].trim());
    }
    // A t.me/<bot>?start=CODE link opens the chat and sends "/start CODE": pairing in one tap.
    if let Some(code) = lower.strip_prefix("/start ").map(str::trim).filter(|c| !c.is_empty()) {
        return Command::Pair(&t[t.len() - code.len()..]);
    }
    if let Some(n) = lower.strip_prefix("/undo").or_else(|| lower.strip_prefix("undo ")).and_then(|r| r.trim().parse().ok()) {
        return Command::Undo(n);
    }
    match lower.as_str() {
        "/start" | "/help" => Command::Start,
        "/memory" => Command::Memory,
        "/why" => Command::Why,
        "/routines" => Command::Routines,
        "/new" => Command::New,
        "/stop" => Command::Stop(true),
        "stop" => Command::Stop(false),
        _ => Command::Chat,
    }
}

/// Button payload "ap:<id>:y|n|a" → (approval id, approve, all like this for the rest of the reply).
/// What a button on a chat-app approval card means: "ap:<id>:<y|n|a|w>".
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Decision {
    Once,
    Deny,
    /// This, and anything else like it while the bot finishes this reply.
    RestOfRun,
    /// This, and from now on without asking (the tool, or the tool on this site).
    Always,
}

pub fn parse_decision(data: &str) -> Option<(i64, Decision)> {
    let (id, d) = data.strip_prefix("ap:")?.split_once(':')?;
    let d = match d {
        "y" => Decision::Once,
        "n" => Decision::Deny,
        "a" => Decision::RestOfRun,
        "w" => Decision::Always,
        _ => return None,
    };
    Some((id.parse().ok()?, d))
}

/// A chat-app approval card offers "Approve all like this" for plain rule asks from a live reply.
pub(crate) fn offers_all(a: &Approval) -> bool {
    a.kind == "rule" && !a.parked
}

/// …and "Always" for plain rule asks (not after outside content, not a saved login, not a plan), like the app.
pub(crate) fn offers_always(a: &Approval) -> bool {
    a.kind == "rule"
}

/// The buttons of a chat-app approval card, in rows: (label, button data "ap:<id>:<y|n|a|w>", or a link's address).
pub(crate) fn card_buttons(a: &Approval) -> Vec<Vec<(String, String)>> {
    let b = |label: String, d: char| (label, format!("ap:{}:{d}", a.id));
    if a.kind == "takeover" {
        return vec![vec![b(tr("Done").into(), 'y'), b(tr("Can't now").into(), 'n')]];
    }
    let mut rows = vec![vec![b(tr("Approve").into(), 'y'), b(tr("Deny").into(), 'n')]];
    // A card whose text you can change: a link to it in the app (its data is the address, not a decision).
    if let Some(link) = &a.edit_link {
        rows.push(vec![(tr("Edit in Mimir").into(), link.clone())]);
    }
    if offers_all(a) {
        rows.push(vec![b(tr("Approve all like this").into(), 'a')]);
    }
    if offers_always(a) {
        // Allowed a few times already: Always comes right after Approve / Deny.
        let at = if a.allowed_before >= 2 { 1 } else { rows.len() };
        rows.insert(at, vec![b(crate::tools::always_label(a), 'w')]);
    }
    rows
}

/// One row of buttons: (label, button data).
pub(crate) type Row = Vec<(String, String)>;

/// What the buttons under one message the bot sent are for, kept as setting "tap:<token>" until one is tapped (their
/// data is "tp:<token>:<n>"). A tap is handled here in code, with no AI turn, except a choice's answer, which is the
/// person's next message. None of them is ever an approval.
#[derive(serde::Serialize, serde::Deserialize)]
struct Tap {
    bot: i64,
    chat: i64,
    at: i64,
    #[serde(flatten)]
    what: TapFor,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
enum TapFor {
    /// ask_choice's question: its options, then Something else.
    Choice { question: String, options: Vec<String> },
    /// A one-off's result (a reminder): Done, Snooze 1h, Tomorrow.
    Remind { prompt: String, due: i64, tainted: bool },
    /// A routine's result: 👍, 👎, Less like this, Pause, For tomorrow…
    Routine { id: i64, prompt: String, said: String, outside: bool },
}

/// The buttons for `what`, kept so a tap can be told apart (empty when that can't be saved: the message goes without).
fn tap_rows(db: &Db, bot: i64, chat: i64, what: TapFor) -> Vec<Row> {
    let labels: Vec<Vec<String>> = match &what {
        TapFor::Choice { options, .. } => options.iter().map(|o| vec![o.clone()]).chain([vec![tr("Something else").into()]]).collect(),
        TapFor::Remind { .. } => vec![vec![tr("Done").into(), tr("Snooze 1h").into(), tr("Tomorrow").into()]],
        TapFor::Routine { .. } => vec![vec!["👍".into(), "👎".into(), tr("Less like this").into()], vec![tr("Pause").into(), tr("For tomorrow…").into()]],
    };
    let Ok(token) = crate::random_hex(4) else { return vec![] };
    let tap = Tap { bot, chat, at: crate::db::now(), what };
    if serde_json::to_string(&tap).map_err(anyhow::Error::from).and_then(|v| db.set_setting(&format!("tap:{token}"), &v)).is_err() {
        return vec![];
    }
    let mut n = 0;
    labels
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|label| {
                    n += 1;
                    (label, format!("tp:{token}:{}", n - 1))
                })
                .collect()
        })
        .collect()
}

/// The same clock time on a later day, at least a minute from now.
// ponytail: whole days of seconds, so it's an hour off across a clock change
fn next_day(due: i64, now: i64) -> i64 {
    let mut at = due + 86400;
    while at <= now + 60 {
        at += 86400;
    }
    at
}

fn clock(t: i64) -> String {
    chrono::TimeZone::timestamp_opt(&chrono::Local, t, 0).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()
}

/// The latest digest time (setting digest_time, "HH:MM", default 08:00) at or before `now`: today's, or yesterday's
/// if today's hasn't come yet. So a digest held back (quiet hours, the app closed) still goes out once it can.
pub(crate) fn digest_due_at(db: &Db, now: chrono::DateTime<chrono::Local>) -> i64 {
    use chrono::TimeZone;
    let hm = db.setting("digest_time").ok().flatten().unwrap_or_else(|| "08:00".into());
    let (h, m) = hm.split_once(':').and_then(|(h, m)| Some((h.parse().ok()?, m.parse().ok()?))).unwrap_or((8u32, 0u32));
    let at = |day: chrono::NaiveDate| {
        let t = day.and_hms_opt(h.min(23), m.min(59), 0).unwrap_or_default();
        chrono::Local.from_local_datetime(&t).earliest().map(|t| t.timestamp()).unwrap_or(0)
    };
    let today = at(now.date_naive());
    if today <= now.timestamp() { today } else { at(now.date_naive() - chrono::Days::new(1)) }
}

/// Inside quiet hours (setting quiet_hours = "HH:MM-HH:MM", may cross midnight; unset = never)? Nothing
/// proactive is sent then: routine results wait for the digest, the digest and weekly review wait until it ends,
/// and so do approval cards from routines. Replies to the person's own messages always go out.
pub fn quiet_now(db: &Db, now: chrono::DateTime<chrono::Local>) -> bool {
    use chrono::Timelike;
    let Some(spec) = db.setting("quiet_hours").ok().flatten() else { return false };
    let mins = |s: &str| s.trim().split_once(':').and_then(|(h, m)| Some(h.parse::<u32>().ok()? * 60 + m.parse::<u32>().ok()?));
    let Some((from, to)) = spec.split_once('-').and_then(|(a, b)| Some((mins(a)?, mins(b)?))) else { return false };
    let t = now.hour() * 60 + now.minute();
    if from <= to { (from..to).contains(&t) } else { t >= from || t < to }
}

/// A file attached to a Slack, Discord or Signal message, as the app describes it; `url` is "" when it can't be fetched.
pub(crate) struct File {
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub url: String,
    /// Recorded in the app as a voice message: the person's own words, unlike an audio file someone made.
    pub voice: bool,
}

/// The most a file may be for the bot to fetch it (Telegram's cap for bots, used for every app).
const MAX_FILE: u64 = 20_000_000;

/// Messages sent while Mimir was away are fetched on reconnect if they're at most this old.
pub(crate) const CATCH_UP_SECS: i64 = 86_400;

/// Replies this long after the message say sorry for the wait.
const LATE_SECS: i64 = 10 * 60;

/// "Sorry for the slow reply…" when a message is answered long after it was sent (the bots' computer was asleep).
pub(crate) fn late_note(sent: Option<i64>, now: i64) -> Option<String> {
    let d = now - sent?;
    if d < LATE_SECS {
        return None;
    }
    let ago = if d < 3600 {
        trf("{n} minutes ago", &[("n", &(d / 60).to_string())])
    } else if d < 7200 {
        tr("an hour ago").into()
    } else if d < 86400 {
        trf("{n} hours ago", &[("n", &(d / 3600).to_string())])
    } else if d < 172800 {
        tr("yesterday").into()
    } else {
        trf("{n} days ago", &[("n", &(d / 86400).to_string())])
    };
    Some(trf("Sorry for the slow reply: I was offline when you wrote {ago}.", &[("ago", &ago)]))
}

/// What a paused bot answers in a messaging app.
pub(crate) fn paused_note(until: i64) -> String {
    match chrono::TimeZone::timestamp_opt(&chrono::Local, until, 0).single().filter(|_| until > 0) {
        Some(t) => format!("⏸ I'm paused until {}, so I can't answer now. Send it again after that.", t.format("%a %-d %b %H:%M")),
        None => "⏸ I'm paused, so I can't answer now. Send it again once I'm back on in the Mimir app.".into(),
    }
}

/// Whether a chat can be the decisions chat: paired, private, yours (not a guest's), and in a messaging app.
pub(crate) fn decisions_ok(db: &Db, chat: i64) -> bool {
    chat != crate::host::LOCAL_CHAT && chat != crate::INCOGNITO_CHAT && db.is_paired(chat).unwrap_or(false) && db.chat_private(chat) && !db.chat_guest(chat)
}

/// The one chat (setting decisions_chat) that also gets the routines' asks, failed routines and memory suggestions
/// from every other chat. None if unset, or if that chat can't be it any more (unpaired, or now a guest's).
pub(crate) fn decisions_chat(db: &Db) -> Option<i64> {
    let chat = db.setting("decisions_chat").ok()??.parse().ok()?;
    decisions_ok(db, chat).then_some(chat)
}

/// A chat's name, for saying where something came from.
fn chat_name(db: &Db, chat: i64) -> String {
    db.chats().unwrap_or_default().into_iter().find(|c| c.chat_id == chat).map(|c| c.title).unwrap_or_else(|| format!("chat {chat}"))
}

/// `quiet_now` at the current local time.
pub fn quiet(db: &Db) -> bool {
    quiet_now(db, chrono::Local::now())
}

/// Where to edit a card's text in the app, for one you can edit: the web app's address (setting public_url, https only,
/// as Telegram refuses other links on buttons), opening Needs you.
fn edit_link(db: &Db, a: &Approval) -> Option<String> {
    let url = db.setting("public_url").ok()??;
    (a.draft.is_some() && url.starts_with("https://")).then(|| format!("{}/#approval-{}", url.trim_end_matches('/'), a.id))
}

pub(crate) fn card_text(a: &Approval) -> String {
    let mins = ((a.expires_at - crate::db::now()) / 60).max(1);
    let wait = if mins >= 120 { trf("{n} h", &[("n", &(mins / 60).to_string())]) } else { trf("{n} min", &[("n", &mins.to_string())]) };
    let then = if a.parked { tr("It runs when you approve.") } else { tr("No answer means deny.") };
    // Allowed the same thing a few times already: offer to stop asking (like the app).
    let again = if offers_always(a) && a.allowed_before >= 2 { format!("\n{}", trf("You've allowed this {n} times. Press Always to stop being asked.", &[("n", &a.allowed_before.to_string())])) } else { String::new() };
    if a.kind == "takeover" {
        return trf("🖐 I need you on the computer\n\n{summary}\n\nWaits {wait}.", &[("summary", &a.summary), ("wait", &wait)]);
    }
    // A reply to the card is a no, with why.
    trf("🔐 Approve this action?\n\n{summary}\n\nWaits {wait}. {then}", &[("summary", &a.summary), ("wait", &wait), ("then", then)]) + " " + tr("Reply to this message to say no and why.") + &again
}

pub(crate) fn verdict(a: &Approval) -> String {
    let outcome = match a.status.as_str() {
        "approved" if a.kind == "takeover" => tr("✅ Done, carrying on"),
        "expired" if a.kind == "takeover" => tr("⌛ Nobody took over in time"),
        _ if a.kind == "takeover" => tr("Not now"),
        "approved" => tr("✅ Approved"),
        "expired" => tr("⌛ No answer in time, so not done"),
        _ => tr("❌ Denied"),
    };
    match a.reason.as_deref().filter(|_| a.status == "denied") {
        Some(why) => format!("{outcome}: {}\n{}", a.summary, trf("Why: {why}", &[("why", why)])),
        None => format!("{outcome}: {}", a.summary),
    }
}

/// The card's text once decided, saying so when it won't ask again.
pub(crate) fn verdict_for(a: &Approval, d: Decision) -> String {
    match d {
        Decision::Always if a.status == "approved" => {
            let after = match a.site.as_deref() {
                Some(site) => trf("From now on this runs on {site} without asking (Settings → Permissions to change it).", &[("site", site)]),
                None => tr("From now on this runs without asking (Settings → Permissions to change it).").into(),
            };
            format!("{}\n{after}", verdict(a))
        }
        _ => verdict(a),
    }
}

impl Core {
    /// An inbound text message from any channel.
    /// A file it can't use yet (video, sticker, a spreadsheet…) in a paired direct chat: say so instead of dropping it.
    pub(crate) async fn incoming_media(&self, chan: &Chan, ext: &str) -> Result<()> {
        if self.is_paired(chan, ext)? {
            chan.send(ext, tr("I can read text, photos, voice notes, PDFs, Word and Excel files and text files, but not this kind of file yet. Could you describe it or send a photo?")).await;
        }
        Ok(())
    }

    /// A document in a paired chat: a PDF or text file goes to the bot as text (and the conversation counts as having
    /// read outside content); other kinds get the honest "can't read this yet".
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn incoming_document(self: &Arc<Self>, chan: &Chan, ext: &str, from: Sender<'_>, name: &str, mime: &str, bytes: &[u8], said: &str, sent: Option<i64>) -> Result<()> {
        let Some(chat) = self.db.chat_lookup(chan.kind(), ext)? else { return Ok(()) };
        if self.sender_ok(chat, from)? != Some(true) {
            return Ok(());
        }
        match crate::read_document_text(name, mime, bytes.to_vec()).await {
            None => self.incoming_media(chan, ext).await,
            Some(Err(e)) => {
                chan.send(ext, &format!("I {e}. Could you send it another way, or paste the part you need?")).await;
                Ok(())
            }
            // A scan: its pages go to the bot as photos.
            Some(Ok(t)) if t.split_whitespace().count() < 5 => {
                let pages = if crate::is_pdf(name, mime) { crate::read_pdf_pages(bytes.to_vec()).await } else { vec![] };
                if pages.is_empty() {
                    chan.send(ext, tr("That file has no text I can read (a scanned PDF?). Send photos of the pages instead.")).await;
                    return Ok(());
                }
                let saved = pages.iter().map(|p| self.save_media(p, "jpg")).collect::<Result<Vec<_>>>()?;
                self.db.conv_taint(self.bot, self.conv(chan, ext, chat)?)?;
                self.incoming_with(chan, ext, from, "", &crate::with_scan(said, name, saved.len()), saved, sent).await
            }
            Some(Ok(t)) => {
                self.db.conv_taint(self.bot, self.conv(chan, ext, chat)?)?;
                self.incoming_with(chan, ext, from, "", &crate::with_document(said, name, &t), vec![], sent).await
            }
        }
    }

    /// Files with a message (Slack, Discord, Signal): photos go to the bot's AI with the text, a voice clip is transcribed, a
    /// PDF or text file is read, anything else gets the honest "can't read this yet". Only the paired person's files
    /// are fetched; `get` downloads one from the app's own file host.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn incoming_files<F>(self: &Arc<Self>, chan: &Chan, ext: &str, from: Sender<'_>, text: &str, files: &[File], sent: Option<i64>, get: impl Fn(&File) -> F) -> Result<()>
    where
        F: std::future::Future<Output = Result<Vec<u8>>>,
    {
        let Some(chat) = self.db.chat_lookup(chan.kind(), ext)? else { return Ok(()) };
        if self.sender_ok(chat, from)? != Some(true) {
            return Ok(());
        }
        let usable: Vec<&File> = files.iter().filter(|f| !f.url.is_empty() && f.size <= MAX_FILE).collect();
        if usable.is_empty() && files.iter().any(|f| f.size > MAX_FILE) {
            chan.send(ext, tr("That file is over 20 MB, more than I take. Could you send a smaller one?")).await;
            return Ok(());
        }
        // Only files that can't be fetched (a Google Drive link shared in Slack): the words are the message.
        if usable.is_empty() && !text.trim().is_empty() {
            return self.incoming_with(chan, ext, from, "", text, vec![], sent).await;
        }
        // A file that won't come (a missing Slack scope, a hiccup) is said in the chat, not just logged.
        let fetch = async |f: &File| match get(f).await {
            Ok(b) => Some(b),
            Err(e) => {
                chan.send(ext, &trf("I couldn't fetch {name} ({error}).", &[("name", &f.name), ("error", &format!("{e:#}"))])).await;
                None
            }
        };
        let photos: Vec<&File> = usable.iter().filter(|f| f.mime == "image/jpeg" || f.mime == "image/png").take(4).copied().collect();
        if !photos.is_empty() {
            let mut saved = vec![];
            for f in photos {
                let Some(bytes) = fetch(f).await else { return Ok(()) };
                saved.push(self.save_media(&bytes, if f.mime == "image/png" { "png" } else { "jpg" })?);
            }
            let text = if text.trim().is_empty() { "(I sent you a photo.)" } else { text };
            return self.incoming_with(chan, ext, from, "", text, saved, sent).await;
        }
        if let Some(f) = usable.iter().find(|f| f.mime.starts_with("audio/")) {
            let Some(bytes) = fetch(f).await else { return Ok(()) };
            return self.incoming_voice(chan, ext, from, bytes, &f.name, (!f.voice).then_some("an audio file"), sent).await;
        }
        if let Some(f) = usable.iter().find(|f| crate::readable_document(&f.name, &f.mime)) {
            let Some(bytes) = fetch(f).await else { return Ok(()) };
            return self.incoming_document(chan, ext, from, &f.name, &f.mime, &bytes, text, sent).await;
        }
        self.incoming_media(chan, ext).await
    }

    /// A voice note: transcribed and shown back (so a mishearing is visible), then answered like typing. `outside` says
    /// what it is when it's someone else's words (a forwarded voice note, an audio file): outside content.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn incoming_voice(self: &Arc<Self>, chan: &Chan, ext: &str, from: Sender<'_>, bytes: Vec<u8>, name: &str, outside: Option<&str>, sent: Option<i64>) -> Result<()> {
        match crate::transcribe::voice_note(&self.db, bytes, name).await {
            Some(Ok(text)) if !text.is_empty() => {
                chan.send(ext, &trf("🎙️ I heard: “{text}”", &[("text", &text)])).await;
                if let Some(what) = outside {
                    let said = crate::with_document(&format!("(I sent you {what}.)"), what, &text);
                    return self.incoming_outside_with(chan, ext, from, "", &said, vec![], sent).await;
                }
                return self.incoming_with(chan, ext, from, "", &text, vec![], sent).await;
            }
            Some(Ok(_)) => chan.send(ext, tr("I couldn't make out any words in that voice note.")).await,
            Some(Err(e)) => chan.send(ext, &trf("I couldn't transcribe that voice note ({error}). Could you type it?", &[("error", &format!("{e:#}"))])).await,
            None => chan.send(ext, tr("To understand voice notes I need whisper.cpp where Mimir runs, or an OpenAI API key (Settings → AI keys in Mimir). Until then, could you type it?")).await,
        }
        Ok(())
    }

    /// The paired chats on this kind of app: (its id there, a direct chat?).
    pub(crate) fn paired_on(&self, chan: &Chan) -> Vec<(String, bool)> {
        self.db.chats_on(chan.kind()).unwrap_or_default()
    }

    /// The last message seen in a chat, in the app's own order (Slack's ts in microseconds, Discord's message id), so
    /// a reconnect can fetch what came while Mimir was away.
    pub(crate) fn seen(&self, chan: &Chan, ext: &str) -> Option<u128> {
        self.db.setting(&format!("seen:{}:{}:{ext}", self.bot, chan.kind())).ok().flatten()?.parse().ok()
    }

    /// Mark a message in a paired chat seen; false if it (or a later one) already was, so nothing is answered twice.
    pub(crate) fn mark_seen(&self, chan: &Chan, ext: &str, at: u128) -> bool {
        if !self.is_paired(chan, ext).unwrap_or(false) {
            return true;
        }
        if self.seen(chan, ext).is_some_and(|s| s >= at) {
            return false;
        }
        let _ = self.db.set_setting(&format!("seen:{}:{}:{ext}", self.bot, chan.kind()), &at.to_string());
        true
    }

    /// Is this the person this paired chat answers? (For bookkeeping that should only follow them, like which message
    /// to reply under.)
    pub(crate) fn is_owner(&self, chan: &Chan, ext: &str, from: &str) -> bool {
        self.db.chat_lookup(chan.kind(), ext).ok().flatten().is_some_and(|chat| self.db.chat_owner(chat).is_ok_and(|o| !o.is_empty() && o == from))
    }

    /// Whether each topic in a private Telegram chat is its own conversation (opt-in: the Bot API for it still shifts).
    pub(crate) fn topics(&self) -> bool {
        self.db.setting(&format!("telegram_topics:{}", self.bot)).ok().flatten().as_deref() == Some("on")
    }

    /// The conversation a message is in: the chat's own, or (Telegram "chat:topic") the topic's.
    fn conv(&self, chan: &Chan, ext: &str, chat: i64) -> Result<i64> {
        match ext.split_once(':').filter(|_| chan.kind() == "telegram").and_then(|(_, t)| t.parse().ok()) {
            Some(topic) => self.db.topic_conv(chat, topic),
            None => Ok(chat),
        }
    }

    pub(crate) fn is_paired(&self, chan: &Chan, ext: &str) -> Result<bool> {
        Ok(self.db.chat_lookup(chan.kind(), ext)?.is_some())
    }

    /// Keep a file the person sent, under <home>/media (the only folder the AI may read files from). Startup deletes
    /// what's older than 60 days (`Host::new`).
    pub(crate) fn save_media(&self, bytes: &[u8], ext: &str) -> Result<std::path::PathBuf> {
        let dir = self.db.dir().join("media");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}-{}.{ext}", crate::db::now(), bytes.len()));
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    /// `sent`: when the person sent it (unix seconds), if the channel says; a reply to an old message says sorry.
    pub(crate) async fn incoming(self: &Arc<Self>, chan: &Chan, ext: &str, from: Sender<'_>, title: &str, text: &str, sent: Option<i64>) -> Result<()> {
        self.incoming_with(chan, ext, from, title, text, vec![], sent).await
    }

    /// A message holding someone else's words (a forward): the conversation counts as having read outside content.
    pub(crate) async fn incoming_outside(self: &Arc<Self>, chan: &Chan, ext: &str, from: Sender<'_>, title: &str, text: &str, sent: Option<i64>) -> Result<()> {
        self.incoming_outside_with(chan, ext, from, title, text, vec![], sent).await
    }

    /// The same, with photos (a forwarded photo, a forwarded voice note's words).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn incoming_outside_with(self: &Arc<Self>, chan: &Chan, ext: &str, from: Sender<'_>, title: &str, text: &str, images: Vec<std::path::PathBuf>, sent: Option<i64>) -> Result<()> {
        if let Some(chat) = self.db.chat_lookup(chan.kind(), ext)?
            && self.sender_ok(chat, from)? == Some(true)
        {
            self.db.conv_taint(self.bot, self.conv(chan, ext, chat)?)?;
        }
        self.incoming_with(chan, ext, from, title, text, images, sent).await
    }

    /// Whether this person may talk to the bot in this paired chat: the person who paired it, and in a family group
    /// everyone in the household (only the person who paired it presses its buttons: see `decide`). In
    /// a group, channel or server everyone else is ignored. A chat paired before Mimir kept who paired it has no owner:
    /// in a direct chat the person writing is the owner; elsewhere (None) it has to be paired again.
    fn sender_ok(&self, chat: i64, from: Sender<'_>) -> Result<Option<bool>> {
        let owner = self.db.chat_owner(chat)?;
        if owner.is_empty() {
            if !from.dm || from.id.is_empty() {
                return Ok(None);
            }
            self.db.chat_set_owner(chat, from.id)?;
            self.db.chat_set_dm(chat, true)?;
            return Ok(Some(true));
        }
        Ok(Some(owner == from.id || self.db.family_member(chat, from.id)?))
    }

    /// A message, possibly with images (photos the person sent).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn incoming_with(self: &Arc<Self>, chan: &Chan, ext: &str, from: Sender<'_>, title: &str, text: &str, images: Vec<std::path::PathBuf>, sent: Option<i64>) -> Result<()> {
        let kind = chan.kind();
        if let Command::Pair(given) = parse_command(text) {
            // Wrong codes count per person, so someone else in a group can't lock the owner out.
            let who = format!("{kind}:{ext}:{}", from.id);
            // Telegram's shared senders (an anonymous group admin, a channel, its service account) are many people
            // behind one id: a chat paired by them would answer all of them.
            if from.id.is_empty() || (kind == "telegram" && ["1087968824", "136817688", "777000"].contains(&from.id)) {
                chan.send(ext, tr("I can only be paired by a person writing as themselves (not anonymously or as a channel).")).await;
                return Ok(());
            }
            let now = crate::db::now();
            let tries = self.misses.lock().unwrap().get(&who).filter(|(_, at)| now - at < 3600).map_or(0, |(n, _)| *n);
            if tries >= MAX_PAIR_TRIES {
                return Ok(());
            }
            let right = {
                let mut code = self.code.lock().unwrap();
                let right = !given.is_empty() && given.eq_ignore_ascii_case(&code);
                if right {
                    // Used up: a code someone saw (in a screenshot, over a shoulder) can't pair another chat.
                    *code = pairing_code()?;
                    self.status.lock().unwrap_or_else(|e| e.into_inner()).pair_code = Some(code.clone());
                    eprintln!("bot #{} paired a chat; the next pairing code is {code}", self.bot);
                }
                right
            };
            if right {
                self.misses.lock().unwrap().remove(&who);
                // Someone other than you (a partner you gave the code to): a guest, with nothing of yours.
                let guest = self.db.pairs_as_guest(kind, from.id)?;
                // Who paired it is who it answers here (in a group, only them); and whether it's a direct chat.
                let chat = self.db.pair_external(kind, ext, title)?;
                self.db.chat_set_owner(chat, from.id)?;
                self.db.chat_set_dm(chat, from.dm)?;
                self.db.chat_set_guest(chat, guest)?;
                chan.send(ext, if guest {
                    tr("Paired ✅ as a guest: I keep your own memory and reminders here, and I can search the web for you. I can't see the email, calendar or meetings of the person who runs me. Commands: /memory, /routines, /new, /stop")
                } else {
                    tr("Paired ✅ Talk to me normally. Commands: /memory, /routines, /new, /stop, /undo, /why")
                }).await;
            } else {
                {
                    let mut misses = self.misses.lock().unwrap();
                    misses.retain(|_, (_, at)| now - *at < 3600);
                    misses.insert(who, (tries + 1, now));
                }
                chan.send(ext, if tries + 1 >= MAX_PAIR_TRIES { tr("Wrong pairing code. Too many tries: try again in an hour.") } else { tr("Wrong pairing code.") }).await;
            }
            return Ok(());
        }
        let Some(chat) = self.db.chat_lookup(kind, ext)? else {
            let first = {
                let mut greeted = self.greeted.lock().unwrap();
                if greeted.len() > 10_000 {
                    greeted.clear(); // ponytail: a bound, not an LRU; strangers may be greeted twice after this
                }
                greeted.insert(format!("{kind}:{ext}"))
            };
            if first {
                let name = self.db.bot(self.bot).map(|b| b.name).unwrap_or_else(|_| tr("this bot").into());
                chan.send(ext, &trf("Hi! I'm {name}, a private assistant. If you set me up, send pair followed by the code shown on my page in the Mimir app.", &[("name", &name)])).await;
            }
            return Ok(());
        };
        match self.sender_ok(chat, from)? {
            Some(true) => {
                // A direct chat paired before Mimir kept that: now it's known.
                if from.dm {
                    self.db.chat_set_dm(chat, true)?;
                }
            }
            // Someone else in a paired group: not a conversation with them.
            Some(false) => return Ok(()),
            None => {
                if self.greeted.lock().unwrap().insert(format!("{kind}:{ext}:repair")) {
                    chan.send(ext, tr("For your safety I now answer only the person who pairs me in a group. Whoever set me up: send pair followed by the code on my page in the Mimir app, here, once more.")).await;
                }
                return Ok(());
            }
        }
        let conv = self.conv(chan, ext, chat)?;
        let mut command = parse_command(text);
        if let Command::Stop(asked) = command {
            if self.stop(chan, ext, from)? {
                return Ok(());
            }
            if asked {
                chan.send(ext, tr("Nothing to stop: I'm not writing anything right now.")).await;
                return Ok(());
            }
            command = Command::Chat;
        }
        match command {
            Command::Start => chan.send(ext, tr("Hi! Just talk to me. /memory shows what I remember, /routines lists schedules, /why shows what my last reply used, /new starts a fresh conversation, /stop cuts off a reply I'm writing, /undo N takes back something from my weekly list of what I learned.")).await,
            Command::Memory => {
                // What bots keep in mind, then what's waiting for you to confirm (bots don't keep those in mind).
                let scope = format!("chat:{chat}");
                let mut lines: Vec<String> = self.db.bootstrap(&scope, self.bot)?.iter().map(|e| e.line()).collect();
                let unconfirmed: Vec<String> = self.db.search_unconfirmed(&scope, self.bot)?.iter().map(|e| e.line()).collect();
                if !unconfirmed.is_empty() {
                    lines.push(format!("\n{}\n{}", tr("Not confirmed yet (saved after reading outside content; check them in the app):"), unconfirmed.join("\n")));
                }
                chan.send(ext, &if lines.is_empty() { tr("Nothing remembered yet.").into() } else { lines.join("\n") }).await;
            }
            Command::Why => chan.send(ext, &why(self.db.last_reply_used(self.bot, chat)?)).await,
            Command::Routines => {
                let out = crate::tools::call(&ToolCtx::new(self.db.clone(), self.bot, chat), "routine_list", &json!({})).await?;
                chan.send(ext, &out).await;
            }
            Command::New => {
                if let Some(s) = self.db.session(self.bot, conv)? {
                    self.provider.forget_session(&s);
                }
                self.db.set_session(self.bot, conv, None)?;
                chan.send(ext, tr("Fresh conversation started (memories kept).")).await;
            }
            Command::Undo(n) => chan.send(ext, &crate::tidy::undo_learned(&self.db, chat, n)?).await,
            Command::Stop(_) => unreachable!("handled above"),
            Command::Pair(_) | Command::Chat => {
                // Paused by you: it says so instead of answering (the commands above still work).
                if let Some(until) = self.db.paused(self.bot, crate::db::now())? {
                    chan.send(ext, &paused_note(until)).await;
                    return Ok(());
                }
                // Measured on arrival, so a long reply (say, waiting for an approval) doesn't count as being offline.
                let late = late_note(sent, crate::db::now());
                // In a family group the bot is told who wrote each message.
                let text = match self.db.family_speaker(chat, from.id)? {
                    Some(who) => format!("{who}: {}", text.trim()),
                    None => text.trim().to_string(),
                };
                // A reply is being written here: this reaches it and changes it ("actually, only this week"), instead of
                // waiting its turn. Not a photo or file (only text can go in mid-run), nor while a routine holds the chat.
                if images.is_empty() && crate::live::steer(self.bot, chat, &text) {
                    return Ok(());
                }
                let (core, chan, ext) = (self.clone(), chan.clone(), ext.to_string());
                tokio::spawn(async move {
                    // Typing at once, even when this chat is still busy; a wait past a few seconds says why, once.
                    let typing = chan.typing(&ext);
                    let wait = core.chat_lock(chat).lock_owned();
                    tokio::pin!(wait);
                    let _g = tokio::select! {
                        g = &mut wait => g,
                        _ = tokio::time::sleep(BUSY_NOTE_AFTER) => {
                            let busy = TURNS.lock().unwrap().contains_key(&(core.bot, chat));
                            chan.send(&ext, if busy { "_Finishing my last reply first…_" } else { "_Finishing a routine first…_" }).await;
                            wait.await
                        }
                    };
                    // The reply shows up as it's written: one message, edited as it grows, then formatted.
                    let follower = chan.follow(core.bot, chat, &ext);
                    let mut ctx = ToolCtx::new(core.db.clone(), core.bot, chat);
                    ctx.conv = conv;
                    ctx.images = images;
                    let (provider, asked) = (core.provider.clone(), text.clone());
                    let task = tokio::spawn(async move { turn(&provider, &ctx, Trigger::Message(&asked)).await });
                    TURNS.lock().unwrap().insert((core.bot, chat), task.abort_handle());
                    let out = task.await;
                    TURNS.lock().unwrap().remove(&(core.bot, chat));
                    if let Some(t) = typing {
                        t.abort();
                    }
                    let choice = core.take_choice(chat);
                    let mut rows = vec![];
                    let reply = match out {
                        // ask_choice: the question and its buttons under the reply.
                        Ok(Ok(text)) => match choice {
                            Some((q, r)) => {
                                rows = r;
                                format!("{}\n\n**{q}**", text.trim_end()).trim_start().to_string()
                            }
                            None => text,
                        },
                        Ok(Err(e)) => crate::friendly_error(&format!("{e:#}")),
                        Err(e) if !e.is_cancelled() => crate::friendly_error(&e.to_string()),
                        // /stop: keep what was asked and written so far, like Stop in the app.
                        Err(_) => {
                            let partial = crate::live::get(core.bot, chat).unwrap_or_default();
                            let steered = crate::live::steered(core.bot, chat);
                            crate::live::end(core.bot, chat);
                            let note = if partial.trim().is_empty() { tr("_Stopped before replying._").to_string() } else { format!("{}\n\n{}", partial.trim_end(), tr("_Stopped._")) };
                            let logged = core
                                .db
                                .run_stop(core.bot, chat, &text)
                                .and_then(|_| std::iter::once(&text).chain(&steered).try_for_each(|t| core.db.log(core.bot, conv, "user", t, None)))
                                .and_then(|_| core.db.log(core.bot, conv, "assistant", &note, None));
                            if let Err(e) = logged {
                                eprintln!("stop: {e:#}");
                            }
                            note
                        }
                    };
                    // Written while this computer was asleep or the app was closed: own up to the wait.
                    let reply = match late {
                        Some(note) => format!("_{note}_\n\n{reply}"),
                        None => reply,
                    };
                    let draft = follower.finish().await;
                    let (reply, files) = crate::files::split_sent(&reply);
                    core.say(&chan, &ext, chat, draft, if reply.trim().is_empty() && !files.is_empty() { "📎" } else { &reply }, &rows, &files, None).await;
                });
            }
        }
        Ok(())
    }

    /// Cut off the reply being written in this chat, if any (/stop, or a channel's own stop button).
    pub(crate) fn stop(&self, chan: &Chan, ext: &str, from: Sender<'_>) -> Result<bool> {
        let Some(chat) = self.db.chat_lookup(chan.kind(), ext)? else { return Ok(false) };
        if self.sender_ok(chat, from)? != Some(true) {
            return Ok(false);
        }
        let running = TURNS.lock().unwrap().remove(&(self.bot, chat));
        Ok(running.map(|h| h.abort()).is_some())
    }

    /// An Approve/Deny button press. Only the chat the request belongs to can decide it (or the decisions chat it was
    /// also sent to), and in it only the person who paired it (an error says so, for the button's reply).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decide(&self, chan: &Chan, ext: &str, from: Sender<'_>, id: i64, d: Decision) -> Result<Option<Approval>> {
        let approve = d != Decision::Deny;
        match self.db.chat_lookup(chan.kind(), ext)? {
            Some(chat) => {
                if self.sender_ok(chat, from)? != Some(true) || self.db.chat_owner(chat)? != from.id {
                    anyhow::bail!(tr("Only the person who paired me here can answer this."));
                }
                let mirror = self.db.approval_mirror(id)? == Some(chat) && decisions_ok(&self.db, chat);
                let scope = (!mirror).then_some(chat);
                if d == Decision::RestOfRun {
                    self.db.approval_allow_rest_of_run(id, scope)?;
                }
                let a = self.db.approval_decide(id, approve, scope)?;
                if let Some(a) = &a {
                    crate::tools::after_decision(&self.db, a);
                    if d == Decision::Always {
                        crate::tools::always_allow(&self.db, a)?;
                    }
                    // The button handler edits its own card; the approvals loop closes the other one.
                    if a.chat_id == chat {
                        self.db.approval_mark_closed(a.id)?;
                    } else {
                        self.db.mirror_mark_closed(a.id)?;
                    }
                }
                Ok(a)
            }
            None => Ok(None),
        }
    }

    /// A text reply to a pending card: no, and this is why (the bot is told, in the person's words). True if it was
    /// one; the approvals loop then shows the outcome on the card. Only from the person who can press its buttons.
    pub(crate) fn deny_by_reply(&self, chan: &Chan, ext: &str, from: Sender<'_>, card: &str, said: &str) -> Result<bool> {
        let Some(chat) = self.db.chat_lookup(chan.kind(), ext)? else { return Ok(false) };
        let Some(id) = self.db.approval_by_card(chat, card)? else { return Ok(false) };
        if self.sender_ok(chat, from)? != Some(true) || self.db.chat_owner(chat)? != from.id || said.trim().is_empty() {
            return Ok(false);
        }
        self.db.approval_set_reason(id, &crate::tools::truncate(said.trim(), 500))?;
        let Some(a) = self.db.approval_decide(id, false, None)? else { return Ok(false) };
        crate::tools::after_decision(&self.db, &a);
        Ok(true)
    }

    /// The question ask_choice left during this chat's reply, as (question, its buttons); taken either way, so a reply
    /// that failed or was stopped leaves none behind for the next one.
    fn take_choice(&self, chat: i64) -> Option<(String, Vec<Row>)> {
        let v: serde_json::Value = serde_json::from_str(&self.db.setting_take(&crate::tools::choice_key(self.bot, chat)).ok()??).ok()?;
        let question = v["question"].as_str()?.to_string();
        let options = v["options"].as_array()?.iter().filter_map(|o| o.as_str().map(String::from)).collect();
        let rows = tap_rows(&self.db, self.bot, chat, TapFor::Choice { question: question.clone(), options });
        Some((question, rows))
    }

    /// The buttons under a routine's result: a one-off's are a reminder's.
    fn result_rows(&self, r: &crate::db::Routine, said: &str) -> Vec<Row> {
        let what = if r.once {
            TapFor::Remind { prompt: r.prompt.clone(), due: r.next_fire, tainted: r.tainted }
        } else {
            // A result written after reading a page is labelled as such where learning sees it, like a correction's.
            let outside = self.db.recent_outside(self.bot, r.chat_id, 1).unwrap_or(true);
            // Without the "anything here you didn't need?" line a new routine's first results end with.
            let said = said.split("\n\n_Anything here you didn't need").next().unwrap_or(said);
            TapFor::Routine { id: r.id, prompt: crate::tools::truncate(&r.prompt, 140), said: crate::tools::truncate(said.trim(), 240), outside }
        };
        tap_rows(&self.db, self.bot, r.chat_id, what)
    }

    /// A tap on buttons from `tap_rows` ("tp:<token>:<n>"): what it did, in a few words for the app's button reply, or
    /// None if the data isn't one of these. Each message's buttons work once, and only for the person who paired the chat.
    pub(crate) async fn tap(self: &Arc<Self>, chan: &Chan, ext: &str, from: Sender<'_>, data: &str) -> Result<Option<String>> {
        let Some((token, n)) = data.strip_prefix("tp:").and_then(|r| r.split_once(':')).and_then(|(t, n)| Some((t, n.parse::<usize>().ok()?))) else { return Ok(None) };
        let Some(chat) = self.db.chat_lookup(chan.kind(), ext)? else { return Ok(Some(tr("This chat isn't paired any more.").into())) };
        if self.sender_ok(chat, from)? != Some(true) {
            anyhow::bail!("Only the person who paired me here can answer this.");
        }
        let key = format!("tap:{token}");
        // Another bot's or chat's buttons can't be used here, and a second tap (or a double tap) finds nothing left.
        let ours = self.db.setting(&key)?.and_then(|v| serde_json::from_str::<Tap>(&v).ok()).is_some_and(|t| t.bot == self.bot && t.chat == chat);
        let tap = if ours { self.db.setting_take(&key)?.and_then(|v| serde_json::from_str::<Tap>(&v).ok()) } else { None };
        let Some(tap) = tap else { return Ok(Some(tr("Already done.").into())) };
        let now = crate::db::now();
        Ok(Some(match tap.what {
            TapFor::Choice { question, options } => match options.get(n) {
                // Their answer, as if typed in reply to the question (so a bot without the question in its history knows it).
                Some(o) => {
                    self.incoming(chan, ext, from, "", &crate::with_reply(o, &question), None).await?;
                    trf("You chose: {choice}", &[("choice", o)])
                }
                None => {
                    let ask = trf("✍️ Type your answer: {question}", &[("question", &question)]);
                    self.db.log(self.bot, chat, "assistant", &ask, None)?;
                    chan.send(ext, &ask).await;
                    tr("Type your answer.").into()
                }
            },
            TapFor::Remind { prompt, due, tainted } => {
                let at = match n {
                    0 => return Ok(Some(tr("✅ Done").into())),
                    1 => now + 3600,
                    _ => next_day(due, now),
                };
                let r = self.db.routine_once(self.bot, chat, at, &prompt, false)?;
                if tainted {
                    self.db.routine_taint(r.id)?;
                }
                if n == 1 { trf("⏰ I'll remind you again today at {at}.", &[("at", &clock(at))]) } else { trf("⏰ I'll remind you again tomorrow at {at}.", &[("at", &clock(at))]) }
            }
            TapFor::Routine { id, prompt, said, outside } => {
                // Feedback is evidence for the tidy-up, which proposes changes for the person to accept; nothing is edited here.
                let feedback = |kind: &str, reply: &str| {
                    let outside = if outside { " [outside content: not the person's words]" } else { "" };
                    self.db.signal_add(self.bot, chat, kind, &format!("On a routine's result (#{id} \"{prompt}\"): You said{outside}: {said} → They replied: {reply}"))
                };
                match n {
                    0 => {
                        feedback("liked", "👍 (tapped: this was useful)")?;
                        tr("👍 Noted.").into()
                    }
                    1 => {
                        feedback("correction", "👎 (tapped: this wasn't useful)")?;
                        tr("👎 Noted: I'll learn from it.").into()
                    }
                    2 => {
                        feedback("correction", "Less like this (tapped)")?;
                        tr("Noted: less like this.").into()
                    }
                    3 => match self.db.routines(chat)?.into_iter().find(|r| r.id == id) {
                        Some(r) => {
                            self.db.routine_update(chat, id, &r.cron, &r.prompt, r.quiet, true)?;
                            trf("⏸ Paused routine #{id}. Ask me, or use Mimir, to start it again.", &[("id", &id.to_string())])
                        }
                        None => tr("That routine is gone.").into(),
                    },
                    _ => {
                        let ask = trf("What should be different next time? Reply here and I'll change routine #{id}.", &[("id", &id.to_string())]);
                        self.db.log(self.bot, chat, "assistant", &ask, None)?;
                        chan.send(ext, &ask).await;
                        tr("Tell me what to change.").into()
                    }
                }
            }
        }))
    }

    /// Once a day at the digest time: everything the digest-lane routines found, as one message per chat.
    // ponytail: one digest time for everyone (setting digest_time, default 08:00); per-bot times if asked for
    async fn send_digest(&self, chans: &[Chan]) -> Result<()> {
        if quiet_now(&self.db, chrono::Local::now()) {
            return Ok(());
        }
        let due_at = digest_due_at(&self.db, chrono::Local::now());
        let key = format!("digest_last:{}", self.bot);
        let last: i64 = self.db.setting(&key)?.and_then(|v| v.parse().ok()).unwrap_or(0);
        if crate::db::now() < due_at || last >= due_at {
            return Ok(());
        }
        self.db.set_setting(&key, &crate::db::now().to_string())?;
        let items = self.db.digest_pending(self.bot)?;
        // Chats with something to report, plus chats whose digest-lane quiet checks may all have found nothing.
        let mut chats: Vec<i64> = items.iter().map(|i| i.1).collect();
        for c in self.db.chats()? {
            if self.db.routines(c.chat_id)?.iter().any(|r| r.bot_id == self.bot && r.delivery == "digest") {
                chats.push(c.chat_id);
            }
        }
        chats.sort_unstable();
        chats.dedup();
        for chat in chats {
            let mine: Vec<_> = items.iter().filter(|i| i.1 == chat).collect();
            let quiet = self.db.quiet_nothing_since(self.bot, chat, last)?;
            if mine.is_empty() && quiet == 0 {
                continue;
            }
            let routines = self.db.routines(chat)?;
            let mut msg = format!("{}\n", if mine.is_empty() { tr("☀️ **All quiet**") } else { tr("☀️ **Your digest**") });
            let mut files = vec![];
            for (_, _, rid, text, _) in &mine {
                let when = routines.iter().find(|r| Some(r.id) == *rid).map(|r| crate::tools::truncate(&r.prompt, 60)).unwrap_or_else(|| tr("A routine").into());
                let (text, sent) = crate::files::split_sent(text);
                files.extend(sent);
                msg.push_str(&format!("\n**{when}**\n{}\n", crate::tools::truncate(text.trim(), 700)));
            }
            if quiet > 0 {
                let ran = if quiet == 1 { tr("1 quiet check ran; nothing needed you.").into() } else { trf("{n} quiet checks ran; nothing needed you.", &[("n", &quiet.to_string())]) };
                msg.push_str(&format!("\n{ran}"));
            }
            let lines: String = files.iter().map(|f| format!("\n{}{f}]]", crate::files::SENT_MARK)).collect();
            self.db.log(self.bot, chat, "assistant", &format!("{msg}{lines}"), None)?;
            // What routines found on the web is outside content in this conversation too.
            if mine.iter().any(|i| i.4) {
                self.db.conv_taint(self.bot, chat)?;
            }
            if let Some((c, ext)) = Self::chan_for(chans, chat, &self.db) {
                self.say(&c, &ext, chat, None, &msg, &[], &files, None).await;
            }
            self.db.digest_mark_sent(&mine.iter().map(|i| i.0).collect::<Vec<_>>())?;
        }
        Ok(())
    }

    /// Once a week at the digest time, in chat apps: what was learned there without the person typing it, with
    /// /undo. Sent by the bot that last talked in that chat, so it arrives from the bot they use.
    async fn send_weekly(&self, chans: &[Chan]) -> Result<()> {
        if quiet_now(&self.db, chrono::Local::now()) {
            return Ok(());
        }
        let (now, due_at) = (crate::db::now(), digest_due_at(&self.db, chrono::Local::now()));
        for c in self.db.chats()? {
            let mine = self.db.messages(c.chat_id, 1)?.first().is_some_and(|m| m.bot_id == self.bot);
            let Some((chan, ext)) = Self::chan_for(chans, c.chat_id, &self.db).filter(|_| mine) else { continue };
            if !crate::tidy::weekly_due(&self.db, c.chat_id, now, due_at) {
                continue;
            }
            let review = crate::tidy::weekly_review(&self.db, c.chat_id, now)?;
            // The slot it was due at, not the send time, so a late week doesn't push every later one back.
            self.db.set_setting(&format!("weekly_sent:{}", c.chat_id), &due_at.to_string())?;
            if let Some((msg, ids)) = review {
                self.db.set_setting(&format!("weekly_items:{}", c.chat_id), &serde_json::to_string(&ids)?)?;
                self.db.set_setting(&format!("weekly_listed:{}", c.chat_id), &now.to_string())?;
                self.db.log(self.bot, c.chat_id, "assistant", &msg, None)?;
                self.say(&chan, &ext, c.chat_id, None, &msg, &[], &[], None).await;
            }
        }
        Ok(())
    }

/// A newer Mimir than this server runs: said once, in your most recent private chat this bot can reach (not a group,
    /// not a guest's). A bot that reaches none leaves it to the others; Needs you in the app shows it either way.
    async fn tell_update(&self, chans: &[Chan]) -> Result<()> {
        let Some(latest) = crate::server_update::latest(&self.db).await else { return Ok(()) };
        let Some(msg) = crate::server_update::to_tell(&self.db, &latest) else { return Ok(()) };
        if let Some((chat, chan, ext)) = self.latest_private(chans)?
            && crate::server_update::told(&self.db, &latest)
        {
            self.say(&chan, &ext, chat, None, &msg, &[], &[], None).await;
        }
        Ok(())
    }

    /// Your most recent private chat this bot can reach in a messaging app (not a group, not a guest's).
    fn latest_private(&self, chans: &[Chan]) -> Result<Option<(i64, Chan, String)>> {
        let mut best: Option<(i64, i64, Chan, String)> = None;
        for c in self.db.chats()? {
            if c.chat_id == crate::host::LOCAL_CHAT || c.guest || !self.db.chat_private(c.chat_id) {
                continue;
            }
            let Some((chan, ext)) = Self::chan_for(chans, c.chat_id, &self.db) else { continue };
            let last = self.db.messages(c.chat_id, 1)?.first().map(|m| m.ts).unwrap_or(0);
            if best.as_ref().is_none_or(|b| last > b.0) {
                best = Some((last, c.chat_id, chan, ext));
            }
        }
        Ok(best.map(|(_, chat, chan, ext)| (chat, chan, ext)))
    }

    /// Mail tasks (Settings → Email, when on): mail you sent to your +mimir address, or filed in the Mimir folder,
    /// is a message to this bot, answered in your most recent private chat (or in the app). The bot never reads the
    /// inbox for this; Mimir looks only at those, and takes only ones from you that DKIM passed (email::task).
    async fn mail_tasks(&self, chans: &[Chan]) -> Result<()> {
        let Some(a) = crate::email::account(&self.db) else { return Ok(()) };
        for (folder, route) in [("INBOX", "to your +mimir address"), (crate::email::TASK_FOLDER, "filed in your Mimir folder")] {
            let key = format!("mail_task_seen:{folder}");
            let after = self.db.setting(&key)?.and_then(|v| v.split_once(':').and_then(|(v, u)| Some((v.parse().ok()?, u.parse().ok()?))));
            let inbox = folder == "INBOX";
            let (cursor, mails) = match crate::email::new_mail(&a, folder, after, |h| !inbox || crate::email::to_plus(h, &a.user)).await {
                Ok(r) => r,
                // No Mimir folder is the usual case.
                Err(e) => {
                    if inbox {
                        eprintln!("mail tasks: {e:#}");
                    }
                    continue;
                }
            };
            // Read past before running: a task that crashes or hangs isn't taken again (and done twice).
            self.db.set_setting(&key, &format!("{}:{}", cursor.0, cursor.1))?;
            for raw in mails {
                match crate::email::task(&raw, &a.user, route) {
                    crate::email::Task::Run { text, outside } => self.mail_task(chans, text, outside).await?,
                    crate::email::Task::Refused(why) => {
                        eprintln!("mail tasks: {why}");
                        let (chat, target) = self.latest_private(chans)?.map_or((crate::host::LOCAL_CHAT, None), |(c, ch, ext)| (c, Some((ch, ext))));
                        self.db.log(self.bot, chat, "assistant", &why, None)?;
                        if let Some((c, ext)) = target {
                            self.say(&c, &ext, chat, None, &why, &[], &[], None).await;
                        }
                    }
                    crate::email::Task::Skip => {}
                }
            }
        }
        Ok(())
    }

    async fn mail_task(&self, chans: &[Chan], text: String, outside: bool) -> Result<()> {
        let (chat, target) = self.latest_private(chans)?.map_or((crate::host::LOCAL_CHAT, None), |(c, ch, ext)| (c, Some((ch, ext))));
        // What you forwarded is someone else's words: the conversation has read outside content, like a forward in chat.
        if outside {
            self.db.conv_taint(self.bot, chat)?;
        }
        let _g = self.chat_lock(chat).lock_owned().await;
        let typing = target.as_ref().and_then(|(c, ext)| c.typing(ext));
        let (provider, ctx) = (self.provider.clone(), ToolCtx::new(self.db.clone(), self.bot, chat));
        let out = self.own_task(chat, async move { turn(&provider, &ctx, Trigger::Message(&text)).await }).await;
        if let Some(t) = typing {
            t.abort();
        }
        let reply = out.unwrap_or_else(|e| crate::friendly_error(&format!("{e:#}")));
        if let Some((c, ext)) = target {
            self.say(&c, &ext, chat, None, &reply, &[], &[], None).await;
        }
        Ok(())
    }

    /// Also said in the decisions chat, unless that's where it happened (`from`) or it's quiet hours (Needs you has it).
    // ponytail: dropped in quiet hours rather than queued; queue it for the digest if that's missed
    async fn tell_decisions(&self, chans: &[Chan], from: Option<i64>, text: &str) {
        let Some(to) = decisions_chat(&self.db).filter(|to| Some(*to) != from && !quiet(&self.db)) else { return };
        if let Some((c, ext)) = Self::chan_for(chans, to, &self.db) {
            self.say(&c, &ext, to, None, text, &[], &[], None).await;
        }
    }

    /// Said in a chat app, kept in the outbox until it's sent: a crash, an update or a restart before it goes doesn't
    /// lose it (`resend` sends what's left), and a send that fails is tried again. A routine's `run` shows where it is.
    #[allow(clippy::too_many_arguments)]
    async fn say(&self, chan: &Chan, ext: &str, chat: i64, draft: Option<String>, text: &str, rows: &[Row], files: &[String], run: Option<i64>) {
        let (rows, files) = (serde_json::to_string(rows).unwrap_or_default(), serde_json::to_string(files).unwrap_or_default());
        let id = self.db.outbox_add(self.bot, chat, chan.kind(), ext, draft.as_deref(), text, &rows, &files, run).unwrap_or_else(|e| {
            // Not kept, but still sent now.
            eprintln!("outbox: {e:#}");
            0
        });
        let o = crate::db::Outgoing { id, chat_id: chat, kind: chan.kind().into(), ext: ext.into(), draft, text: text.into(), rows, files, run_id: run };
        self.send_out(chan, &o).await;
    }

    /// One message from the outbox: gone from it once its text is sent (a crash while its files upload loses those
    /// rather than sending the text twice), or marked to try again.
    async fn send_out(&self, chan: &Chan, o: &crate::db::Outgoing) {
        let rows: Vec<Row> = serde_json::from_str(&o.rows).unwrap_or_default();
        let how = match chan.post(&o.ext, o.draft.as_deref(), &o.text, &rows).await {
            Ok(()) => {
                if let Err(e) = self.db.outbox_sent(o.id) {
                    eprintln!("outbox: {e:#}");
                }
                let files: Vec<String> = serde_json::from_str(&o.files).unwrap_or_default();
                chan.attach(&o.ext, &self.db, o.chat_id, &files).await;
                format!("sent to {}", chan.kind())
            }
            Err(e) => {
                eprintln!("{} send to {} failed: {e:#}", chan.kind(), o.ext);
                let last = self.db.outbox_failed(o.id, &format!("{e:#}")).unwrap_or(true);
                let e = crate::tools::truncate(&format!("{e:#}"), 200);
                if last { format!("{} to {}: {e}", crate::db::NOT_SENT, chan.kind()) } else { format!("{} to {} yet, trying again: {e}", crate::db::NOT_SENT, chan.kind()) }
            }
        };
        if let Some(run) = o.run_id
            && let Err(e) = self.db.run_redelivered(run, &how)
        {
            eprintln!("run #{run}: {e:#}");
        }
    }

    /// What's still in the outbox (left by a crash, an update or a restart, or a send that failed): sent now.
    async fn resend(&self, chans: &[Chan]) -> Result<()> {
        for o in self.db.outbox_take(self.bot, crate::db::now())? {
            // Its channel isn't set up any more: it waits (and is cleared out after a few days).
            if let Some(c) = chans.iter().find(|c| c.kind() == o.kind) {
                self.send_out(c, &o).await;
            }
        }
        Ok(())
    }

    fn chan_for(chans: &[Chan], chat: i64, db: &Db) -> Option<(Chan, String)> {
        let (kind, ext) = db.chat_address(chat).ok()??;
        chans.iter().find(|c| c.kind() == kind).map(|c| (c.clone(), ext))
    }

    /// One routine that's due: the change gate, a setup check, the run, what happens after a failure (wait for the
    /// usage limit, try again soon, or say so), and where its result went.
    /// The fallback account this bot's routines run on right now, with when they go back to its own (see Db::on_fallback).
    fn fallback(&self, now: i64) -> Option<(Arc<Provider>, i64)> {
        let (kind, model, until) = self.db.on_fallback(self.bot, now).unwrap_or_else(|e| { eprintln!("fallback: {e:#}"); None })?;
        let home = crate::data_dir().ok()?;
        match Provider::new(&kind, Some(model), None, crate::host::api_key(&kind), home) {
            Ok(p) => Some((Arc::new(p), until)),
            Err(e) => {
                eprintln!("bot #{}'s fallback account: {e:#}", self.bot);
                None
            }
        }
    }

    async fn run_routine(&self, chans: &[Chan], mut r: crate::db::Routine, fallback: Option<&(Arc<Provider>, i64)>) -> Result<()> {
        let provider = fallback.map_or_else(|| self.provider.clone(), |(p, _)| p.clone());
        // Quiet hours: what would message right away goes into the next digest instead.
        if r.delivery == "now" && quiet_now(&self.db, chrono::Local::now()) {
            r.delivery = "digest".into();
        }
        // Over a daily cap: pause (it stays due, so it runs once when the day's over) and ask, in Needs you.
        if let Some((key, h)) = self.db.over_cap(&r, crate::day_start())? {
            let said = format!("⏸ Paused until tomorrow: {}. You can let it go on today in Mimir's Needs you.", h.why);
            self.db.hold_set(&key, &h)?;
            if let Some((c, ext)) = Self::chan_for(chans, r.chat_id, &self.db).filter(|_| r.delivery == "now") {
                self.say(&c, &ext, r.chat_id, None, &said, &[], &[], None).await;
            }
            return Ok(());
        }
        // Timed off the calendar: the run is told which event it's for (the invite is someone else's words). An event
        // gone or moved since is skipped (the scheduler finds its new time), and so is one that's over by now.
        if let Ok(Some(t)) = crate::calendar::trigger(&r.cron) {
            let start = r.next_fire + t.before;
            match crate::calendar::events(start - 1, start + 1).await.ok().and_then(|evs| t.event_at(&evs, r.next_fire)).filter(|e| e.end > crate::db::now()) {
                Some(ev) => {
                    r.tainted = true;
                    r.prompt.push_str(&format!("\n\nThe calendar event this run is for:\n{}", crate::tools::calendar_lines(&[ev])));
                }
                None => return self.db.routine_advance(&r),
            }
        }
        self.db.routine_advance(&r)?;
        // Started by an event: the run is told what came (someone else's mail or call). Nothing came (it was due for
        // another reason): no run.
        let seen_upto = match crate::events::on(&r.cron) {
            Ok(Some(on)) => {
                let evs = self.db.routine_events(r.id)?;
                let Some(&(upto, _)) = evs.last() else { return Ok(()) };
                r.tainted = true;
                r.prompt.push_str(&crate::events::note(&on, &evs.into_iter().map(|e| e.1).collect::<Vec<_>>()));
                Some(upto)
            }
            _ => None,
        };
        // A watched quiet check: Mimir looks first, and only wakes the AI when something changed.
        let seen = match crate::watch::gate(&self.db, &r).await {
            Some(crate::watch::Gate::Same) => {
                self.db.run_unchanged(&r)?;
                return self.db.routine_done(&r, true);
            }
            Some(crate::watch::Gate::Run { note, seen }) => {
                // What changed is someone else's page or mail: the run has read outside content.
                r.tainted |= !note.is_empty();
                r.prompt.push_str(&note);
                Some(seen)
            }
            None => None,
        };
        // Nothing is spent on an AI account that isn't there: that's a failure (tried again, like the network) instead.
        let ready = provider.ready().await;
        let _g = self.chat_lock(r.chat_id).lock_owned().await;
        // Local-chat routines (or ones whose channel isn't connected) still run; the reply lands in the app.
        let target = Self::chan_for(chans, r.chat_id, &self.db);
        let out = match ready {
            Err(e) => {
                self.db.run_not_started(&r, &provider.name(), &format!("{e:#}"))?;
                Err(e)
            }
            Ok(()) => {
                let typing = target.as_ref().and_then(|(c, ext)| c.typing(ext));
                let (provider, ctx, routine) = (provider.clone(), ToolCtx::new(self.db.clone(), self.bot, r.chat_id), r.clone());
                let out = self.own_task(r.chat_id, async move { turn(&provider, &ctx, Trigger::Routine(&routine)).await }).await;
                if let Some(t) = typing {
                    t.abort();
                }
                out
            }
        };
        let run = self.db.routine_last_run(r.id)?;
        let worked = out.is_ok();
        let text = match out {
            Ok(text) => {
                // Compared against next time only once a run has seen this change (a failed one sees it again).
                if let Some(seen) = seen {
                    self.db.routine_set_seen(r.id, &seen)?;
                }
                if let Some(upto) = seen_upto {
                    self.db.routine_events_clear(r.id, upto)?;
                }
                self.db.routine_done(&r, true)?;
                text
            }
            Err(e) => {
                let e = format!("{e:#}");
                let now = crate::db::now();
                let clock = |t: i64| chrono::TimeZone::timestamp_opt(&chrono::Local, t, 0).single().map(|d| d.format(if t - now < 18 * 3600 { "%H:%M" } else { "%a %-d %b %H:%M" }).to_string()).unwrap_or_default();
                let again = match crate::cause(&e) {
                    // The fallback is at its limit too: back to waiting for the bot's own account.
                    crate::Cause::Quota if let Some((_, until)) = fallback => {
                        self.db.fallback_spent(self.bot, *until)?;
                        self.db.routine_retry(&r, *until)?;
                        Some(format!("The fallback account was at its usage limit too; this one runs again at {}, when the bot's own account resets.", clock(*until)))
                    }
                    crate::Cause::Quota => {
                        // ponytail: an hour when the AI didn't say when it resets; a run then finds out again
                        let until = crate::provider::quota_reset(&self.db, self.provider.kind(), &e, now).unwrap_or(now + 3600).min(now + 8 * 86400);
                        let why = "the AI plan was at its usage limit, so Mimir waited for it to reset".to_string();
                        // Every bot on the same signed-in AI (Claude, ChatGPT or Grok is one login for all of them) waits
                        // too, instead of each finding out with a failed run. Bots on an API key may be limited per model.
                        let held = self.db.hold_account(self.bot, self.provider.kind(), &crate::db::Hold { until, why, waive: None })?;
                        // A fallback account set: this one runs again on it straight away (the scheduler's next pass).
                        if self.db.on_fallback(self.bot, now)?.is_some() {
                            self.db.routine_retry(&r, now)?;
                            if let Some(run) = run {
                                self.db.run_note(run, &format!("Routines run on the fallback account until {}, then on this one again.", clock(until)))?;
                            }
                            return Ok(());
                        }
                        self.db.routine_retry(&r, until)?;
                        let who = if held.len() > 1 { "Routines on this AI account wait" } else { "This bot's routines wait" };
                        let note = format!("{who} until {}, when the limit resets; this one runs again then.", clock(until));
                        // Said once, where this routine speaks; Needs you has it too (in English, like the rest of the app).
                        if let Some((c, ext)) = target.as_ref().filter(|_| r.delivery == "now") {
                            let who = if held.len() > 1 { tr("Routines on this AI account wait") } else { tr("This bot's routines wait") };
                            let said = trf("⏸ I've hit my AI plan's usage limit. {who} until {at}, when the limit resets; this one runs again then.", &[("who", who), ("at", &clock(until))]);
                            self.say(c, ext, r.chat_id, None, &said, &[], &[], None).await;
                        }
                        Some(note)
                    }
                    crate::Cause::Network | crate::Cause::Setup if r.retries < 3 => {
                        let at = now + [5, 15, 30][r.retries as usize] * 60;
                        self.db.routine_retry(&r, at)?;
                        Some(format!("Trying again at {}.", clock(at)))
                    }
                    _ => None,
                };
                if let (Some(run), Some(note)) = (run, &again) {
                    self.db.run_note(run, note)?;
                }
                if again.is_some() {
                    return Ok(());
                }
                if let Some(upto) = seen_upto {
                    self.db.routine_events_clear(r.id, upto)?;
                }
                self.db.routine_done(&r, false)?;
                let text = format!("Routine #{}: {}", r.id, crate::friendly_error(&e));
                // A failed digest-lane routine (incl. one held for quiet hours) says so in the digest.
                if r.delivery == "digest" {
                    self.db.digest_add(self.bot, r.chat_id, r.id, &text, false)?;
                }
                self.tell_decisions(chans, Some(r.chat_id), &trf("From {chat}: {text}", &[("chat", &chat_name(&self.db, r.chat_id)), ("text", &text)])).await;
                text
            }
        };
        // The run log says when a run was on the fallback account (shown after where the result went).
        let fell_back = fallback.map(|(_, until)| {
            let at = chrono::TimeZone::timestamp_opt(&chrono::Local, *until, 0).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default();
            format!("on the fallback account until {at}")
        });
        // Empty = a quiet check that found nothing worth saying. Digest / log-only routines don't message now.
        if text.trim().is_empty() {
            if let (Some(run), Some(f)) = (run, &fell_back) {
                self.db.run_delivered(run, f)?;
            }
            return Ok(());
        }
        // Matched the person's "come through right away if …" (within the day's cap): sent now, whatever its lane.
        let urgent = run.map(|run| self.db.run_urgent(run)).transpose()?.unwrap_or(false);
        let mut send = None;
        let how = match (if urgent { "now" } else { r.delivery.as_str() }, &target) {
            ("digest", _) => "in the digest".to_string(),
            ("log", _) => "only in the log".to_string(),
            ("card", _) => if worked { "on its card" } else { "only in the log" }.to_string(),
            (_, None) => "in the app".to_string(),
            (_, Some((c, ext))) => {
                send = Some((c, ext));
                // Until it's sent (say updates it): a crash or restart before then leaves this showing, and the outbox sends it.
                format!("waiting to send to {}", c.kind())
            }
        };
        if let Some(run) = run {
            self.db.run_delivered(run, &fell_back.map_or(how.clone(), |f| format!("{how} · {f}")))?;
        }
        if let Some((c, ext)) = send {
            // A result gets one-tap feedback (a reminder, Done or Snooze); a failure doesn't.
            let (said, files) = crate::files::split_sent(&text);
            let rows = if worked { self.result_rows(&r, &said) } else { vec![] };
            self.say(c, ext, r.chat_id, None, &said, &rows, &files, run).await;
        }
        Ok(())
    }

    /// Tasks this bot handed over to report back (ask_bot's `later`): new ones start on their own, and finished ones
    /// come back as a turn of this bot in the chat they came from, sent there like a routine's result.
    // ponytail: started on the next scheduler pass (up to 30s); a wake-up from the tool if that's too slow
    async fn delegations(self: &Arc<Self>, chans: &[Chan]) -> Result<()> {
        let now = crate::db::now();
        for d in self.db.delegations(self.bot, "pending")? {
            // A helper paused by the person waits, like its routines.
            if self.db.paused(d.to_bot, now)?.is_some() {
                continue;
            }
            self.db.delegation_status(d.id, "running")?;
            let db = self.db.clone();
            tokio::spawn(async move {
                let mut asker = ToolCtx::new(db.clone(), d.bot_id, d.chat_id).with_run(d.run_id);
                asker.conv = d.conv;
                asker.unattended = d.unattended;
                if d.tainted {
                    asker.pre_taint();
                }
                let private = crate::tools::delegation_private_key(d.id);
                if db.setting(&private).ok().flatten().is_some() {
                    asker.pre_private();
                }
                let (ctx, task) = (asker.clone(), d.task.clone());
                // In its own task: a crash in it fails this hand-off only, and says so.
                let work = tokio::spawn(async move { crate::tools::delegate(&ctx, &ctx.db.bot(d.to_bot)?, &task).await });
                let out = work.await.unwrap_or_else(|e| Err(anyhow::anyhow!("Mimir crashed during this task ({e})")));
                let (reply, failed) = match out {
                    Ok(r) => (r, false),
                    Err(e) => (crate::friendly_error(&format!("{e:#}")), true),
                };
                // The helper read email or a meeting: the report-back turn knows (shared with the asker, like taint).
                if asker.saw_private() {
                    let _ = db.set_setting(&private, "1");
                }
                if let Err(e) = db.delegation_done(d.id, &reply, failed, asker.is_tainted()) {
                    eprintln!("handed-over task #{}: {e:#}", d.id);
                }
            });
        }
        for d in self.db.delegations(self.bot, "done")? {
            // Marked first: told at most once (a crash mid-report leaves the helper's answer in the run log).
            self.db.delegation_status(d.id, "told")?;
            let from = self.db.bot(d.to_bot).map(|b| b.name).unwrap_or_else(|_| "Another bot".into());
            let reply = d.reply.unwrap_or_default();
            let reply = if d.failed { format!("It didn't finish: {reply}") } else { reply };
            let _g = self.chat_lock(d.chat_id).lock_owned().await;
            let target = Self::chan_for(chans, d.chat_id, &self.db);
            let mut ctx = ToolCtx::new(self.db.clone(), self.bot, d.chat_id).with_run(d.run_id);
            ctx.conv = d.conv;
            if self.db.setting_take(&crate::tools::delegation_private_key(d.id))?.is_some() {
                ctx.pre_private();
            }
            let (provider, task, tainted) = (self.provider.clone(), d.task, d.tainted);
            let out = self.own_task(d.chat_id, async move { turn(&provider, &ctx, Trigger::Reported { from: &from, task: &task, reply: &reply, tainted }).await }).await;
            let text = match out {
                Ok(t) => t,
                Err(e) => {
                    let text = format!("{}\n\n{}", crate::friendly_error(&format!("{e:#}")), tr("A task I handed to another bot is done: its answer is in the run log."));
                    self.db.log(self.bot, d.conv, "assistant", &text, None)?;
                    text
                }
            };
            if let Some((c, ext)) = target {
                let (said, files) = crate::files::split_sent(&text);
                self.say(&c, &ext, d.chat_id, None, &said, &[], &files, None).await;
            }
        }
        Ok(())
    }

    /// A routine or tidy-up turn in its own task, the way a message's runs: a crash in it fails that run (its open
    /// run log entry says so), instead of stopping the scheduler and with it the whole bot.
    async fn own_task<T: Send + 'static>(&self, chat: i64, work: impl Future<Output = Result<T>> + Send + 'static) -> Result<T> {
        match tokio::spawn(work).await {
            Ok(out) => out,
            Err(e) => {
                let e = anyhow::anyhow!("Mimir crashed during this run ({e})");
                if let Ok(Some(run)) = self.db.running_run(self.bot, chat) {
                    let _ = self.db.run_finish(run, Err(&format!("{e:#}")));
                }
                Err(e)
            }
        }
    }

    /// Routines timed off the calendar: each one's next run is its next matching event's start, less the lead time.
    /// Worked out again on every pass (the calendar is fetched at most every 10 minutes), so a meeting that's added,
    /// moved or cancelled moves the run too. One the calendar can't be read for keeps the time it has.
    async fn calendar_times(&self, now: i64) {
        for r in self.db.calendar_routines(self.bot, now).unwrap_or_default() {
            let Ok(Some(t)) = crate::calendar::trigger(&r.cron) else { continue };
            // ponytail: a meeting added after its run time has passed (with less notice than the lead time) gets no run
            let Ok(events) = crate::calendar::events(now + t.before, now + t.before + 7 * 86_400).await else { return };
            let next = t.next(&events, now).unwrap_or(crate::db::IDLE);
            if next != r.next_fire
                && let Err(e) = self.db.routine_set_next(r.id, next)
            {
                eprintln!("routine #{}: {e:#}", r.id);
            }
        }
    }

    async fn scheduler(self: Arc<Self>, chans: Vec<Chan>) -> Result<()> {
        // A routine due before this was Mimir not running; one due since, the computer asleep or a long run.
        let started = crate::db::now();
        let (mut mail_looked, mut rules_looked) = (0, 0);
        loop {
            let now = crate::db::now();
            // Messages a crash, an update or a failed send left unsent: first, and while paused too (they were said).
            if let Err(e) = self.resend(&chans).await {
                eprintln!("outbox: {e:#}");
            }
            // Mail tasks, looked for every two minutes by one bot (the memory bot) for everyone. You sent them, like a
            // message, so they run while the bot is paused too.
            if now - mail_looked >= 120 && crate::tidy::memory_bot(&self.db) == Some(self.bot) && self.db.setting("email_tasks").ok().flatten().as_deref() == Some("on") {
                mail_looked = now;
                if let Err(e) = self.mail_tasks(&chans).await {
                    eprintln!("mail tasks: {e:#}");
                }
            }
            // Paused by you: nothing runs on its own (routines, digest, weekly review, tidy-up) until it ends.
            if self.db.paused(self.bot, now).unwrap_or_else(|e| { eprintln!("pause: {e:#}"); None }).is_some() {
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }
            // Routines on hold (a usage limit) wait; the first pass after it ends runs each of them once.
            let bot_hold = format!("hold:{}", self.bot);
            let hold = self.db.hold(&bot_hold).unwrap_or_default();
            let held = hold.as_ref().is_some_and(|h| h.until > now);
            if hold.is_some() && !held {
                let _ = self.db.hold_clear(&bot_hold, false);
            }
            // A bot with a fallback account runs on it instead, until the hold ends: then it's back on its own.
            let fallback = if held { self.fallback(now) } else { None };
            let held = held && fallback.is_none();
            self.calendar_times(now).await;
            // Mail triggers look at new mail every two minutes; a match makes its routine due in this pass.
            if now - rules_looked >= 120 {
                rules_looked = now;
                crate::events::look_mail(&self.db, self.bot).await;
            }
            match self.db.due(self.bot, now) {
                Ok(due) if !held => {
                    for r in due {
                        // One routine paused at its daily cap waits on its own.
                        let own_key = format!("hold:routine:{}", r.id);
                        let own = self.db.hold(&own_key).unwrap_or_default();
                        if own.as_ref().is_some_and(|h| h.until > crate::db::now()) {
                            continue;
                        }
                        if own.is_some() {
                            let _ = self.db.hold_clear(&own_key, false);
                        }
                        let why = match (&own, &hold) {
                            (Some(h), _) | (_, Some(h)) if r.next_fire < h.until => h.why.as_str(),
                            _ if r.next_fire < started => crate::db::AWAY,
                            _ => crate::db::ASLEEP,
                        };
                        if let Err(e) = self.db.routine_missed(&r, crate::db::now(), why) {
                            eprintln!("routine #{} missed: {e:#}", r.id);
                        }
                        let id = r.id;
                        if let Err(e) = self.run_routine(&chans, r, fallback.as_ref()).await {
                            eprintln!("routine #{id}: {e:#}");
                        }
                        // It hit the usage limit or the bot's cap: the rest wait too (or switch to the fallback, next pass).
                        let now = crate::db::now();
                        if self.db.hold(&bot_hold).ok().flatten().is_some_and(|h| h.until > now) && (fallback.is_none() || self.fallback(now).is_none()) {
                            break;
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => eprintln!("scheduler: {e:#}"),
            }
            if let Err(e) = self.delegations(&chans).await {
                eprintln!("handed-over tasks: {e:#}");
            }
            if let Err(e) = self.send_digest(&chans).await {
                eprintln!("digest: {e:#}");
            }
            if let Err(e) = self.send_weekly(&chans).await {
                eprintln!("weekly review: {e:#}");
            }
            if crate::host::is_server()
                && let Err(e) = self.tell_update(&chans).await
            {
                eprintln!("update notice: {e:#}");
            }
            // Sleep-time memory tidy-up, run by one bot for everyone.
            if crate::tidy::memory_bot(&self.db) == Some(self.bot) {
                let now = crate::db::now();
                for chat in self.db.chats().unwrap_or_default().into_iter().filter(|c| crate::tidy::is_due(&self.db, c.chat_id, now)) {
                    // No chat lock: it reads the chat and calls the AI without a session, so replies needn't wait.
                    let (db, provider, bot, id) = (self.db.clone(), self.provider.clone(), self.bot, chat.chat_id);
                    match self.own_task(id, async move { crate::tidy::run(&db, &provider, bot, id).await }).await {
                        Ok(n) if n > 0 => {
                            eprintln!("tidy-up: {n} memory suggestions for chat {}", chat.chat_id);
                            let at = [("n", n.to_string()), ("chat", chat.title.clone())];
                            let at = at.each_ref().map(|(k, v)| (*k, v.as_str()));
                            let what = if n == 1 {
                                trf("🧠 1 memory suggestion for {chat}: accept or dismiss it in Mimir's Needs you.", &at)
                            } else {
                                trf("🧠 {n} memory suggestions for {chat}: accept or dismiss them in Mimir's Needs you.", &at)
                            };
                            self.tell_decisions(&chans, None, &what).await;
                        }
                        Ok(_) => {}
                        Err(e) => eprintln!("tidy-up for chat {}: {e:#}", chat.chat_id),
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    /// Sends an Approve/Deny card for each new pending approval, on the chat's own channel, and closes
    /// cards once they're decided.
    /// The waiting tool call (possibly in an MCP subprocess) sees the decision through SQLite.
    async fn approvals(self: Arc<Self>, chans: Vec<Chan>) -> Result<()> {
        loop {
            let quiet = quiet_now(&self.db, chrono::Local::now());
            // A routine's ask waits hours anyway: its card can wait until quiet hours end. Live asks still go.
            for mut a in self.db.approvals_to_notify(self.bot).unwrap_or_default().into_iter().filter(|a| !(quiet && a.parked)) {
                // Channel not connected: leave it pending for the app banner (and a later reconnect).
                let Some((c, ext)) = Self::chan_for(&chans, a.chat_id, &self.db) else { continue };
                a.edit_link = edit_link(&self.db, &a);
                let card = c.card(&ext, &a).await.unwrap_or_else(|e| {
                    eprintln!("approval #{} card failed: {e:#}", a.id);
                    None
                });
                let _ = self.db.approval_mark_notified(a.id, card.as_deref());
            }
            // A routine's ask from any other chat (the app's too) also goes to the decisions chat, saying where it's from.
            if let Some(to) = decisions_chat(&self.db).filter(|_| !quiet)
                && let Some((c, ext)) = Self::chan_for(&chans, to, &self.db)
            {
                for mut a in self.db.approvals_to_mirror(self.bot, to).unwrap_or_default() {
                    let id = a.id;
                    a.edit_link = edit_link(&self.db, &a);
                    a.summary = format!("{}\n\n{}", a.summary, trf("From: {chat}", &[("chat", &chat_name(&self.db, a.chat_id))]));
                    let card = c.card(&ext, &a).await.unwrap_or_else(|e| {
                        eprintln!("approval #{id} card to the decisions chat failed: {e:#}");
                        None
                    });
                    let _ = self.db.approval_mark_mirrored(id, to, card.as_deref());
                }
            }
            for (a, chat, card) in self.db.mirrors_to_close(self.bot).unwrap_or_default() {
                if let Some((c, ext)) = Self::chan_for(&chans, chat, &self.db) {
                    c.draft_edit(&ext, &card, &verdict(&a)).await;
                    let _ = self.db.mirror_mark_closed(a.id);
                }
            }
            // Decided in the app, the tray or the CLI, or ran out: the card shows the outcome, without buttons.
            for (a, card) in self.db.approvals_to_close(self.bot).unwrap_or_default() {
                // Channel not connected: try again on a later pass.
                if let Some((c, ext)) = Self::chan_for(&chans, a.chat_id, &self.db) {
                    c.draft_edit(&ext, &card, &verdict(&a)).await;
                    let _ = self.db.approval_mark_closed(a.id);
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

/// How long a message waits on its chat's routine or earlier reply before saying so.
const BUSY_NOTE_AFTER: Duration = Duration::from_secs(4);

/// Wrong pairing codes a chat may send in an hour.
const MAX_PAIR_TRIES: u32 = 5;

fn pairing_code() -> Result<String> {
    Ok(crate::random_hex(5)?.to_uppercase())
}

/// Runs one bot until the task is aborted. Channels reconnect on their own with backoff;
/// the scheduler runs even with no channels (local-chat routines).
pub async fn serve(db: Db, bot: i64, provider: Arc<Provider>, channels: Channels, status: SharedStatus) -> Result<()> {
    let code = pairing_code()?;
    let mut chans = vec![];
    if let Some(t) = channels.telegram {
        chans.push(Chan::Telegram(Tg::new(&t)?));
    }
    if let Some((b, a)) = channels.slack {
        chans.push(Chan::Slack(Slack::new(&b, &a)?));
    }
    if let Some(t) = channels.discord {
        chans.push(Chan::Discord(Discord::new(&t)?));
    }
    if let Some((api, number)) = channels.signal {
        chans.push(Chan::Signal(Signal::new(&api, &number)?));
    }
    // Started again after a crash: what happened stays on show.
    let crashed = status.lock().unwrap_or_else(|e| e.into_inner()).error.take();
    *status.lock().unwrap_or_else(|e| e.into_inner()) = Status {
        bot_id: bot,
        provider: provider.name(),
        pair_code: (!chans.is_empty()).then(|| code.clone()),
        channels: chans.iter().map(|c| ChannelStatus { kind: c.kind().into(), ..Default::default() }).collect(),
        error: crashed,
    };
    if !chans.is_empty() {
        eprintln!("bot #{bot} via {}; pair a chat by sending:  /pair {code}", provider.name());
    }
    let core = Arc::new(Core {
        db,
        bot,
        provider,
        code: std::sync::Mutex::new(code),
        status: status.clone(),
        misses: Default::default(),
        locks: Default::default(),
        greeted: Default::default(),
    });

    // Dropping the JoinSet (when this task is aborted) aborts every listener.
    let mut listeners = JoinSet::new();
    for chan in chans.clone() {
        let (core, slot) = (core.clone(), Slot { status: status.clone(), kind: chan.kind() });
        listeners.spawn(async move {
            let mut backoff = 5;
            loop {
                let started = std::time::Instant::now();
                // A crash while handling one message ends this connection, not the bot's channel: it reconnects.
                let run = std::panic::AssertUnwindSafe(chan.listen(core.clone(), slot.clone()));
                match futures_util::FutureExt::catch_unwind(run).await {
                    Ok(Err(e)) => {
                        eprintln!("{} disconnected: {e:#}", chan.kind());
                        slot.failed(&e);
                    }
                    Err(_) => {
                        let e = anyhow::anyhow!("something in a message made it stop; reconnecting");
                        eprintln!("{} crashed handling a message; reconnecting", chan.kind());
                        slot.failed(&e);
                    }
                    Ok(Ok(())) => {}
                }
                // A connection that lived a while resets the backoff.
                backoff = if started.elapsed() > Duration::from_secs(120) { 5 } else { (backoff * 2).min(300) };
                tokio::time::sleep(Duration::from_secs(backoff)).await;
            }
        });
    }
    tokio::select! {
        r = core.clone().scheduler(chans.clone()) => r,
        r = core.approvals(chans) => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_held_back_still_goes_out() {
        use chrono::TimeZone;
        let p = std::env::temp_dir().join(format!("mimir-due-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        db.set_setting("digest_time", "23:00").unwrap();
        let at = |d, h| chrono::Local.with_ymd_and_hms(2026, 10, d, h, 0, 0).unwrap();
        assert_eq!(digest_due_at(&db, at(6, 23)), at(6, 23).timestamp());
        // Quiet hours 22:00-07:00 held it back last night: this morning it's still due (yesterday's slot).
        assert_eq!(digest_due_at(&db, at(7, 8)), at(6, 23).timestamp());
    }

    #[test]
    fn quiet_hours() {
        use chrono::TimeZone;
        let p = std::env::temp_dir().join(format!("mimir-quiet-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let at = |h, m| chrono::Local.with_ymd_and_hms(2026, 10, 5, h, m, 0).unwrap();
        assert!(!quiet_now(&db, at(23, 0)), "off unless set");
        db.set_setting("quiet_hours", "22:00-07:00").unwrap();
        assert!(quiet_now(&db, at(23, 0)) && quiet_now(&db, at(2, 30)) && quiet_now(&db, at(22, 0)));
        assert!(!quiet_now(&db, at(7, 0)) && !quiet_now(&db, at(12, 0)) && !quiet_now(&db, at(21, 59)));
        db.set_setting("quiet_hours", "13:00-14:00").unwrap();
        assert!(quiet_now(&db, at(13, 30)) && !quiet_now(&db, at(14, 0)));
        db.set_setting("quiet_hours", "").unwrap();
        assert!(!quiet_now(&db, at(23, 0)), "empty = off");
    }

    /// A pretend AI service answering every request with this status and body.
    async fn fake_ai(status: &'static str, body: &'static str) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut s, _)) = l.accept().await {
                let mut buf = vec![0u8; 65536];
                let _ = s.read(&mut buf).await;
                let _ = s.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            }
        });
        format!("http://{addr}/v1")
    }

    fn core_on(db: &Db, base: &str) -> Core {
        Core {
            db: db.clone(),
            bot: 1,
            provider: Arc::new(Provider::new("openai", Some("m".into()), Some(base.into()), Some("k".into()), std::env::temp_dir()).unwrap()),
            code: Default::default(),
            status: Default::default(),
            misses: Default::default(),
            locks: Default::default(),
            greeted: Default::default(),
        }
    }

    /// Against scripts/fake-mail.py: `python3 scripts/fake-mail.py`, then
    /// `cargo test -p mimir-core fake_mail_tasks -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn fake_mail_tasks() {
        let p = std::env::temp_dir().join(format!("mimir-mailtask-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        db.set_setting("email_accounts", r#"[{"host":"127.0.0.1","port":1143,"user":"me@example.com"}]"#).unwrap();
        crate::host::install_secrets(Arc::new(crate::host::DbSecrets(db.clone())));
        crate::host::secret_set("email_password:me@example.com", "pw").unwrap();
        let core = core_on(&db, &fake_ai("200 OK", r#"{"choices":[{"message":{"role":"assistant","content":"Done."}}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#).await);
        // Turned on: what's there already isn't a task.
        core.mail_tasks(&[]).await.unwrap();
        assert!(db.messages(0, 10).unwrap().is_empty());
        assert_eq!(db.setting("mail_task_seen:INBOX").unwrap().as_deref(), Some("1:4"));
        // Mail since: the signed forward runs, the one posing as you is refused, the invoice isn't to +mimir.
        db.set_setting("mail_task_seen:INBOX", "1:2").unwrap();
        db.set_setting("mail_task_seen:Mimir", "1:0").unwrap();
        core.mail_tasks(&[]).await.unwrap();
        let said: Vec<String> = db.messages(0, 10).unwrap().into_iter().map(|m| format!("{}: {}", m.role, m.content)).collect();
        assert_eq!(said.len(), 5, "{said:#?}");
        assert!(said[0].starts_with("user: [By email, to your +mimir address.] Add this to my calendar for the 20th.") && said[0].contains("<document") && said[0].contains("password file"), "{said:#?}");
        assert!(said[2].starts_with("assistant: An email from your address (“Send the bank details") && said[2].contains("DKIM didn't pass"), "{said:#?}");
        assert_eq!(said[3], "user: [By email, filed in your Mimir folder.] Remind me to call the dentist at 4");
        assert!(db.conv_tainted(1, 0).unwrap(), "a forward is outside content");
        assert_eq!(db.setting("mail_task_seen:INBOX").unwrap().as_deref(), Some("1:4"));
        // Nothing new: nothing runs again.
        core.mail_tasks(&[]).await.unwrap();
        assert_eq!(db.messages(0, 10).unwrap().len(), 5);
    }

    #[tokio::test]
    async fn routine_failures_by_cause() {
        let p = std::env::temp_dir().join(format!("mimir-causes-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let now = crate::db::now();
        let r = db.routine_create(1, 0, "0 8 * * *", "brief me", false).unwrap();
        let fresh = |db: &Db| db.routines(0).unwrap().into_iter().find(|x| x.id == r.id).unwrap();

        // Can't reach the AI: tried again in 5 minutes, without a message; one item in Needs you that says so.
        let down = core_on(&db, "http://127.0.0.1:9/v1");
        down.run_routine(&[], fresh(&db), None).await.unwrap();
        let after = fresh(&db);
        assert!((after.retries, after.next_fire > now + 200 && after.next_fire <= now + 400) == (1, true), "{after:?}");
        down.run_routine(&[], after, None).await.unwrap();
        assert_eq!(fresh(&db).retries, 2);
        let failed = db.runs_failed(0).unwrap();
        assert_eq!((failed.len(), failed[0].repeats), (1, 2), "one item for the same failure");
        assert!(failed[0].error.as_deref().unwrap().contains("Trying again at"), "{:?}", failed[0].error);

        // The plan's usage limit: the bot's routines wait (an hour, when nobody says when it resets), this one runs then.
        let limited = core_on(&db, &fake_ai("429 Too Many Requests", r#"{"error":{"message":"Rate limit reached"}}"#).await);
        limited.run_routine(&[], fresh(&db), None).await.unwrap();
        let h = db.hold("hold:1").unwrap().expect("held");
        assert!(h.until > now + 3000 && h.until <= now + 3700 && fresh(&db).next_fire <= h.until, "{h:?}");
        assert_eq!(db.holds(now).unwrap().len(), 1, "another bot on an API key isn't held");
        assert_eq!(db.runs_failed(0).unwrap().len(), 2, "a usage limit is its own item");
        let ok = core_on(&db, &fake_ai("200 OK", r#"{"choices":[{"message":{"role":"assistant","content":"Three headlines."}}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#).await);

        // With a fallback account: it runs again there straight away, the run log says so, and at the fallback's own
        // limit it waits for the bot's account after all.
        db.set_fallback(1, "local", "m").unwrap();
        limited.run_routine(&[], fresh(&db), None).await.unwrap();
        assert!(fresh(&db).next_fire <= crate::db::now(), "runs again on the next pass");
        assert!(db.runs(Some(0), 1).unwrap()[0].error.as_deref().unwrap().contains("on the fallback account until"));
        let until = db.on_fallback(1, now).unwrap().expect("on the fallback").2;
        ok.run_routine(&[], fresh(&db), Some(&(ok.provider.clone(), until))).await.unwrap();
        assert!(db.runs(Some(0), 1).unwrap()[0].delivered.as_deref().unwrap().starts_with("in the app · on the fallback account until"));
        limited.run_routine(&[], fresh(&db), Some(&(limited.provider.clone(), until))).await.unwrap();
        assert!(db.runs(Some(0), 1).unwrap()[0].error.as_deref().unwrap().contains("fallback account was at its usage limit too"));
        assert!(db.on_fallback(1, now).unwrap().is_none(), "the fallback is spent until the hold ends");
        db.set_fallback(1, "", "").unwrap();

        // It works again: its failures leave Needs you, the tries start over, and the result's whereabouts are logged.
        ok.run_routine(&[], fresh(&db), None).await.unwrap();
        assert_eq!(fresh(&db).retries, 0);
        assert!(db.runs_failed(0).unwrap().is_empty(), "healed");
        assert_eq!(db.runs(Some(0), 1).unwrap()[0].delivered.as_deref(), Some("in the app"));

        // A one-off stays until its run is over (a crash mid-run tries it again), then it's gone.
        let once = db.routine_once(1, 0, now + 60, "call mum", false).unwrap();
        db.routine_advance(&once).unwrap();
        assert!(db.routines(0).unwrap().iter().any(|x| x.id == once.id && x.next_fire > now + 3000));
        ok.run_routine(&[], once.clone(), None).await.unwrap();
        assert!(!db.routines(0).unwrap().iter().any(|x| x.id == once.id));
    }

    /// A pretend AI that answers "Seen." and keeps what each request asked.
    async fn recording_ai() -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (addr, seen) = (l.local_addr().unwrap(), Arc::new(std::sync::Mutex::new(vec![])));
        let keep = seen.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut s, _)) = l.accept().await {
                let mut req = vec![];
                let mut buf = vec![0u8; 65536];
                // Until the whole body is in (Content-Length past the headers).
                while let Ok(n) = s.read(&mut buf).await {
                    req.extend_from_slice(&buf[..n]);
                    let t = String::from_utf8_lossy(&req).to_string();
                    let len = t.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()));
                    if n == 0 || t.find("\r\n\r\n").zip(len).is_some_and(|(h, len)| req.len() >= h + 4 + len) {
                        break;
                    }
                }
                keep.lock().unwrap().push(String::from_utf8_lossy(&req).to_string());
                let body = r#"{"choices":[{"message":{"role":"assistant","content":"Seen."}}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#;
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            }
        });
        (format!("http://{addr}/v1"), seen)
    }

    #[tokio::test]
    async fn event_routines_run_on_what_came() {
        let p = std::env::temp_dir().join(format!("mimir-events-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let r = db.routine_create(1, 0, "webhook", "say what the build did", false).unwrap();
        let fresh = |db: &Db| db.routine(r.id).unwrap();
        // Due with nothing come: no run.
        let (base, asked) = recording_ai().await;
        let core = core_on(&db, &base);
        core.run_routine(&[], r.clone(), None).await.unwrap();
        assert!(asked.lock().unwrap().is_empty() && db.runs(Some(0), 5).unwrap().is_empty());
        // Can't reach the AI: what came waits for the try after.
        db.routine_event_add(r.id, "{\"build\":\"red\"}").unwrap();
        core_on(&db, "http://127.0.0.1:9/v1").run_routine(&[], fresh(&db).unwrap(), None).await.unwrap();
        assert_eq!(db.routine_events(r.id).unwrap().len(), 1);
        core.run_routine(&[], fresh(&db).unwrap(), None).await.unwrap();
        let req = asked.lock().unwrap().pop().unwrap();
        assert!(req.contains("its webhook was called") && req.contains("build") && req.contains("red") && req.contains("outside content"), "{req}");
        assert!(db.routine_events(r.id).unwrap().is_empty(), "seen");
        assert_eq!(fresh(&db).unwrap().next_fire, crate::db::IDLE, "waits for the next call");
        assert!(db.runs(Some(0), 1).unwrap()[0].status == "ok");
        // "Tell me when it arrives": gone after the first run that worked.
        db.routine_set_once(r.id).unwrap();
        db.routine_event_add(r.id, "{\"build\":\"green\"}").unwrap();
        core.run_routine(&[], fresh(&db).unwrap(), None).await.unwrap();
        assert!(fresh(&db).is_none());
    }

    #[tokio::test]
    async fn handed_over_tasks_report_back_later() {
        let p = std::env::temp_dir().join(format!("mimir-later-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let (base, asked) = recording_ai().await;
        let scout = crate::db::Bot { id: 0, name: "Scout".into(), persona: String::new(), provider: "openai".into(), model: "m".into(), base_url: base.clone(), tools: String::new(), look: String::new() };
        let scout = db.bot_save(&scout).unwrap();
        let run = db.run_start(1, 0, "message", None, "test").unwrap();
        let ctx = ToolCtx::new(db.clone(), 1, 0).with_run(Some(run));
        let out = crate::tools::call(&ctx, "ask_bot", &serde_json::json!({"bot": "Scout", "task": "Find three padel courts", "later": true})).await.unwrap();
        assert!(out.starts_with("Handed to Scout") && asked.lock().unwrap().is_empty(), "nothing waits for it: {out}");
        // The helper runs on its own; a pass after it's done brings its result back to the chat, from the bot asked.
        let core = Arc::new(core_on(&db, &base));
        core.delegations(&[]).await.unwrap();
        for _ in 0..100 {
            if !db.delegations(1, "done").unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        core.delegations(&[]).await.unwrap();
        let reqs = asked.lock().unwrap().clone();
        assert_eq!(reqs.len(), 2);
        assert!(reqs[0].contains("[Task from ") && reqs[0].contains("padel courts"), "{}", reqs[0]);
        assert!(reqs[1].contains("[Report from Scout") && reqs[1].contains("padel courts") && reqs[1].contains("Seen."), "{}", reqs[1]);
        let linked: Vec<_> = db.child_runs(run).unwrap().into_iter().map(|r| (r.bot_id, r.trigger)).collect();
        assert_eq!(linked, [(scout, "delegated".to_string()), (1, "reported".to_string())]);
        let said = db.messages(0, 1).unwrap().pop().unwrap();
        assert!(said.bot_id == 1 && said.content.starts_with("Seen."), "{}", said.content);
        assert!(db.delegations(1, "done").unwrap().is_empty() && db.delegations(1, "pending").unwrap().is_empty(), "told once");
        // Stopped mid-task by a restart: not run again; the bot that asked says it didn't finish. Outside content the
        // asker had read stays marked on the report.
        let d = crate::db::Delegation { bot_id: 1, to_bot: scout, chat_id: 0, conv: 0, run_id: Some(run), task: "x".into(), tainted: true, ..Default::default() };
        let id = db.delegation_add(&d).unwrap();
        db.delegation_status(id, "running").unwrap();
        db.close_interrupted().unwrap();
        core.delegations(&[]).await.unwrap();
        let reqs = asked.lock().unwrap().clone();
        assert_eq!(reqs.len(), 3);
        assert!(reqs[2].contains("It didn't finish: Mimir stopped"), "{}", reqs[2]);
        assert!(db.run_tainted(db.runs(Some(0), 1).unwrap()[0].id).unwrap());
    }

    #[tokio::test]
    async fn a_later_hand_off_keeps_the_askers_limits() {
        let p = std::env::temp_dir().join(format!("mimir-later-limits-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let scout = crate::db::Bot { id: 0, name: "Scout".into(), persona: String::new(), provider: "openai".into(), model: "m".into(), base_url: String::new(), tools: String::new(), look: String::new() };
        db.bot_save(&scout).unwrap();
        db.pair(200, "Sam").unwrap();
        db.chat_set_guest(200, true).unwrap();
        // After reading your email, the helper running later still asks before any web address.
        let ctx = ToolCtx::new(db.clone(), 1, 200);
        ctx.mark_private();
        crate::tools::call(&ctx, "ask_bot", &serde_json::json!({"bot": "Scout", "task": "Find three padel courts", "later": true})).await.unwrap();
        let d = db.delegations(1, "pending").unwrap().pop().unwrap();
        assert!(db.setting(&crate::tools::delegation_private_key(d.id)).unwrap().is_some());
        // A guest's hand-off runs in the guest's chat, so the helper gets the guest's tools, not yours.
        assert_eq!(d.chat_id, 200);
        let helper = crate::tools::specs_for(&ToolCtx::new(db.clone(), d.to_bot, d.chat_id)).into_iter().map(|s| s.0).collect::<Vec<_>>();
        assert!(!helper.iter().any(|t| t.starts_with("email_") || t.starts_with("computer_") || t.starts_with("browser__") || t == "bot_create"), "{helper:?}");
    }

    /// Against scripts/fake-mail.py: `python3 scripts/fake-mail.py`, then
    /// `cargo test -p mimir-core fake_mail_triggers -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn fake_mail_triggers() {
        let p = std::env::temp_dir().join(format!("mimir-mailrule-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        db.set_setting("email_accounts", r#"[{"host":"127.0.0.1","port":1143,"user":"me@example.com"}]"#).unwrap();
        crate::host::install_secrets(Arc::new(crate::host::DbSecrets(db.clone())));
        crate::host::secret_set("email_password:me@example.com", "pw").unwrap();
        crate::events::ready(&db, 0, "mail:from:example.com invoice").unwrap();
        let r = db.routine_create(1, 0, "mail:from:sam@example.com invoice", "tell me the amount", false).unwrap();
        let other = db.routine_create(1, 0, "mail:subject:\"account notice\" invoices", "tell me", false).unwrap();
        let never = db.routine_create(1, 0, "mail:from:nobody.example", "tell me", false).unwrap();
        // The first look only notes where the inbox ends.
        crate::events::look_mail(&db, 1).await;
        assert!(db.routine_events(r.id).unwrap().is_empty());
        for id in [r.id, other.id, never.id] {
            db.routine_set_seen(id, "1:0").unwrap();
        }
        crate::events::look_mail(&db, 1).await;
        let evs = db.routine_events(r.id).unwrap();
        // Sam's invoice, and Sam's mail forwarded by you (from you, so not from Sam): one.
        assert_eq!(evs.len(), 1, "{evs:?}");
        assert!(evs[0].1.contains("Subject: Invoice 4411") && evs[0].1.contains("EUR 1,250"), "{evs:?}");
        assert_eq!(db.routine_events(other.id).unwrap().len(), 1, "the HTML one, matched on its text");
        assert!(db.routine_events(never.id).unwrap().is_empty());
        assert_eq!(db.routine_seen(r.id).unwrap().as_deref(), Some("1:4"));
        assert!(db.due(1, crate::db::now()).unwrap().iter().any(|x| x.id == r.id) && !db.due(1, crate::db::now()).unwrap().iter().any(|x| x.id == never.id));
        // Nothing new: nothing more.
        crate::events::look_mail(&db, 1).await;
        assert_eq!(db.routine_events(r.id).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_crash_in_a_routine_fails_that_run_only() {
        let p = std::env::temp_dir().join(format!("mimir-crash-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let core = core_on(&db, "http://127.0.0.1:9/v1");
        let r = db.routine_create(1, 0, "0 8 * * *", "brief me", false).unwrap();
        let d = db.clone();
        let out = core
            .own_task(0, async move {
                d.run_start(1, 0, "routine", Some(r.id), "test")?;
                if r.id > 0 {
                    panic!("boom");
                }
                Ok(())
            })
            .await;
        assert!(out.unwrap_err().to_string().contains("boom"));
        // The run it left open is a failed run, in Needs you, and the caller (the scheduler) carries on.
        let failed = db.runs_failed(0).unwrap();
        assert!(failed.len() == 1 && failed[0].error.as_deref().unwrap().contains("crashed"), "{failed:?}");
        assert_eq!(core.own_task(0, async { Ok(7) }).await.unwrap(), 7);
    }

    #[tokio::test]
    async fn first_runs_ask_what_to_trim() {
        let p = std::env::temp_dir().join(format!("mimir-tune-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let ok = core_on(&db, &fake_ai("200 OK", r#"{"choices":[{"message":{"role":"assistant","content":"Three headlines."}}]}"#).await);
        let r = db.routine_create(1, 0, "0 8 * * *", "brief me", false).unwrap();
        for _ in 0..4 {
            let r = db.routines(0).unwrap().into_iter().find(|x| x.id == r.id).unwrap();
            ok.run_routine(&[], r, None).await.unwrap();
        }
        let said: Vec<String> = db.messages(0, 8).unwrap().into_iter().filter(|m| m.role == "assistant").map(|m| m.content).collect();
        let asks = said.iter().filter(|m| m.contains(&format!("adjust this routine (#{})", r.id))).count();
        assert_eq!((said.len(), asks), (4, 3), "{said:?}");
        // That AI didn't say what it used: estimated from the text, and marked so.
        let run = &db.runs(Some(0), 1).unwrap()[0];
        assert!(run.estimated && run.tokens_in > 0 && run.tokens_out > 0, "{run:?}");
    }

    #[tokio::test]
    async fn daily_caps_pause_and_ask() {
        let p = std::env::temp_dir().join(format!("mimir-caps-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let ok = core_on(&db, &fake_ai("200 OK", r#"{"choices":[{"message":{"role":"assistant","content":"Done."}}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#).await);
        let r = db.routine_create(1, 0, "0 8 * * *", "brief me", false).unwrap();
        let other = db.routine_create(1, 0, "0 9 * * *", "and this", false).unwrap();
        let get = |id: i64| db.routines(0).unwrap().into_iter().find(|x| x.id == id).unwrap();
        db.set_setting("cap_routine", "5").unwrap();
        ok.run_routine(&[], get(r.id), None).await.unwrap();
        // 5 tokens used: the next run would go over this routine's cap. It pauses until tomorrow, still due.
        let before = get(r.id).next_fire;
        ok.run_routine(&[], get(r.id), None).await.unwrap();
        let key = format!("hold:routine:{}", r.id);
        let h = db.hold(&key).unwrap().expect("paused");
        assert!(h.why.contains("this routine reached the daily cap of 5 tokens") && h.until > crate::db::now(), "{h:?}");
        assert_eq!((get(r.id).next_fire, db.runs(Some(0), 9).unwrap().len()), (before, 1), "no run, nothing advanced");
        let held = db.holds(0).unwrap();
        assert_eq!((held.len(), held[0].2), (1, Some(r.id)));
        // Another routine has its own cap.
        ok.run_routine(&[], get(other.id), None).await.unwrap();
        assert_eq!(db.runs(Some(0), 9).unwrap().len(), 2);
        // "Let it go on today": it runs, and doesn't pause again today.
        db.hold_clear(&key, true).unwrap();
        assert!(db.holds(crate::db::now()).unwrap().is_empty(), "ended, out of Needs you");
        assert!(db.hold(&key).unwrap().is_some_and(|h| h.until < crate::db::now()), "kept, so a late run says why");
        ok.run_routine(&[], get(r.id), None).await.unwrap();
        ok.run_routine(&[], get(r.id), None).await.unwrap();
        assert!(db.hold(&key).unwrap().is_some_and(|h| h.until < crate::db::now()), "not paused again today");
        assert_eq!(db.runs(Some(0), 9).unwrap().len(), 4);
        // All bots together: a bot-wide pause.
        db.set_setting("cap_routine", "0").unwrap();
        db.set_setting("cap_day", "10").unwrap();
        ok.run_routine(&[], get(other.id), None).await.unwrap();
        assert!(db.hold("hold:1").unwrap().unwrap().why.contains("all your bots together"));
        // Ending on its own (the day is over) doesn't let tomorrow go over the cap.
        db.hold_clear("hold:1", false).unwrap();
        assert_eq!(db.setting("cap_waived:day").unwrap(), None);
        // Cache reads weigh a tenth, as they're billed: 20,000 read from the cache + 1,000 more is 3,000.
        let p2 = std::env::temp_dir().join(format!("mimir-caps2-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p2);
        let db = Db::open(&p2).unwrap();
        let r = db.routine_create(1, 0, "0 8 * * *", "brief me", false).unwrap();
        let run = db.run_start(1, 0, "routine", Some(r.id), "claude -p").unwrap();
        db.run_finish(run, Ok(&crate::db::Usage { tokens_in: 20_500, tokens_out: 500, cache_read: 20_000, ..Default::default() })).unwrap();
        assert_eq!(db.routine_stats(0, crate::db::now()).unwrap()[0].tokens, 3_000);
        db.set_setting("cap_routine", "3500").unwrap();
        assert!(db.over_cap(&r, 0).unwrap().is_none());
        db.set_setting("cap_routine", "3000").unwrap();
        assert!(db.over_cap(&r, 0).unwrap().is_some());
    }

    /// A Telegram double that counts what it's sent and answers ok.
    async fn fake_telegram() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (addr, sent) = (l.local_addr().unwrap(), Arc::new(std::sync::atomic::AtomicUsize::new(0)));
        let count = sent.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut s, _)) = l.accept().await {
                let mut buf = vec![0u8; 65536];
                let n = s.read(&mut buf).await.unwrap_or(0);
                if String::from_utf8_lossy(&buf[..n]).contains("/sendMessage") {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                let body = r#"{"ok":true,"result":{"message_id":1}}"#;
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            }
        });
        (format!("http://{addr}"), sent)
    }

    #[tokio::test]
    async fn the_outbox_sends_what_a_crash_left_once() {
        let p = std::env::temp_dir().join(format!("mimir-outbox-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let core = core_on(&db, "http://127.0.0.1:9");
        let chat = db.pair_external("telegram", "42", "me").unwrap();
        let (api, sent) = fake_telegram().await;
        let tg = Chan::Telegram(Tg::at(&api, "t").unwrap());
        let n = || sent.load(std::sync::atomic::Ordering::SeqCst);
        // A routine's result another process kept and died sending (a crash, an update): sent at the next pass, once.
        let run = db.run_start(1, chat, "routine", None, "m").unwrap();
        db.run_delivered(run, "waiting to send to telegram · on the fallback account until 10:00").unwrap();
        let id = db.outbox_add(1, chat, "telegram", "42", None, "the result", "[]", "[]", Some(run)).unwrap();
        db.c().execute("UPDATE outbox SET claimed_by='gone' WHERE id=?1", [id]).unwrap();
        core.resend(std::slice::from_ref(&tg)).await.unwrap();
        assert_eq!(n(), 1);
        assert_eq!(db.runs(Some(chat), 1).unwrap()[0].delivered.as_deref(), Some("sent to telegram · on the fallback account until 10:00"));
        core.resend(std::slice::from_ref(&tg)).await.unwrap();
        assert_eq!(n(), 1, "not sent twice");
        // Said now: sent, and nothing left over.
        core.say(&tg, "42", chat, None, "a reply", &[], &[], None).await;
        assert_eq!(n(), 2);
        assert!(db.outbox_take(1, crate::db::now() + 3600).unwrap().is_empty());
        // Telegram unreachable: kept, and not taken again by this process until it's stale (its own send may still be going).
        let down = Chan::Telegram(Tg::at("http://127.0.0.1:9", "t").unwrap());
        core.say(&down, "42", chat, None, "lost?", &[], &[], None).await;
        assert!(db.outbox_take(1, crate::db::now()).unwrap().is_empty());
        core.resend(std::slice::from_ref(&tg)).await.unwrap();
        assert_eq!(n(), 2);
        let left = db.outbox_take(1, crate::db::now() + crate::db::OUTBOX_STALE + 1).unwrap();
        assert_eq!(left.iter().map(|o| o.text.as_str()).collect::<Vec<_>>(), vec!["lost?"]);
        // Claimed by that take: a second one finds nothing.
        assert!(db.outbox_take(1, crate::db::now() + crate::db::OUTBOX_STALE + 1).unwrap().is_empty());
    }

    #[tokio::test]
    async fn routine_asks_also_go_to_the_decisions_chat_and_close_everywhere() {
        let p = std::env::temp_dir().join(format!("mimir-decisions-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let core = Core {
            db: db.clone(),
            bot: 1,
            provider: Arc::new(Provider::new("claude", None, None, None, std::env::temp_dir()).unwrap()),
            code: Default::default(),
            status: Default::default(),
            misses: Default::default(),
            locks: Default::default(),
            greeted: Default::default(),
        };
        let tg = Chan::Telegram(Tg::new("t").unwrap());
        let me = Sender { id: "42", dm: true };
        // Only your own private chat in a messaging app can be it: not a group, a guest's chat, or the app.
        for (ext, owner) in [("42", "42"), ("-100", "42"), ("77", "77"), ("5", "42")] {
            let chat = db.pair_external("telegram", ext, ext).unwrap();
            db.chat_set_owner(chat, owner).unwrap();
        }
        db.chat_set_guest(77, true).unwrap();
        db.pair(crate::host::LOCAL_CHAT, "This computer").unwrap();
        assert!(decisions_ok(&db, 42) && !decisions_ok(&db, -100) && !decisions_ok(&db, 77) && !decisions_ok(&db, crate::host::LOCAL_CHAT));
        db.set_setting("decisions_chat", "77").unwrap();
        assert_eq!(decisions_chat(&db), None, "a guest's chat is never used, even if saved");
        db.set_setting("decisions_chat", "42").unwrap();
        assert_eq!(decisions_chat(&db), Some(42));
        let ask = |chat, parked| {
            let id = db.approval_create(1, chat, "email_send", "send it", 3600, "rule", None).unwrap();
            if parked {
                db.approval_park(id, "{}", false).unwrap();
            }
            id
        };
        // A routine's ask in the app chat goes to the decisions chat; a live ask, or one from the decisions chat itself, doesn't.
        let (app, live) = (ask(crate::host::LOCAL_CHAT, true), ask(5, false));
        ask(42, true);
        assert_eq!(db.approvals_to_mirror(1, 42).unwrap().iter().map(|a| a.id).collect::<Vec<_>>(), vec![app]);
        db.approval_mark_mirrored(app, 42, Some("m1")).unwrap();
        assert!(db.approvals_to_mirror(1, 42).unwrap().is_empty(), "sent once");
        // Another chat can't answer it; the decisions chat can, and its own card is the one its button edits.
        assert!(core.decide(&tg, "5", me, app, Decision::Once).unwrap().is_none());
        assert_eq!(core.decide(&tg, "42", me, app, Decision::Deny).unwrap().unwrap().status, "denied");
        assert!(db.mirrors_to_close(1).unwrap().is_empty());
        // A routine's ask from another chat, answered there: the decisions chat's card closes too.
        let other = ask(5, true);
        db.approval_mark_notified(other, Some("c5")).unwrap();
        db.approval_mark_mirrored(other, 42, Some("m2")).unwrap();
        assert!(core.decide(&tg, "5", me, other, Decision::Once).unwrap().is_some());
        assert_eq!(db.mirrors_to_close(1).unwrap().into_iter().map(|(a, c, r)| (a.id, c, r)).collect::<Vec<_>>(), vec![(other, 42, "m2".to_string())]);
        assert!(db.approvals_to_close(1).unwrap().is_empty(), "the button edited its own card");
        // The decisions chat can't answer asks that weren't sent to it.
        assert!(core.decide(&tg, "42", me, live, Decision::Once).unwrap().is_none());
        // A text reply to a card: no, and why. Only that card, in its own chat, from the person who pairs it.
        let card = ask(5, false);
        db.approval_mark_notified(card, Some("c9")).unwrap();
        assert!(!core.deny_by_reply(&tg, "42", me, "c9", "not now").unwrap(), "another chat");
        assert!(!core.deny_by_reply(&tg, "5", Sender { id: "9", dm: false }, "c9", "not now").unwrap(), "someone else");
        assert!(!core.deny_by_reply(&tg, "5", me, "c8", "not now").unwrap(), "another message");
        assert!(core.deny_by_reply(&tg, "5", me, "c9", " not now, after lunch ").unwrap());
        let a = db.approval(card).unwrap().unwrap();
        assert_eq!((a.status.as_str(), a.reason.as_deref()), ("denied", Some("not now, after lunch")));
        assert!(verdict(&a).ends_with("Why: not now, after lunch"), "{}", verdict(&a));
        assert!(db.approvals_to_close(1).unwrap().iter().any(|(a, _)| a.id == card), "the approvals loop shows it on the card");
        assert!(!core.deny_by_reply(&tg, "5", me, "c9", "again").unwrap(), "answered once");
        // Discord keeps its cards as channel/message.
        let dc = ask(5, false);
        db.approval_mark_notified(dc, Some("C1/123")).unwrap();
        assert!(core.deny_by_reply(&tg, "5", me, "123", "no").unwrap());
    }

    #[test]
    fn a_card_you_can_edit_links_to_the_app() {
        let p = std::env::temp_dir().join(format!("mimir-link-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let id = db.approval_create(1, 5, "email_send", "Send an email to sam@example.com: \"Hi\" — Hello", 300, "send", None).unwrap();
        db.approval_set_draft(id, &json!({"to": "sam@example.com", "subject": "Hi", "body": "Hello"})).unwrap();
        let plain = db.approval_create(1, 5, "memory_forget", "Forget memory #1", 300, "rule", None).unwrap();
        let (mut a, mut b) = (db.approval(id).unwrap().unwrap(), db.approval(plain).unwrap().unwrap());
        assert_eq!(edit_link(&db, &a), None, "no web app address known");
        db.set_setting("public_url", "http://192.168.1.5:8080").unwrap();
        assert_eq!(edit_link(&db, &a), None, "Telegram refuses links that aren't https");
        db.set_setting("public_url", "https://bots.example.com/").unwrap();
        a.edit_link = edit_link(&db, &a);
        b.edit_link = edit_link(&db, &b);
        assert_eq!(a.edit_link.as_deref(), Some(format!("https://bots.example.com/#approval-{id}").as_str()));
        assert!(b.edit_link.is_none(), "nothing to edit");
        assert!(card_buttons(&a).concat().iter().any(|(l, d)| l == "Edit in Mimir" && d.starts_with("https://")));
        assert!(card_buttons(&b).concat().iter().all(|(_, d)| d.starts_with("ap:")));
        assert!(card_text(&a).contains("Reply to this message to say no and why."));
    }

    #[tokio::test]
    async fn catch_up_cursor_and_strangers_files() {
        let p = std::env::temp_dir().join(format!("mimir-seen-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let core = Arc::new(Core {
            db: db.clone(),
            bot: 1,
            provider: Arc::new(Provider::new("claude", None, None, None, std::env::temp_dir()).unwrap()),
            code: Default::default(),
            status: Default::default(),
            misses: Default::default(),
            locks: Default::default(),
            greeted: Default::default(),
        });
        let chan = Chan::Discord(Discord::new("t").unwrap());
        assert!(core.mark_seen(&chan, "C1", 10) && core.seen(&chan, "C1").is_none(), "unpaired chats keep no cursor");
        let chat = db.pair_external("discord", "C1", "#c1").unwrap();
        db.chat_set_owner(chat, "U1").unwrap();
        assert!(core.mark_seen(&chan, "C1", 10));
        assert!(!core.mark_seen(&chan, "C1", 10), "the same message twice (a resend)");
        assert!(!core.mark_seen(&chan, "C1", 5), "an older one");
        assert!(core.mark_seen(&chan, "C1", 11) && core.seen(&chan, "C1") == Some(11));
        assert_eq!(core.paired_on(&chan), vec![("C1".to_string(), false)]);
        // A stranger's file in the owner's channel is never even fetched.
        let files = [File { name: "a.png".into(), mime: "image/png".into(), size: 10, url: "https://cdn.discordapp.com/a.png".into(), voice: false }];
        let stranger = Sender { id: "U2", dm: false };
        core.incoming_files(&chan, "C1", stranger, "", &files, None, |_| async { panic!("fetched a stranger's file") }).await.unwrap();
        // Unpaired, the chat forgets where catch-up left off (another app's chat with the same id keeps its own).
        let other = Chan::Slack(crate::slack::Slack::new("xoxb-1", "xapp-1").unwrap());
        db.pair_external("slack", "C1", "c1").unwrap();
        assert!(core.mark_seen(&other, "C1", 3));
        db.unpair(chat).unwrap();
        assert_eq!((core.seen(&chan, "C1"), core.seen(&other, "C1")), (None, Some(3)));
    }

    #[tokio::test]
    async fn tap_to_answer_snooze_and_rate() {
        let p = std::env::temp_dir().join(format!("mimir-taps-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let core = Arc::new(core_on(&db, "http://127.0.0.1:9/v1"));
        let chan = Chan::Discord(Discord::new("t").unwrap());
        let chat = db.pair_external("discord", "C1", "#c1").unwrap();
        db.chat_set_owner(chat, "U1").unwrap();
        let me = Sender { id: "U1", dm: false };
        let labels = |rows: &[Row]| rows.iter().map(|r| r.iter().map(|b| b.0.as_str()).collect::<Vec<_>>().join("|")).collect::<Vec<_>>();
        let now = crate::db::now();

        // ask_choice: only where someone can tap (a paired messaging-app chat, not a routine, not the app's own chat).
        let ctx = ToolCtx::new(db.clone(), 1, chat);
        let offered = |c: &ToolCtx| crate::tools::specs_for(c).iter().any(|s| s.0 == "ask_choice");
        let mut unattended = ctx.clone();
        unattended.unattended = true;
        assert!(offered(&ctx) && !offered(&unattended) && !offered(&ToolCtx::new(db.clone(), 1, 0)));
        assert!(unattended.mcp_args().contains(&"--unattended".to_string()), "the MCP process knows too");
        assert!(crate::tools::call(&ctx, "ask_choice", &json!({"question": "Which day?", "options": ["Thu"]})).await.is_err());
        crate::tools::call(&ctx, "ask_choice", &json!({"question": "Which day?", "options": ["Thu", " Fri "]})).await.unwrap();
        let (q, rows) = core.take_choice(chat).unwrap();
        assert_eq!((q.as_str(), labels(&rows)), ("Which day?", vec!["Thu".to_string(), "Fri".into(), "Something else".into()]));
        assert!(core.take_choice(chat).is_none(), "one reply's question only");

        // A reminder: Snooze makes it again in an hour; the message's other buttons are then used up.
        let once = db.routine_once(1, chat, now + 60, "call mum", false).unwrap();
        let rows = core.result_rows(&once, "Call mum!");
        assert_eq!(labels(&rows), ["Done|Snooze 1h|Tomorrow"]);
        assert!(core.tap(&chan, "C1", me, &rows[0][1].1).await.unwrap().unwrap().contains("again today"));
        let again: Vec<_> = db.routines(chat).unwrap().into_iter().filter(|r| r.once && r.id != once.id).collect();
        assert!(again.len() == 1 && again[0].prompt == "call mum" && (again[0].next_fire - now - 3600).abs() < 5, "{again:?}");
        assert_eq!(core.tap(&chan, "C1", me, &rows[0][2].1).await.unwrap().as_deref(), Some("Already done."));
        assert_eq!(next_day(now - 600, now), now - 600 + 86400, "Tomorrow: the same time tomorrow");

        // Someone else in the channel can't press them (and doesn't use them up); not a tap button at all: None.
        let rows = core.result_rows(&once, "Call mum!");
        assert!(core.tap(&chan, "C1", Sender { id: "U2", dm: false }, &rows[0][0].1).await.is_err());
        assert_eq!(core.tap(&chan, "C1", me, &rows[0][0].1).await.unwrap().as_deref(), Some("✅ Done"));
        assert!(core.tap(&chan, "C1", me, "ap:1:y").await.unwrap().is_none());

        // A routine's result: 👎 is a correction for the tidy-up to learn from (nothing is changed); Pause pauses it.
        let r = db.routine_create(1, chat, "0 8 * * *", "brief me", false).unwrap();
        let rows = core.result_rows(&r, "Three headlines.");
        assert_eq!(labels(&rows), ["👍|👎|Less like this", "Pause|For tomorrow…"]);
        core.tap(&chan, "C1", me, &rows[0][1].1).await.unwrap();
        let signals = db.signals_since(chat, 0).unwrap();
        assert!(signals.iter().any(|(k, d)| k == "correction" && d.contains(&format!("#{}", r.id)) && d.contains("Three headlines.") && d.contains("👎")), "{signals:?}");
        assert_eq!(db.routines(chat).unwrap().iter().find(|x| x.id == r.id).unwrap().prompt, "brief me");
        let rows = core.result_rows(&r, "Three headlines.");
        core.tap(&chan, "C1", me, &rows[1][0].1).await.unwrap();
        assert!(db.routines(chat).unwrap().iter().find(|x| x.id == r.id).unwrap().paused);

        // Another chat's buttons don't work here.
        let other = db.pair_external("discord", "C2", "#c2").unwrap();
        db.chat_set_owner(other, "U1").unwrap();
        let rows = core.result_rows(&r, "x");
        assert_eq!(core.tap(&chan, "C2", me, &rows[0][0].1).await.unwrap().as_deref(), Some("Already done."));
        assert_eq!(core.tap(&chan, "C1", me, &rows[0][0].1).await.unwrap().as_deref(), Some("👍 Noted."), "still there for its own chat");
    }

    #[test]
    fn always_from_a_chat_card_makes_a_rule_only_for_plain_asks() {
        let p = std::env::temp_dir().join(format!("mimir-always-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let ask = |tool: &str, kind: &str, site: Option<&str>| {
            let id = db.approval_create(1, 5, tool, "do it", 60, kind, site).unwrap();
            db.approval_decide(id, true, Some(5)).unwrap().unwrap()
        };
        // A plain ask: the card offers Always, and it allows the tool from now on.
        let a = ask("memory_forget", "rule", None);
        let rows = card_buttons(&a);
        assert!(rows.iter().flatten().any(|(l, d)| l.starts_with("Always") && d.ends_with(":w")), "{rows:?}");
        crate::tools::always_allow(&db, &a).unwrap();
        assert_eq!(db.rule("memory_forget").unwrap().as_deref(), Some("allow"));
        assert!(verdict_for(&a, Decision::Always).contains("without asking"));
        // A website: only that site.
        let w = ask("web_fetch", "rule", Some("example.com"));
        assert_eq!(crate::tools::always_label(&w), "Always on example.com");
        crate::tools::always_allow(&db, &w).unwrap();
        assert!(db.site_rules().unwrap().iter().any(|(t, h, act)| t == "web_fetch" && h == "example.com" && act == "allow"));
        assert_ne!(db.rule("web_fetch").unwrap().as_deref(), Some("allow"), "not the whole tool");
        // After outside content, saved logins and plans keep asking: no button, and refused if forced.
        for kind in ["guarded", "login", "plan"] {
            let g = ask("routine_create", kind, None);
            assert!(!card_buttons(&g).iter().flatten().any(|(_, d)| d.ends_with(":w")), "{kind}");
            assert!(crate::tools::always_allow(&db, &g).is_err(), "{kind}");
        }
        assert_ne!(db.rule("routine_create").unwrap().as_deref(), Some("allow"));
        // Allowed twice before: the card says so and Always moves up.
        let mut again = ask("memory_store", "rule", None);
        again.allowed_before = 2;
        assert!(card_text(&again).contains("allowed this 2 times"));
        assert!(card_buttons(&again)[1][0].1.ends_with(":w"), "{:?}", card_buttons(&again));
        assert!(!card_text(&ask("memory_store", "guarded", None)).contains("allowed this"));
    }

    #[test]
    fn commands_and_decisions() {
        assert!(matches!(parse_command("/pair AB12"), Command::Pair("AB12")));
        assert!(matches!(parse_command("pair  ab12 "), Command::Pair("ab12")));
        assert!(matches!(parse_command("Pair AB12"), Command::Pair("AB12")));
        assert!(matches!(parse_command("pairing is fun"), Command::Chat));
        assert!(matches!(parse_command("/memory"), Command::Memory));
        assert!(matches!(parse_command("/why"), Command::Why));
        assert!(matches!(parse_command("what's in my memory?"), Command::Chat));
        assert!(matches!(parse_command("/stop"), Command::Stop(true)));
        assert!(matches!(parse_command("/undo 2"), Command::Undo(2)));
        assert!(matches!(parse_command("/start 1F971F85CE"), Command::Pair("1F971F85CE")), "deep link");
        assert!(matches!(parse_command("/start"), Command::Start));
        assert_eq!(late_note(Some(1000), 1000 + 300), None);
        assert_eq!(late_note(None, 99999), None);
        assert_eq!(late_note(Some(0), 25 * 60).unwrap(), "Sorry for the slow reply: I was offline when you wrote 25 minutes ago.");
        assert!(late_note(Some(0), 3 * 3600 + 5).unwrap().ends_with("3 hours ago."));
        assert!(late_note(Some(0), 30 * 3600).unwrap().ends_with("yesterday."));
        assert!(matches!(parse_command("undo 3"), Command::Undo(3)));
        assert!(matches!(parse_command("undo that please"), Command::Chat), "not a number: just chat");
        assert!(matches!(parse_command(" Stop "), Command::Stop(false)));
        assert!(matches!(parse_command("stop the alarm routine"), Command::Chat));
        assert_eq!(parse_decision("ap:42:y"), Some((42, Decision::Once)));
        assert_eq!(parse_decision("ap:42:n"), Some((42, Decision::Deny)));
        assert_eq!(parse_decision("ap:42:a"), Some((42, Decision::RestOfRun)));
        assert_eq!(parse_decision("ap:42:w"), Some((42, Decision::Always)));
        assert_eq!(parse_decision("ap:42:q"), None);
        assert_eq!(parse_decision("ap:x:y"), None);
        assert_eq!(parse_decision("other"), None);
    }
}
