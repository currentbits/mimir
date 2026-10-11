//! The always-on runtime for one bot: channel listeners (Telegram, Slack, Discord) + routine
//! scheduler + approval cards. Shared by the CLI (`mimir serve`) and the desktop app, which
//! aborts/respawns it on settings change.

use crate::db::{Approval, Db};
use crate::{Trigger, discord::Discord, provider::Provider, slack::Slack, telegram::Tg, tools::ToolCtx, turn};
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
}

/// A connected messaging channel. Enum dispatch keeps the adapters plain structs.
#[derive(Clone)]
pub(crate) enum Chan {
    Telegram(Tg),
    Slack(Slack),
    Discord(Discord),
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
        }
    }

    pub(crate) async fn send(&self, ext: &str, text: &str) {
        if let Err(e) = self.try_send(ext, text).await {
            eprintln!("{} send to {ext} failed: {e:#}", self.kind());
        }
    }

    async fn try_send(&self, ext: &str, text: &str) -> Result<()> {
        let text = if text.trim().is_empty() { "(no reply)" } else { text };
        match self {
            Chan::Telegram(c) => c.send(ext, text).await,
            Chan::Slack(c) => c.send(ext, text).await,
            Chan::Discord(c) => c.send(ext, text).await,
        }
    }

    /// A routine's result: tried once more after a moment if the app didn't take it, and the error if it still didn't.
    async fn deliver(&self, ext: &str, text: &str) -> Result<()> {
        if self.try_send(ext, text).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
        self.try_send(ext, text).await
    }

    /// Final reply; replaces the streamed draft if there is one.
    async fn finish(&self, ext: &str, draft: Option<String>, text: &str) {
        let text = if text.trim().is_empty() { "(no reply)" } else { text };
        let r = match self {
            Chan::Telegram(c) => c.finish(ext, draft.as_deref(), text).await,
            Chan::Slack(c) => c.finish(ext, draft.as_deref(), text).await,
            Chan::Discord(c) => c.finish(ext, draft.as_deref(), text).await,
        };
        if let Err(e) = r {
            eprintln!("{} reply to {ext} failed: {e:#}", self.kind());
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
        }
    }

    async fn draft_edit(&self, ext: &str, id: &str, text: &str) {
        match self {
            Chan::Telegram(c) => c.draft_edit(ext, id, text).await,
            Chan::Slack(c) => c.draft_edit(ext, id, text).await,
            Chan::Discord(c) => c.draft_edit(ext, id, text).await,
        }
    }

    /// "typing…" indicator until the handle is aborted (Slack bots have none).
    fn typing(&self, ext: &str) -> Option<JoinHandle<()>> {
        match self {
            Chan::Telegram(c) => Some(c.typing(ext)),
            Chan::Discord(c) => Some(c.typing(ext)),
            Chan::Slack(_) => None,
        }
    }

    async fn card(&self, ext: &str, a: &Approval) -> Result<Option<String>> {
        match self {
            Chan::Telegram(c) => c.card(ext, a).await,
            Chan::Slack(c) => c.card(ext, a).await,
            Chan::Discord(c) => c.card(ext, a).await,
        }
    }

    async fn listen(&self, core: Arc<Core>, slot: Slot) -> Result<()> {
        match self {
            Chan::Telegram(c) => c.listen(self, core, slot).await,
            Chan::Slack(c) => c.listen(self, core, slot).await,
            Chan::Discord(c) => c.listen(self, core, slot).await,
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
    // ponytail: one turn at a time per bot (keeps CLI --resume sessions serial); per-chat locks if a bot gets busy
    lock: Mutex<()>,
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
        return "My last reply didn't use any memories, skills or pages.".into();
    };
    let mut out = vec!["My last reply used:".to_string()];
    for m in &u.memories {
        let gone = match m.status { "forgotten" => " (forgotten since)", "superseded" => " (corrected since)", _ => "" };
        out.push(format!("• #{} {}: {}{gone}", m.id, m.title, crate::tools::truncate(&m.body, 200)));
    }
    for (_, name) in &u.skills {
        out.push(format!("• skill: {name}"));
    }
    for p in &u.pages {
        out.push(format!("• {}", crate::tools::truncate(p, 200)));
    }
    if let Some(m) = u.memories.iter().find(|m| m.status == "active") {
        out.push(format!("\nIf a memory is wrong, tell me (\"forget #{}\", or what's true now), or fix it in the app under Memory.", m.id));
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

/// The buttons of a chat-app approval card, in rows: (label, button data "ap:<id>:<y|n|a|w>").
pub(crate) fn card_buttons(a: &Approval) -> Vec<Vec<(String, String)>> {
    let b = |label: String, d: char| (label, format!("ap:{}:{d}", a.id));
    if a.kind == "takeover" {
        return vec![vec![b("Done".into(), 'y'), b("Can't now".into(), 'n')]];
    }
    let mut rows = vec![vec![b("Approve".into(), 'y'), b("Deny".into(), 'n')]];
    if offers_all(a) {
        rows.push(vec![b("Approve all like this".into(), 'a')]);
    }
    if offers_always(a) {
        // Allowed a few times already: Always comes right after Approve / Deny.
        let at = if a.allowed_before >= 2 { 1 } else { rows.len() };
        rows.insert(at, vec![b(crate::tools::always_label(a), 'w')]);
    }
    rows
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

/// A file attached to a Slack or Discord message, as the app describes it; `url` is "" when it can't be fetched.
pub(crate) struct File {
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub url: String,
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
        format!("{} minutes ago", d / 60)
    } else if d < 7200 {
        "an hour ago".into()
    } else if d < 86400 {
        format!("{} hours ago", d / 3600)
    } else if d < 172800 {
        "yesterday".into()
    } else {
        format!("{} days ago", d / 86400)
    };
    Some(format!("Sorry for the slow reply: I was offline when you wrote {ago}."))
}

/// `quiet_now` at the current local time.
pub fn quiet(db: &Db) -> bool {
    quiet_now(db, chrono::Local::now())
}

pub(crate) fn card_text(a: &Approval) -> String {
    let mins = ((a.expires_at - crate::db::now()) / 60).max(1);
    let wait = if mins >= 120 { format!("{} h", mins / 60) } else { format!("{mins} min") };
    let then = if a.parked { "It runs when you approve." } else { "No answer means deny." };
    // Allowed the same thing a few times already: offer to stop asking (like the app).
    let again = if offers_always(a) && a.allowed_before >= 2 { format!("\nYou've allowed this {} times. Press Always to stop being asked.", a.allowed_before) } else { String::new() };
    if a.kind == "takeover" {
        return format!("🖐 I need you on the computer\n\n{}\n\nWaits {wait}.", a.summary);
    }
    format!("🔐 Approve this action?\n\n{}\n\nWaits {wait}. {then}{again}", a.summary)
}

pub(crate) fn verdict(a: &Approval) -> String {
    let outcome = match a.status.as_str() {
        "approved" if a.kind == "takeover" => "✅ Done, carrying on",
        "expired" if a.kind == "takeover" => "⌛ Nobody took over in time",
        _ if a.kind == "takeover" => "Not now",
        "approved" => "✅ Approved",
        "expired" => "⌛ No answer in time, so not done",
        _ => "❌ Denied",
    };
    format!("{outcome}: {}", a.summary)
}

/// The card's text once decided, saying so when it won't ask again.
pub(crate) fn verdict_for(a: &Approval, d: Decision) -> String {
    match d {
        Decision::Always if a.status == "approved" => {
            let scope = a.site.as_deref().map(|s| format!(" on {s}")).unwrap_or_default();
            format!("{}\nFrom now on this runs{scope} without asking (Settings → Permissions to change it).", verdict(a))
        }
        _ => verdict(a),
    }
}

impl Core {
    /// An inbound text message from any channel.
    /// A file it can't use yet (video, sticker, a spreadsheet…) in a paired direct chat: say so instead of dropping it.
    pub(crate) async fn incoming_media(&self, chan: &Chan, ext: &str) -> Result<()> {
        if self.is_paired(chan, ext)? {
            chan.send(ext, "I can read text, photos, voice notes, PDFs and text files, but not this kind of file yet. Could you describe it or send a photo?").await;
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
            Some(Ok(t)) if t.split_whitespace().count() < 5 => {
                chan.send(ext, "That file has no text I can read (a scanned PDF?). Send photos of the pages instead.").await;
                Ok(())
            }
            Some(Ok(t)) => {
                self.db.conv_taint(self.bot, chat)?;
                self.incoming_with(chan, ext, from, "", &crate::with_document(said, name, &t), vec![], sent).await
            }
        }
    }

    /// Files with a message (Slack, Discord): photos go to the bot's AI with the text, a voice clip is transcribed, a
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
            chan.send(ext, "That file is over 20 MB, more than I take. Could you send a smaller one?").await;
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
                chan.send(ext, &format!("I couldn't fetch {} ({e:#}).", f.name)).await;
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
            return self.incoming_voice(chan, ext, from, bytes, &f.name, false, sent).await;
        }
        if let Some(f) = usable.iter().find(|f| crate::readable_document(&f.name, &f.mime)) {
            let Some(bytes) = fetch(f).await else { return Ok(()) };
            return self.incoming_document(chan, ext, from, &f.name, &f.mime, &bytes, text, sent).await;
        }
        self.incoming_media(chan, ext).await
    }

    /// A voice note: transcribed and shown back (so a mishearing is visible), then answered like typing. A forwarded
    /// one is someone else's words: outside content.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn incoming_voice(self: &Arc<Self>, chan: &Chan, ext: &str, from: Sender<'_>, bytes: Vec<u8>, name: &str, forwarded: bool, sent: Option<i64>) -> Result<()> {
        match crate::provider::transcribe(bytes, name).await {
            Some(Ok(text)) if !text.is_empty() => {
                chan.send(ext, &format!("🎙️ I heard: “{text}”")).await;
                if forwarded {
                    let said = crate::with_document("(I forwarded you a voice note.)", "a forwarded voice note", &text);
                    return self.incoming_outside_with(chan, ext, from, "", &said, vec![], sent).await;
                }
                return self.incoming_with(chan, ext, from, "", &text, vec![], sent).await;
            }
            Some(Ok(_)) => chan.send(ext, "I couldn't make out any words in that voice note.").await,
            Some(Err(e)) => chan.send(ext, &format!("I couldn't transcribe that voice note ({e:#}). Could you type it?")).await,
            None => chan.send(ext, "To understand voice notes I need an OpenAI API key for transcription (Settings → AI keys in Mimir). Until then, could you type it?").await,
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
            self.db.conv_taint(self.bot, chat)?;
        }
        self.incoming_with(chan, ext, from, title, text, images, sent).await
    }

    /// Whether this person may talk to the bot in this paired chat, or press its buttons: the person who paired it. In
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
        Ok(Some(owner == from.id))
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
                chan.send(ext, "I can only be paired by a person writing as themselves (not anonymously or as a channel).").await;
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
                    "Paired ✅ as a guest: I keep your own memory and reminders here, and I can search the web for you. I can't see the email, calendar or meetings of the person who runs me. Commands: /memory, /routines, /new, /stop"
                } else {
                    "Paired ✅ Talk to me normally. Commands: /memory, /routines, /new, /stop, /undo, /why"
                }).await;
            } else {
                {
                    let mut misses = self.misses.lock().unwrap();
                    misses.retain(|_, (_, at)| now - *at < 3600);
                    misses.insert(who, (tries + 1, now));
                }
                chan.send(ext, if tries + 1 >= MAX_PAIR_TRIES { "Wrong pairing code. Too many tries: try again in an hour." } else { "Wrong pairing code." }).await;
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
                let name = self.db.bot(self.bot).map(|b| b.name).unwrap_or_else(|_| "this bot".into());
                chan.send(ext, &format!("Hi! I'm {name}, a private assistant. If you set me up, send pair followed by the code shown on my page in the Mimir app.")).await;
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
                    chan.send(ext, "For your safety I now answer only the person who pairs me in a group. Whoever set me up: send pair followed by the code on my page in the Mimir app, here, once more.").await;
                }
                return Ok(());
            }
        }
        let mut command = parse_command(text);
        if let Command::Stop(asked) = command {
            if self.stop(chan, ext, from)? {
                return Ok(());
            }
            if asked {
                chan.send(ext, "Nothing to stop: I'm not writing anything right now.").await;
                return Ok(());
            }
            command = Command::Chat;
        }
        match command {
            Command::Start => chan.send(ext, "Hi! Just talk to me. /memory shows what I remember, /routines lists schedules, /why shows what my last reply used, /new starts a fresh conversation, /stop cuts off a reply I'm writing, /undo N takes back something from my weekly list of what I learned.").await,
            Command::Memory => {
                // What bots keep in mind, then what's waiting for you to confirm (bots don't keep those in mind).
                let scope = format!("chat:{chat}");
                let mut lines: Vec<String> = self.db.bootstrap(&scope)?.iter().map(|e| e.line()).collect();
                let unconfirmed: Vec<String> = self.db.search_unconfirmed(&scope)?.iter().map(|e| e.line()).collect();
                if !unconfirmed.is_empty() {
                    lines.push(format!("\nNot confirmed yet (saved after reading outside content; check them in the app):\n{}", unconfirmed.join("\n")));
                }
                chan.send(ext, &if lines.is_empty() { "Nothing remembered yet.".into() } else { lines.join("\n") }).await;
            }
            Command::Why => chan.send(ext, &why(self.db.last_reply_used(self.bot, chat)?)).await,
            Command::Routines => {
                let out = crate::tools::call(&ToolCtx::new(self.db.clone(), self.bot, chat), "routine_list", &json!({})).await?;
                chan.send(ext, &out).await;
            }
            Command::New => {
                if let Some(s) = self.db.session(self.bot, chat)? {
                    self.provider.forget_session(&s);
                }
                self.db.set_session(self.bot, chat, None)?;
                chan.send(ext, "Fresh conversation started (memories kept).").await;
            }
            Command::Undo(n) => chan.send(ext, &crate::tidy::undo_learned(&self.db, chat, n)?).await,
            Command::Stop(_) => unreachable!("handled above"),
            Command::Pair(_) | Command::Chat => {
                // Measured on arrival, so a long reply (say, waiting for an approval) doesn't count as being offline.
                let late = late_note(sent, crate::db::now());
                let (core, chan, ext, text) = (self.clone(), chan.clone(), ext.to_string(), text.trim().to_string());
                tokio::spawn(async move {
                    let _g = core.lock.lock().await;
                    let typing = chan.typing(&ext);
                    // The reply shows up as it's written: one message, edited as it grows, then formatted.
                    let follower = chan.follow(core.bot, chat, &ext);
                    let mut ctx = ToolCtx::new(core.db.clone(), core.bot, chat);
                    ctx.images = images;
                    let (provider, asked) = (core.provider.clone(), text.clone());
                    let task = tokio::spawn(async move { turn(&provider, &ctx, Trigger::Message(&asked)).await });
                    TURNS.lock().unwrap().insert((core.bot, chat), task.abort_handle());
                    let out = task.await;
                    TURNS.lock().unwrap().remove(&(core.bot, chat));
                    if let Some(t) = typing {
                        t.abort();
                    }
                    let reply = match out {
                        Ok(out) => out.unwrap_or_else(|e| crate::friendly_error(&format!("{e:#}"))),
                        Err(e) if !e.is_cancelled() => crate::friendly_error(&e.to_string()),
                        // /stop: keep what was asked and written so far, like Stop in the app.
                        Err(_) => {
                            let partial = crate::live::get(core.bot, chat).unwrap_or_default();
                            crate::live::end(core.bot, chat);
                            let note = if partial.trim().is_empty() { "_Stopped before replying._".to_string() } else { format!("{}\n\n_Stopped._", partial.trim_end()) };
                            let logged = core.db.run_stop(core.bot, chat, &text).and_then(|_| core.db.log(core.bot, chat, "user", &text, None)).and_then(|_| core.db.log(core.bot, chat, "assistant", &note, None));
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
                    chan.finish(&ext, draft, &reply).await;
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

    /// An Approve/Deny button press. Only the chat the request belongs to can decide it, and in it only the person who
    /// paired it (an error says so, for the button's reply).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decide(&self, chan: &Chan, ext: &str, from: Sender<'_>, id: i64, d: Decision) -> Result<Option<Approval>> {
        let approve = d != Decision::Deny;
        match self.db.chat_lookup(chan.kind(), ext)? {
            Some(chat) => {
                if self.sender_ok(chat, from)? != Some(true) {
                    anyhow::bail!("Only the person who paired me here can answer this.");
                }
                if d == Decision::RestOfRun {
                    self.db.approval_allow_rest_of_run(id, Some(chat))?;
                }
                let a = self.db.approval_decide(id, approve, Some(chat))?;
                if let Some(a) = &a {
                    crate::tools::after_decision(&self.db, a);
                    if d == Decision::Always {
                        crate::tools::always_allow(&self.db, a)?;
                    }
                    // The button handler edits its own card.
                    self.db.approval_mark_closed(a.id)?;
                }
                Ok(a)
            }
            None => Ok(None),
        }
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
            let mut msg = String::from(if mine.is_empty() { "☀️ **All quiet**\n" } else { "☀️ **Your digest**\n" });
            for (_, _, rid, text, _) in &mine {
                let when = routines.iter().find(|r| Some(r.id) == *rid).map(|r| crate::tools::truncate(&r.prompt, 60)).unwrap_or_else(|| "A routine".into());
                msg.push_str(&format!("\n**{when}**\n{}\n", crate::tools::truncate(text.trim(), 700)));
            }
            if quiet > 0 {
                msg.push_str(&format!("\n{quiet} quiet {} ran; nothing needed you.", if quiet == 1 { "check" } else { "checks" }));
            }
            self.db.log(self.bot, chat, "assistant", &msg, None)?;
            // What routines found on the web is outside content in this conversation too.
            if mine.iter().any(|i| i.4) {
                self.db.conv_taint(self.bot, chat)?;
            }
            if let Some((c, ext)) = Self::chan_for(chans, chat, &self.db) {
                c.send(&ext, &msg).await;
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
                chan.send(&ext, &msg).await;
            }
        }
        Ok(())
    }

/// A newer Mimir than this server runs: said once, in your most recent private chat this bot can reach (not a group,
    /// not a guest's). A bot that reaches none leaves it to the others; Needs you in the app shows it either way.
    async fn tell_update(&self, chans: &[Chan]) -> Result<()> {
        let Some(latest) = crate::server_update::latest(&self.db).await else { return Ok(()) };
        let Some(msg) = crate::server_update::to_tell(&self.db, &latest) else { return Ok(()) };
        let mut best: Option<(i64, Chan, String)> = None;
        for c in self.db.chats()? {
            if c.chat_id == crate::host::LOCAL_CHAT || c.guest || !self.db.chat_private(c.chat_id) {
                continue;
            }
            let Some((chan, ext)) = Self::chan_for(chans, c.chat_id, &self.db) else { continue };
            let last = self.db.messages(c.chat_id, 1)?.first().map(|m| m.ts).unwrap_or(0);
            if best.as_ref().is_none_or(|b| last > b.0) {
                best = Some((last, chan, ext));
            }
        }
        if let Some((_, chan, ext)) = best
            && crate::server_update::told(&self.db, &latest)
        {
            chan.send(&ext, &msg).await;
        }
        Ok(())
    }

    fn chan_for(chans: &[Chan], chat: i64, db: &Db) -> Option<(Chan, String)> {
        let (kind, ext) = db.chat_address(chat).ok()??;
        chans.iter().find(|c| c.kind() == kind).map(|c| (c.clone(), ext))
    }

    /// One routine that's due: the change gate, a setup check, the run, what happens after a failure (wait for the
    /// usage limit, try again soon, or say so), and where its result went.
    async fn run_routine(&self, chans: &[Chan], mut r: crate::db::Routine) -> Result<()> {
        // Quiet hours: what would message right away goes into the next digest instead.
        if r.delivery == "now" && quiet_now(&self.db, chrono::Local::now()) {
            r.delivery = "digest".into();
        }
        // Over a daily cap: pause (it stays due, so it runs once when the day's over) and ask, in Needs you.
        let day_start = chrono::Local::now().date_naive().and_hms_opt(0, 0, 0).and_then(|t| chrono::TimeZone::from_local_datetime(&chrono::Local, &t).earliest()).map_or(0, |t| t.timestamp());
        if let Some((key, h)) = self.db.over_cap(&r, day_start)? {
            let said = format!("⏸ Paused until tomorrow: {}. You can let it go on today in Mimir's Needs you.", h.why);
            self.db.hold_set(&key, &h)?;
            if let Some((c, ext)) = Self::chan_for(chans, r.chat_id, &self.db).filter(|_| r.delivery == "now") {
                c.send(&ext, &said).await;
            }
            return Ok(());
        }
        self.db.routine_advance(&r)?;
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
        let ready = self.provider.ready().await;
        let _g = self.lock.lock().await;
        // Local-chat routines (or ones whose channel isn't connected) still run; the reply lands in the app.
        let target = Self::chan_for(chans, r.chat_id, &self.db);
        let out = match ready {
            Err(e) => {
                self.db.run_not_started(&r, &self.provider.name(), &format!("{e:#}"))?;
                Err(e)
            }
            Ok(()) => {
                let typing = target.as_ref().and_then(|(c, ext)| c.typing(ext));
                let (provider, ctx, routine) = (self.provider.clone(), ToolCtx::new(self.db.clone(), self.bot, r.chat_id), r.clone());
                let out = self.own_task(r.chat_id, async move { turn(&provider, &ctx, Trigger::Routine(&routine)).await }).await;
                if let Some(t) = typing {
                    t.abort();
                }
                out
            }
        };
        let run = self.db.routine_last_run(r.id)?;
        let text = match out {
            Ok(text) => {
                // Compared against next time only once a run has seen this change (a failed one sees it again).
                if let Some(seen) = seen {
                    self.db.routine_set_seen(r.id, &seen)?;
                }
                self.db.routine_done(&r, true)?;
                text
            }
            Err(e) => {
                let e = format!("{e:#}");
                let now = crate::db::now();
                let clock = |t: i64| chrono::TimeZone::timestamp_opt(&chrono::Local, t, 0).single().map(|d| d.format(if t - now < 18 * 3600 { "%H:%M" } else { "%a %-d %b %H:%M" }).to_string()).unwrap_or_default();
                let again = match crate::cause(&e) {
                    crate::Cause::Quota => {
                        // ponytail: an hour when the AI didn't say when it resets; a run then finds out again
                        let until = crate::provider::quota_reset(&self.db, self.provider.kind(), &e, now).unwrap_or(now + 3600).min(now + 8 * 86400);
                        let why = "the AI plan was at its usage limit, so Mimir waited for it to reset".to_string();
                        // Every bot on the same signed-in AI (Claude, ChatGPT or Grok is one login for all of them) waits
                        // too, instead of each finding out with a failed run. Bots on an API key may be limited per model.
                        let held = self.db.hold_account(self.bot, self.provider.kind(), &crate::db::Hold { until, why, waive: None })?;
                        self.db.routine_retry(&r, until)?;
                        let who = if held.len() > 1 { "Routines on this AI account wait" } else { "This bot's routines wait" };
                        let note = format!("{who} until {}, when the limit resets; this one runs again then.", clock(until));
                        // Said once, where this routine speaks; Needs you has it too.
                        if let Some((c, ext)) = target.as_ref().filter(|_| r.delivery == "now") {
                            c.send(ext, &format!("⏸ I've hit my AI plan's usage limit. {note}")).await;
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
                self.db.routine_done(&r, false)?;
                let text = format!("Routine #{}: {}", r.id, crate::friendly_error(&e));
                // A failed digest-lane routine (incl. one held for quiet hours) says so in the digest.
                if r.delivery == "digest" {
                    self.db.digest_add(self.bot, r.chat_id, r.id, &text, false)?;
                }
                text
            }
        };
        // Empty = a quiet check that found nothing worth saying. Digest / log-only routines don't message now.
        if text.trim().is_empty() {
            return Ok(());
        }
        let how = match (r.delivery.as_str(), &target) {
            ("digest", _) => "in the digest".to_string(),
            ("log", _) => "only in the log".to_string(),
            (_, None) => "in the app".to_string(),
            (_, Some((c, ext))) => match c.deliver(ext, &text).await {
                Ok(()) => format!("sent to {}", c.kind()),
                Err(e) => format!("{} to {}: {}", crate::db::NOT_SENT, c.kind(), crate::tools::truncate(&format!("{e:#}"), 200)),
            },
        };
        if let Some(run) = run {
            self.db.run_delivered(run, &how)?;
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

    async fn scheduler(self: Arc<Self>, chans: Vec<Chan>) -> Result<()> {
        // A routine due before this was Mimir not running; one due since, the computer asleep or a long run.
        let started = crate::db::now();
        loop {
            let now = crate::db::now();
            // Routines on hold (a usage limit) wait; the first pass after it ends runs each of them once.
            let bot_hold = format!("hold:{}", self.bot);
            let hold = self.db.hold(&bot_hold).unwrap_or_default();
            let held = hold.as_ref().is_some_and(|h| h.until > now);
            if hold.is_some() && !held {
                let _ = self.db.hold_clear(&bot_hold, false);
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
                        if let Err(e) = self.run_routine(&chans, r).await {
                            eprintln!("routine #{id}: {e:#}");
                        }
                        // It hit the usage limit or the bot's cap: the rest wait too.
                        if self.db.hold(&bot_hold).ok().flatten().is_some_and(|h| h.until > crate::db::now()) {
                            break;
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => eprintln!("scheduler: {e:#}"),
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
                    let _g = self.lock.lock().await;
                    let (db, provider, bot, id) = (self.db.clone(), self.provider.clone(), self.bot, chat.chat_id);
                    match self.own_task(id, async move { crate::tidy::run(&db, &provider, bot, id).await }).await {
                        Ok(n) if n > 0 => eprintln!("tidy-up: {n} memory suggestions for chat {}", chat.chat_id),
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
            for a in self.db.approvals_to_notify(self.bot).unwrap_or_default().into_iter().filter(|a| !(quiet && a.parked)) {
                // Channel not connected: leave it pending for the app banner (and a later reconnect).
                let Some((c, ext)) = Self::chan_for(&chans, a.chat_id, &self.db) else { continue };
                let card = c.card(&ext, &a).await.unwrap_or_else(|e| {
                    eprintln!("approval #{} card failed: {e:#}", a.id);
                    None
                });
                let _ = self.db.approval_mark_notified(a.id, card.as_deref());
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
        lock: Mutex::new(()),
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
            lock: Mutex::new(()),
            greeted: Default::default(),
        }
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
        down.run_routine(&[], fresh(&db)).await.unwrap();
        let after = fresh(&db);
        assert!((after.retries, after.next_fire > now + 200 && after.next_fire <= now + 400) == (1, true), "{after:?}");
        down.run_routine(&[], after).await.unwrap();
        assert_eq!(fresh(&db).retries, 2);
        let failed = db.runs_failed(0).unwrap();
        assert_eq!((failed.len(), failed[0].repeats), (1, 2), "one item for the same failure");
        assert!(failed[0].error.as_deref().unwrap().contains("Trying again at"), "{:?}", failed[0].error);

        // The plan's usage limit: the bot's routines wait (an hour, when nobody says when it resets), this one runs then.
        let limited = core_on(&db, &fake_ai("429 Too Many Requests", r#"{"error":{"message":"Rate limit reached"}}"#).await);
        limited.run_routine(&[], fresh(&db)).await.unwrap();
        let h = db.hold("hold:1").unwrap().expect("held");
        assert!(h.until > now + 3000 && h.until <= now + 3700 && fresh(&db).next_fire <= h.until, "{h:?}");
        assert_eq!(db.holds(now).unwrap().len(), 1, "another bot on an API key isn't held");
        assert_eq!(db.runs_failed(0).unwrap().len(), 2, "a usage limit is its own item");

        // It works again: its failures leave Needs you, the tries start over, and the result's whereabouts are logged.
        let ok = core_on(&db, &fake_ai("200 OK", r#"{"choices":[{"message":{"role":"assistant","content":"Three headlines."}}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#).await);
        ok.run_routine(&[], fresh(&db)).await.unwrap();
        assert_eq!(fresh(&db).retries, 0);
        assert!(db.runs_failed(0).unwrap().is_empty(), "healed");
        assert_eq!(db.runs(Some(0), 1).unwrap()[0].delivered.as_deref(), Some("in the app"));

        // A one-off stays until its run is over (a crash mid-run tries it again), then it's gone.
        let once = db.routine_once(1, 0, now + 60, "call mum", false).unwrap();
        db.routine_advance(&once).unwrap();
        assert!(db.routines(0).unwrap().iter().any(|x| x.id == once.id && x.next_fire > now + 3000));
        ok.run_routine(&[], once.clone()).await.unwrap();
        assert!(!db.routines(0).unwrap().iter().any(|x| x.id == once.id));
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
            ok.run_routine(&[], r).await.unwrap();
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
        ok.run_routine(&[], get(r.id)).await.unwrap();
        // 5 tokens used: the next run would go over this routine's cap. It pauses until tomorrow, still due.
        let before = get(r.id).next_fire;
        ok.run_routine(&[], get(r.id)).await.unwrap();
        let key = format!("hold:routine:{}", r.id);
        let h = db.hold(&key).unwrap().expect("paused");
        assert!(h.why.contains("this routine reached the daily cap of 5 tokens") && h.until > crate::db::now(), "{h:?}");
        assert_eq!((get(r.id).next_fire, db.runs(Some(0), 9).unwrap().len()), (before, 1), "no run, nothing advanced");
        let held = db.holds(0).unwrap();
        assert_eq!((held.len(), held[0].2), (1, Some(r.id)));
        // Another routine has its own cap.
        ok.run_routine(&[], get(other.id)).await.unwrap();
        assert_eq!(db.runs(Some(0), 9).unwrap().len(), 2);
        // "Let it go on today": it runs, and doesn't pause again today.
        db.hold_clear(&key, true).unwrap();
        assert!(db.holds(crate::db::now()).unwrap().is_empty(), "ended, out of Needs you");
        assert!(db.hold(&key).unwrap().is_some_and(|h| h.until < crate::db::now()), "kept, so a late run says why");
        ok.run_routine(&[], get(r.id)).await.unwrap();
        ok.run_routine(&[], get(r.id)).await.unwrap();
        assert!(db.hold(&key).unwrap().is_some_and(|h| h.until < crate::db::now()), "not paused again today");
        assert_eq!(db.runs(Some(0), 9).unwrap().len(), 4);
        // All bots together: a bot-wide pause.
        db.set_setting("cap_routine", "0").unwrap();
        db.set_setting("cap_day", "10").unwrap();
        ok.run_routine(&[], get(other.id)).await.unwrap();
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
            lock: Mutex::new(()),
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
        let files = [File { name: "a.png".into(), mime: "image/png".into(), size: 10, url: "https://cdn.discordapp.com/a.png".into() }];
        let stranger = Sender { id: "U2", dm: false };
        core.incoming_files(&chan, "C1", stranger, "", &files, None, |_| async { panic!("fetched a stranger's file") }).await.unwrap();
        // Unpaired, the chat forgets where catch-up left off (another app's chat with the same id keeps its own).
        let other = Chan::Slack(crate::slack::Slack::new("xoxb-1", "xapp-1").unwrap());
        db.pair_external("slack", "C1", "c1").unwrap();
        assert!(core.mark_seen(&other, "C1", 3));
        db.unpair(chat).unwrap();
        assert_eq!((core.seen(&chan, "C1"), core.seen(&other, "C1")), (None, Some(3)));
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
