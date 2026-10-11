//! SQLite store: typed memory entries (FTS5), chats, message log, routines.
//! Memory is append-only: entries are superseded or invalidated, never edited.

use anyhow::{Context, Result, bail};
use chrono::{Local, TimeZone};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const TYPES: [&str; 3] = ["convention", "knowledge", "task"];

#[derive(Clone)]
/// Connection + the data directory it lives in.
pub struct Db(Arc<Mutex<Connection>>, Arc<std::path::PathBuf>);

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub source: String,
    /// A bot saved it in a conversation that had read untrusted content.
    pub tainted: bool,
    /// When it was saved (unix seconds; 0 if unknown), so a bot can tell an old fact from a fresh one.
    pub created_at: i64,
    /// When it stops being true (unix seconds), if it has an end.
    pub valid_until: Option<i64>,
}

impl Entry {
    pub fn line(&self) -> String {
        // One line each, and no angle brackets: a memory's text can't close the block it's in or pass for another entry.
        let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").replace('<', "‹").replace('>', "›");
        let src = if self.source.is_empty() { String::new() } else { format!(" (source: {})", flat(&self.source)) };
        // So a planted "memory" from a web page doesn't read like something the person said: said first, before its
        // text (which could otherwise push the warning out of sight).
        let warn = if self.tainted { " UNCONFIRMED (saved after reading outside content, not by the person)" } else { "" };
        // Dated, so "lives in Utrecht" from two years ago reads as possibly stale next to last week's facts.
        let day = |t: i64| chrono::TimeZone::timestamp_opt(&chrono::Local, t, 0).single().map(|t| t.format("%Y-%m-%d").to_string());
        let saved = (self.created_at > 0).then(|| day(self.created_at)).flatten().map(|d| format!(", saved {d}")).unwrap_or_default();
        // An end date: "until" while it holds; "ENDED" once past it (search still finds it, as history).
        let until = match self.valid_until.and_then(|t| Some((t, day(t)?))) {
            Some((t, d)) if t <= now() => format!(", ENDED {d}"),
            Some((_, d)) => format!(", until {d}"),
            None => String::new(),
        };
        format!("[#{} {}{saved}{until}{warn}] {} — {}{}", self.id, self.kind, flat(&self.title), flat(&self.body), src)
    }
}

/// Who wrote a memory, set by Mimir (never by the model): `you` (in the app), `bot`, `tidy` (an accepted
/// tidy-up suggestion), `import`; empty for memories from older versions.
#[derive(Debug, Clone, Default)]
pub struct Prov {
    pub origin: &'static str,
    pub bot: Option<i64>,
    pub run: Option<i64>,
    /// Written in a conversation that had read untrusted content (web pages, tool results).
    pub tainted: bool,
}

impl Prov {
    pub fn by(origin: &'static str) -> Self {
        Prov { origin, ..Default::default() }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EntryRow {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub source: String,
    pub created_at: i64,
    pub status: &'static str,
    pub superseded_by: Option<i64>,
    pub access_count: i64,
    /// Always in the bot's standing memory, whatever else is there.
    pub pinned: bool,
    /// Moved out of standing memory (not wrong, rarely needed): search still finds it.
    pub background: bool,
    pub origin: String,
    pub bot_id: Option<i64>,
    pub run_id: Option<i64>,
    pub tainted: bool,
    /// Replies that used it (via search) were corrected this many times: a hint it's wrong or outdated.
    pub missteps: i64,
    pub valid_until: Option<i64>,
    /// What the person wrote that the run saving it answered (bot-written memories), so a claim made firmer than
    /// they said it shows.
    pub asked: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Chat {
    pub chat_id: i64,
    pub title: String,
    /// telegram | slack | discord | local
    pub channel: String,
    pub memories: i64,
    /// Paired by someone other than you (their own memory and routines; none of your email, calendar, meetings…).
    pub guest: bool,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Bot {
    pub id: i64,
    pub name: String,
    /// Extra instructions appended to the base persona.
    pub persona: String,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    /// Comma-separated tool allowlist; empty = all tools.
    pub tools: String,
    /// How its creature looks, chosen by you: {"shape","eyes","extra","color"} as JSON. Empty = the look its id gives it.
    #[serde(default)]
    pub look: String,
}

/// A bot's look: empty, or a small JSON object of short names and a #RRGGBB colour (the UI ignores names it doesn't know).
fn valid_look(look: &str) -> bool {
    if look.is_empty() {
        return true;
    }
    let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(look) else { return false };
    look.len() <= 200
        && m.iter().all(|(k, v)| {
            let v = v.as_str().unwrap_or("\u{0}");
            match k.as_str() {
                "color" => v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit()),
                "shape" | "eyes" | "extra" => v.len() <= 16 && v.chars().all(|c| c.is_ascii_lowercase()),
                _ => false,
            }
        })
}

impl Bot {
    /// Allowlist check; "browser__*" style entries match a whole tool server.
    pub fn allows(&self, tool: &str) -> bool {
        // A bot that may set routines may also change them.
        if tool == "routine_update" {
            return self.allows("routine_create");
        }
        self.tools.trim().is_empty()
            || self.tools.split(',').map(str::trim).any(|t| t == tool || t.strip_suffix('*').is_some_and(|p| !p.is_empty() && tool.starts_with(p)))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: i64,
    pub bot_id: i64,
    pub role: String,
    pub content: String,
    pub ts: i64,
    /// Replies: what the bot did on the way (the run's tool calls, without results).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    /// Replies: the run that wrote it (to open what another bot did for it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Action {
    pub tool: String,
    pub summary: String,
    pub decision: String,
    pub ok: bool,
    /// What the tool said (short), e.g. "routine #5 created", so the UI can link to what was made.
    pub result: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Usage {
    pub model: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    /// API-list-price equivalent when the provider reports it (claude -p); subscriptions aren't billed per token.
    pub cost_usd: Option<f64>,
    /// Of tokens_in, how many were read back from the prompt cache (billed at about a tenth).
    #[serde(default)]
    pub cache_read: i64,
    /// The account didn't say what it used: worked out from the text's length.
    #[serde(default)]
    pub estimated: bool,
}

/// A run's tokens as they weigh on a plan (caps, "about N tokens each"): cache reads count a tenth, as they're billed.
pub const WEIGHED: &str = "(tokens_in + tokens_out - cache_read * 9 / 10)";

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub id: i64,
    pub bot_id: i64,
    pub chat_id: i64,
    pub trigger: String,
    pub routine_id: Option<i64>,
    pub provider: String,
    pub model: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: Option<f64>,
    pub tool_calls: i64,
    /// What the run replied (empty for a quiet check that found nothing), for the run log.
    pub reply: Option<String>,
    /// A task handed over by another bot: the run that asked.
    pub parent_run: Option<i64>,
    /// Where a routine's result went: "sent to telegram", "in the digest", "not sent: …" (None: nothing to send).
    pub delivered: Option<String>,
    /// In Needs you: how many times this same failure happened (one item for all of them).
    pub repeats: i64,
    /// Of tokens_in, read back from the prompt cache.
    pub cache_read: i64,
    /// Tokens worked out from the text, the account didn't report them.
    pub estimated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCall {
    pub id: i64,
    pub tool: String,
    pub summary: String,
    pub decision: String,
    pub ok: bool,
    pub result: String,
    pub ts: i64,
}

/// One week of how a chat went, from the run log (no model calls): the learning trend metrics.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Week {
    pub start: i64,
    /// Messages the person sent.
    pub turns: i64,
    pub corrections: i64,
    pub stops: i64,
    pub denials: i64,
    pub tool_calls: i64,
    pub tool_errors: i64,
    pub accepted: i64,
    pub dismissed: i64,
    /// Corrections on a request like one corrected before: the same mistake twice, what learning should prevent.
    pub repeats: i64,
    /// "I told you…" corrections: something the bot should have remembered.
    pub forgot: i64,
    /// Memories learned without the person typing them (a bot, a tidy-up, an import) that were taken back with Undo.
    pub undone: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Totals {
    pub runs: i64,
    pub errors: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: f64,
}

/// A proposed memory change from the nightly tidy-up. Nothing is applied until the user accepts it.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
pub struct Suggestion {
    #[serde(default)]
    pub id: i64,
    /// add | update | forget | demote (move to background)
    pub action: String,
    /// The entry to update/forget.
    #[serde(default)]
    pub entry_id: Option<i64>,
    #[serde(default = "default_kind", rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub reason: String,
    /// What replaying past corrections with this change showed, once you asked for it ("avoids 2 of 3 …").
    #[serde(default)]
    pub check: Option<String>,
    /// Drawn from outside content (a bot proposing it after reading a page, a tidy-up that saw such text, or an
    /// update of an unconfirmed memory): accepting it keeps it marked unconfirmed, and "Accept all" leaves it out.
    #[serde(default)]
    pub tainted: bool,
}

fn default_kind() -> String {
    "knowledge".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct Approval {
    pub id: i64,
    pub bot_id: i64,
    pub chat_id: i64,
    pub tool: String,
    pub summary: String,
    pub status: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub tg_message_id: Option<i64>,
    /// Why it asks: `rule` (your rules say ask), `guarded` (the conversation read untrusted content), or
    /// `login` (a saved login would be typed). Only `rule` asks can become an "always allow".
    pub kind: String,
    /// The website involved, for a per-site "always allow".
    pub site: Option<String>,
    /// Parked by an unattended run (a routine): the exact call is stored and runs when approved.
    pub parked: bool,
    /// How often you already allowed this same kind of request (tool and site): the card offers "always" after a few.
    pub allowed_before: i64,
}

/// A procedure distilled from a run that worked. The tidy-up proposes it; bots use it only once you accept it.
/// Each belongs to one bot in one chat (skills aren't shared between bots).
#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    pub id: i64,
    pub bot_id: i64,
    pub name: String,
    /// When to use it: the only part that sits in the bot's prompt.
    pub description: String,
    /// The steps, read on demand with skill_view.
    pub body: String,
    /// The run it was learned from.
    pub run_id: Option<i64>,
    /// proposed | active | dismissed | removed
    pub status: String,
    pub created_at: i64,
    /// What a proposal does: learned (new, from a run that went well), patch (a fix after trouble), merge, retire.
    pub kind: String,
    /// Why it's proposed (the evidence), shown on the card.
    pub reason: String,
    /// Skills it takes out of use when accepted (merge, retire).
    pub replaces: Vec<i64>,
    /// How many runs read it, and when last.
    pub uses: i64,
    pub last_used: Option<i64>,
}

/// A run that used a skill and then went wrong: what was asked, what it did, and what went wrong.
pub struct SkillTrouble {
    pub skill: Skill,
    /// The run had read outside content (its evidence may carry a page's text).
    pub tainted: bool,
    pub run_id: i64,
    pub asked: String,
    pub steps: Vec<String>,
    pub trouble: String,
}

/// A run that could teach a skill: what was asked and the actions it took.
pub struct GoodRun {
    pub id: i64,
    pub bot_id: i64,
    pub asked: String,
    pub calls: Vec<ToolCall>,
}

/// How late a routine may run before it counts as missed: the scheduler checks every 30 s, and a run before it can
/// take minutes.
pub const MISSED_AFTER: i64 = 15 * 60;

#[derive(Debug, Clone, Serialize)]
pub struct RunMark {
    pub status: String,
    pub at: i64,
    pub tokens: i64,
}

/// A routine's last 30 days (runs it did, their tokens and cost, how many failed, slots it missed) and its last runs.
#[derive(Debug, Clone, Serialize)]
pub struct RoutineStats {
    pub id: i64,
    pub runs: i64,
    pub tokens: i64,
    pub cost_usd: Option<f64>,
    pub failed: i64,
    pub missed: i64,
    /// Watched quiet checks that found nothing changed (no AI run; not in `runs`).
    pub unchanged: i64,
    /// Kinds of line on the watched page left out because they change on every load (e.g. "Views: #").
    pub ignored: Vec<String>,
    pub last: Vec<RunMark>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Routine {
    pub id: i64,
    pub bot_id: i64,
    pub chat_id: i64,
    pub cron: String,
    pub prompt: String,
    pub next_fire: i64,
    /// A heartbeat check: read-only, and silent unless something needs the user's attention.
    pub quiet: bool,
    /// Kept but not run.
    pub paused: bool,
    /// now (message right away) | digest (in the daily digest) | log (only in the run log).
    pub delivery: String,
    /// Made by a bot in a conversation that had read outside content: every run of it counts as having read it.
    pub tainted: bool,
    /// Runs once, at next_fire, then is deleted (no cron).
    #[serde(default)]
    pub once: bool,
    /// A quiet check's page or feed address, or "mail": Mimir looks first and runs the AI only when it changed.
    #[serde(default)]
    pub watch: String,
    /// Tries since the last run that worked, after failures that may pass (the network, the setup).
    #[serde(default)]
    pub retries: i64,
}

/// Routines waiting: until a usage limit resets, or a daily cap's day is over. `why` finishes "Missed 3 runs, …: ".
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Hold {
    pub until: i64,
    pub why: String,
    /// A daily cap's hold: the setting that lets it go on today anyway when the person says so (None: a usage limit).
    #[serde(default)]
    pub waive: Option<String>,
}

/// Daily token caps (none when unset): each routine, each bot, all bots together.
pub const CAPS: [&str; 3] = ["cap_routine", "cap_bot", "cap_day"];

const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
-- Deleted rows are overwritten, not left in free pages (what incognito chats asked, erased memories).
PRAGMA secure_delete=ON;
CREATE TABLE IF NOT EXISTS entries(
  id INTEGER PRIMARY KEY,
  scope TEXT NOT NULL,
  type TEXT NOT NULL CHECK(type IN ('convention','knowledge','task')),
  title TEXT NOT NULL,
  body TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT '',
  importance INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  superseded_by INTEGER,
  invalid INTEGER NOT NULL DEFAULT 0,
  access_count INTEGER NOT NULL DEFAULT 0
);
CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(title, body, content='entries', content_rowid='id');
CREATE TRIGGER IF NOT EXISTS entries_ai AFTER INSERT ON entries BEGIN
  INSERT INTO entries_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
END;
CREATE TRIGGER IF NOT EXISTS entries_ad AFTER DELETE ON entries BEGIN
  INSERT INTO entries_fts(entries_fts, rowid, title, body) VALUES ('delete', old.id, old.title, old.body);
END;
CREATE TABLE IF NOT EXISTS chats(chat_id INTEGER PRIMARY KEY, claude_session TEXT);
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS bots(
  id INTEGER PRIMARY KEY, name TEXT NOT NULL, persona TEXT NOT NULL DEFAULT '', provider TEXT NOT NULL DEFAULT 'claude',
  model TEXT NOT NULL DEFAULT '', base_url TEXT NOT NULL DEFAULT '', tools TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions(bot_id INTEGER NOT NULL, chat_id INTEGER NOT NULL, session TEXT NOT NULL, PRIMARY KEY(bot_id, chat_id));
-- A conversation that has read untrusted content stays tainted until a fresh conversation starts.
CREATE TABLE IF NOT EXISTS conv_taint(bot_id INTEGER NOT NULL, chat_id INTEGER NOT NULL, since INTEGER NOT NULL, PRIMARY KEY(bot_id, chat_id));
CREATE TABLE IF NOT EXISTS runs(
  id INTEGER PRIMARY KEY, chat_id INTEGER NOT NULL, trigger TEXT NOT NULL, routine_id INTEGER, provider TEXT NOT NULL,
  model TEXT, status TEXT NOT NULL DEFAULT 'running', error TEXT, started_at INTEGER NOT NULL, ended_at INTEGER,
  tokens_in INTEGER NOT NULL DEFAULT 0, tokens_out INTEGER NOT NULL DEFAULT 0, cost_usd REAL
);
CREATE INDEX IF NOT EXISTS runs_chat ON runs(chat_id, id);
CREATE TABLE IF NOT EXISTS tool_calls(
  id INTEGER PRIMARY KEY, run_id INTEGER, chat_id INTEGER NOT NULL, tool TEXT NOT NULL, summary TEXT NOT NULL,
  decision TEXT NOT NULL, ok INTEGER NOT NULL, result TEXT NOT NULL, ts INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS tool_calls_run ON tool_calls(run_id);
CREATE TABLE IF NOT EXISTS suggestions(
  id INTEGER PRIMARY KEY, scope TEXT NOT NULL, action TEXT NOT NULL CHECK(action IN ('add','update','forget','demote')),
  entry_id INTEGER, type TEXT NOT NULL DEFAULT 'knowledge', title TEXT NOT NULL DEFAULT '', body TEXT NOT NULL DEFAULT '',
  reason TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'pending', created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS policy_rules(tool TEXT PRIMARY KEY, action TEXT NOT NULL CHECK(action IN ('allow','ask','deny')));
CREATE TABLE IF NOT EXISTS approvals(
  id INTEGER PRIMARY KEY, chat_id INTEGER NOT NULL, tool TEXT NOT NULL, summary TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','approved','denied','expired')),
  created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, notified INTEGER NOT NULL DEFAULT 0, tg_message_id INTEGER
);
CREATE TABLE IF NOT EXISTS messages(
  id INTEGER PRIMARY KEY, chat_id INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, ts INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS routines(
  id INTEGER PRIMARY KEY, chat_id INTEGER NOT NULL, cron TEXT NOT NULL, prompt TEXT NOT NULL, next_fire INTEGER NOT NULL
);
";

pub fn now() -> i64 {
    Local::now().timestamp()
}

fn importance(kind: &str) -> i64 {
    if kind == "convention" { 5 } else { 3 }
}

/// Accepts standard 5-field cron ("0 8 * * Mon-Fri") or the cron crate's 6/7-field form.
pub fn next_fire(expr: &str, after: i64) -> Result<i64> {
    let expr = expr.trim();
    let full = if expr.split_whitespace().count() == 5 { format!("0 {expr}") } else { expr.to_string() };
    let schedule = cron::Schedule::from_str(&full).map_err(|e| anyhow::anyhow!("bad cron '{expr}': {e}"))?;
    let after = Local.timestamp_opt(after, 0).single().unwrap_or_else(Local::now);
    match schedule.after(&after).next() {
        Some(t) => Ok(t.timestamp()),
        None => bail!("cron '{expr}' never fires"),
    }
}

/// The shortest gap between two runs of a schedule over the next couple of days, in minutes.
pub fn min_gap_minutes(expr: &str) -> Result<i64> {
    let expr = expr.trim();
    let full = if expr.split_whitespace().count() == 5 { format!("0 {expr}") } else { expr.to_string() };
    let schedule = cron::Schedule::from_str(&full).map_err(|e| anyhow::anyhow!("bad cron '{expr}': {e}"))?;
    let times: Vec<i64> = schedule.upcoming(chrono::Utc).take(200).map(|t| t.timestamp()).collect();
    Ok(times.windows(2).map(|w| (w[1] - w[0]) / 60).min().unwrap_or(i64::MAX))
}

/// Turn free text into an FTS5 OR-query of quoted prefix terms (no user syntax reaches FTS).
fn fts_query(q: &str) -> Option<String> {
    let terms: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
        .take(16)
        .map(|w| format!("\"{}\"*", w.to_lowercase()))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

/// Two short texts (a suggestion, a skill's "when to use") about the same thing: most of the shorter one's real words
/// are in the other. Against the shorter one, so a wordy rewording of the same rule still matches.
// ponytail: word overlap; synonyms ("brief" vs "short") don't match, and a near-miss just shows a second card
pub(crate) fn alike(a: &str, b: &str) -> bool {
    // Without the words every suggestion or description is made of ("When the user wants…").
    const FILLER: [&str; 13] = ["when", "asked", "asks", "user", "person", "they", "their", "prefers", "wants", "needs", "someone", "with", "always"];
    let words = |t: &str| -> std::collections::HashSet<String> {
        t.split(|ch: char| !ch.is_alphanumeric()).filter(|w| w.chars().count() > 3).map(str::to_lowercase).filter(|w| !FILLER.contains(&w.as_str())).collect()
    };
    let (a, b) = (words(a), words(b));
    let shared = a.intersection(&b).count();
    shared >= 2 && shared * 5 >= 3 * a.len().min(b.len())
}

/// A chat and its skills (name, description), for a messaging app's "/" menu.
pub type SkillMenu = (i64, Vec<(String, String)>);

/// A past message found by chat_search.
#[derive(Debug, Clone)]
pub struct ChatHit {
    pub ts: i64,
    pub bot_id: i64,
    pub role: String,
    pub excerpt: String,
    /// A bot message whose run read outside content, or whose run isn't known (a digest, a stopped reply): not the
    /// person's words, and not trusted.
    pub tainted: bool,
    /// A routine's scheduled prompt (logged as the user's turn, but a bot may have written it).
    pub routine: bool,
}

/// How a routine result that couldn't be sent is marked (`runs.delivered`).
pub const NOT_SENT: &str = "not sent";
/// Why a routine ran late, for its run log line.
pub const AWAY: &str = "Mimir wasn't running then";
pub const ASLEEP: &str = "the computer was asleep, or a long run held it up";

/// A run's status when a watched quiet check found nothing changed, so no AI ran.
pub const UNCHANGED: &str = "unchanged";

/// Reason prefix of a habit a bot wanted to remember but the person never stated.
pub const BOT_PROPOSED: &str = "A bot wants to remember this";

/// Standing memory's share of every prompt (~2k tokens); a long memory that doesn't fit is left to search.
const BOOTSTRAP_CHARS: usize = 8000;
/// Facts and tasks in standing memory: only the newest few (preferences come first).
const BOOTSTRAP_RECENT: usize = 10;

/// (id, chat, routine, text) of an unsent digest item.
/// (id, chat, routine, text, from a run that read outside content)
pub type DigestItem = (i64, i64, Option<i64>, String, bool);

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let c = Connection::open(path)?;
        c.busy_timeout(Duration::from_secs(5))?;
        c.execute_batch(SCHEMA)?;
        // Migrations: additive columns on DBs created by older builds.
        let add_col = |table: &str, col: &str, decl: &str| -> Result<()> {
            if !c.prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name='{col}'"))?.exists([])? {
                c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl}"))?;
            }
            Ok(())
        };
        add_col("chats", "title", "TEXT NOT NULL DEFAULT ''")?;
        add_col("bots", "look", "TEXT NOT NULL DEFAULT ''")?;
        add_col("chats", "owner", "TEXT NOT NULL DEFAULT ''")?;
        add_col("suggestions", "tainted", "INTEGER NOT NULL DEFAULT 0")?;
        add_col("chats", "dm", "INTEGER NOT NULL DEFAULT 0")?;
        add_col("chats", "guest", "INTEGER NOT NULL DEFAULT 0")?;
        // Chats paired by one of Telegram's shared senders (an anonymous admin, a channel) would answer everyone
        // behind that id: they need pairing again by a person.
        c.execute("UPDATE chats SET owner='' WHERE owner IN ('1087968824', '136817688', '777000')", [])?;
        add_col("approvals", "tainted", "INTEGER NOT NULL DEFAULT 0")?;
        add_col("routines", "paused", "INTEGER NOT NULL DEFAULT 0")?;
        add_col("routines", "tainted", "INTEGER NOT NULL DEFAULT 0")?;
        add_col("messages", "run_id", "INTEGER")?;
        // Full-text search over past conversations (chat_search). Built once from existing messages.
        let fresh = !c.prepare("SELECT 1 FROM sqlite_master WHERE name='messages_fts'")?.exists([])?;
        c.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(content, content='messages', content_rowid='id');
             CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
               INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
             END;
             CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
               INSERT INTO messages_fts(messages_fts, rowid, content) VALUES ('delete', old.id, old.content);
             END;",
        )?;
        if fresh {
            c.execute_batch("INSERT INTO messages_fts(messages_fts) VALUES('rebuild')")?;
        }
        // Deleted text leaves the search indexes at once (not only when their segments merge later).
        let _ = c.execute_batch("INSERT INTO entries_fts(entries_fts, rank) VALUES('secure-delete', 1); INSERT INTO messages_fts(messages_fts, rank) VALUES('secure-delete', 1);");
        // The approval card's message on its channel (Telegram message id, Slack ts, Discord id), to close it later.
        add_col("approvals", "card_ref", "TEXT")?;
        add_col("entries", "pinned", "INTEGER NOT NULL DEFAULT 0")?;
        add_col("entries", "origin", "TEXT NOT NULL DEFAULT ''")?;
        add_col("entries", "bot_id", "INTEGER")?;
        add_col("entries", "run_id", "INTEGER")?;
        add_col("entries", "tainted", "INTEGER NOT NULL DEFAULT 0")?;
        add_col("runs", "tainted", "INTEGER NOT NULL DEFAULT 0")?;
        add_col("approvals", "kind", "TEXT NOT NULL DEFAULT 'rule'")?;
        add_col("approvals", "site", "TEXT")?;
        add_col("approvals", "parked_args", "TEXT")?;
        add_col("runs", "reply", "TEXT")?;
        // Memories search pulled into the turn ("3,17"), so a correction of the reply can count against them.
        add_col("runs", "recalled", "TEXT")?;
        add_col("entries", "missteps", "INTEGER NOT NULL DEFAULT 0")?;
        add_col("routines", "delivery", "TEXT NOT NULL DEFAULT 'now'")?;
        add_col("entries", "background", "INTEGER NOT NULL DEFAULT 0")?;
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS skills(id INTEGER PRIMARY KEY, scope TEXT NOT NULL, bot_id INTEGER NOT NULL, name TEXT NOT NULL,
               description TEXT NOT NULL, body TEXT NOT NULL, run_id INTEGER, created_at INTEGER NOT NULL,
               status TEXT NOT NULL DEFAULT 'proposed' CHECK(status IN ('proposed','active','dismissed','removed')))",
        )?;
        // Skills that improve with use: why a proposal was made, what kind it is, what it replaces; and every use.
        add_col("skills", "reason", "TEXT NOT NULL DEFAULT ''")?;
        add_col("skills", "kind", "TEXT NOT NULL DEFAULT 'learned'")?;
        add_col("skills", "replaces", "TEXT NOT NULL DEFAULT ''")?;
        c.execute_batch("CREATE TABLE IF NOT EXISTS skill_uses(skill_id INTEGER NOT NULL, run_id INTEGER, ts INTEGER NOT NULL)")?;
        // When search last brought a memory up (memories nobody looks up any more can leave standing memory).
        add_col("entries", "last_recalled", "INTEGER")?;
        // When a memory stops being true ("away until Friday"): past it, it leaves standing memory; search still finds it.
        add_col("entries", "valid_until", "INTEGER")?;
        // When something learned was taken back (Undo): how often learning gets it wrong.
        add_col("entries", "undone_at", "INTEGER")?;
        // What the person wrote that started the run (chat messages only), so a tool can tell their own words apart.
        add_col("runs", "asked", "TEXT")?;
        add_col("suggestions", "check_note", "TEXT")?;
        // The run an approval belongs to, so "allow all like this" can cover the rest of that reply.
        add_col("approvals", "run_id", "INTEGER")?;
        // Meetings and calls you transcribed: who said what (you = your microphone, others = the call's sound).
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS meetings(id INTEGER PRIMARY KEY, title TEXT NOT NULL, started_at INTEGER NOT NULL, ended_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS meeting_lines(id INTEGER PRIMARY KEY, meeting_id INTEGER NOT NULL, ts INTEGER NOT NULL, who TEXT NOT NULL, text TEXT NOT NULL);",
        )?;
        // What the person jotted down during the call.
        add_col("meetings", "notes", "TEXT NOT NULL DEFAULT ''")?;
        // Names for the speakers told apart ({"others:2": "Sam"}).
        add_col("meetings", "speakers", "TEXT NOT NULL DEFAULT '{}'")?;
        // Who was invited (from the calendar), a JSON list of names.
        add_col("meetings", "attendees", "TEXT NOT NULL DEFAULT '[]'")?;
        // The notes a bot wrote after the call (summary, decisions, action items).
        add_col("meetings", "summary", "TEXT NOT NULL DEFAULT ''")?;
        // A task handed to another bot (ask_bot): the run that asked, so the hand-off can be followed.
        add_col("runs", "parent_run", "INTEGER")?;
        // A correction, kept whole so a proposed preference can be replayed against it: what was asked, the reply
        // the person pushed back on, and what they said.
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS cases(id INTEGER PRIMARY KEY, bot_id INTEGER NOT NULL, chat_id INTEGER NOT NULL,
               asked TEXT NOT NULL, bad TEXT NOT NULL, correction TEXT NOT NULL, ts INTEGER NOT NULL)",
        )?;
        // 'demote' joined the suggestion actions; SQLite can't alter a CHECK, so older DBs rebuild the table once.
        let sugg: String = c.query_row("SELECT sql FROM sqlite_master WHERE name='suggestions'", [], |r| r.get(0))?;
        if !sugg.contains("'demote'") {
            let new = sugg.replace("'forget')", "'forget','demote')");
            c.execute_batch(&format!(
                "BEGIN; ALTER TABLE suggestions RENAME TO suggestions_old; {new}; INSERT INTO suggestions SELECT * FROM suggestions_old; DROP TABLE suggestions_old; COMMIT;"
            ))?;
        }
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS digest_items(id INTEGER PRIMARY KEY, bot_id INTEGER NOT NULL, chat_id INTEGER NOT NULL,
               routine_id INTEGER, text TEXT NOT NULL, ts INTEGER NOT NULL, sent_at INTEGER)",
        )?;
        add_col("digest_items", "tainted", "INTEGER NOT NULL DEFAULT 0")?;
        c.execute_batch("CREATE TABLE IF NOT EXISTS inbox_dismissed(kind TEXT NOT NULL, ref INTEGER NOT NULL, PRIMARY KEY(kind, ref))")?;
        // Files in a chat's workspace that Mimir wrote, and whether each holds outside content.
        c.execute_batch("CREATE TABLE IF NOT EXISTS files(chat_id INTEGER NOT NULL, path TEXT NOT NULL, tainted INTEGER NOT NULL, PRIMARY KEY(chat_id, path))")?;
        // The file's modification time when Mimir last wrote it: changed since (by hand, or deleted and made again) means
        // it isn't only what Mimir wrote.
        add_col("files", "mtime", "INTEGER NOT NULL DEFAULT 0")?;
        // What the person did that says how they want things done: `correction` (pushed back on a reply),
        // `stopped` (cut a reply off), `denied` (said no to an action).
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS signals(id INTEGER PRIMARY KEY, bot_id INTEGER NOT NULL, chat_id INTEGER NOT NULL,
               kind TEXT NOT NULL, detail TEXT NOT NULL, ts INTEGER NOT NULL)",
        )?;
        c.execute_batch("CREATE TABLE IF NOT EXISTS site_rules(tool TEXT NOT NULL, host TEXT NOT NULL, action TEXT NOT NULL CHECK(action IN ('allow','ask','deny')), PRIMARY KEY(tool, host))")?;
        // Chats from non-Telegram channels: (channel, external id) → allocated chat_id. Telegram rows keep their native id.
        add_col("chats", "channel", "TEXT NOT NULL DEFAULT 'telegram'")?;
        add_col("chats", "external", "TEXT")?;
        c.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS chats_external ON chats(channel, external) WHERE external IS NOT NULL")?;
        // Multi-bot: rows from single-bot builds belong to bot 1.
        for t in ["routines", "runs", "approvals", "messages"] {
            add_col(t, "bot_id", "INTEGER NOT NULL DEFAULT 1")?;
        }
        add_col("routines", "quiet", "INTEGER NOT NULL DEFAULT 0")?;
        // A one-off (a reminder): runs at next_fire, then it's gone.
        add_col("routines", "once", "INTEGER NOT NULL DEFAULT 0")?;
        // The change gate: what a quiet check watches, and how it looked when last checked (watch.rs).
        add_col("routines", "watch", "TEXT NOT NULL DEFAULT ''")?;
        add_col("routines", "watch_seen", "TEXT")?;
        // Shapes of a watched page's lines that change on every load (watch.rs), one per line.
        add_col("routines", "watch_noise", "TEXT NOT NULL DEFAULT ''")?;
        add_col("routines", "retries", "INTEGER NOT NULL DEFAULT 0")?;
        add_col("runs", "delivered", "TEXT")?;
        add_col("runs", "cache_read", "INTEGER NOT NULL DEFAULT 0")?;
        add_col("runs", "estimated", "INTEGER NOT NULL DEFAULT 0")?;
        c.execute(
            "INSERT OR IGNORE INTO bots(id, name, provider, model, base_url, created_at) VALUES (1, 'Mimir',
               coalesce((SELECT value FROM settings WHERE key='provider'), 'claude'),
               coalesce((SELECT value FROM settings WHERE key='model'), ''),
               coalesce((SELECT value FROM settings WHERE key='base_url'), ''), ?1)",
            [now()],
        )?;
        c.execute_batch(
            "INSERT OR IGNORE INTO sessions(bot_id, chat_id, session) SELECT 1, chat_id, claude_session FROM chats WHERE claude_session IS NOT NULL;
             UPDATE chats SET claude_session = NULL WHERE claude_session IS NOT NULL;",
        )?;
        // Runs left 'running' by a crash (older than the max turn length) are closed out.
        c.execute(
            "UPDATE runs SET status='error', error='interrupted', ended_at=started_at WHERE status='running' AND started_at < ?1",
            [now() - 900],
        )?;
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| ".".into());
        Ok(Db(Arc::new(Mutex::new(c)), Arc::new(dir)))
    }

    /// The data directory (MIMIR_HOME) this database lives in.
    /// A workspace file's mark: (holds outside content, its modification time when Mimir wrote it); None for a file
    /// Mimir didn't write. `key` is the file's canonical name (files::key).
    pub fn file_tainted(&self, chat: i64, key: &str) -> Result<Option<(bool, i64)>> {
        Ok(self.c().query_row("SELECT tainted, mtime FROM files WHERE chat_id=?1 AND path=?2", params![chat, key], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
    }

    pub fn file_mark(&self, chat: i64, key: &str, tainted: bool, mtime: i64) -> Result<()> {
        self.c().execute(
            "INSERT INTO files(chat_id, path, tainted, mtime) VALUES (?1,?2,?3,?4) ON CONFLICT(chat_id, path) DO UPDATE SET tainted = excluded.tainted, mtime = excluded.mtime",
            params![chat, key, tainted, mtime],
        )?;
        Ok(())
    }

    pub fn file_forget(&self, chat: i64, path: &str) -> Result<()> {
        self.c().execute("DELETE FROM files WHERE chat_id=?1 AND path=?2", params![chat, path])?;
        Ok(())
    }

    pub fn dir(&self) -> std::path::PathBuf {
        (*self.1).clone()
    }

    fn c(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------- memory ----------

    pub fn store(&self, scope: &str, kind: &str, title: &str, body: &str, source: &str, supersedes: Option<i64>) -> Result<i64> {
        self.store_by(scope, kind, title, body, source, supersedes, &Prov::default())
    }

    /// `store`, recording who wrote it.
    #[allow(clippy::too_many_arguments)]
    pub fn store_by(&self, scope: &str, kind: &str, title: &str, body: &str, source: &str, supersedes: Option<i64>, prov: &Prov) -> Result<i64> {
        if !TYPES.contains(&kind) {
            bail!("type must be one of {TYPES:?}");
        }
        if title.trim().is_empty() || body.trim().is_empty() {
            bail!("title and body are required");
        }
        let mut c = self.c();
        let tx = c.transaction()?;
        let dup: Option<i64> = tx
            .query_row(
                "SELECT id FROM entries WHERE scope=?1 AND title=?2 AND body=?3 AND superseded_by IS NULL AND invalid=0",
                params![scope, title, body],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = dup {
            return Ok(id);
        }
        tx.execute(
            "INSERT INTO entries(scope,type,title,body,source,importance,created_at,origin,bot_id,run_id,tainted) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![scope, kind, title, body, source, importance(kind), now(), prov.origin, prov.bot, prov.run, prov.tainted],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(old) = supersedes {
            let n = tx.execute(
                // id != ?1: the row just inserted can't be the one it replaces (an unknown id could match it).
                "UPDATE entries SET superseded_by=?1 WHERE id=?2 AND id != ?1 AND scope=?3 AND superseded_by IS NULL",
                params![id, old, scope],
            )?;
            if n == 0 {
                bail!("entry #{old} not found or already superseded");
            }
            // A correction keeps the pin (or the background) and the end date; an unconfirmed one doesn't get the pin,
            // which would put it in every prompt.
            tx.execute(
                "UPDATE entries SET (pinned, background, valid_until)=(SELECT pinned AND NOT ?3, background, valid_until FROM entries WHERE id=?2) WHERE id=?1",
                params![id, old, prov.tainted],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    pub fn forget(&self, scope: &str, id: i64) -> Result<bool> {
        Ok(self.c().execute("UPDATE entries SET invalid=1 WHERE id=?1 AND scope=?2", params![id, scope])? > 0)
    }

    /// Take back something learned: forget it, and if it replaced an earlier version, that one is current again.
    pub fn undo(&self, scope: &str, id: i64) -> Result<bool> {
        let mut c = self.c();
        let tx = c.transaction()?;
        let n = tx.execute("UPDATE entries SET invalid=1, undone_at=?3 WHERE id=?1 AND scope=?2 AND invalid=0 AND superseded_by IS NULL", params![id, scope, now()])?;
        if n > 0 {
            tx.execute("UPDATE entries SET superseded_by=NULL WHERE superseded_by=?1 AND scope=?2", params![id, scope])?;
        }
        tx.commit()?;
        Ok(n > 0)
    }

    /// You checked a memory a bot saved after reading outside content: it's no longer marked unconfirmed.
    pub fn confirm(&self, scope: &str, id: i64) -> Result<bool> {
        Ok(self.c().execute("UPDATE entries SET tainted=0 WHERE id=?1 AND scope=?2", params![id, scope])? > 0)
    }

    /// Pinning brings a background memory back.
    pub fn pin(&self, scope: &str, id: i64, on: bool) -> Result<bool> {
        let sql = "UPDATE entries SET pinned=?3, background=background AND NOT ?3 WHERE id=?1 AND scope=?2 AND superseded_by IS NULL AND invalid=0";
        Ok(self.c().execute(sql, params![id, scope, on])? > 0)
    }

    /// Demote instead of delete: a background memory leaves standing memory (and loses its pin), but
    /// search still finds it when a message needs it.
    /// Give a memory an end date (unix seconds), or take it away (None).
    pub fn set_until(&self, scope: &str, id: i64, until: Option<i64>) -> Result<bool> {
        Ok(self.c().execute("UPDATE entries SET valid_until=?3 WHERE id=?1 AND scope=?2 AND superseded_by IS NULL AND invalid=0", params![id, scope, until])? > 0)
    }

    pub fn background(&self, scope: &str, id: i64, on: bool) -> Result<bool> {
        let sql = "UPDATE entries SET background=?3, pinned=pinned AND NOT ?3 WHERE id=?1 AND scope=?2 AND superseded_by IS NULL AND invalid=0";
        Ok(self.c().execute(sql, params![id, scope, on])? > 0)
    }

    /// Delete a memory for good, with every earlier version of it (forget only hides it). Its search
    /// index rows go with it; suggestions about it are dropped. Returns how many entries were erased.
    pub fn erase(&self, scope: &str, id: i64) -> Result<usize> {
        let mut c = self.c();
        let tx = c.transaction()?;
        let mut ids = vec![id];
        let mut next = vec![id];
        while let Some(cur) = next.pop() {
            let mut st = tx.prepare("SELECT id FROM entries WHERE superseded_by=?1 AND scope=?2")?;
            for older in st.query_map(params![cur, scope], |r| r.get::<_, i64>(0))? {
                let older = older?;
                ids.push(older);
                next.push(older);
            }
        }
        let mut n = 0;
        for i in &ids {
            // Copies of its text elsewhere go too: an accepted suggestion that added it, the tool log's lines about it.
            let row: Option<(String, String)> = tx.query_row("SELECT title, body FROM entries WHERE id=?1 AND scope=?2", params![i, scope], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            if let Some((title, body)) = row {
                tx.execute("DELETE FROM suggestions WHERE scope=?1 AND lower(title)=lower(?2) AND body=?3", params![scope, title, body])?;
                if body.len() >= 8 {
                    tx.execute("UPDATE tool_calls SET summary='(an erased memory)', result='' WHERE instr(summary, ?1) > 0 OR instr(result, ?1) > 0", [&body])?;
                }
            }
            n += tx.execute("DELETE FROM entries WHERE id=?1 AND scope=?2", params![i, scope])?;
            tx.execute("DELETE FROM suggestions WHERE entry_id=?1 AND scope=?2", params![i, scope])?;
        }
        tx.commit()?;
        drop(c);
        self.wipe_free_space();
        Ok(n)
    }

    /// bm25 × importance × recency decay (0.995/day, floor 0.05); drops hits under 20% of the top score.
    pub fn search(&self, scope: &str, query: &str, limit: usize) -> Result<Vec<Entry>> {
        let Some(q) = fts_query(query) else { return Ok(vec![]) };
        let c = self.c();
        let mut st = c.prepare(
            "SELECT e.id, e.type, e.title, e.body, e.source, e.importance, e.created_at, bm25(entries_fts), e.tainted, e.valid_until
             FROM entries_fts JOIN entries e ON e.id = entries_fts.rowid
             WHERE entries_fts MATCH ?1 AND e.scope = ?2 AND e.superseded_by IS NULL AND e.invalid = 0
             ORDER BY bm25(entries_fts) LIMIT 50",
        )?;
        let t = now();
        let mut hits: Vec<(f64, Entry)> = st
            .query_map(params![q, scope], |r| {
                let imp: i64 = r.get(5)?;
                let created: i64 = r.get(6)?;
                let bm25: f64 = r.get(7)?;
                let days = ((t - created).max(0) as f64) / 86400.0;
                let score = -bm25 * (imp as f64 / 3.0) * 0.995f64.powf(days).max(0.05);
                Ok((score, Entry { id: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, body: r.get(3)?, source: r.get(4)?, tainted: r.get(8)?, created_at: created, valid_until: r.get(9)? }))
            })?
            .collect::<Result<_, _>>()?;
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        let top = hits.first().map(|h| h.0).unwrap_or(0.0);
        let out: Vec<Entry> = hits.into_iter().filter(|h| h.0 >= top * 0.2).take(limit).map(|h| h.1).collect();
        for e in &out {
            c.execute("UPDATE entries SET access_count = access_count + 1, last_recalled = ?2 WHERE id=?1", params![e.id, now()])?;
        }
        Ok(out)
    }

    /// Standing context injected every turn: pinned memories (always), then preferences, then the most recent
    /// facts and tasks, within a size budget. Everything else is still found by search when a message needs it.
    pub fn bootstrap(&self, scope: &str) -> Result<Vec<Entry>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, type, title, body, source, pinned, tainted, created_at, valid_until FROM entries
             WHERE scope=?1 AND superseded_by IS NULL AND invalid=0 AND background=0 AND tainted=0 AND (valid_until IS NULL OR valid_until > ?2)
             ORDER BY pinned DESC, (type='convention') DESC, created_at DESC, id DESC LIMIT 300",
        )?;
        let rows: Vec<(bool, Entry)> = st
            .query_map(params![scope, now()], |r| Ok((r.get(5)?, Entry { id: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, body: r.get(3)?, source: r.get(4)?, tainted: r.get(6)?, created_at: r.get(7)?, valid_until: r.get(8)? })))?
            .collect::<Result<_, _>>()?;
        let (mut used, mut recent, mut out) = (0, 0, vec![]);
        for (pinned, e) in rows {
            let size = e.line().len();
            let fits = used + size <= BOOTSTRAP_CHARS && (e.kind == "convention" || recent < BOOTSTRAP_RECENT);
            // Pinned is a promise: always in, though it still uses up the budget.
            if pinned || fits {
                used += size;
                recent += usize::from(!pinned && e.kind != "convention");
                out.push(e);
            }
        }
        Ok(out)
    }

    /// Memories saved after reading outside content and not confirmed yet (for showing, never for prompts).
    pub fn search_unconfirmed(&self, scope: &str) -> Result<Vec<Entry>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, type, title, body, source, tainted, created_at, valid_until FROM entries WHERE scope=?1 AND superseded_by IS NULL AND invalid=0 AND tainted=1 ORDER BY id DESC LIMIT 30")?;
        let v = st.query_map([scope], |r| Ok(Entry { id: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, body: r.get(3)?, source: r.get(4)?, tainted: r.get(5)?, created_at: r.get(6)?, valid_until: r.get(7)? }))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// How full standing memory is: (characters used, the budget, memories in it). Pinned ones can push it past 100%.
    pub fn standing_usage(&self, scope: &str) -> Result<(usize, usize, usize)> {
        let b = self.bootstrap(scope)?;
        Ok((b.iter().map(|e| e.line().len()).sum(), BOOTSTRAP_CHARS, b.len()))
    }

    /// Every entry in a scope incl. superseded/forgotten history, newest first (memory browser).
    pub fn entries(&self, scope: &str) -> Result<Vec<EntryRow>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, type, title, body, source, created_at, superseded_by, invalid, access_count, pinned, origin, bot_id, run_id, tainted, missteps, background, valid_until,
               (SELECT asked FROM runs WHERE runs.id = entries.run_id)
             FROM entries WHERE scope=?1 ORDER BY id DESC LIMIT 1000",
        )?;
        let v = st
            .query_map([scope], |r| {
                let superseded_by: Option<i64> = r.get(6)?;
                let invalid: bool = r.get(7)?;
                Ok(EntryRow {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    title: r.get(2)?,
                    body: r.get(3)?,
                    source: r.get(4)?,
                    created_at: r.get(5)?,
                    status: if invalid { "forgotten" } else if superseded_by.is_some() { "superseded" } else { "active" },
                    superseded_by,
                    access_count: r.get(8)?,
                    pinned: r.get(9)?,
                    origin: r.get(10)?,
                    bot_id: r.get(11)?,
                    run_id: r.get(12)?,
                    tainted: r.get(13)?,
                    missteps: r.get(14)?,
                    background: r.get(15)?,
                    valid_until: r.get(16)?,
                    // What they typed, without a file's text after it.
                    asked: r.get::<_, Option<String>>(17)?.map(|a| a.split("\n\n[Attached:").next().unwrap_or("").to_string()),
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    // ---------- settings (non-secret; secrets live in the OS keychain) ----------

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional()?)
    }

    /// Read a setting and delete it in one step: only one of two callers racing for it gets it.
    pub fn setting_take(&self, key: &str) -> Result<Option<String>> {
        Ok(self.c().query_row("DELETE FROM settings WHERE key=?1 RETURNING value", [key], |r| r.get(0)).optional()?)
    }

    /// Take or renew the computer for `who` ("run:<id>" or "you") for `secs`. Free, expired or already ours: it's
    /// ours. Otherwise returns who holds it ({who, name}). Atomic, so bots in separate processes can't both get it.
    pub fn computer_lease(&self, who: &str, name: &str, secs: i64) -> Result<Option<(String, String)>> {
        let c = self.c();
        let lease = serde_json::json!({"who": who, "name": name, "until": now() + secs}).to_string();
        let n = c.execute(
            "INSERT INTO settings(key, value) VALUES ('computer_lease', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value
             WHERE json_extract(settings.value, '$.until') < ?2 OR json_extract(settings.value, '$.who') = ?3",
            params![lease, now(), who],
        )?;
        if n == 1 {
            return Ok(None);
        }
        let held: Option<String> = c.query_row("SELECT value FROM settings WHERE key='computer_lease'", [], |r| r.get(0)).optional()?;
        let v: serde_json::Value = held.and_then(|h| serde_json::from_str(&h).ok()).unwrap_or_default();
        Ok(Some((v["who"].as_str().unwrap_or("").to_string(), v["name"].as_str().unwrap_or("someone").to_string())))
    }

    /// Who's using the computer right now, if anyone: (who, name).
    pub fn computer_holder(&self) -> Result<Option<(String, String)>> {
        let held: Option<String> = self.c().query_row("SELECT value FROM settings WHERE key='computer_lease'", [], |r| r.get(0)).optional()?;
        let v: serde_json::Value = held.and_then(|h| serde_json::from_str(&h).ok()).unwrap_or_default();
        Ok((v["until"].as_i64().unwrap_or(0) >= now()).then(|| (v["who"].as_str().unwrap_or("").to_string(), v["name"].as_str().unwrap_or("").to_string())))
    }

    /// Let go of the computer (only if `who` holds it).
    pub fn computer_release(&self, who: &str) -> Result<()> {
        self.c().execute("DELETE FROM settings WHERE key='computer_lease' AND json_extract(value, '$.who') = ?1", [who])?;
        Ok(())
    }

    /// Save a transcribed meeting; returns its id. Lines are (unix time, "you"|"others", text).
    pub fn meeting_save(&self, title: &str, started: i64, ended: i64, lines: &[(i64, String, String)]) -> Result<i64> {
        let mut c = self.c();
        let tx = c.transaction()?;
        tx.execute("INSERT INTO meetings(title, started_at, ended_at) VALUES (?1,?2,?3)", params![title.trim(), started, ended])?;
        let id = tx.last_insert_rowid();
        for (ts, who, text) in lines {
            tx.execute("INSERT INTO meeting_lines(meeting_id, ts, who, text) VALUES (?1,?2,?3,?4)", params![id, ts, who, text])?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Meetings, newest first: (id, title, started, ended, words).
    pub fn meetings(&self) -> Result<Vec<serde_json::Value>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT m.id, m.title, m.started_at, m.ended_at, (SELECT coalesce(sum(length(text) - length(replace(text, ' ', '')) + 1), 0) FROM meeting_lines l WHERE l.meeting_id = m.id), m.notes, m.speakers, m.attendees, m.summary
             FROM meetings m ORDER BY m.id DESC LIMIT 200",
        )?;
        let v = st
            .query_map([], |r| Ok(serde_json::json!({"id": r.get::<_, i64>(0)?, "title": r.get::<_, String>(1)?, "started_at": r.get::<_, i64>(2)?, "ended_at": r.get::<_, i64>(3)?, "words": r.get::<_, i64>(4)?, "notes": r.get::<_, String>(5)?, "speakers": serde_json::from_str::<serde_json::Value>(&r.get::<_, String>(6)?).unwrap_or_default(), "attendees": serde_json::from_str::<serde_json::Value>(&r.get::<_, String>(7)?).unwrap_or_default(), "summary": r.get::<_, String>(8)?})))?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// One meeting's lines, in order.
    pub fn meeting_lines(&self, id: i64) -> Result<Vec<(i64, String, String)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT ts, who, text FROM meeting_lines WHERE meeting_id=?1 ORDER BY ts, id")?;
        let v = st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// A line transcribed during a meeting that's still going (saved as it comes, so nothing is lost if the app quits).
    pub fn meeting_line(&self, id: i64, ts: i64, who: &str, text: &str) -> Result<()> {
        let c = self.c();
        c.execute("INSERT INTO meeting_lines(meeting_id, ts, who, text) VALUES (?1,?2,?3,?4)", params![id, ts, who, text])?;
        c.execute("UPDATE meetings SET ended_at=max(ended_at, ?2) WHERE id=?1", params![id, now()])?;
        Ok(())
    }

    pub fn meeting_update(&self, id: i64, title: Option<&str>, ended: Option<i64>) -> Result<bool> {
        let c = self.c();
        Ok(c.execute("UPDATE meetings SET title=coalesce(?2, title), ended_at=coalesce(?3, ended_at) WHERE id=?1", params![id, title.map(str::trim), ended])? > 0)
    }

    /// The other side's lines, (line id, ts), for telling speakers apart.
    pub fn meeting_others(&self, id: i64) -> Result<Vec<(i64, i64)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, ts FROM meeting_lines WHERE meeting_id=?1 AND who LIKE 'others%' ORDER BY ts, id")?;
        let v = st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Who said each line, once speakers are told apart ("others:2").
    pub fn meeting_relabel(&self, labels: &[(i64, String)]) -> Result<()> {
        let mut c = self.c();
        let tx = c.transaction()?;
        for (line, who) in labels {
            tx.execute("UPDATE meeting_lines SET who=?2 WHERE id=?1", params![line, who])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn meeting_summary(&self, id: i64, summary: &str) -> Result<bool> {
        Ok(self.c().execute("UPDATE meetings SET summary=?2 WHERE id=?1", params![id, summary.trim()])? > 0)
    }

    pub fn meeting_attendees(&self, id: i64, names: &[String]) -> Result<bool> {
        Ok(self.c().execute("UPDATE meetings SET attendees=?2 WHERE id=?1", params![id, serde_json::to_string(names)?])? > 0)
    }

    /// Name a speaker ("others:2" → "Sam"); an empty name forgets it.
    pub fn meeting_speaker(&self, id: i64, key: &str, name: &str) -> Result<bool> {
        let c = self.c();
        let cur: String = c.query_row("SELECT speakers FROM meetings WHERE id=?1", [id], |r| r.get(0))?;
        let mut map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&cur).unwrap_or_default();
        if name.trim().is_empty() {
            map.remove(key);
        } else {
            map.insert(key.into(), name.trim().into());
        }
        Ok(c.execute("UPDATE meetings SET speakers=?2 WHERE id=?1", params![id, serde_json::Value::Object(map).to_string()])? > 0)
    }

    /// What the person jotted down during the meeting (their own words: what mattered to them).
    pub fn meeting_notes(&self, id: i64, notes: &str) -> Result<bool> {
        Ok(self.c().execute("UPDATE meetings SET notes=?2 WHERE id=?1", params![id, notes])? > 0)
    }

    pub fn has_meetings(&self) -> bool {
        self.c().query_row("SELECT EXISTS(SELECT 1 FROM meeting_lines)", [], |r| r.get(0)).unwrap_or(false)
    }

    /// Lines that mention `query` (any case), newest meetings first: (meeting id, title, started, ts, who, text).
    #[allow(clippy::type_complexity)]
    pub fn meeting_search(&self, query: &str, limit: usize) -> Result<Vec<(i64, String, i64, i64, String, String)>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT m.id, m.title, m.started_at, l.ts, l.who, l.text FROM meeting_lines l JOIN meetings m ON m.id = l.meeting_id
             WHERE instr(lower(l.text), lower(?1)) > 0 ORDER BY m.started_at DESC, l.ts LIMIT ?2",
        )?;
        let v = st.query_map(params![query.trim(), limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn meeting_delete(&self, id: i64) -> Result<bool> {
        let c = self.c();
        c.execute("DELETE FROM meeting_lines WHERE meeting_id=?1", [id])?;
        Ok(c.execute("DELETE FROM meetings WHERE id=?1", [id])? > 0)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---------- bots ----------

    pub fn bots(&self) -> Result<Vec<Bot>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, name, persona, provider, model, base_url, tools, look FROM bots ORDER BY id")?;
        let v = st
            .query_map([], |r| {
                Ok(Bot {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    persona: r.get(2)?,
                    provider: r.get(3)?,
                    model: r.get(4)?,
                    base_url: r.get(5)?,
                    tools: r.get(6)?,
                    look: r.get(7)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn bot(&self, id: i64) -> Result<Bot> {
        self.bots()?.into_iter().find(|b| b.id == id).ok_or_else(|| anyhow::anyhow!("no bot #{id}"))
    }

    /// Insert (id <= 0) or update. Returns the bot id.
    pub fn bot_save(&self, b: &Bot) -> Result<i64> {
        if b.name.trim().is_empty() {
            bail!("a bot needs a name");
        }
        if !valid_look(&b.look) {
            bail!("that look isn't valid");
        }
        let c = self.c();
        if b.id <= 0 {
            c.execute(
                "INSERT INTO bots(name, persona, provider, model, base_url, tools, look, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![b.name.trim(), b.persona, b.provider, b.model.trim(), b.base_url.trim(), b.tools, b.look, now()],
            )?;
            return Ok(c.last_insert_rowid());
        }
        let n = c.execute(
            "UPDATE bots SET name=?2, persona=?3, provider=?4, model=?5, base_url=?6, tools=?7, look=?8 WHERE id=?1",
            params![b.id, b.name.trim(), b.persona, b.provider, b.model.trim(), b.base_url.trim(), b.tools, b.look],
        )?;
        if n == 0 {
            bail!("no bot #{}", b.id);
        }
        Ok(b.id)
    }

    /// Removes the bot and its routines/sessions. Memory is the person's, so it stays.
    pub fn bot_delete(&self, id: i64) -> Result<()> {
        if self.bots()?.len() <= 1 {
            bail!("keep at least one bot");
        }
        // Its conversations and what it did go with it (memories are the person's, shared by all bots: they stay).
        self.c().execute_batch(&format!(
            "DELETE FROM tool_calls WHERE run_id IN (SELECT id FROM runs WHERE bot_id={id});
             DELETE FROM runs WHERE bot_id={id}; DELETE FROM messages WHERE bot_id={id};
             DELETE FROM signals WHERE bot_id={id}; DELETE FROM cases WHERE bot_id={id}; DELETE FROM digest_items WHERE bot_id={id};
             DELETE FROM routines WHERE bot_id={id}; DELETE FROM sessions WHERE bot_id={id}; DELETE FROM conv_taint WHERE bot_id={id};
             DELETE FROM settings WHERE key LIKE 'seen:{id}:%'; DELETE FROM bots WHERE id={id};"
        ))?;
        self.wipe_free_space();
        Ok(())
    }

    /// Delete a chat's history: its conversation, the run log and tool calls, corrections, digests, decided
    /// approvals and decided suggestions. Memories, routines and the pairing stay. Returns how many messages went.
    pub fn chat_history_clear(&self, chat: i64) -> Result<usize> {
        let mut c = self.c();
        let tx = c.transaction()?;
        let n = tx.execute("DELETE FROM messages WHERE chat_id=?1", [chat])?;
        for sql in [
            "DELETE FROM tool_calls WHERE chat_id=?1",
            "DELETE FROM runs WHERE chat_id=?1 AND status != 'running'",
            "DELETE FROM signals WHERE chat_id=?1",
            "DELETE FROM cases WHERE chat_id=?1",
            "DELETE FROM digest_items WHERE chat_id=?1",
            "DELETE FROM approvals WHERE chat_id=?1 AND status != 'pending'",
            "DELETE FROM sessions WHERE chat_id=?1",
            "DELETE FROM conv_taint WHERE chat_id=?1",
        ] {
            tx.execute(sql, [chat])?;
        }
        tx.execute("DELETE FROM suggestions WHERE scope=?1 AND status != 'pending'", [format!("chat:{chat}")])?;
        tx.commit()?;
        drop(c);
        self.wipe_free_space();
        Ok(n)
    }

    /// Everything kept about the person, for them to take away: memories (with earlier versions), conversations,
    /// meetings and their transcripts, skills, routines, chats and bots (tokens live in the keychain, not here).
    pub fn export_all(&self) -> Result<serde_json::Value> {
        let c = self.c();
        let mut out = serde_json::Map::new();
        out.insert("exported_at".into(), serde_json::json!(now()));
        for table in ["entries", "messages", "meetings", "meeting_lines", "skills", "routines", "chats", "bots", "suggestions"] {
            let mut st = c.prepare(&format!("SELECT * FROM {table}"))?;
            let cols: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
            let rows: Vec<serde_json::Value> = st
                .query_map([], |r| {
                    let mut m = serde_json::Map::new();
                    for (i, col) in cols.iter().enumerate() {
                        let v = match r.get_ref(i)? {
                            rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                            rusqlite::types::ValueRef::Integer(n) => serde_json::json!(n),
                            rusqlite::types::ValueRef::Real(f) => serde_json::json!(f),
                            rusqlite::types::ValueRef::Text(t) => serde_json::json!(String::from_utf8_lossy(t)),
                            rusqlite::types::ValueRef::Blob(_) => serde_json::Value::Null,
                        };
                        m.insert(col.clone(), v);
                    }
                    Ok(serde_json::Value::Object(m))
                })?
                .collect::<Result<_, _>>()?;
            out.insert(table.into(), serde_json::Value::Array(rows));
        }
        Ok(serde_json::Value::Object(out))
    }

    /// After deleting: fold the write-ahead log back into the file, so deleted text isn't left there either.
    fn wipe_free_space(&self) {
        let _ = self.c().execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }

    /// At startup: the run log's details, corrections and signals older than six months go.
    pub fn prune_old(&self) -> Result<()> {
        let before = now() - 180 * 86400;
        let c = self.c();
        c.execute("DELETE FROM tool_calls WHERE ts < ?1", [before])?;
        c.execute("DELETE FROM cases WHERE ts < ?1", [before])?;
        c.execute("DELETE FROM signals WHERE ts < ?1", [before])?;
        c.execute("DELETE FROM digest_items WHERE sent_at IS NOT NULL AND sent_at < ?1", [before])?;
        // Buttons nobody tapped in a month (runtime::Tap): they'd only say "already done" by then anyway.
        c.execute("DELETE FROM settings WHERE key LIKE 'tap:%' AND CAST(json_extract(value, '$.at') AS INTEGER) < ?1", [now() - 30 * 86400])?;
        Ok(())
    }

    // ---------- run log ----------

    pub fn run_start(&self, bot: i64, chat: i64, trigger: &str, routine_id: Option<i64>, provider: &str) -> Result<i64> {
        let c = self.c();
        c.execute(
            "INSERT INTO runs(bot_id, chat_id, trigger, routine_id, provider, started_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![bot, chat, trigger, routine_id, provider, now()],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// Everything being worked on right now, newest first: (bot, chat, trigger: message | routine | delegated | tidy …).
    /// Not runs older than the longest a turn may take: one a crash left open isn't work (it's closed at the next start).
    pub fn busy(&self) -> Result<Vec<(i64, i64, String)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT bot_id, chat_id, trigger FROM runs WHERE status='running' AND started_at > ?1 ORDER BY id DESC")?;
        let v = st.query_map([now() - 900], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// What started a run: message, routine, rehearsal, delegated…
    pub fn run_trigger(&self, run: i64) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT trigger FROM runs WHERE id=?1", [run], |r| r.get(0)).optional()?)
    }

    /// What a rehearsal would have done (the steps it only described) and what it looked at.
    pub fn rehearsal_steps(&self, run: i64) -> Result<(Vec<String>, Vec<String>)> {
        let c = self.c();
        let mut st = c.prepare("SELECT summary, decision FROM tool_calls WHERE run_id=?1 ORDER BY id")?;
        let v: Vec<(String, String)> = st.query_map([run], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        let (would, looked): (Vec<_>, Vec<_>) = v.into_iter().partition(|(_, d)| d == "rehearsal");
        Ok((would.into_iter().map(|w| w.0).collect(), looked.into_iter().map(|l| l.0).collect()))
    }

    /// The run in progress for (bot, chat), if any.
    /// A run's (bot, chat), while it's still running.
    pub fn run_where(&self, run: i64) -> Result<Option<(i64, i64)>> {
        Ok(self.c().query_row("SELECT bot_id, chat_id FROM runs WHERE id=?1 AND status='running'", [run], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
    }

    pub fn running_run(&self, bot: i64, chat: i64) -> Result<Option<i64>> {
        Ok(self.c().query_row("SELECT id FROM runs WHERE bot_id=?1 AND chat_id=?2 AND status='running' ORDER BY id DESC LIMIT 1", [bot, chat], |r| r.get(0)).optional()?)
    }

    /// The user stopped a reply: close its run, and deny anything it was still waiting on.
    /// Stop the reply in progress (`asked`: what it was answering; recorded as a signal for the tidy-up).
    pub fn run_stop(&self, bot: i64, chat: i64, asked: &str) -> Result<()> {
        self.signal_add(bot, chat, "stopped", asked)?;
        let c = self.c();
        c.execute("UPDATE runs SET status='error', error='Stopped by you', ended_at=?3 WHERE bot_id=?1 AND chat_id=?2 AND status='running'", params![bot, chat, now()])?;
        c.execute("UPDATE approvals SET status='denied' WHERE bot_id=?1 AND chat_id=?2 AND status='pending' AND parked_args IS NULL", [bot, chat])?;
        Ok(())
    }

    pub fn run_asked(&self, run: i64, text: &str) -> Result<()> {
        self.c().execute("UPDATE runs SET asked=?2 WHERE id=?1", params![run, text])?;
        Ok(())
    }

    /// The person's message a run answers; None for routines and tasks from other bots.
    pub fn asked(&self, run: i64) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT asked FROM runs WHERE id=?1", [run], |r| r.get(0)).optional()?.flatten())
    }

    pub fn run_recalled(&self, run: i64, ids: &[i64]) -> Result<()> {
        let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        self.c().execute("UPDATE runs SET recalled=?2 WHERE id=?1", params![run, list])?;
        Ok(())
    }

    /// The person corrected the bot's last reply here: count it against the memories that reply recalled.
    /// `said`: the correction itself. Besides the memories recalled for the reply, preferences in the standing
    /// instructions that it's about (they shape every reply, so they never show up as "recalled") get a misstep too.
    pub fn blame_last_reply(&self, bot: i64, chat: i64, said: &str) -> Result<usize> {
        let c = self.c();
        let recalled: Option<String> = c
            .query_row(
                "SELECT r.recalled FROM messages m JOIN runs r ON r.id = m.run_id
                 WHERE m.bot_id=?1 AND m.chat_id=?2 AND m.role='assistant' ORDER BY m.id DESC LIMIT 1",
                [bot, chat],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let mut n = 0;
        let mut blamed: Vec<i64> = recalled.unwrap_or_default().split(',').filter_map(|s| s.parse::<i64>().ok()).collect();
        for id in &blamed {
            n += c.execute("UPDATE entries SET missteps = missteps + 1 WHERE id=?1 AND scope=?2", params![id, format!("chat:{chat}")])?;
        }
        // Words that say what the correction is about (not "please", "that", "your"…).
        // ponytail: word overlap; a wrong match only adds a misstep the tidy-up weighs, and you accept any change
        let words = |s: &str| -> std::collections::HashSet<String> {
            const FILLER: [&str; 24] = [
                "that", "this", "with", "from", "your", "please", "don't", "dont", "just", "like", "what", "when", "then", "than",
                "they", "them", "have", "about", "always", "never", "instead", "should", "would", "said",
            ];
            s.split(|ch: char| !ch.is_alphanumeric() && ch != '\'').map(str::to_lowercase).filter(|w| w.chars().count() > 3 && !FILLER.contains(&w.as_str())).collect()
        };
        let about = words(said);
        let mut st = c.prepare(
            "SELECT id, title, body FROM entries WHERE scope=?1 AND type='convention' AND superseded_by IS NULL AND invalid=0 AND background=0 AND tainted=0",
        )?;
        let hits: Vec<i64> = st
            .query_map([format!("chat:{chat}")], |r| Ok((r.get::<_, i64>(0)?, format!("{} {}", r.get::<_, String>(1)?, r.get::<_, String>(2)?))))?
            .filter_map(|r| r.ok())
            .filter(|(id, text)| !blamed.contains(id) && !words(text).is_disjoint(&about))
            .map(|(id, _)| id)
            .take(3)
            .collect();
        for id in hits {
            n += c.execute("UPDATE entries SET missteps = missteps + 1 WHERE id=?1", [id])?;
            blamed.push(id);
        }
        Ok(n)
    }

    pub fn signal_add(&self, bot: i64, chat: i64, kind: &str, detail: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO signals(bot_id, chat_id, kind, detail, ts) VALUES (?1,?2,?3,?4,?5)",
            params![bot, chat, kind, crate::tools::truncate(detail, 600), now()],
        )?;
        Ok(())
    }

    /// (kind, detail) of a chat's signals since `since`, oldest first.
    pub fn signals_since(&self, chat: i64, since: i64) -> Result<Vec<(String, String)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT kind, detail FROM signals WHERE chat_id=?1 AND ts > ?2 ORDER BY id LIMIT 30")?;
        let v = st.query_map([chat, since], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Corrections, stops and denials since `since`: how much new evidence a tidy-up would have.
    pub fn signal_count(&self, chat: i64, since: i64) -> Result<i64> {
        Ok(self.c().query_row("SELECT count(*) FROM signals WHERE chat_id=?1 AND ts > ?2", [chat, since], |r| r.get(0))?)
    }

    pub fn run_reply(&self, id: i64, text: &str) -> Result<()> {
        self.c().execute("UPDATE runs SET reply=?2 WHERE id=?1", params![id, crate::tools::truncate(text, 4000)])?;
        Ok(())
    }

    pub fn run_finish(&self, id: i64, outcome: Result<&Usage, &str>) -> Result<()> {
        let c = self.c();
        match outcome {
            Ok(u) => c.execute(
                "UPDATE runs SET status='ok', ended_at=?2, model=?3, tokens_in=?4, tokens_out=?5, cost_usd=?6, cache_read=?7, estimated=?8 WHERE id=?1",
                params![id, now(), u.model, u.tokens_in, u.tokens_out, u.cost_usd, u.cache_read, u.estimated],
            )?,
            Err(e) => c.execute("UPDATE runs SET status='error', ended_at=?2, error=?3 WHERE id=?1", params![id, now(), e])?,
        };
        // An approved plan only lasts for its run, and so does its hold on the computer.
        c.execute("DELETE FROM settings WHERE key = ?1", [format!("plan:{id}")])?;
        c.execute("DELETE FROM settings WHERE key = 'computer_lease' AND json_extract(value, '$.who') = ?1", [format!("run:{id}")])?;
        Ok(())
    }

    /// Newest first. chat = None for all chats.
    pub fn runs(&self, chat: Option<i64>, limit: usize) -> Result<Vec<Run>> {
        self.runs_where(chat, None, limit)
    }

    /// The runs another bot did for this one (tasks it handed over), oldest first.
    pub fn child_runs(&self, parent: i64) -> Result<Vec<Run>> {
        let mut v = self.runs_where(None, Some(parent), 20)?;
        v.reverse();
        Ok(v)
    }

    pub fn run_set_parent(&self, run: i64, parent: i64) -> Result<()> {
        self.c().execute("UPDATE runs SET parent_run=?2 WHERE id=?1", [run, parent])?;
        Ok(())
    }

    fn runs_where(&self, chat: Option<i64>, parent: Option<i64>, limit: usize) -> Result<Vec<Run>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT r.id, r.chat_id, r.trigger, r.routine_id, r.provider, r.model, r.status, r.error, r.started_at, r.ended_at,
                    r.tokens_in, r.tokens_out, r.cost_usd, (SELECT count(*) FROM tool_calls t WHERE t.run_id = r.id), r.bot_id, r.reply, r.parent_run, r.delivered, r.cache_read, r.estimated
             FROM runs r WHERE (?1 IS NULL OR r.chat_id = ?1) AND (?3 IS NULL OR r.parent_run = ?3) ORDER BY r.id DESC LIMIT ?2",
        )?;
        let v = st
            .query_map(params![chat, limit as i64, parent], |r| {
                Ok(Run {
                    id: r.get(0)?,
                    chat_id: r.get(1)?,
                    trigger: r.get(2)?,
                    routine_id: r.get(3)?,
                    provider: r.get(4)?,
                    model: r.get(5)?,
                    status: r.get(6)?,
                    error: r.get(7)?,
                    started_at: r.get(8)?,
                    ended_at: r.get(9)?,
                    tokens_in: r.get(10)?,
                    tokens_out: r.get(11)?,
                    cost_usd: r.get(12)?,
                    tool_calls: r.get(13)?,
                    bot_id: r.get(14)?,
                    reply: r.get(15)?,
                    parent_run: r.get(16)?,
                    delivered: r.get(17)?,
                    repeats: 1,
                    cache_read: r.get(18)?,
                    estimated: r.get(19)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn run_totals(&self, chat: Option<i64>, since: i64) -> Result<Totals> {
        Ok(self.c().query_row(
            "SELECT count(*), coalesce(sum(status='error'),0), coalesce(sum(tokens_in),0), coalesce(sum(tokens_out),0), coalesce(sum(cost_usd),0)
             FROM runs WHERE (?1 IS NULL OR chat_id = ?1) AND started_at >= ?2 AND status NOT IN ('missed','unchanged')",
            params![chat, since],
            |r| Ok(Totals { runs: r.get(0)?, errors: r.get(1)?, tokens_in: r.get(2)?, tokens_out: r.get(3)?, cost_usd: r.get(4)? }),
        )?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tool_call_log(&self, run: Option<i64>, chat: i64, tool: &str, summary: &str, decision: &str, ok: bool, result: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO tool_calls(run_id, chat_id, tool, summary, decision, ok, result, ts) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![run, chat, tool, summary, decision, ok, result, now()],
        )?;
        Ok(())
    }

    /// Failed or refused tool calls for a chat since `since` (tidy-up evidence).
    pub fn failed_tool_calls(&self, chat: i64, since: i64) -> Result<Vec<ToolCall>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, tool, summary, decision, ok, result, ts FROM tool_calls WHERE chat_id=?1 AND ts > ?2 AND ok = 0 ORDER BY id DESC LIMIT 30",
        )?;
        let v = st
            .query_map([chat, since], |r| {
                Ok(ToolCall { id: r.get(0)?, tool: r.get(1)?, summary: r.get(2)?, decision: r.get(3)?, ok: r.get(4)?, result: r.get(5)?, ts: r.get(6)? })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn tool_calls(&self, run: i64) -> Result<Vec<ToolCall>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, tool, summary, decision, ok, result, ts FROM tool_calls WHERE run_id=?1 ORDER BY id")?;
        let v = st
            .query_map([run], |r| {
                Ok(ToolCall { id: r.get(0)?, tool: r.get(1)?, summary: r.get(2)?, decision: r.get(3)?, ok: r.get(4)?, result: r.get(5)?, ts: r.get(6)? })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    // ---------- skills ----------

    /// Runs since `since` that worked cleanly enough to learn from: finished, never read outside content, took at
    /// least two actions besides memory, none failed or was denied, and the person's next message wasn't a correction.
    pub fn good_runs(&self, chat: i64, since: i64) -> Result<Vec<GoodRun>> {
        let ids: Vec<(i64, i64, Option<String>, Option<String>)> = {
            let c = self.c();
            let mut st = c.prepare(
                "SELECT r.id, r.bot_id, r.asked,
                   (SELECT u.content FROM messages u WHERE u.chat_id=r.chat_id AND u.bot_id=r.bot_id AND u.role='user'
                      AND u.id > (SELECT max(a.id) FROM messages a WHERE a.run_id=r.id) ORDER BY u.id LIMIT 1)
                 FROM runs r WHERE r.chat_id=?1 AND r.started_at>?2 AND r.status='ok' AND r.tainted=0 AND r.trigger IN ('message','routine')
                   AND NOT EXISTS (SELECT 1 FROM skills s WHERE s.run_id=r.id)
                 ORDER BY r.id DESC LIMIT 50",
            )?;
            st.query_map(params![chat, since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?
        };
        let mut out = vec![];
        for (id, bot_id, asked, next) in ids {
            let calls = self.tool_calls(id)?;
            let actions = calls.iter().filter(|c| !c.tool.starts_with("memory_")).count();
            let clean = calls.iter().all(|c| c.ok && c.decision != "deny");
            if actions >= 2 && clean && !next.as_deref().is_some_and(crate::is_correction) {
                out.push(GoodRun { id, bot_id, asked: asked.unwrap_or_default(), calls });
            }
        }
        Ok(out.into_iter().take(10).collect())
    }

    /// Queue a proposed skill; one with the name of an accepted skill is a new version of it (the old one stays in use
    /// until you accept). Skipped (None) when one by that name is already waiting, the bot already has these exact
    /// steps, you turned one down by that name in the last 90 days, or that run already taught one.
    pub fn skill_propose(&self, scope: &str, bot: i64, name: &str, description: &str, body: &str, run: Option<i64>) -> Result<Option<i64>> {
        self.skill_insert(scope, bot, name, description, body, run, "learned", "", &[])
    }

    #[allow(clippy::too_many_arguments)]
    fn skill_insert(&self, scope: &str, bot: i64, name: &str, description: &str, body: &str, run: Option<i64>, kind: &str, reason: &str, replaces: &[i64]) -> Result<Option<i64>> {
        let (name, description, body) = (name.trim(), description.trim(), body.trim());
        if name.is_empty() || description.is_empty() || body.is_empty() {
            bail!("a skill needs a name, a description and steps");
        }
        let c = self.c();
        // A skill with the name of one shared with all bots is a new version of that one.
        let shared = c.prepare("SELECT 1 FROM skills WHERE scope=?1 AND bot_id=0 AND status='active' AND lower(name)=lower(?2)")?.exists(params![scope, name])?;
        let bot = if shared { 0 } else { bot };
        let dup = c
            .prepare(
                "SELECT 1 FROM skills WHERE scope=?1 AND kind != 'retire' AND ((bot_id=?2 AND lower(name)=lower(?3) AND (status='proposed'
                   OR (status='active' AND body=?6) OR (status='dismissed' AND created_at > ?5 AND kind='learned' AND ?7='learned')))
                   OR (?4 IS NOT NULL AND run_id=?4))",
            )?
            .exists(params![scope, bot, name, run, now() - 90 * 86400, body, kind])?;
        if dup {
            return Ok(None);
        }
        // A new skill whose "when to use" reads like one the bot already has: two skills for one job make a bot pick
        // the wrong one, so the card says so (merges and new versions are about that already).
        let similar: Option<String> = if replaces.is_empty() {
            let mut st = c.prepare("SELECT name, description FROM skills WHERE scope=?1 AND bot_id IN (?2, 0) AND status='active' AND lower(name) != lower(?3)")?;
            let rows: Vec<(String, String)> = st.query_map(params![scope, bot, name], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            rows.into_iter().find(|(_, d)| alike(description, d)).map(|(n, _)| n)
        } else {
            None
        };
        let reason = match similar {
            Some(n) => format!("{} Similar to your skill “{n}”: if they do the same job, keep one.", reason.trim()),
            None => reason.to_string(),
        };
        let replaces = replaces.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        c.execute(
            "INSERT INTO skills(scope, bot_id, name, description, body, run_id, created_at, kind, reason, replaces) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![scope, bot, name, description, body, run, now(), kind, reason.trim(), replaces],
        )?;
        Ok(Some(c.last_insert_rowid()))
    }

    fn skills_where(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Skill>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT id, bot_id, name, description, body, run_id, status, created_at, kind, reason, replaces,
               (SELECT count(*) FROM skill_uses u WHERE u.skill_id = skills.id), (SELECT max(ts) FROM skill_uses u WHERE u.skill_id = skills.id)
             FROM skills WHERE {sql} ORDER BY id"
        ))?;
        let v = st
            .query_map(p, |r| {
                let replaces: String = r.get(10)?;
                Ok(Skill {
                    id: r.get(0)?,
                    bot_id: r.get(1)?,
                    name: r.get(2)?,
                    description: r.get(3)?,
                    body: r.get(4)?,
                    run_id: r.get(5)?,
                    status: r.get(6)?,
                    created_at: r.get(7)?,
                    kind: r.get(8)?,
                    reason: r.get(9)?,
                    replaces: replaces.split(',').filter_map(|x| x.trim().parse().ok()).collect(),
                    uses: r.get(11)?,
                    last_used: r.get(12)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Proposed and accepted skills in a chat (the Memory tab).
    pub fn skills(&self, scope: &str) -> Result<Vec<Skill>> {
        self.skills_where("scope=?1 AND status IN ('proposed','active')", [scope])
    }

    /// The skills one bot may use here: its own, and those shared with all bots (bot_id 0).
    pub fn skills_active(&self, scope: &str, bot: i64) -> Result<Vec<Skill>> {
        self.skills_where("scope=?1 AND bot_id IN (?2, 0) AND status='active'", params![scope, bot])
    }

    /// Give an accepted skill to one bot, or to all bots in this chat (`bot` 0). Its name must stay unique for the bots
    /// that would see it.
    pub fn skill_share(&self, scope: &str, id: i64, bot: i64) -> Result<bool> {
        let k = self.skills_where("id=?1 AND scope=?2 AND status='active'", params![id, scope])?.pop().context("no such skill")?;
        let clash = self
            .skills_where("scope=?1 AND status='active' AND id != ?2 AND lower(name)=lower(?3)", params![scope, id, k.name])?
            .into_iter()
            .any(|o| bot == 0 || o.bot_id == 0 || o.bot_id == bot);
        if clash {
            bail!("another skill here is already called \"{}\"; rename or remove one first", k.name);
        }
        Ok(self.c().execute("UPDATE skills SET bot_id=?3 WHERE id=?1 AND scope=?2", params![id, scope, bot])? > 0)
    }

    /// Accept or dismiss a proposed skill. Accepting one with the name of a skill the bot already has replaces it.
    pub fn skill_decide(&self, scope: &str, id: i64, accept: bool) -> Result<bool> {
        let proposal = self.skills_where("id=?1 AND scope=?2 AND status='proposed'", params![id, scope])?.pop();
        // A new version goes to whoever has the skill now: it may have been shared with all bots, or given to one bot,
        // since it was proposed.
        let owner = match &proposal {
            Some(p) if accept && p.kind != "retire" => self
                .skills_where("scope=?1 AND status='active' AND lower(name)=lower(?2) AND id != ?3", params![scope, p.name, id])?
                .into_iter()
                .find(|o| o.bot_id == p.bot_id || o.bot_id == 0 || p.bot_id == 0)
                .map(|o| o.bot_id),
            _ => None,
        };
        let mut c = self.c();
        let tx = c.transaction()?;
        if let Some(bot) = owner {
            tx.execute("UPDATE skills SET bot_id=?2 WHERE id=?1", params![id, bot])?;
        }
        // A merge or retirement takes the skills it names out of use (kept as removed, so they can come back).
        if accept && let Some(p) = &proposal {
            for old in &p.replaces {
                tx.execute("UPDATE skills SET status='removed' WHERE id=?1 AND scope=?2 AND status='active'", params![old, scope])?;
            }
            if p.kind == "retire" {
                let n = tx.execute("UPDATE skills SET status='removed' WHERE id=?1 AND scope=?2", params![id, scope])?;
                tx.commit()?;
                return Ok(n > 0);
            }
        }
        if accept {
            tx.execute(
                "UPDATE skills SET status='removed' WHERE status='active' AND id != ?1 AND scope=?2
                   AND (bot_id, lower(name)) = (SELECT bot_id, lower(name) FROM skills WHERE id=?1 AND scope=?2 AND status='proposed')",
                params![id, scope],
            )?;
        }
        let n = tx.execute("UPDATE skills SET status=?3 WHERE id=?1 AND scope=?2 AND status='proposed'", params![id, scope, if accept { "active" } else { "dismissed" }])?;
        tx.commit()?;
        Ok(n > 0)
    }

    /// A bot read this skill in this run.
    pub fn skill_used(&self, skill: i64, run: Option<i64>) -> Result<()> {
        self.c().execute("INSERT INTO skill_uses(skill_id, run_id, ts) VALUES (?1,?2,?3)", params![skill, run, now()])?;
        Ok(())
    }

    /// Runs since `since` that read one of this chat's active skills and then went wrong: the run failed or was stopped,
    /// an action failed or was denied, or the person's next message corrected it. The evidence for a fix.
    #[allow(clippy::type_complexity)]
    pub fn skill_troubles(&self, scope: &str, chat: i64, since: i64) -> Result<Vec<SkillTrouble>> {
        let rows: Vec<(i64, i64, String, Option<String>, Option<String>, bool, Option<String>)> = {
            let c = self.c();
            let mut st = c.prepare(
                "SELECT DISTINCT u.skill_id, r.id, r.status, r.error, r.asked, r.tainted,
                   (SELECT m.content FROM messages m WHERE m.chat_id=r.chat_id AND m.bot_id=r.bot_id AND m.role='user'
                      AND m.id > (SELECT max(a.id) FROM messages a WHERE a.run_id=r.id) ORDER BY m.id LIMIT 1)
                 FROM skill_uses u JOIN runs r ON r.id = u.run_id JOIN skills s ON s.id = u.skill_id
                 WHERE s.scope=?1 AND s.status='active' AND r.chat_id=?2 AND u.ts > ?3
                 ORDER BY r.id DESC LIMIT 20",
            )?;
            st.query_map(params![scope, chat, since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?.collect::<Result<_, _>>()?
        };
        let mut out = vec![];
        for (skill_id, run_id, status, error, asked, tainted, next) in rows {
            let calls = self.tool_calls(run_id)?;
            let mut trouble = vec![];
            if status != "ok" && status != "running" {
                trouble.push(format!("the run didn't finish: {}", error.unwrap_or_else(|| status.clone())));
            }
            for c in calls.iter().filter(|c| !c.ok || c.decision == "deny") {
                trouble.push(format!("{} [{}]: {}", c.summary, if c.decision == "deny" { "denied" } else { "failed" }, crate::tools::truncate(&c.result, 150)));
            }
            if let Some(n) = next.filter(|n| crate::is_correction(n)) {
                trouble.push(format!("they corrected it: \"{}\"", crate::tools::truncate(&n, 200)));
            }
            if trouble.is_empty() {
                continue;
            }
            let Some(skill) = self.skills_where("id=?1", [skill_id])?.pop() else { continue };
            out.push(SkillTrouble { skill, tainted, run_id, asked: asked.unwrap_or_default(), steps: calls.iter().map(|c| c.summary.clone()).collect(), trouble: trouble.join("; ") });
        }
        Ok(out)
    }

    /// Propose a fix to an active skill: replace `old` (which must appear in its steps) with `new`. The result is a new
    /// version waiting for the person, with the evidence as its reason. Patches, never rewrites; at most 4,000 chars.
    pub fn skill_patch_propose(&self, scope: &str, skill: i64, old: &str, new: &str, reason: &str) -> Result<Option<i64>> {
        let Some(k) = self.skills_where("id=?1 AND scope=?2 AND status='active'", params![skill, scope])?.pop() else { bail!("no active skill #{skill}") };
        if old.trim().is_empty() || !k.body.contains(old) {
            bail!("the text to change isn't in that skill");
        }
        let body = k.body.replacen(old, new, 1);
        // A small fix: at most 500 characters longer (taught skills can be long already), never past 12,000.
        let limit = (k.body.chars().count().max(4000) + 500).min(12_000);
        if body.chars().count() > limit || body.trim() == k.body.trim() {
            bail!("the fixed skill would be too long, or unchanged");
        }
        self.skill_insert(scope, k.bot_id, &k.name, &k.description, &body, None, "patch", reason, &[])
    }

    /// What this conversation actually did lately: the successful actions of the bot's last few runs here (not memory,
    /// skill or search bookkeeping), oldest first. What a skill saved on request may be built from.
    pub fn recent_steps(&self, bot: i64, chat: i64, runs: usize) -> Result<Vec<String>> {
        let ids: Vec<i64> = {
            let c = self.c();
            let mut st = c.prepare("SELECT id FROM runs WHERE bot_id=?1 AND chat_id=?2 AND started_at > ?3 ORDER BY id DESC LIMIT ?4")?;
            st.query_map(params![bot, chat, now() - 6 * 3600, runs as i64], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        let mut steps = vec![];
        for id in ids.into_iter().rev() {
            for c in self.tool_calls(id)? {
                let bookkeeping = c.tool.starts_with("memory_") || c.tool.starts_with("skill_") || c.tool.ends_with("_search") || c.tool == "plan_propose" || c.tool == "ask_choice";
                if c.ok && c.decision != "deny" && !bookkeeping {
                    steps.push(c.summary);
                }
            }
        }
        Ok(steps)
    }

    /// Telegram chats where this bot has skills, with them: (chat id, [(name, description)]). For the "/" menu.
    pub fn skill_menus(&self, bot: i64) -> Result<Vec<SkillMenu>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT CAST(substr(s.scope, 6) AS INTEGER), s.name, s.description FROM skills s JOIN chats ch ON ch.chat_id = CAST(substr(s.scope, 6) AS INTEGER)
             WHERE s.bot_id IN (?1, 0) AND s.status='active' AND s.scope LIKE 'chat:%' AND ch.channel='telegram' AND ch.chat_id != 0 ORDER BY s.id",
        )?;
        let rows: Vec<(i64, String, String)> = st.query_map([bot], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        let mut out: Vec<SkillMenu> = vec![];
        for (chat, name, desc) in rows {
            match out.iter_mut().find(|(c, _)| *c == chat) {
                Some((_, v)) => v.push((name, desc)),
                None => out.push((chat, vec![(name, desc)])),
            }
        }
        Ok(out)
    }

    /// A skill the person asked a bot to save from a conversation: a proposal (Save / Not now), never active by itself.
    #[allow(clippy::too_many_arguments)]
    pub fn skill_saved_propose(&self, scope: &str, bot: i64, name: &str, description: &str, body: &str, reason: &str, run: Option<i64>) -> Result<Option<i64>> {
        if body.chars().count() > 4000 {
            bail!("the skill would be too long (keep it under 4,000 characters)");
        }
        self.skill_insert(scope, bot, name, description, body, run, "saved", reason, &[])
    }

    /// Add a skill from Mimir's library: you chose it, so it's accepted straight away (a new version of one by that name).
    /// None when that bot already has exactly this one.
    pub fn skill_from_library(&self, scope: &str, bot: i64, name: &str, description: &str, body: &str) -> Result<Option<i64>> {
        let Some(id) = self.skill_insert(scope, bot, name, description, body, None, "library", "From Mimir's skill library.", &[])? else { return Ok(None) };
        self.skill_decide(scope, id, true)?;
        Ok(Some(id))
    }

    /// Propose a skill taught from a source (a page, a document, pasted text): from outside, so it always waits for the
    /// person, with where it came from and any warnings as its reason. One with an accepted skill's name is a new version.
    pub fn skill_taught_propose(&self, scope: &str, bot: i64, name: &str, description: &str, body: &str, reason: &str) -> Result<Option<i64>> {
        if body.chars().count() > 12_000 {
            bail!("the skill would be too long");
        }
        self.skill_insert(scope, bot, name, description, body, None, "taught", reason, &[])
    }

    /// Propose merging overlapping skills of one bot into one (they're retired when it's accepted).
    pub fn skill_merge_propose(&self, scope: &str, ids: &[i64], name: &str, description: &str, body: &str, reason: &str) -> Result<Option<i64>> {
        let skills: Vec<Skill> = ids.iter().filter_map(|id| self.skills_where("id=?1 AND scope=?2 AND status='active'", params![id, scope]).ok()?.pop()).collect();
        if skills.len() < 2 || skills.len() != ids.len() || skills.iter().any(|k| k.bot_id != skills[0].bot_id) {
            bail!("a merge needs two or more active skills of the same bot");
        }
        if body.chars().count() > 4000 {
            bail!("the merged skill would be too long");
        }
        // Accepting replaces the bot's active skill of that name, so it must be one of the merged ones.
        if self.skills_active(scope, skills[0].bot_id)?.iter().any(|k| k.name.eq_ignore_ascii_case(name.trim()) && !ids.contains(&k.id)) {
            bail!("another skill already has that name");
        }
        self.skill_insert(scope, skills[0].bot_id, name, description, body, None, "merge", reason, ids)
    }

    /// Propose retiring a skill nobody has used for a while (it's kept, and can come back).
    pub fn skill_retire_propose(&self, scope: &str, skill: &Skill, reason: &str) -> Result<Option<i64>> {
        let recent_no = self
            .c()
            .prepare("SELECT 1 FROM skills WHERE scope=?1 AND kind='retire' AND replaces=?2 AND (status='proposed' OR (status='dismissed' AND created_at > ?3))")?
            .exists(params![scope, skill.id.to_string(), now() - 90 * 86400])?;
        if recent_no {
            return Ok(None);
        }
        let c = self.c();
        c.execute(
            "INSERT INTO skills(scope, bot_id, name, description, body, created_at, kind, reason, replaces) VALUES (?1,?2,?3,?4,?5,?6,'retire',?7,?8)",
            params![scope, skill.bot_id, skill.name, skill.description, skill.body, now(), reason, skill.id.to_string()],
        )?;
        Ok(Some(c.last_insert_rowid()))
    }

    /// Active skills not used (nor added) in the last `days` days.
    pub fn skills_unused(&self, scope: &str, days: i64) -> Result<Vec<Skill>> {
        let cutoff = now() - days * 86400;
        Ok(self.skills_where("scope=?1 AND status='active'", [scope])?.into_iter().filter(|k| k.last_used.unwrap_or(k.created_at) < cutoff && k.created_at < cutoff).collect())
    }

    /// Facts (not preferences or tasks) in standing memory that search hasn't brought up in `days` days.
    pub fn memories_unrecalled(&self, scope: &str, days: i64) -> Result<Vec<(i64, String)>> {
        // Only ones in standing memory: demoting the rest would change nothing.
        let standing: std::collections::HashSet<i64> = self.bootstrap(scope)?.iter().map(|e| e.id).collect();
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, title FROM entries WHERE scope=?1 AND type='knowledge' AND superseded_by IS NULL AND invalid=0 AND pinned=0
               AND background=0 AND coalesce(last_recalled, created_at) < ?2 ORDER BY coalesce(last_recalled, created_at) LIMIT 10",
        )?;
        let v: Vec<(i64, String)> = st.query_map(params![scope, now() - days * 86400], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v.into_iter().filter(|(id, _)| standing.contains(id)).collect())
    }

    /// Bring back a retired or removed skill, unless the bot has another active one by that name now.
    pub fn skill_restore(&self, scope: &str, id: i64) -> Result<bool> {
        let Some(k) = self.skills_where("id=?1 AND scope=?2 AND status='removed' AND kind != 'retire'", params![id, scope])?.pop() else { return Ok(false) };
        // A shared skill clashes with any bot's skill of that name; a bot's own with its own or a shared one.
        let clash = self.skills_where("scope=?1 AND status='active' AND lower(name)=lower(?2)", params![scope, k.name])?;
        if clash.iter().any(|a| k.bot_id == 0 || a.bot_id == 0 || a.bot_id == k.bot_id) {
            bail!("the bot has another skill called \"{}\" now", k.name);
        }
        Ok(self.c().execute("UPDATE skills SET status='active' WHERE id=?1", [id])? > 0)
    }

    /// Skills taken out of use (retired, merged away or removed) in the last 180 days, to bring back.
    pub fn skills_removed(&self, scope: &str) -> Result<Vec<Skill>> {
        self.skills_where("scope=?1 AND status='removed' AND kind != 'retire' AND created_at > ?2", params![scope, now() - 180 * 86400])
    }

    pub fn skill_remove(&self, scope: &str, id: i64) -> Result<bool> {
        Ok(self.c().execute("UPDATE skills SET status='removed' WHERE id=?1 AND scope=?2 AND status='active'", params![id, scope])? > 0)
    }

    // ---------- memory suggestions ----------

    /// Validates and queues a suggestion; skips one that's already pending. Returns its id (None if skipped).
    pub fn suggestion_add(&self, scope: &str, s: &Suggestion) -> Result<Option<i64>> {
        match s.action.as_str() {
            "add" | "update" if !TYPES.contains(&s.kind.as_str()) || s.title.trim().is_empty() || s.body.trim().is_empty() => {
                bail!("add/update needs a valid type, title and body")
            }
            "update" | "forget" | "demote" if s.entry_id.is_none() => bail!("{} needs entry_id", s.action),
            "add" | "update" | "forget" | "demote" => {}
            other => bail!("unknown action {other}"),
        }
        let mut tainted = s.tainted;
        if let Some(id) = s.entry_id {
            let Some(e) = self.entries(scope)?.into_iter().find(|e| e.id == id && e.status == "active") else {
                bail!("entry #{id} isn't an active memory here");
            };
            // A new version of an unconfirmed memory stays unconfirmed.
            tainted |= e.tainted;
            // A pin is the person's call; the tidy-up doesn't argue with it.
            if s.action == "demote" && (e.pinned || e.background) {
                bail!("entry #{id} is pinned or already in the background");
            }
        }
        let c = self.c();
        // One pending suggestion per memory, and no second pending "add" with the same title:
        // conflicting proposals (e.g. forget A and forget B for a duplicate pair) could lose the fact.
        // A suggestion you dismissed doesn't come back for 90 days.
        let dup: bool = c
            .prepare(
                "SELECT 1 FROM suggestions WHERE scope=?1
                   AND (status='pending' OR (status='dismissed' AND action=?4 AND created_at > ?5))
                   AND ((?2 IS NOT NULL AND entry_id = ?2) OR (?2 IS NULL AND entry_id IS NULL AND lower(title) = lower(?3)))",
            )?
            .exists(params![scope, s.entry_id, s.title.trim(), s.action, now() - 90 * 86400])?;
        if dup {
            return Ok(None);
        }
        let text = format!("{} {}", s.title, s.body);
        // The same change in other words as one already waiting (two proposals in one tidy-up, or a bot's and the
        // tidy-up's): one card is enough.
        if matches!(s.action.as_str(), "add" | "update") {
            let mut st = c.prepare("SELECT title || ' ' || body FROM suggestions WHERE scope=?1 AND status='pending' AND action IN ('add','update') AND type=?2")?;
            let pending: Vec<String> = st.query_map(params![scope, s.kind], |r| r.get(0))?.collect::<Result<_, _>>()?;
            if pending.iter().any(|p| alike(&text, p)) {
                return Ok(None);
            }
        }
        // Like ones you dismissed lately, in other words: twice is a pattern (it isn't shown), once is said on the card.
        // Not for imports, which you pasted yourself. (Rules drawn from dismissals, or examples in the prompt, make a
        // proposer worse; filtering lookalikes is what works: Greptile, 19% → 55% of comments acted on.)
        // ponytail: the 90 days let a real need come back
        let mut reason = s.reason.trim().to_string();
        if matches!(s.action.as_str(), "add" | "update") && !reason.starts_with("Imported") {
            let mut st = c.prepare(
                "SELECT title, body, created_at FROM suggestions WHERE scope=?1 AND status='dismissed' AND action IN ('add','update') AND type=?2 AND created_at > ?3
                 ORDER BY id DESC LIMIT 200",
            )?;
            let dismissed: Vec<(String, String, i64)> = st.query_map(params![scope, s.kind, now() - 90 * 86400], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            let similar: Vec<(String, i64)> = dismissed.into_iter().filter(|(t, b, _)| alike(&text, &format!("{t} {b}"))).map(|(t, _, at)| (t, at)).collect();
            match similar.as_slice() {
                [] => {}
                [(t, at)] => {
                    let day = chrono::TimeZone::timestamp_opt(&chrono::Local, *at, 0).single().map(|d| d.format("%-d %b").to_string()).unwrap_or_default();
                    reason.push_str(&format!(" You dismissed a similar one on {day} (“{t}”)."));
                }
                _ => return Ok(None),
            }
        }
        c.execute(
            "INSERT INTO suggestions(scope, action, entry_id, type, title, body, reason, created_at, tainted) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![scope, s.action, s.entry_id, s.kind, s.title.trim(), s.body.trim(), reason, now(), tainted],
        )?;
        Ok(Some(c.last_insert_rowid()))
    }

    pub fn suggestions(&self, scope: &str) -> Result<Vec<Suggestion>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, action, entry_id, type, title, body, reason, check_note, tainted FROM suggestions WHERE scope=?1 AND status='pending' ORDER BY id")?;
        let v = st
            .query_map([scope], |r| {
                Ok(Suggestion {
                    id: r.get(0)?,
                    action: r.get(1)?,
                    entry_id: r.get(2)?,
                    kind: r.get(3)?,
                    title: r.get(4)?,
                    body: r.get(5)?,
                    reason: r.get(6)?,
                    check: r.get(7)?,
                    tainted: r.get(8)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// How many times a tool ran successfully in a chat in the last 24 hours (a daily cap on sending).
    pub fn sent_today(&self, chat: i64, tool: &str) -> Result<i64> {
        Ok(self.c().query_row("SELECT count(*) FROM tool_calls WHERE chat_id=?1 AND tool=?2 AND ok=1 AND ts > ?3", params![chat, tool, now() - 86400], |r| r.get(0))?)
    }

    pub fn suggestion_set_tainted(&self, scope: &str, id: i64, tainted: bool) -> Result<()> {
        self.c().execute("UPDATE suggestions SET tainted=?3 WHERE id=?1 AND scope=?2", params![id, scope, tainted])?;
        Ok(())
    }

    pub fn suggestion_set_check(&self, scope: &str, id: i64, note: &str) -> Result<()> {
        self.c().execute("UPDATE suggestions SET check_note=?3 WHERE id=?1 AND scope=?2", params![id, scope, note])?;
        Ok(())
    }

    pub fn case_add(&self, bot: i64, chat: i64, asked: &str, bad: &str, correction: &str) -> Result<()> {
        let t = |s: &str| crate::tools::truncate(s, 1500);
        self.c().execute(
            "INSERT INTO cases(bot_id, chat_id, asked, bad, correction, ts) VALUES (?1,?2,?3,?4,?5,?6)",
            params![bot, chat, t(asked), t(bad), t(correction), now()],
        )?;
        Ok(())
    }

    /// Up to `limit` past corrections in a chat that share the most words with `text` (newest wins ties);
    /// ones sharing none are left out. (asked, bad reply, correction)
    pub fn cases_like(&self, chat: i64, text: &str, limit: usize) -> Result<Vec<(String, String, String)>> {
        let words = |s: &str| -> std::collections::HashSet<String> { s.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() > 3).map(str::to_lowercase).collect() };
        let want = words(text);
        let c = self.c();
        let mut st = c.prepare("SELECT asked, bad, correction FROM cases WHERE chat_id=?1 ORDER BY id DESC LIMIT 200")?;
        let all: Vec<(String, String, String)> = st.query_map([chat], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        let mut scored: Vec<(usize, (String, String, String))> =
            all.into_iter().map(|k| (words(&format!("{} {} {}", k.0, k.1, k.2)).intersection(&want).count(), k)).filter(|(n, _)| *n > 0).collect();
        scored.sort_by_key(|k| std::cmp::Reverse(k.0));
        Ok(scored.into_iter().take(limit).map(|(_, k)| k).collect())
    }

    /// Accept (apply to memory) or dismiss. A suggestion whose target changed since is dismissed, not applied.
    pub fn suggestion_decide(&self, scope: &str, id: i64, accept: bool) -> Result<String> {
        let Some(s) = self.suggestions(scope)?.into_iter().find(|s| s.id == id) else { bail!("no pending suggestion #{id}") };
        let applied = if !accept {
            Ok("dismissed".to_string())
        } else {
            // ponytail: import proposals are told apart by their reason; a column if more origins appear
            let (src, mut prov) = if s.reason.starts_with("Imported") {
                ("imported, approved by you", Prov::by("import"))
            } else if s.reason.starts_with(BOT_PROPOSED) {
                ("proposed by a bot, approved by you", Prov::by("bot"))
            } else {
                ("tidy-up, approved by you", Prov::by("tidy"))
            };
            // Accepting text drawn from outside content doesn't make it the person's own: it stays unconfirmed
            // until they confirm it ("Looks right" in Memory).
            prov.tainted = s.tainted;
            match s.action.as_str() {
                "add" => self.store_by(scope, &s.kind, &s.title, &s.body, src, None, &prov).map(|n| format!("added #{n}")),
                "update" => self.store_by(scope, &s.kind, &s.title, &s.body, src, s.entry_id, &prov).map(|n| format!("#{} replaced by #{n}", s.entry_id.unwrap_or(0))),
                // The target may have changed since (pinned, corrected, forgotten): then nothing happens and the
                // suggestion is dismissed. A pin is the person's call, so a demote never undoes one.
                "demote" => {
                    let n = self.c().execute(
                        "UPDATE entries SET background=1 WHERE id=?1 AND scope=?2 AND pinned=0 AND background=0 AND superseded_by IS NULL AND invalid=0",
                        params![s.entry_id.unwrap_or(0), scope],
                    )?;
                    if n == 0 { Err(anyhow::anyhow!("#{} changed since: it's pinned, replaced or gone", s.entry_id.unwrap_or(0))) } else { Ok(format!("#{} moved to background", s.entry_id.unwrap_or(0))) }
                }
                _ => {
                    let n = self.c().execute("UPDATE entries SET invalid=1 WHERE id=?1 AND scope=?2 AND superseded_by IS NULL AND invalid=0", params![s.entry_id.unwrap_or(0), scope])?;
                    if n == 0 { Err(anyhow::anyhow!("#{} changed since: it's replaced or already gone", s.entry_id.unwrap_or(0))) } else { Ok(format!("forgot #{}", s.entry_id.unwrap_or(0))) }
                }
            }
        };
        let status = if accept && applied.is_ok() { "accepted" } else { "dismissed" };
        self.c().execute("UPDATE suggestions SET status=?2 WHERE id=?1", params![id, status])?;
        applied
    }

    // ---------- policy rules & approvals ----------

    /// User override for a tool, if any (None = use the built-in default).
    pub fn rule(&self, tool: &str) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT action FROM policy_rules WHERE tool=?1", [tool], |r| r.get(0)).optional()?)
    }

    /// action = allow | ask | deny, or "default" to remove the override.
    pub fn set_rule(&self, tool: &str, action: &str) -> Result<()> {
        if action == "default" {
            self.c().execute("DELETE FROM policy_rules WHERE tool=?1", [tool])?;
        } else {
            self.c().execute(
                "INSERT INTO policy_rules(tool, action) VALUES (?1, ?2) ON CONFLICT(tool) DO UPDATE SET action = excluded.action",
                params![tool, action],
            )?;
        }
        Ok(())
    }

    /// A rule for one tool on one website (e.g. open github.com in the browser), checked before the tool's rule.
    pub fn site_rule(&self, tool: &str, host: &str) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT action FROM site_rules WHERE tool=?1 AND host=?2", [tool, host], |r| r.get(0)).optional()?)
    }

    /// action = allow | ask | deny, or "default" to remove it.
    pub fn set_site_rule(&self, tool: &str, host: &str, action: &str) -> Result<()> {
        if action == "default" {
            self.c().execute("DELETE FROM site_rules WHERE tool=?1 AND host=?2", [tool, host])?;
        } else {
            self.c().execute(
                "INSERT INTO site_rules(tool, host, action) VALUES (?1,?2,?3) ON CONFLICT(tool, host) DO UPDATE SET action = excluded.action",
                params![tool, host, action],
            )?;
        }
        Ok(())
    }

    pub fn site_rules(&self) -> Result<Vec<(String, String, String)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT tool, host, action FROM site_rules ORDER BY host, tool")?;
        let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// A new incognito chat starts clean: the last one's "read a web page" flag goes.
    pub fn incognito_reset(&self, bot: i64) -> Result<()> {
        self.set_session(bot, crate::INCOGNITO_CHAT, None)?;
        self.incognito_forget_approvals(bot)
    }

    /// An incognito turn's approvals (what it wanted to do) are deleted as soon as the turn ends.
    pub fn incognito_forget_approvals(&self, bot: i64) -> Result<()> {
        self.c().execute("DELETE FROM approvals WHERE bot_id=?1 AND chat_id=?2", params![bot, crate::INCOGNITO_CHAT])?;
        Ok(())
    }

    pub fn approval_set_run(&self, id: i64, run: i64) -> Result<()> {
        self.c().execute("UPDATE approvals SET run_id=?2 WHERE id=?1", [id, run])?;
        Ok(())
    }

    /// "Allow all like this": the approval's tool (and site) needs no more OKs for the rest of its run. Call it
    /// before approving, so the plan is in place when the waiting tool goes on. Only for a pending plain ask from a
    /// run that's still going, in `chat` if given; returns false otherwise (and nothing changes).
    pub fn approval_allow_rest_of_run(&self, id: i64, chat: Option<i64>) -> Result<bool> {
        let row: Option<(i64, String, Option<String>)> = self
            .c()
            .query_row(
                "SELECT a.run_id, a.tool, a.site FROM approvals a JOIN runs r ON r.id = a.run_id
                 WHERE a.id=?1 AND a.status='pending' AND a.kind='rule' AND a.parked_args IS NULL AND r.status='running'
                   AND (?2 IS NULL OR a.chat_id=?2)",
                params![id, chat],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((run, tool, site)) = row else { return Ok(false) };
        let key = format!("plan:{run}");
        let mut plan: Vec<(String, Option<String>)> = self.setting(&key)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        if !plan.contains(&(tool.clone(), site.clone())) {
            plan.push((tool, site));
        }
        self.set_setting(&key, &serde_json::to_string(&plan)?)?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn approval_create(&self, bot: i64, chat: i64, tool: &str, summary: &str, ttl_secs: i64, kind: &str, site: Option<&str>) -> Result<i64> {
        let c = self.c();
        let t = now();
        c.execute(
            "INSERT INTO approvals(bot_id, chat_id, tool, summary, created_at, expires_at, kind, site) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![bot, chat, tool, summary, t, t + ttl_secs, kind, site],
        )?;
        Ok(c.last_insert_rowid())
    }

    fn expire_stale(c: &Connection) -> Result<()> {
        c.execute("UPDATE approvals SET status='expired' WHERE status='pending' AND expires_at <= ?1", [now()])?;
        Ok(())
    }

    /// Current status; flips overdue pending approvals to 'expired' first.
    pub fn approval_status(&self, id: i64) -> Result<String> {
        let c = self.c();
        Self::expire_stale(&c)?;
        Ok(c.query_row("SELECT status FROM approvals WHERE id=?1", [id], |r| r.get(0))?)
    }

    /// Decide a still-pending approval. `chat` (when given) must match, so one chat can't approve another's request.
    pub fn approval_decide(&self, id: i64, approve: bool, chat: Option<i64>) -> Result<Option<Approval>> {
        let c = self.c();
        Self::expire_stale(&c)?;
        let n = c.execute(
            "UPDATE approvals SET status=?2 WHERE id=?1 AND status='pending' AND (?3 IS NULL OR chat_id=?3)",
            params![id, if approve { "approved" } else { "denied" }, chat],
        )?;
        if n == 0 {
            return Ok(None);
        }
        drop(c);
        Ok(self.approvals_where("id = ?1", id)?.pop())
    }

    fn approvals_where(&self, sql: &str, p: i64) -> Result<Vec<Approval>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT id, chat_id, tool, summary, status, created_at, expires_at, tg_message_id, bot_id, kind, site, parked_args IS NOT NULL,
             (SELECT COUNT(*) FROM approvals p WHERE p.tool = approvals.tool AND p.site IS approvals.site AND p.kind = 'rule' AND p.status = 'approved' AND p.id < approvals.id)
             FROM approvals WHERE {sql} ORDER BY id"
        ))?;
        let v = st
            .query_map([p], |r| {
                Ok(Approval {
                    id: r.get(0)?,
                    chat_id: r.get(1)?,
                    tool: r.get(2)?,
                    summary: r.get(3)?,
                    status: r.get(4)?,
                    created_at: r.get(5)?,
                    expires_at: r.get(6)?,
                    tg_message_id: r.get(7)?,
                    bot_id: r.get(8)?,
                    kind: r.get(9)?,
                    site: r.get(10)?,
                    parked: r.get(11)?,
                    allowed_before: r.get(12)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Park an approval for an unattended run: the call's arguments are kept so it can run once approved.
    /// Keep a call to run once approved, and whether the run that asked had read outside content.
    pub fn approval_park(&self, id: i64, args: &str, tainted: bool) -> Result<()> {
        self.c().execute("UPDATE approvals SET parked_args=?2, tainted=?3 WHERE id=?1", params![id, args, tainted])?;
        Ok(())
    }

    /// A parked call's arguments, and whether the run that asked had read outside content.
    pub fn approval_args(&self, id: i64) -> Result<Option<(String, bool)>> {
        Ok(self
            .c()
            .query_row("SELECT parked_args, tainted FROM approvals WHERE id=?1", [id], |r| Ok(r.get::<_, Option<String>>(0)?.map(|a| (a, r.get::<_, bool>(1).unwrap_or(true)))))
            .optional()?
            .flatten())
    }

    /// Approvals that ran out unanswered since `since` (not yet dismissed from the inbox).
    pub fn approvals_missed(&self, since: i64) -> Result<Vec<Approval>> {
        Self::expire_stale(&self.c())?;
        self.approvals_where("status = 'expired' AND chat_id != -1 AND created_at > ?1 AND id NOT IN (SELECT ref FROM inbox_dismissed WHERE kind='approval')", since)
    }

    /// Failed runs since `since` (not yet dismissed from the inbox), newest first: one item per routine and cause (a
    /// routine failing every 15 minutes is one item, with how often), and results that couldn't be sent.
    pub fn runs_failed(&self, since: i64) -> Result<Vec<Run>> {
        let gone: std::collections::HashSet<i64> = {
            let c = self.c();
            let mut st = c.prepare("SELECT ref FROM inbox_dismissed WHERE kind='run'")?;
            st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        let failed = |r: &Run| r.status == "missed" || r.status == "error" && r.error.as_deref() != Some("Stopped by you") || r.delivered.as_deref().is_some_and(|d| d.starts_with(NOT_SENT));
        let key = |r: &Run| match r.routine_id {
            Some(id) if r.status == "error" => format!("{id}:{:?}", crate::cause(r.error.as_deref().unwrap_or(""))),
            Some(id) if r.status == "missed" => format!("{id}:missed"),
            Some(id) => format!("{id}:delivery"),
            None => format!("run:{}", r.id),
        };
        let mut out: Vec<Run> = vec![];
        let mut seen: std::collections::HashMap<String, usize> = Default::default();
        // Newest first: the first of each kind stands for the rest. Dismissing it dismisses them all (a new one shows again).
        // A rehearsal's failure was seen where it was run.
        for r in self.runs(None, 300)?.into_iter().filter(|r| r.started_at > since && r.trigger != "rehearsal" && failed(r)) {
            match seen.get(&key(&r)) {
                Some(&i) => out[i].repeats += i64::from(!gone.contains(&r.id)),
                None => {
                    seen.insert(key(&r), out.len());
                    out.push(r);
                }
            }
        }
        Ok(out.into_iter().filter(|r| !gone.contains(&r.id)).collect())
    }

    /// Pending memory suggestions per chat.
    /// The last `n` weeks up to `now`, oldest first.
    pub fn weeks(&self, chat: i64, n: i64, now: i64) -> Result<Vec<Week>> {
        let c = self.c();
        let scope = format!("chat:{chat}");
        let count = |sql: &str, p: &dyn rusqlite::ToSql, from: i64, to: i64| -> Result<i64> { Ok(c.query_row(sql, params![p, from, to], |r| r.get(0))?) };
        let signals = "SELECT count(*) FROM signals WHERE chat_id=?1 AND ts>=?2 AND ts<?3 AND kind=";
        let sugg = "SELECT count(*) FROM suggestions WHERE scope=?1 AND created_at>=?2 AND created_at<?3 AND status=";
        (0..n)
            .rev()
            .map(|i| {
                let (from, to) = (now - (i + 1) * 7 * 86400, now - i * 7 * 86400);
                Ok(Week {
                    start: from,
                    turns: count("SELECT count(*) FROM messages WHERE chat_id=?1 AND ts>=?2 AND ts<?3 AND role='user'", &chat, from, to)?,
                    corrections: count(&format!("{signals}'correction'"), &chat, from, to)?,
                    stops: count(&format!("{signals}'stopped'"), &chat, from, to)?,
                    denials: count(&format!("{signals}'denied'"), &chat, from, to)?,
                    tool_calls: count("SELECT count(*) FROM tool_calls WHERE chat_id=?1 AND ts>=?2 AND ts<?3", &chat, from, to)?,
                    tool_errors: count("SELECT count(*) FROM tool_calls WHERE chat_id=?1 AND ts>=?2 AND ts<?3 AND ok=0", &chat, from, to)?,
                    accepted: count(&format!("{sugg}'accepted'"), &scope, from, to)?,
                    dismissed: count(&format!("{sugg}'dismissed'"), &scope, from, to)?,
                    repeats: count(&format!("{signals}'repeated'"), &chat, from, to)?,
                    undone: count("SELECT count(*) FROM entries WHERE scope=?1 AND undone_at>=?2 AND undone_at<?3 AND origin IN ('bot','tidy','import')", &scope, from, to)?,
                    forgot: count(
                        &format!("{signals}'correction' AND lower(detail) LIKE '%they replied: %told you%'"),
                        &chat,
                        from,
                        to,
                    )?,
                })
            })
            .collect()
    }

    /// Memories saved after reading outside content and not confirmed yet, per chat: the bots don't use them until then.
    pub fn unconfirmed_counts(&self) -> Result<Vec<(i64, i64)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT scope, count(*) FROM entries WHERE tainted=1 AND invalid=0 AND superseded_by IS NULL GROUP BY scope")?;
        let v: Vec<(String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v.into_iter().filter_map(|(s, n)| Some((s.strip_prefix("chat:")?.parse().ok()?, n))).collect())
    }

    pub fn suggestion_counts(&self) -> Result<Vec<(i64, i64)>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT scope, count(*) FROM (SELECT scope FROM suggestions WHERE status='pending' UNION ALL SELECT scope FROM skills WHERE status='proposed') GROUP BY scope",
        )?;
        let v: Vec<(String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v.into_iter().filter_map(|(s, n)| Some((s.strip_prefix("chat:")?.parse().ok()?, n))).collect())
    }

    pub fn inbox_dismiss(&self, kind: &str, id: i64) -> Result<()> {
        self.c().execute("INSERT OR IGNORE INTO inbox_dismissed(kind, ref) VALUES (?1,?2)", params![kind, id])?;
        Ok(())
    }

    pub fn approvals_pending(&self) -> Result<Vec<Approval>> {
        Self::expire_stale(&self.c())?;
        self.approvals_where("status = 'pending' AND ?1 = ?1", 0)
    }

    /// This bot's pending Telegram approvals that haven't been sent as a card yet.
    pub fn approvals_to_notify(&self, bot: i64) -> Result<Vec<Approval>> {
        self.approvals_where("status = 'pending' AND notified = 0 AND chat_id NOT IN (0, -1) AND bot_id = ?1", bot)
    }

    /// The card was sent (`card_ref`: its message, if the channel said).
    pub fn approval_mark_notified(&self, id: i64, card_ref: Option<&str>) -> Result<()> {
        self.c().execute("UPDATE approvals SET notified=1, card_ref=?2 WHERE id=?1", params![id, card_ref])?;
        Ok(())
    }

    /// This bot's cards whose approval was decided elsewhere (app, tray, CLI) or ran out: (approval, card message).
    pub fn approvals_to_close(&self, bot: i64) -> Result<Vec<(Approval, String)>> {
        Self::expire_stale(&self.c())?;
        let list = self.approvals_where("status != 'pending' AND notified = 1 AND card_ref IS NOT NULL AND bot_id = ?1", bot)?;
        let c = self.c();
        let mut out = vec![];
        for a in list {
            let card: String = c.query_row("SELECT card_ref FROM approvals WHERE id=?1", [a.id], |r| r.get(0))?;
            out.push((a, card));
        }
        Ok(out)
    }

    /// The card now shows the outcome; don't edit it again.
    pub fn approval_mark_closed(&self, id: i64) -> Result<()> {
        self.c().execute("UPDATE approvals SET notified=2 WHERE id=?1", [id])?;
        Ok(())
    }

    // ---------- chats & messages ----------

    pub fn is_paired(&self, chat: i64) -> Result<bool> {
        Ok(self.c().query_row("SELECT 1 FROM chats WHERE chat_id=?1", [chat], |_| Ok(())).optional()?.is_some())
    }

    pub fn pair(&self, chat: i64, title: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO chats(chat_id, title) VALUES (?1, ?2) ON CONFLICT(chat_id) DO UPDATE SET title = excluded.title",
            params![chat, title],
        )?;
        Ok(())
    }

    /// Paired chats with their live memory counts (for UIs).
    pub fn chats(&self) -> Result<Vec<Chat>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT c.chat_id, c.title,
               (SELECT count(*) FROM entries e WHERE e.scope = 'chat:' || c.chat_id AND e.superseded_by IS NULL AND e.invalid = 0),
               CASE WHEN c.chat_id = 0 THEN 'local' ELSE c.channel END, c.guest
             FROM chats c ORDER BY c.chat_id",
        )?;
        let v = st
            .query_map([], |r| Ok(Chat { chat_id: r.get(0)?, title: r.get(1)?, memories: r.get(2)?, channel: r.get(3)?, guest: r.get(4)? }))?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// The chat id for an inbound message, if that chat was paired. Telegram ids are used as-is.
    pub fn chat_lookup(&self, channel: &str, external: &str) -> Result<Option<i64>> {
        if channel == "telegram" {
            let id: i64 = external.parse()?;
            return Ok(self.is_paired(id)?.then_some(id));
        }
        Ok(self
            .c()
            .query_row("SELECT chat_id FROM chats WHERE channel=?1 AND external=?2", params![channel, external], |r| r.get(0))
            .optional()?)
    }

    /// Paired chats on one app (not Telegram): (its id there, a direct chat?).
    pub fn chats_on(&self, channel: &str) -> Result<Vec<(String, bool)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT external, dm FROM chats WHERE channel=?1 AND external IS NOT NULL")?;
        let v = st.query_map([channel], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Pair a chat from any channel; returns its chat id. Non-Telegram chats get ids from a range
    /// (9e15+) above Telegram's, so no external id can collide with another.
    /// The person (their id in the messaging app) who paired this chat; "" for chats paired before it was kept.
    pub fn chat_owner(&self, chat: i64) -> Result<String> {
        Ok(self.c().query_row("SELECT owner FROM chats WHERE chat_id=?1", [chat], |r| r.get(0)).optional()?.unwrap_or_default())
    }

    /// At startup: an incognito chat that ended in a crash leaves nothing behind either. (Not on every open: the
    /// tools server each turn opens the database, and a live incognito chat may be waiting on an approval.)
    pub fn incognito_leftovers(&self) -> Result<()> {
        self.c().execute("DELETE FROM approvals WHERE chat_id = -1", [])?;
        self.c().execute("DELETE FROM conv_taint WHERE chat_id = -1", [])?;
        Ok(())
    }

    pub fn chat_set_dm(&self, chat: i64, dm: bool) -> Result<()> {
        self.c().execute("UPDATE chats SET dm=?2 WHERE chat_id=?1", params![chat, dm])?;
        Ok(())
    }

    /// A conversation only you are in: the app's own chats, or a direct chat in a messaging app.
    pub fn chat_private(&self, chat: i64) -> bool {
        chat == crate::host::LOCAL_CHAT
            || chat == crate::INCOGNITO_CHAT
            // Telegram's private chats have positive ids (a group's are negative); others say so when paired.
            || self.c().query_row("SELECT dm OR (coalesce(channel, 'telegram')='telegram' AND chat_id > 0 AND chat_id < 9000000000000000) FROM chats WHERE chat_id=?1", [chat], |r| r.get::<_, bool>(0)).unwrap_or(false)
    }

    /// A chat someone else paired: see `Chat::guest`. The app's own chats are always yours.
    pub fn chat_guest(&self, chat: i64) -> bool {
        chat != crate::host::LOCAL_CHAT && self.c().query_row("SELECT guest FROM chats WHERE chat_id=?1", [chat], |r| r.get::<_, bool>(0)).unwrap_or(false)
    }

    pub fn chat_set_guest(&self, chat: i64, guest: bool) -> Result<bool> {
        Ok(chat != crate::host::LOCAL_CHAT && self.c().execute("UPDATE chats SET guest=?2 WHERE chat_id=?1", params![chat, guest])? > 0)
    }

    /// Whether a person pairing a chat on this app is someone else: you are whoever paired your chats there before
    /// (the first person to pair on an app is you). Telegram chats from before channels were kept count as Telegram.
    pub fn pairs_as_guest(&self, channel: &str, person: &str) -> Result<bool> {
        let c = self.c();
        let mut st = c.prepare("SELECT DISTINCT owner FROM chats WHERE coalesce(channel, 'telegram')=?1 AND guest=0 AND owner != '' AND chat_id != 0")?;
        let you: Vec<String> = st.query_map([channel], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(!you.is_empty() && !you.iter().any(|o| o == person))
    }

    pub fn chat_set_owner(&self, chat: i64, owner: &str) -> Result<()> {
        self.c().execute("UPDATE chats SET owner=?2 WHERE chat_id=?1", params![chat, owner])?;
        Ok(())
    }

    pub fn pair_external(&self, channel: &str, external: &str, title: &str) -> Result<i64> {
        if channel == "telegram" {
            let id: i64 = external.parse()?;
            self.pair(id, title)?;
            return Ok(id);
        }
        if let Some(id) = self.chat_lookup(channel, external)? {
            self.c().execute("UPDATE chats SET title=?2 WHERE chat_id=?1", params![id, title])?;
            return Ok(id);
        }
        const BASE: i64 = 9_000_000_000_000_000;
        let c = self.c();
        let id: i64 = c.query_row("SELECT max(?1, coalesce(max(chat_id) + 1, ?1)) FROM chats WHERE chat_id >= ?1", [BASE], |r| r.get(0))?;
        c.execute("INSERT INTO chats(chat_id, title, channel, external) VALUES (?1,?2,?3,?4)", params![id, title, channel, external])?;
        Ok(id)
    }

    /// Where to send messages for a chat: (channel, external id). None for the local chat.
    pub fn chat_address(&self, chat: i64) -> Result<Option<(String, String)>> {
        if chat == 0 || chat == crate::INCOGNITO_CHAT {
            return Ok(None);
        }
        let row: Option<(String, Option<String>)> =
            self.c().query_row("SELECT channel, external FROM chats WHERE chat_id=?1", [chat], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        // Older Telegram chats use the Telegram id as chat id (no `external`). A chat that isn't paired has no address.
        Ok(row.map(|(ch, ext)| (ch, ext.unwrap_or_else(|| chat.to_string()))))
    }

    pub fn unpair(&self, chat: i64) -> Result<()> {
        // Where catch-up left off goes too: paired again later, the chat starts afresh (not with what came between).
        self.c().execute(
            "DELETE FROM settings WHERE key LIKE 'seen:%' AND key LIKE (SELECT 'seen:%:' || channel || ':' || external FROM chats WHERE chat_id=?1)",
            [chat],
        )?;
        self.c().execute("DELETE FROM chats WHERE chat_id=?1", [chat])?;
        Ok(())
    }

    /// CLI provider session (claude/codex) for a bot's conversation with a chat.
    /// Every conversation a bot has open (to delete the AI CLIs' copies when it goes).
    pub fn sessions_of(&self, bot: i64) -> Result<Vec<String>> {
        let c = self.c();
        let mut st = c.prepare("SELECT session FROM sessions WHERE bot_id=?1")?;
        let v = st.query_map([bot], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn session(&self, bot: i64, chat: i64) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT session FROM sessions WHERE bot_id=?1 AND chat_id=?2", [bot, chat], |r| r.get(0)).optional()?)
    }

    pub fn set_session(&self, bot: i64, chat: i64, sid: Option<&str>) -> Result<()> {
        let c = self.c();
        match sid {
            Some(s) => c.execute(
                "INSERT INTO sessions(bot_id, chat_id, session) VALUES (?1,?2,?3) ON CONFLICT(bot_id, chat_id) DO UPDATE SET session = excluded.session",
                params![bot, chat, s],
            )?,
            // A fresh conversation also leaves behind whatever untrusted content the old one read; accounts that are
            // handed the history again (API keys) start from here too.
            None => {
                let last: i64 = c.query_row("SELECT coalesce(max(id), 0) FROM messages WHERE bot_id=?1 AND chat_id=?2", [bot, chat], |r| r.get(0))?;
                c.execute(
                    "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![format!("fresh:{bot}:{chat}"), last.to_string()],
                )?;
                c.execute("DELETE FROM sessions WHERE bot_id=?1 AND chat_id=?2", [bot, chat])? + c.execute("DELETE FROM conv_taint WHERE bot_id=?1 AND chat_id=?2", [bot, chat])?
            }
        };
        Ok(())
    }

    // ---------- taint (untrusted content read) ----------

    /// Has this run read untrusted content? (A tools server started mid-run must not start clean.)
    pub fn run_tainted(&self, run: i64) -> Result<bool> {
        Ok(self.c().query_row("SELECT tainted FROM runs WHERE id=?1", [run], |r| r.get::<_, bool>(0)).optional()?.unwrap_or(false))
    }

    pub fn run_taint(&self, run: i64) -> Result<()> {
        self.c().execute("UPDATE runs SET tainted=1 WHERE id=?1", [run])?;
        Ok(())
    }

    pub fn conv_taint(&self, bot: i64, chat: i64) -> Result<()> {
        self.c().execute("INSERT OR IGNORE INTO conv_taint(bot_id, chat_id, since) VALUES (?1,?2,?3)", params![bot, chat, now()])?;
        Ok(())
    }

    pub fn conv_tainted(&self, bot: i64, chat: i64) -> Result<bool> {
        Ok(self.c().prepare("SELECT 1 FROM conv_taint WHERE bot_id=?1 AND chat_id=?2")?.exists([bot, chat])?)
    }

    /// Whether `bot`'s latest handed-over task in this chat read untrusted content.
    pub fn delegated_tainted(&self, bot: i64, chat: i64) -> Result<bool> {
        Ok(self
            .c()
            .query_row("SELECT tainted FROM runs WHERE bot_id=?1 AND chat_id=?2 AND trigger='delegated' ORDER BY id DESC LIMIT 1", [bot, chat], |r| r.get::<_, bool>(0))
            .optional()?
            .unwrap_or(false))
    }

    pub fn log(&self, bot: i64, chat: i64, role: &str, content: &str, run: Option<i64>) -> Result<()> {
        self.c().execute(
            "INSERT INTO messages(bot_id, chat_id, role, content, ts, run_id) VALUES (?1,?2,?3,?4,?5,?6)",
            params![bot, chat, role, content, now(), run],
        )?;
        Ok(())
    }

    /// Earlier messages with this chat (all bots) that match `query`, best first: an excerpt around the match.
    /// `tainted`: a bot message from a run that read outside content, or from no known run. The person's words never are.
    /// `range`: (from, to) in unix seconds, inclusive.
    pub fn chat_search(&self, chat: i64, query: &str, exclude_run: Option<i64>, range: (i64, i64), limit: usize) -> Result<Vec<ChatHit>> {
        let Some(q) = fts_query(query) else { return Ok(vec![]) };
        let c = self.c();
        let mut st = c.prepare(
            "SELECT m.ts, m.bot_id, m.role, snippet(messages_fts, 0, '', '', '…', 40), (m.role = 'assistant' AND coalesce(r.tainted, 1)) OR (m.role = 'user' AND m.content LIKE '%<document name=%'),
               m.role = 'user' AND m.content LIKE '[Scheduled%'
             FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid LEFT JOIN runs r ON r.id = m.run_id
             WHERE messages_fts MATCH ?1 AND m.chat_id = ?2 AND (?3 IS NULL OR m.run_id IS NOT ?3) AND m.ts BETWEEN ?5 AND ?6
             ORDER BY bm25(messages_fts) LIMIT ?4",
        )?;
        let v = st
            .query_map(params![q, chat, exclude_run, limit as i64, range.0, range.1], |r| Ok(ChatHit { ts: r.get(0)?, bot_id: r.get(1)?, role: r.get(2)?, excerpt: r.get(3)?, tainted: r.get(4)?, routine: r.get(5)? }))?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// A bot's last `n` messages with a chat, oldest first (provider history).
    pub fn recent(&self, bot: i64, chat: i64, n: usize) -> Result<Vec<(String, String)>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT role, content FROM messages WHERE bot_id=?1 AND chat_id=?2
               AND id > coalesce((SELECT CAST(value AS INTEGER) FROM settings WHERE key='fresh:' || ?1 || ':' || ?2), 0)
             ORDER BY id DESC LIMIT ?3",
        )?;
        let mut v: Vec<(String, String)> = st.query_map(params![bot, chat, n as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        v.reverse();
        Ok(v)
    }

    /// All bots' messages with a chat, oldest first (UI).
    /// Whether the last `n` messages a bot would be given as history hold outside content: a file's or forward's
    /// text, or a reply from a run that had read some.
    pub fn recent_outside(&self, bot: i64, chat: i64, n: usize) -> Result<bool> {
        Ok(self.c().query_row(
            "SELECT EXISTS(SELECT 1 FROM (SELECT role, content, run_id FROM messages WHERE bot_id=?1 AND chat_id=?2
                 AND id > coalesce((SELECT CAST(value AS INTEGER) FROM settings WHERE key='fresh:' || ?1 || ':' || ?2), 0)
                 ORDER BY id DESC LIMIT ?3) m
               WHERE (m.role='user' AND m.content LIKE '%<document name=%')
                  OR (m.role='assistant' AND m.run_id IN (SELECT id FROM runs WHERE tainted=1)))",
            params![bot, chat, n as i64],
            |r| r.get(0),
        )?)
    }

    /// The bot's latest message with a chat, if it was sent outside the chat's CLI session: a routine's result
    /// ("routine result"), or one from no run (digest, weekly review, a note after an approval: "message").
    pub fn last_outside_session(&self, bot: i64, chat: i64) -> Result<Option<(&'static str, String)>> {
        let last: Option<(String, String, Option<String>)> = self
            .c()
            .query_row(
                "SELECT m.role, m.content, CASE WHEN m.run_id IS NULL THEN NULL ELSE coalesce(r.trigger, '') END
                   FROM messages m LEFT JOIN runs r ON r.id = m.run_id WHERE m.bot_id=?1 AND m.chat_id=?2
                   AND m.id > coalesce((SELECT CAST(value AS INTEGER) FROM settings WHERE key='fresh:' || ?1 || ':' || ?2), 0)
                 ORDER BY m.id DESC LIMIT 1",
                [bot, chat],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match last {
            Some((role, text, trigger)) if role == "assistant" => match trigger.as_deref() {
                None => Some(("message", text)),
                Some("message") => None,
                Some(_) => Some(("routine result", text)),
            },
            _ => None,
        })
    }

    pub fn messages(&self, chat: i64, n: usize) -> Result<Vec<Message>> {
        self.messages_of(chat, None, n)
    }

    /// The last `n` messages of a chat, oldest first; with `bot`, only that bot's conversation in it (what you said to
    /// it and what it said), as when you open a bot from the sidebar.
    pub fn messages_of(&self, chat: i64, bot: Option<i64>, n: usize) -> Result<Vec<Message>> {
        let c = self.c();
        let mut st = c.prepare("SELECT bot_id, role, content, ts, run_id, id FROM messages WHERE chat_id=?1 AND (?3 IS NULL OR bot_id=?3) ORDER BY id DESC LIMIT ?2")?;
        let mut v: Vec<(Message, Option<i64>)> = st
            .query_map(params![chat, n as i64, bot], |r| Ok((Message { id: r.get(5)?, bot_id: r.get(0)?, role: r.get(1)?, content: r.get(2)?, ts: r.get(3)?, actions: vec![], run_id: None }, r.get(4)?)))?
            .collect::<Result<_, _>>()?;
        v.reverse();
        // One query for every reply's actions.
        let first = v.iter().filter_map(|(_, r)| *r).min().unwrap_or(i64::MAX);
        let mut acts = c.prepare("SELECT run_id, tool, summary, decision, ok, substr(result, 1, 200) FROM tool_calls WHERE chat_id=?1 AND run_id >= ?2 ORDER BY id")?;
        let mut by_run: std::collections::HashMap<i64, Vec<Action>> = Default::default();
        for row in acts.query_map(params![chat, first], |r| Ok((r.get::<_, i64>(0)?, Action { tool: r.get(1)?, summary: r.get(2)?, decision: r.get(3)?, ok: r.get(4)?, result: r.get(5)? })))? {
            let (run, a) = row?;
            by_run.entry(run).or_default().push(a);
        }
        for (m, run) in v.iter_mut() {
            if m.role == "assistant" {
                m.run_id = *run;
            }
            if let Some(a) = run.filter(|_| m.role == "assistant").and_then(|r| by_run.remove(&r)) {
                m.actions = a;
            }
        }
        Ok(v.into_iter().map(|(m, _)| m).collect())
    }

    // ---------- routines ----------

    pub fn routine_create(&self, bot: i64, chat: i64, cron: &str, prompt: &str, quiet: bool) -> Result<Routine> {
        if prompt.trim().is_empty() {
            bail!("a routine needs a prompt");
        }
        let next = next_fire(cron, now())?;
        let c = self.c();
        c.execute(
            "INSERT INTO routines(bot_id, chat_id, cron, prompt, next_fire, quiet) VALUES (?1,?2,?3,?4,?5,?6)",
            params![bot, chat, cron.trim(), prompt.trim(), next, quiet],
        )?;
        Ok(Routine { id: c.last_insert_rowid(), bot_id: bot, chat_id: chat, cron: cron.trim().into(), prompt: prompt.trim().into(), next_fire: next, quiet, paused: false, delivery: "now".into(), tainted: false, once: false, watch: String::new(), retries: 0 })
    }

    /// A one-off at `at` (unix seconds): a reminder, or a task for later. It's deleted once it has run.
    pub fn routine_once(&self, bot: i64, chat: i64, at: i64, prompt: &str, quiet: bool) -> Result<Routine> {
        if prompt.trim().is_empty() {
            bail!("a routine needs a prompt");
        }
        if at <= now() {
            bail!("that time has already passed");
        }
        let c = self.c();
        c.execute(
            "INSERT INTO routines(bot_id, chat_id, cron, prompt, next_fire, quiet, once) VALUES (?1,?2,'',?3,?4,?5,1)",
            params![bot, chat, prompt.trim(), at, quiet],
        )?;
        Ok(Routine { id: c.last_insert_rowid(), bot_id: bot, chat_id: chat, cron: String::new(), prompt: prompt.trim().into(), next_fire: at, quiet, paused: false, delivery: "now".into(), tainted: false, once: true, watch: String::new(), retries: 0 })
    }

    /// A routine a bot made after reading outside content: its prompt may carry someone else's words.
    pub fn routine_taint(&self, id: i64) -> Result<()> {
        self.c().execute("UPDATE routines SET tainted=1 WHERE id=?1", [id])?;
        Ok(())
    }

    fn routines_where(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Routine>> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT id, chat_id, cron, prompt, next_fire, bot_id, quiet, paused, delivery, tainted, once, watch, retries FROM routines WHERE {sql} ORDER BY id"))?;
        let v = st
            .query_map(p, |r| {
                Ok(Routine {
                    id: r.get(0)?,
                    chat_id: r.get(1)?,
                    cron: r.get(2)?,
                    prompt: r.get(3)?,
                    next_fire: r.get(4)?,
                    bot_id: r.get(5)?,
                    quiet: r.get(6)?,
                    paused: r.get(7)?,
                    delivery: r.get(8)?,
                    tainted: r.get(9)?,
                    once: r.get(10)?,
                    watch: r.get(11)?,
                    retries: r.get(12)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn routines(&self, chat: i64) -> Result<Vec<Routine>> {
        self.routines_where("chat_id = ?1", [chat])
    }

    /// A bot's routines that are due.
    pub fn due(&self, bot: i64, t: i64) -> Result<Vec<Routine>> {
        self.routines_where("bot_id = ?1 AND next_fire <= ?2 AND paused = 0", [bot, t])
    }

    /// Edit, pause or resume. The next run is worked out from now, so resuming doesn't fire missed slots.
    pub fn routine_update(&self, chat: i64, id: i64, cron: &str, prompt: &str, quiet: bool, paused: bool) -> Result<bool> {
        if prompt.trim().is_empty() {
            bail!("a routine needs a prompt");
        }
        // A one-off keeps its time (it has no schedule to work one out from).
        let once: bool = self.c().query_row("SELECT once FROM routines WHERE id=?1 AND chat_id=?2", [id, chat], |r| r.get(0)).optional()?.unwrap_or(false);
        if once {
            return Ok(self.c().execute("UPDATE routines SET prompt=?3, quiet=?4, paused=?5 WHERE id=?1 AND chat_id=?2", params![id, chat, prompt.trim(), quiet, paused])? > 0);
        }
        let next = next_fire(cron, now())?;
        Ok(self.c().execute(
            "UPDATE routines SET cron=?3, prompt=?4, quiet=?5, paused=?6, next_fire=?7 WHERE id=?1 AND chat_id=?2",
            params![id, chat, cron.trim(), prompt.trim(), quiet, paused, next],
        )? > 0)
    }

    pub fn routine_set_delivery(&self, chat: i64, id: i64, delivery: &str) -> Result<bool> {
        if !["now", "digest", "log"].contains(&delivery) {
            bail!("delivery is now, digest or log");
        }
        Ok(self.c().execute("UPDATE routines SET delivery=?3 WHERE id=?1 AND chat_id=?2", params![id, chat, delivery])? > 0)
    }

    /// What a quiet check watches ("" for nothing, a page's address, or "mail"). A new target starts from a fresh look.
    pub fn routine_set_watch(&self, chat: i64, id: i64, watch: &str) -> Result<bool> {
        crate::watch::valid(watch)?;
        Ok(self.c().execute("UPDATE routines SET watch=?3, watch_seen=NULL, watch_noise='' WHERE id=?1 AND chat_id=?2 AND watch != ?3", params![id, chat, watch.trim()])? > 0)
    }

    /// How the watched page or mailbox looked at the last check that ran.
    pub fn routine_seen(&self, id: i64) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT watch_seen FROM routines WHERE id=?1", [id], |r| r.get(0)).optional()?.flatten())
    }

    /// Start noticing every line of the watched page again.
    pub fn routine_reset_noise(&self, chat: i64, id: i64) -> Result<bool> {
        Ok(self.c().execute("UPDATE routines SET watch_noise='' WHERE id=?1 AND chat_id=?2", [id, chat])? > 0)
    }

    /// Line shapes of the watched page that churn on their own.
    pub fn routine_noise(&self, id: i64) -> Result<Vec<String>> {
        let v: Option<String> = self.c().query_row("SELECT watch_noise FROM routines WHERE id=?1", [id], |r| r.get(0)).optional()?;
        Ok(v.unwrap_or_default().lines().filter(|l| !l.is_empty()).map(String::from).collect())
    }

    /// ponytail: the 100 newest shapes; a page with more churn than that is mostly noise anyway
    pub fn routine_add_noise(&self, id: i64, shapes: &[String]) -> Result<()> {
        let mut all = self.routine_noise(id)?;
        all.extend(shapes.iter().map(|s| s.replace('\n', " ")).filter(|s| !s.is_empty()));
        let keep = all.len().saturating_sub(100);
        self.c().execute("UPDATE routines SET watch_noise=?2 WHERE id=?1", params![id, all[keep..].join("\n")])?;
        Ok(())
    }

    pub fn routine_set_seen(&self, id: i64, seen: &str) -> Result<()> {
        self.c().execute("UPDATE routines SET watch_seen=?2 WHERE id=?1", params![id, seen])?;
        Ok(())
    }

    /// A watched quiet check that found nothing changed: a run log row with no AI run (and no tokens).
    pub fn run_unchanged(&self, r: &Routine) -> Result<()> {
        let t = now();
        self.c().execute(
            "INSERT INTO runs(bot_id, chat_id, trigger, routine_id, provider, status, started_at, ended_at, reply) VALUES (?1,?2,'routine',?3,'',?4,?5,?5,'')",
            params![r.bot_id, r.chat_id, r.id, UNCHANGED, t],
        )?;
        Ok(())
    }

    /// After a failure that may pass: run again at `at` (or at the next slot, if that's sooner).
    pub fn routine_retry(&self, r: &Routine, at: i64) -> Result<()> {
        self.c().execute("UPDATE routines SET next_fire=min(next_fire, ?2), retries=retries+1 WHERE id=?1", params![r.id, at])?;
        Ok(())
    }

    /// A routine's run is over and won't be tried again: a one-off is gone. When it worked, its earlier failures leave
    /// Needs you (it healed), and the count of tries starts over.
    pub fn routine_done(&self, r: &Routine, worked: bool) -> Result<()> {
        let c = self.c();
        if r.once {
            c.execute("DELETE FROM routines WHERE id=?1", [r.id])?;
        } else {
            c.execute("UPDATE routines SET retries=0 WHERE id=?1", [r.id])?;
        }
        if worked {
            c.execute("INSERT OR IGNORE INTO inbox_dismissed(kind, ref) SELECT 'run', id FROM runs WHERE routine_id=?1 AND status='error'", [r.id])?;
        }
        Ok(())
    }

    /// How many times a routine has run and said something: its first few ask what to trim.
    pub fn routine_replies(&self, routine: i64) -> Result<i64> {
        Ok(self.c().query_row("SELECT count(*) FROM runs WHERE routine_id=?1 AND trigger='routine' AND status='ok' AND coalesce(reply, '') != ''", [routine], |r| r.get(0))?)
    }

    pub fn routine(&self, id: i64) -> Result<Option<Routine>> {
        Ok(self.routines_where("id = ?1", [id])?.pop())
    }

    /// Run a routine at the scheduler's next pass (within half a minute).
    pub fn routine_run_soon(&self, id: i64) -> Result<()> {
        self.c().execute("UPDATE routines SET next_fire=?2 WHERE id=?1", params![id, now()])?;
        Ok(())
    }

    /// The routine a run was for.
    pub fn run_routine_id(&self, run: i64) -> Result<Option<i64>> {
        Ok(self.c().query_row("SELECT routine_id FROM runs WHERE id=?1", [run], |r| r.get(0)).optional()?.flatten())
    }

    pub fn routine_prompt(&self, id: i64) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT prompt FROM routines WHERE id=?1", [id], |r| r.get(0)).optional()?)
    }

    /// A routine's newest run.
    pub fn routine_last_run(&self, routine: i64) -> Result<Option<i64>> {
        Ok(self.c().query_row("SELECT max(id) FROM runs WHERE routine_id=?1 AND trigger='routine'", [routine], |r| r.get(0))?)
    }

    /// Where a routine's result went.
    pub fn run_delivered(&self, run: i64, how: &str) -> Result<()> {
        self.c().execute("UPDATE runs SET delivered=?2 WHERE id=?1", params![run, how])?;
        Ok(())
    }

    /// What happens next, after a failed run's error ("Trying again at 10:35.").
    pub fn run_note(&self, run: i64, note: &str) -> Result<()> {
        self.c().execute("UPDATE runs SET error = coalesce(error, '') || ' ' || ?2 WHERE id=?1", params![run, note])?;
        Ok(())
    }

    /// A routine that couldn't start (its AI account isn't set up): a failed run, with nothing spent.
    pub fn run_not_started(&self, r: &Routine, provider: &str, error: &str) -> Result<i64> {
        let id = self.run_start(r.bot_id, r.chat_id, "routine", Some(r.id), provider)?;
        self.run_finish(id, Err(error))?;
        Ok(id)
    }

    /// A bot's routines on hold ("hold:3"), or one routine's ("hold:routine:12"). An expired hold too, so a late run
    /// can say why; hold_clear removes it.
    pub fn hold(&self, key: &str) -> Result<Option<Hold>> {
        Ok(self.setting(key)?.and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub fn hold_set(&self, key: &str, h: &Hold) -> Result<()> {
        self.set_setting(key, &serde_json::to_string(h)?)
    }

    /// A usage limit: this bot's routines wait, and so do those of every bot signed in to the same AI (a Claude,
    /// ChatGPT or Grok login is one for all bots). Bots on an API key may be limited per model, so only this one.
    /// Returns the bots held.
    pub fn hold_account(&self, bot: i64, kind: &str, h: &Hold) -> Result<Vec<i64>> {
        let mut held = vec![bot];
        if ["claude", "codex", "grok"].contains(&kind) {
            held.extend(self.bots()?.into_iter().filter(|b| b.provider == kind && b.id != bot).map(|b| b.id));
        }
        for b in &held {
            self.hold_set(&format!("hold:{b}"), h)?;
        }
        Ok(held)
    }

    /// End a hold. `go_on`: the person said to run anyway, so a daily cap's lets the rest of today go over it, and the
    /// hold is left ended rather than gone (the scheduler removes it, and says why the routine ran late).
    pub fn hold_clear(&self, key: &str, go_on: bool) -> Result<()> {
        match self.hold(key)? {
            Some(mut h) if go_on => {
                if let Some(w) = &h.waive {
                    self.set_setting(w, &Local::now().format("%Y-%m-%d").to_string())?;
                }
                h.until = now() - 1;
                self.hold_set(key, &h)
            }
            _ => {
                self.c().execute("DELETE FROM settings WHERE key=?1", [key])?;
                Ok(())
            }
        }
    }

    /// Every hold that hasn't ended, for Needs you: (its key, the bot, the routine if it's one routine's, the hold).
    pub fn holds(&self, now: i64) -> Result<Vec<(String, i64, Option<i64>, Hold)>> {
        let v: Vec<(String, String)> = {
            let c = self.c();
            let mut st = c.prepare("SELECT key, value FROM settings WHERE key LIKE 'hold:%'")?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
        };
        let mut out = vec![];
        for (k, v) in v {
            let Ok(h) = serde_json::from_str::<Hold>(&v) else { continue };
            if h.until <= now {
                continue;
            }
            let rest = k.strip_prefix("hold:").unwrap_or_default();
            if let Some(id) = rest.strip_prefix("routine:").and_then(|i| i.parse().ok()) {
                let bot: Option<i64> = self.c().query_row("SELECT bot_id FROM routines WHERE id=?1", [id], |r| r.get(0)).optional()?;
                if let Some(bot) = bot {
                    out.push((k, bot, Some(id), h));
                }
            } else if let Ok(bot) = rest.parse() {
                out.push((k, bot, None, h));
            }
        }
        Ok(out)
    }

    /// The daily caps in tokens: each routine, each bot, all bots (None: no cap).
    pub fn caps(&self) -> Result<[Option<i64>; 3]> {
        let get = |k: &str| -> Result<Option<i64>> { Ok(self.setting(k)?.and_then(|v| v.parse().ok()).filter(|n: &i64| *n > 0)) };
        Ok([get(CAPS[0])?, get(CAPS[1])?, get(CAPS[2])?])
    }

    /// The first daily cap a run of this routine would go over, counting tokens since `day_start`: (the hold's key, the
    /// hold). None when it's under all of them, or the person let it go over today.
    pub fn over_cap(&self, r: &Routine, day_start: i64) -> Result<Option<(String, Hold)>> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let tokens = |sql: &str, p: i64| -> Result<i64> {
            Ok(self.c().query_row(&format!("SELECT coalesce(sum({WEIGHED}), 0) FROM runs WHERE started_at >= ?1 AND {sql}"), params![day_start, p], |r| r.get(0))?)
        };
        let [per_routine, per_bot, per_day] = self.caps()?;
        let k = |n: i64| if n >= 10_000 { format!("{}k", n / 1000) } else { n.to_string() };
        let checks = [
            (per_routine, format!("cap_waived:routine:{}", r.id), format!("hold:routine:{}", r.id), tokens("routine_id = ?2", r.id)?, "this routine"),
            (per_bot, format!("cap_waived:bot:{}", r.bot_id), format!("hold:{}", r.bot_id), tokens("bot_id = ?2", r.bot_id)?, "this bot"),
            (per_day, "cap_waived:day".to_string(), format!("hold:{}", r.bot_id), tokens("?2 = ?2", 0)?, "all your bots together"),
        ];
        for (cap, waive, key, used, who) in checks {
            let Some(cap) = cap else { continue };
            if used < cap || self.setting(&waive)?.as_deref() == Some(today.as_str()) {
                continue;
            }
            let why = format!("{who} reached the daily cap of {} tokens ({} used today)", k(cap), k(used));
            let until = (Local::now().date_naive() + chrono::Duration::days(1)).and_hms_opt(0, 0, 0).and_then(|t| Local.from_local_datetime(&t).earliest()).map_or(now() + 86400, |t| t.timestamp());
            return Ok(Some((key, Hold { until, why, waive: Some(waive) })));
        }
        Ok(None)
    }

    // ---------- daily digest ----------

    pub fn digest_add(&self, bot: i64, chat: i64, routine: i64, text: &str, tainted: bool) -> Result<()> {
        self.c().execute(
            "INSERT INTO digest_items(bot_id, chat_id, routine_id, text, ts, tainted) VALUES (?1,?2,?3,?4,?5,?6)",
            params![bot, chat, routine, text, now(), tainted],
        )?;
        Ok(())
    }

    /// A bot's unsent digest items, oldest first.
    pub fn digest_pending(&self, bot: i64) -> Result<Vec<DigestItem>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, chat_id, routine_id, text, tainted FROM digest_items WHERE bot_id=?1 AND sent_at IS NULL ORDER BY id")?;
        let v = st.query_map([bot], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn digest_mark_sent(&self, ids: &[i64]) -> Result<()> {
        let c = self.c();
        for id in ids {
            c.execute("UPDATE digest_items SET sent_at=?2 WHERE id=?1", params![id, now()])?;
        }
        Ok(())
    }

    /// Quiet checks in the digest lane that ran since `since` and found nothing.
    pub fn quiet_nothing_since(&self, bot: i64, chat: i64, since: i64) -> Result<i64> {
        Ok(self.c().query_row(
            "SELECT count(*) FROM runs r JOIN routines o ON o.id = r.routine_id
              WHERE r.bot_id=?1 AND r.chat_id=?2 AND r.trigger='routine' AND r.started_at > ?3 AND r.status IN ('ok','unchanged') AND coalesce(r.reply,'') = ''
                AND o.quiet = 1 AND o.delivery = 'digest'",
            params![bot, chat, since],
            |r| r.get(0),
        )?)
    }

    /// A routine that came due late gets one `missed` row in the run log, for the slots it didn't run at, so its history
    /// and Needs you show it. Then it runs once, now (the scheduler goes on as before). `away`: Mimir wasn't running at
    /// the time (it started after); otherwise the computer slept or a long run held the scheduler up.
    pub fn routine_missed(&self, r: &Routine, now: i64, why: &str) -> Result<Option<i64>> {
        if now - r.next_fire < MISSED_AFTER {
            return Ok(None);
        }
        let (mut slots, mut last) = (1, r.next_fire);
        // ponytail: walks the skipped slots one by one, capped; a 15-minute check off for a week is 672.
        while !r.once && slots < 1000 {
            match next_fire(&r.cron, last) {
                Ok(n) if n <= now - MISSED_AFTER => (slots, last) = (slots + 1, n),
                _ => break,
            }
        }
        let day = |t: i64| Local.timestamp_opt(t, 0).single().map(|d| d.date_naive());
        let at = |t: i64| match Local.timestamp_opt(t, 0).single() {
            Some(d) if day(t) == day(now) => d.format("%H:%M").to_string(),
            Some(d) => d.format("%a %-d %b %H:%M").to_string(),
            None => String::new(),
        };
        let what = if slots == 1 { format!("Didn't run at {}", at(r.next_fire)) } else { format!("Missed {slots} runs, {} to {}", at(r.next_fire), at(last)) };
        let c = self.c();
        c.execute(
            "INSERT INTO runs(bot_id, chat_id, trigger, routine_id, provider, status, error, started_at, ended_at)
             VALUES (?1, ?2, 'routine', ?3, '', 'missed', ?4, ?5, ?6)",
            params![r.bot_id, r.chat_id, r.id, format!("{what}: {why}. It ran at {} instead.", at(now)), r.next_fire, now],
        )?;
        Ok(Some(c.last_insert_rowid()))
    }

    /// Per routine in a chat: its last runs (newest first: ok | error | missed | running, and when) and its use over
    /// 30 days, so a routine that keeps failing, or quietly costs a lot, shows on the routine itself.
    pub fn routine_stats(&self, chat: i64, now: i64) -> Result<Vec<RoutineStats>> {
        let c = self.c();
        let mut st = c.prepare(
            &format!("SELECT routine_id, coalesce(sum(status NOT IN ('missed','unchanged')), 0), coalesce(sum({WEIGHED}), 0), sum(cost_usd), coalesce(sum(status='error'), 0), coalesce(sum(status='missed'), 0), coalesce(sum(status='unchanged'), 0)
               FROM runs WHERE chat_id=?1 AND routine_id IS NOT NULL AND trigger='routine' AND started_at >= ?2 GROUP BY routine_id"),
        )?;
        let mut v: Vec<RoutineStats> = st
            .query_map(params![chat, now - 30 * 86400], |r| {
                Ok(RoutineStats { id: r.get(0)?, runs: r.get(1)?, tokens: r.get(2)?, cost_usd: r.get(3)?, failed: r.get(4)?, missed: r.get(5)?, unchanged: r.get(6)?, ignored: vec![], last: vec![] })
            })?
            .collect::<Result<_, _>>()?;
        let mut last = c.prepare(&format!("SELECT status, started_at, {WEIGHED} FROM runs WHERE routine_id=?1 AND chat_id=?2 AND trigger='routine' ORDER BY started_at DESC, id DESC LIMIT 8"))?;
        for s in &mut v {
            s.last = last.query_map(params![s.id, chat], |r| Ok(RunMark { status: r.get(0)?, at: r.get(1)?, tokens: r.get(2)? }))?.collect::<Result<_, _>>()?;
        }
        // After the connection is let go (routine_noise takes it again).
        drop(last);
        drop(st);
        drop(c);
        for s in &mut v {
            s.ignored = self.routine_noise(s.id)?;
        }
        Ok(v)
    }

    pub fn routine_delete(&self, chat: i64, id: i64) -> Result<bool> {
        let gone = self.c().execute("DELETE FROM routines WHERE id=?1 AND chat_id=?2", params![id, chat])? > 0;
        if gone {
            self.c().execute("DELETE FROM settings WHERE key=?1", [format!("hold:routine:{id}")])?;
        }
        Ok(gone)
    }

    /// Advance before running so a crash mid-run can't fire the same slot twice. A one-off stays until its run is done
    /// (routine_done), with an hour's grace: a crash mid-run tries it once more then.
    pub fn routine_advance(&self, r: &Routine) -> Result<()> {
        if r.once {
            self.c().execute("UPDATE routines SET next_fire=?2 WHERE id=?1", params![r.id, now() + 3600])?;
            return Ok(());
        }
        let next = next_fire(&r.cron, now().max(r.next_fire))?;
        self.c().execute("UPDATE routines SET next_fire=?2 WHERE id=?1", params![r.id, next])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh DB per test. A counter, not the clock: macOS time is µs-resolution and parallel tests collided.
    fn db() -> Db {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("mimir-test-{}-{n}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        Db::open(&p).unwrap()
    }

    #[test]
    fn memory_roundtrip_supersede_forget_and_scope() {
        let d = db();
        let a = d.store("chat:1", "convention", "Coffee", "User drinks oat flat whites", "message", None).unwrap();
        d.store("chat:1", "knowledge", "Dentist", "Dentist is Dr. Lee in Cape Town", "", None).unwrap();
        d.store("chat:2", "knowledge", "Other user", "Dentist is Dr. Who", "", None).unwrap();
        assert_eq!(d.store("chat:1", "convention", "Coffee", "User drinks oat flat whites", "x", None).unwrap(), a, "dedup");
        assert!(d.store("chat:1", "bogus", "t", "b", "", None).is_err());

        let hits = d.search("chat:1", "who is my dentist?", 5).unwrap();
        assert_eq!(hits.len(), 1, "scoped + relevance gated");
        assert!(hits[0].body.contains("Dr. Lee"));

        let b = d.store("chat:1", "convention", "Coffee", "User switched to black coffee", "message", Some(a)).unwrap();
        let hits = d.search("chat:1", "coffee", 5).unwrap();
        assert_eq!(hits.iter().map(|e| e.id).collect::<Vec<_>>(), vec![b], "superseded entry hidden");
        assert!(d.store("chat:1", "knowledge", "x", "y", "", Some(a)).is_err(), "can't supersede twice");

        assert!(d.forget("chat:1", b).unwrap());
        assert!(d.search("chat:1", "coffee", 5).unwrap().is_empty());
        assert!(d.search("chat:1", "\"); DROP TABLE entries; --", 5).is_ok(), "FTS input is sanitized");
        assert_eq!(d.bootstrap("chat:1").unwrap().len(), 1);
        let hist: Vec<_> = d.entries("chat:1").unwrap().into_iter().map(|e| (e.id, e.status)).collect();
        assert_eq!(hist, vec![(b, "forgotten"), (2, "active"), (a, "superseded")]);

        d.pair(1, "Willem").unwrap();
        d.pair(1, "Willem V").unwrap();
        let chats = d.chats().unwrap();
        assert_eq!((chats.len(), chats[0].title.as_str(), chats[0].memories), (1, "Willem V", 1));
        d.set_setting("provider", "claude").unwrap();
        d.set_setting("provider", "codex").unwrap();
        assert_eq!(d.setting("provider").unwrap().as_deref(), Some("codex"));
    }

    #[test]
    fn pin_and_erase() {
        let d = db();
        let s = "chat:1";
        // 40 newer knowledge entries would push an old one out of standing memory, unless it's pinned.
        let old = d.store(s, "knowledge", "Blood type", "O negative", "", None).unwrap();
        for i in 0..40 {
            d.store(s, "knowledge", &format!("Fact {i}"), "x", "", None).unwrap();
        }
        assert!(!d.bootstrap(s).unwrap().iter().any(|e| e.id == old));
        assert!(d.pin(s, old, true).unwrap());
        assert_eq!(d.bootstrap(s).unwrap()[0].id, old, "pinned comes first");
        let fixed = d.store(s, "knowledge", "Blood type", "O positive", "", Some(old)).unwrap();
        assert!(d.entries(s).unwrap().iter().any(|e| e.id == fixed && e.pinned), "a correction keeps the pin");
        assert!(!d.pin("chat:2", fixed, false).unwrap(), "other scope");
        let next = d.entries(s).unwrap().iter().map(|e| e.id).max().unwrap() + 1;
        assert!(d.store(s, "knowledge", "Ghost", "y", "", Some(next)).is_err(), "can't replace itself");
        assert!(!d.entries(s).unwrap().iter().any(|e| e.title == "Ghost"), "rolled back");

        // Erasing a version takes it and everything before it, and its search entry; the current one stays.
        let third = d.store(s, "knowledge", "Blood type", "AB", "", Some(fixed)).unwrap();
        assert_eq!(d.erase(s, fixed).unwrap(), 2);
        let ids: Vec<i64> = d.entries(s).unwrap().iter().map(|e| e.id).collect();
        assert!(ids.contains(&third) && !ids.contains(&fixed) && !ids.contains(&old));
        assert!(d.search(s, "negative", 5).unwrap().is_empty());
        assert_eq!(d.erase("chat:2", third).unwrap(), 0, "other scope");
    }

    #[test]
    fn background_demotes_without_deleting() {
        let d = db();
        let s = "chat:1";
        let id = d.store(s, "knowledge", "Old flat", "Lived in Observatory in 2019", "", None).unwrap();
        let sug = |action: &str| Suggestion { id: 0, action: action.into(), entry_id: Some(id), kind: "knowledge".into(), title: String::new(), body: String::new(), reason: "rarely matters".into(), check: None, tainted: false };
        let q = d.suggestion_add(s, &sug("demote")).unwrap().unwrap();
        assert_eq!(d.suggestion_decide(s, q, true).unwrap(), format!("#{id} moved to background"));
        assert!(d.bootstrap(s).unwrap().is_empty(), "out of standing memory");
        assert_eq!(d.search(s, "Observatory", 5).unwrap()[0].id, id, "still found by search");
        assert!(d.suggestion_add(s, &sug("demote")).is_err(), "already in the background");
        let fixed = d.store(s, "knowledge", "Old flat", "Lived in Woodstock in 2019", "", Some(id)).unwrap();
        assert!(d.entries(s).unwrap().iter().any(|e| e.id == fixed && e.background), "a correction stays in the background");
        assert!(d.pin(s, fixed, true).unwrap());
        assert_eq!(d.bootstrap(s).unwrap()[0].id, fixed, "pinning brings it back");
        assert!(d.suggestion_add(s, &Suggestion { entry_id: Some(fixed), ..sug("demote") }).is_err(), "pins aren't demoted");
        // Queued before the pin, accepted after: dismissed, and the pin stays.
        assert!(d.pin(s, fixed, false).unwrap());
        let late = d.suggestion_add(s, &Suggestion { entry_id: Some(fixed), ..sug("demote") }).unwrap().unwrap();
        d.pin(s, fixed, true).unwrap();
        assert!(d.suggestion_decide(s, late, true).is_err());
        assert!(d.entries(s).unwrap().iter().any(|e| e.id == fixed && e.pinned && !e.background), "still pinned");
        assert!(d.background(s, fixed, true).unwrap());
        assert!(d.entries(s).unwrap().iter().any(|e| e.id == fixed && e.background && !e.pinned), "backgrounding drops the pin");
    }

    #[test]
    fn undo_and_weekly_trends() {
        let d = db();
        let s = "chat:1";
        let old = d.store(s, "knowledge", "Gym", "Mondays", "", None).unwrap();
        let new = d.store_by(s, "knowledge", "Gym", "Tuesdays", "", Some(old), &Prov::by("tidy")).unwrap();
        assert!(d.undo(s, new).unwrap());
        let e = d.entries(s).unwrap();
        assert_eq!(e.iter().filter(|e| e.status == "active").map(|e| e.id).collect::<Vec<_>>(), vec![old], "the earlier version is back");
        assert!(!d.undo(s, old - 100).unwrap() && !d.undo("chat:2", old).unwrap());
        assert!(!d.undo(s, new).unwrap(), "only once");

        d.log(1, 1, "user", "hi", None).unwrap();
        d.log(1, 1, "assistant", "hello", None).unwrap();
        d.signal_add(1, 1, "correction", "x").unwrap();
        d.signal_add(1, 1, "correction", "You said: you're in Paris → They replied: I told you I moved to Lyon").unwrap();
        d.signal_add(1, 1, "repeated", "Asked: x → corrected again: y").unwrap();
        d.tool_call_log(None, 1, "web_fetch", "Read x", "allow", false, "404").unwrap();
        let w = d.weeks(1, 4, now() + 1).unwrap();
        assert_eq!(w.len(), 4);
        // The tidy-up's "Tuesdays" was taken back.
        assert_eq!(w[3], Week { start: w[3].start, turns: 1, corrections: 2, tool_calls: 1, tool_errors: 1, repeats: 1, forgot: 1, undone: 1, ..Default::default() });
        assert_eq!(w[0].turns, 0, "oldest first");
    }

    #[test]
    fn skills_from_runs_that_worked() {
        let d = db();
        let s = "chat:0";
        let run = |calls: &[(&str, bool)], reply_then: Option<&str>| {
            let r = d.run_start(1, 0, "message", None, "test").unwrap();
            d.run_asked(r, "Book my usual Thursday padel court").unwrap();
            for (tool, ok) in calls {
                d.tool_call_log(Some(r), 0, tool, "x", "allow", *ok, "").unwrap();
            }
            d.run_finish(r, Ok(&Usage::default())).unwrap();
            d.log(1, 0, "user", "Book my usual Thursday padel court", None).unwrap();
            d.log(1, 0, "assistant", "Booked", Some(r)).unwrap();
            if let Some(t) = reply_then {
                d.log(1, 0, "user", t, None).unwrap();
            }
            r
        };
        let good = run(&[("browser__navigate", true), ("memory_search", true), ("browser__click", true)], Some("thanks!"));
        run(&[("browser__navigate", true), ("browser__click", false)], None);
        run(&[("browser__navigate", true), ("browser__click", true)], Some("No, I said Thursday"));
        run(&[("memory_search", true), ("web_fetch", true)], None);
        let found: Vec<i64> = d.good_runs(0, 0).unwrap().iter().map(|g| g.id).collect();
        assert_eq!(found, vec![good], "only the clean, uncorrected run with 2+ real actions");

        let id = d.skill_propose(s, 1, "Book padel", "When asked to book padel", "1. Open the club site\n2. Pick Thursday 18:00", Some(good)).unwrap().unwrap();
        assert!(d.good_runs(0, 0).unwrap().is_empty(), "a run teaches one skill");
        assert_eq!(d.skill_propose(s, 1, "book PADEL", "x", "y", None).unwrap(), None, "one per name");
        assert!(d.skills_active(s, 1).unwrap().is_empty(), "not used before you accept it");
        assert_eq!(d.suggestion_counts().unwrap(), vec![(0, 1)], "shows in Needs you");
        assert!(d.skill_decide(s, id, true).unwrap());
        assert_eq!(d.skills_active(s, 1).unwrap()[0].name, "Book padel");
        assert!(d.skills_active(s, 2).unwrap().is_empty() && d.skills_active("chat:9", 1).unwrap().is_empty(), "per bot, per chat");
        assert_eq!(d.skill_propose(s, 1, "Book padel", "x", "1. Open the club site\n2. Pick Thursday 18:00", None).unwrap(), None, "same steps");
        let v2 = d.skill_propose(s, 1, "Book padel", "When asked to book padel", "1. Open the app", None).unwrap().unwrap();
        assert_eq!(d.skills_active(s, 1).unwrap()[0].id, id, "the old version stays in use until you accept");
        assert!(!d.skill_decide("chat:9", v2, true).unwrap(), "other chat");
        assert!(d.skill_decide(s, v2, true).unwrap());
        assert_eq!(d.skills_active(s, 1).unwrap().iter().map(|k| k.id).collect::<Vec<_>>(), vec![v2], "replaced");
        // A different skill for what reads like the same job: the card says so.
        let twin = d.skill_propose(s, 1, "Padel court", "When asked to book a padel court at the club", "1. Call the club", None).unwrap().unwrap();
        let other = d.skill_propose(s, 1, "Dinner", "When asked to book a dinner table", "1. Call the restaurant", None).unwrap().unwrap();
        let reason = |id: i64| d.skills(s).unwrap().into_iter().find(|k| k.id == id).unwrap().reason;
        assert!(reason(twin).contains("Similar to your skill “Book padel”"), "{}", reason(twin));
        assert!(reason(other).is_empty());
        d.skill_decide(s, twin, false).unwrap();
        d.skill_decide(s, other, false).unwrap();
        assert!(d.skill_remove(s, v2).unwrap() && d.skills_active(s, 1).unwrap().is_empty());

        // Shared with all bots: every bot sees it, a new version stays shared, and names stay unique.
        let k = d.skill_propose(s, 2, "Weekly report", "When asked for the weekly report", "1. Collect", None).unwrap().unwrap();
        d.skill_decide(s, k, true).unwrap();
        assert!(d.skills_active(s, 1).unwrap().is_empty());
        assert!(d.skill_share(s, k, 0).unwrap());
        assert_eq!(d.skills_active(s, 1).unwrap()[0].id, k, "bot 1 sees the shared skill");
        let v = d.skill_propose(s, 1, "weekly report", "When asked for the weekly report", "1. Collect\n2. Send", None).unwrap().unwrap();
        d.skill_decide(s, v, true).unwrap();
        assert_eq!(d.skills_active(s, 2).unwrap().iter().map(|k| (k.id, k.bot_id)).collect::<Vec<_>>(), vec![(v, 0)], "a new version of a shared skill");
        let other = d.skill_propose(s, 1, "Book padel", "When booking", "1. Open", None).unwrap().unwrap();
        d.skill_decide(s, other, true).unwrap();
        let mine = d.skill_propose(s, 2, "Book padel", "When booking", "1. Call", None).unwrap().unwrap();
        d.skill_decide(s, mine, true).unwrap();
        assert!(d.skill_share(s, mine, 0).is_err(), "bot 1 has its own Book padel");
        assert!(d.skill_share(s, mine, 3).unwrap(), "moving it to a bot without one is fine");
        // A fix proposed before the skill was shared still replaces it, and stays shared.
        let fix = d.skill_patch_propose(s, mine, "1. Call", "1. Call ahead", "it failed").unwrap().unwrap();
        assert!(d.skill_share(s, other, 3).is_err(), "bot 3 now has its own Book padel");
        d.skill_remove(s, other).unwrap();
        assert!(d.skill_share(s, mine, 0).unwrap());
        d.skill_decide(s, fix, true).unwrap();
        assert_eq!(d.skills_active(s, 1).unwrap().iter().filter(|k| k.name == "Book padel").map(|k| (k.id, k.bot_id)).collect::<Vec<_>>(), vec![(fix, 0)]);
        let v3 = d.skill_propose(s, 1, "Book padel", "a", "b", None).unwrap().unwrap();
        assert!(d.skill_decide(s, v3, false).unwrap());
        assert_eq!(d.skill_propose(s, 1, "Book padel", "a", "c", None).unwrap(), None, "dismissed stays dismissed");
    }

    #[test]
    fn skills_improve_and_retire_by_proposal() {
        let d = db();
        let s = "chat:0";
        let id = d.skill_propose(s, 1, "Book padel", "When asked to book padel", "1. Open the club site\n2. Pick Thursday 18:00\n3. Pay", None).unwrap().unwrap();
        d.skill_decide(s, id, true).unwrap();
        // Used in a run that the person then corrected.
        let r = d.run_start(1, 0, "message", None, "test").unwrap();
        d.run_asked(r, "Book padel for Thursday").unwrap();
        d.skill_used(id, Some(r)).unwrap();
        d.tool_call_log(Some(r), 0, "browser__click", "Click 18:00", "allow", true, "").unwrap();
        d.run_finish(r, Ok(&Usage::default())).unwrap();
        d.log(1, 0, "user", "Book padel for Thursday", None).unwrap();
        d.log(1, 0, "assistant", "Booked 18:00", Some(r)).unwrap();
        d.log(1, 0, "user", "No, I wanted the indoor court, it's always indoor", None).unwrap();
        // And a run that went fine: no trouble from it.
        let ok = d.run_start(1, 0, "message", None, "test").unwrap();
        d.skill_used(id, Some(ok)).unwrap();
        d.run_finish(ok, Ok(&Usage::default())).unwrap();
        let t = d.skill_troubles(s, 0, 0).unwrap();
        assert_eq!(t.len(), 1, "only the corrected run");
        assert!(t[0].trouble.contains("indoor") && t[0].skill.id == id && t[0].skill.uses == 2);

        // A fix must patch text that's there, and waits for you.
        assert!(d.skill_patch_propose(s, id, "Pick Friday", "x", "r").is_err(), "not in the skill");
        let fix = d.skill_patch_propose(s, id, "2. Pick Thursday 18:00", "2. Pick an indoor court, Thursday 18:00", "They want indoor courts").unwrap().unwrap();
        let p = d.skills(s).unwrap().into_iter().find(|k| k.id == fix).unwrap();
        assert_eq!((p.kind.as_str(), p.status.as_str()), ("patch", "proposed"));
        assert!(p.body.contains("indoor") && p.body.contains("3. Pay"), "a patch, not a rewrite: {}", p.body);
        assert_eq!(d.skills_active(s, 1).unwrap()[0].id, id, "old version in use until accepted");
        d.skill_decide(s, fix, true).unwrap();
        assert_eq!(d.skills_active(s, 1).unwrap().iter().map(|k| k.id).collect::<Vec<_>>(), vec![fix]);

        // Merge two overlapping skills; retire one nobody uses.
        let other = d.skill_propose(s, 1, "Reserve a court", "When reserving padel", "1. Open the club site\n2. Book", None).unwrap().unwrap();
        d.skill_decide(s, other, true).unwrap();
        assert!(d.skill_merge_propose(s, &[fix], "x", "y", "z", "r").is_err(), "needs two");
        let merged = d.skill_merge_propose(s, &[fix, other], "Book padel", "When booking or reserving padel", "1. Open the club site\n2. Indoor court\n3. Pay", "Same job").unwrap().unwrap();
        d.skill_decide(s, merged, true).unwrap();
        assert_eq!(d.skills_active(s, 1).unwrap().iter().map(|k| k.id).collect::<Vec<_>>(), vec![merged], "both replaced by the merge");
        assert!(d.skills_unused(s, 60).unwrap().is_empty(), "new");
        d.c().execute("UPDATE skills SET created_at = created_at - 100 * 86400 WHERE id=?1", [merged]).unwrap();
        let stale = d.skills_unused(s, 60).unwrap();
        assert_eq!(stale.len(), 1);
        let retire = d.skill_retire_propose(s, &stale[0], "unused").unwrap().unwrap();
        assert_eq!(d.skill_retire_propose(s, &stale[0], "unused").unwrap(), None, "one at a time");
        d.skill_decide(s, retire, true).unwrap();
        assert!(d.skills_active(s, 1).unwrap().is_empty(), "retired");
        assert!(d.skills(s).unwrap().is_empty(), "a retire card doesn't become a skill");
        // It can come back.
        assert_eq!(d.skills_removed(s).unwrap().iter().filter(|k| k.id == merged).count(), 1);
        assert!(d.skill_restore(s, merged).unwrap());
        assert_eq!(d.skills_active(s, 1).unwrap()[0].id, merged);
        // "Keep it" on a retire card doesn't block fixes or re-teaching afterwards.
        let r2 = d.skill_retire_propose(s, &d.skills_active(s, 1).unwrap()[0], "unused").unwrap().unwrap();
        d.skill_decide(s, r2, false).unwrap();
        assert!(d.skill_patch_propose(s, merged, "2. Indoor court", "2. Indoor court, never outdoor", "r").unwrap().is_some());
        // A merge can't take the name of a skill it doesn't replace.
        let a = d.skill_propose(s, 1, "Pay bills", "When paying", "1. Open bank", None).unwrap().unwrap();
        d.skill_decide(s, a, true).unwrap();
        let b = d.skill_propose(s, 1, "Pay invoices", "When paying invoices", "1. Open bank\n2. Pay", None).unwrap().unwrap();
        d.skill_decide(s, b, true).unwrap();
        assert!(d.skill_merge_propose(s, &[a, b], "Book padel", "x", "y", "r").is_err(), "Book padel is another active skill");

        // Facts nobody looks up can leave standing memory; looking one up counts.
        let e = d.store(s, "knowledge", "Old wifi hint", "the cat is called Bob", "user said", None).unwrap();
        d.c().execute("UPDATE entries SET created_at = created_at - 100 * 86400 WHERE id=?1", [e]).unwrap();
        assert_eq!(d.memories_unrecalled(s, 90).unwrap().iter().map(|m| m.0).collect::<Vec<_>>(), vec![e]);
        d.search(s, "wifi hint", 5).unwrap();
        assert!(d.memories_unrecalled(s, 90).unwrap().is_empty(), "recalled just now");
    }

    #[test]
    fn past_conversations_are_searchable() {
        let d = db();
        let r1 = d.run_start(1, 7, "message", None, "claude").unwrap();
        d.log(1, 7, "user", "We're flying to Lisbon on the 12th, back on the 19th.", Some(r1)).unwrap();
        d.log(1, 7, "assistant", "Noted: Lisbon, 12th to 19th.", Some(r1)).unwrap();
        // A reply written after reading a web page.
        let r2 = d.run_start(1, 7, "message", None, "claude").unwrap();
        d.run_taint(r2).unwrap();
        d.log(1, 7, "assistant", "The page says Lisbon flights are cheapest on Tuesdays.", Some(r2)).unwrap();
        d.log(2, 8, "user", "Lisbon is lovely", None).unwrap(); // someone else's chat
        let r3 = d.run_start(1, 7, "message", None, "claude").unwrap();
        d.log(1, 7, "user", "when do we fly to lisbon?", Some(r3)).unwrap(); // the question being answered now
        let hits = d.chat_search(7, "Lisbon flights", Some(r3), (0, i64::MAX), 10).unwrap();
        assert_eq!(hits.len(), 3, "this chat only, not the question itself: {hits:?}");
        assert!(hits.iter().any(|h| h.role == "user" && h.excerpt.contains("12th") && !h.tainted));
        assert!(hits.iter().any(|h| h.excerpt.contains("Tuesdays") && h.tainted), "a reply from a run that read outside content");
        assert!(d.chat_search(7, "?!", None, (0, i64::MAX), 10).unwrap().is_empty());
        // A bot message from no known run (a digest, a stopped reply) isn't trusted either; without a current run,
        // what the person wrote is still found.
        d.log(1, 7, "assistant", "Digest: Lisbon hotels are cheap this week", None).unwrap();
        let hits = d.chat_search(7, "Lisbon hotels", None, (0, i64::MAX), 10).unwrap();
        assert!(hits.iter().any(|h| h.excerpt.contains("Digest") && h.tainted));
        assert!(d.chat_search(7, "12th", None, (0, i64::MAX), 10).unwrap().iter().any(|h| h.role == "user"));
        // A time range: only messages in it.
        assert!(d.chat_search(7, "12th", None, (now() + 3600, i64::MAX), 10).unwrap().is_empty());
        assert!(!d.chat_search(7, "12th", None, (now() - 3600, now() + 3600), 10).unwrap().is_empty());
    }

    #[test]
    fn messages_outside_the_session_are_found() {
        let d = db();
        let m = d.run_start(1, 11, "message", None, "claude").unwrap();
        d.log(1, 11, "user", "hi", None).unwrap();
        d.log(1, 11, "assistant", "hello", Some(m)).unwrap();
        assert_eq!(d.last_outside_session(1, 11).unwrap(), None, "a reply in the session");
        let r = d.run_start(1, 11, "routine", Some(1), "claude").unwrap();
        d.log(1, 11, "user", "[Scheduled routine #1 \"0 7 * * *\"] morning briefing", None).unwrap();
        d.log(1, 11, "assistant", "BTC is up 4%", Some(r)).unwrap();
        assert_eq!(d.last_outside_session(1, 11).unwrap(), Some(("routine result", "BTC is up 4%".into())));
        assert_eq!(d.last_outside_session(2, 11).unwrap(), None, "another bot's");
        d.log(1, 11, "assistant", "☀️ Your digest", None).unwrap();
        assert_eq!(d.last_outside_session(1, 11).unwrap(), Some(("message", "☀️ Your digest".into())));
    }

    #[test]
    fn meetings_keep_who_said_what() {
        let d = db();
        let id = d.meeting_save("Standup", 100, 400, &[(130, "others".into(), "Can you send the deck?".into()), (110, "you".into(), "Morning all".into())]).unwrap();
        let m = d.meetings().unwrap();
        // Lines saved as a meeting goes, then renamed and ended.
        let live = d.meeting_save("", 100, 100, &[]).unwrap();
        d.meeting_line(live, 110, "you", "first point").unwrap();
        d.meeting_line(live, 120, "others", "second point").unwrap();
        assert!(d.meeting_update(live, Some(" Sync "), Some(200)).unwrap());
        assert!(d.meeting_notes(live, "- budget!").unwrap());
        // Voices told apart afterwards: the others' lines relabelled, a speaker named.
        let others = d.meeting_others(live).unwrap();
        d.meeting_relabel(&[(others[0].0, "others:2".into())]).unwrap();
        assert!(d.meeting_speaker(live, "others:2", " Sam ").unwrap());
        assert_eq!(d.meeting_lines(live).unwrap(), vec![(110, "you".to_string(), "first point".to_string()), (120, "others:2".to_string(), "second point".to_string())]);
        let row = d.meetings().unwrap().into_iter().find(|r| r["id"] == live).unwrap();
        assert_eq!((row["title"].as_str(), row["ended_at"].as_i64(), row["notes"].as_str()), (Some("Sync"), Some(200), Some("- budget!")));
        assert_eq!(row["speakers"]["others:2"], "Sam");
        assert!(d.meeting_attendees(live, &["Sam Lee".into(), "Priya Shah".into()]).unwrap());
        assert!(d.meeting_summary(live, " **Decisions:** ship Friday ").unwrap());
        assert_eq!(d.meetings().unwrap().into_iter().find(|r| r["id"] == live).unwrap()["summary"], "**Decisions:** ship Friday");
        assert_eq!(d.meetings().unwrap().into_iter().find(|r| r["id"] == live).unwrap()["attendees"][1], "Priya Shah");
        assert_eq!(d.meeting_search("FIRST", 5).unwrap().len(), 1, "any case");
        assert!(d.meeting_search("nothing like it", 5).unwrap().is_empty());
        d.meeting_delete(live).unwrap();
        assert_eq!((m[0]["title"].as_str(), m[0]["words"].as_i64()), (Some("Standup"), Some(7)));
        assert_eq!(d.meeting_lines(id).unwrap()[0].2, "Morning all", "in time order");
        assert!(d.meeting_delete(id).unwrap() && d.meeting_lines(id).unwrap().is_empty());
    }

    #[test]
    fn one_user_of_the_computer_at_a_time() {
        let d = db();
        assert_eq!(d.computer_lease("run:100", "Ada", 120).unwrap(), None, "free: Ada gets it");
        assert_eq!(d.computer_lease("run:100", "Ada", 120).unwrap(), None, "and keeps renewing it");
        assert_eq!(d.computer_lease("run:2", "Researcher", 120).unwrap(), Some(("run:100".into(), "Ada".into())), "Researcher waits");
        assert_eq!(d.computer_holder().unwrap().map(|h| h.1), Some("Ada".into()));
        let run = d.run_start(1, 0, "message", None, "test").unwrap();
        assert_eq!(d.computer_lease(&format!("run:{run}"), "Ada", 120).unwrap(), Some(("run:100".into(), "Ada".into())));
        d.computer_release("run:100").unwrap();
        assert_eq!(d.computer_lease(&format!("run:{run}"), "Ada", 120).unwrap(), None);
        d.run_finish(run, Ok(&Usage::default())).unwrap();
        assert_eq!(d.computer_holder().unwrap(), None, "a finished run lets go");
        assert_eq!(d.computer_lease("you", "you", 60).unwrap(), None);
        assert!(d.computer_lease("run:9", "Ada", 120).unwrap().is_some(), "you took over: bots wait");
        assert_eq!(d.computer_lease("you", "you", -1).unwrap(), None, "an expired lease (here: renewed into the past)");
        assert_eq!(d.computer_lease("run:9", "Ada", 120).unwrap(), None, "is free again");
    }

    #[test]
    fn handed_over_runs_link_to_the_run_that_asked() {
        let d = db();
        let parent = d.run_start(1, 0, "message", None, "test").unwrap();
        let child = d.run_start(2, 0, "delegated", None, "test").unwrap();
        d.run_set_parent(child, parent).unwrap();
        d.run_reply(child, "Canberra").unwrap();
        d.log(1, 0, "assistant", "Researcher says Canberra", Some(parent)).unwrap();
        let kids = d.child_runs(parent).unwrap();
        assert_eq!((kids.len(), kids[0].id, kids[0].reply.as_deref()), (1, child, Some("Canberra")));
        assert!(d.child_runs(child).unwrap().is_empty());
        assert_eq!(d.messages(0, 5).unwrap().last().unwrap().run_id, Some(parent), "a reply carries its run");
        assert_eq!(d.runs(Some(0), 10).unwrap().len(), 2, "the run log still lists both");
    }

    #[test]
    fn corrections_are_kept_as_cases() {
        let d = db();
        d.case_add(1, 3, "Book a table for dinner", "Booked Friday 19:00", "No, I said Thursday").unwrap();
        d.case_add(1, 3, "Brief me on the news", "Here are 12 stories with long summaries", "Too long, three bullets please").unwrap();
        d.case_add(1, 4, "Brief me on the news", "x", "y").unwrap();
        let hits = d.cases_like(3, "When briefing news, use three bullets", 4).unwrap();
        assert_eq!(hits.len(), 1, "shares words, this chat only");
        assert_eq!(hits[0].2, "Too long, three bullets please");
        assert!(d.cases_like(3, "Prefers metric units", 4).unwrap().is_empty());
    }

    #[test]
    fn old_suggestions_table_learns_demote() {
        let p = std::env::temp_dir().join(format!("mimir-mig-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        {
            let c = Connection::open(&p).unwrap();
            c.execute_batch(&SCHEMA.replace(",'demote'", "")).unwrap();
            c.execute("INSERT INTO suggestions(scope, action, title, created_at) VALUES ('chat:1', 'add', 'Kept', 0)", []).unwrap();
        }
        let d = Db::open(&p).unwrap();
        let id = d.store("chat:1", "knowledge", "Gym", "Virgin Active", "", None).unwrap();
        let s = Suggestion { id: 0, action: "demote".into(), entry_id: Some(id), kind: "knowledge".into(), title: String::new(), body: String::new(), reason: String::new(), check: None, tainted: false };
        assert!(d.suggestion_add("chat:1", &s).unwrap().is_some());
        assert_eq!(d.suggestions("chat:1").unwrap().len(), 2, "old rows kept");
        drop(d);
        Db::open(&p).unwrap();
    }

    #[test]
    fn corrections_count_against_recalled_memories() {
        let d = db();
        let used = d.store("chat:7", "knowledge", "Office", "Office is in Paarl", "", None).unwrap();
        let other = d.store("chat:8", "knowledge", "Office", "Office is in Paarl", "", None).unwrap();
        assert_eq!(d.blame_last_reply(1, 7, "no").unwrap(), 0, "no reply yet");
        let run = d.run_start(1, 7, "message", None, "test").unwrap();
        d.run_recalled(run, &[used, other]).unwrap();
        d.log(1, 7, "assistant", "See you at the Paarl office", Some(run)).unwrap();
        assert_eq!(d.blame_last_reply(1, 7, "no, wrong").unwrap(), 1, "only this chat's memory");
        let count = |scope: &str, id| d.entries(scope).unwrap().into_iter().find(|e| e.id == id).unwrap().missteps;
        assert_eq!((count("chat:7", used), count("chat:8", other)), (1, 0));
        // A standing preference the correction is about gets one too (it's in every reply, never "recalled").
        let bullets = d.store("chat:7", "convention", "News format", "Give tech news as three bullet points", "", None).unwrap();
        let tone = d.store("chat:7", "convention", "Tone", "Keep it friendly", "", None).unwrap();
        assert_eq!(d.blame_last_reply(1, 7, "Please, no bullet points for this").unwrap(), 2, "the recalled memory and the bullets rule");
        assert_eq!((count("chat:7", bullets), count("chat:7", tone)), (1, 0));
        // Lines in a prompt carry the date they were saved.
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert!(d.bootstrap("chat:7").unwrap().iter().all(|e| e.line().contains(&format!(", saved {today}]"))));
    }

    #[test]
    fn standing_memory_has_a_size_budget() {
        let d = db();
        let s = "chat:3";
        let long = "x".repeat(1500);
        let pin = d.store(s, "knowledge", "Allergies", &long, "", None).unwrap();
        d.pin(s, pin, true).unwrap();
        for i in 0..10 {
            d.store(s, "convention", &format!("Rule {i}"), &long, "", None).unwrap();
        }
        let short = d.store(s, "convention", "Tone", "Be brief", "", None).unwrap();
        let b = d.bootstrap(s).unwrap();
        let size: usize = b.iter().map(|e| e.line().len()).sum();
        assert_eq!(b[0].id, pin, "pinned first, always");
        assert!(size <= BOOTSTRAP_CHARS, "{size}");
        assert!(b.iter().any(|e| e.id == short), "a small one still fits after the big ones run out of room");
        assert!(b.len() < 12, "not every long rule fits");
    }

    #[test]
    fn approvals_and_rules() {
        let d = db();
        assert_eq!(d.rule("memory_forget").unwrap(), None);
        d.set_rule("memory_forget", "deny").unwrap();
        assert_eq!(d.rule("memory_forget").unwrap().as_deref(), Some("deny"));
        d.set_rule("memory_forget", "default").unwrap();
        assert_eq!(d.rule("memory_forget").unwrap(), None);

        let a = d.approval_create(1, 5, "memory_forget", "Forget #1", 300, "rule", None).unwrap();
        assert_eq!(d.approvals_to_notify(1).unwrap().len(), 1);
        assert!(d.approvals_to_notify(2).unwrap().is_empty(), "only the asking bot sends the card");
        assert!(d.approval_decide(a, true, Some(6)).unwrap().is_none(), "other chat can't decide");
        assert_eq!(d.approval_decide(a, true, Some(5)).unwrap().unwrap().status, "approved");
        assert!(d.approval_decide(a, false, None).unwrap().is_none(), "decided once");
        assert_eq!(d.approval_status(a).unwrap(), "approved");

        let b = d.approval_create(1, 0, "routine_delete", "Delete #2", 0, "rule", None).unwrap();
        assert_eq!(d.approval_status(b).unwrap(), "expired", "ttl 0 expires immediately");
        assert!(d.approval_decide(b, true, None).unwrap().is_none(), "can't approve expired");
        assert!(d.approvals_pending().unwrap().is_empty());
        assert!(d.approvals_to_notify(1).unwrap().is_empty(), "local chat 0 never notified via Telegram");

        // A card decided elsewhere (or expired) gets closed once.
        let e = d.approval_create(1, 5, "memory_forget", "Forget #3", 300, "rule", None).unwrap();
        d.approval_mark_notified(e, Some("77")).unwrap();
        assert!(d.approvals_to_close(1).unwrap().is_empty(), "still pending");
        d.approval_decide(e, false, None).unwrap();
        let close = d.approvals_to_close(1).unwrap();
        assert_eq!((close.len(), close[0].1.as_str(), close[0].0.status.as_str()), (1, "77", "denied"));
        d.approval_mark_closed(e).unwrap();
        assert!(d.approvals_to_close(1).unwrap().is_empty());
    }

    #[test]
    fn routine_missed_and_stats() {
        let d = db();
        let r = d.routine_create(1, 8, "0 * * * *", "check the page", true).unwrap();
        // On time (within 15 minutes): nothing missed.
        assert_eq!(d.routine_missed(&r, r.next_fire + 600, ASLEEP).unwrap(), None);
        // Five hours late: the slot it's running for and the four after it that are also past.
        let late = d.routine_missed(&r, r.next_fire + 5 * 3600, AWAY).unwrap().unwrap();
        let run = d.runs(Some(8), 5).unwrap().into_iter().find(|x| x.id == late).unwrap();
        assert_eq!(run.status, "missed");
        assert!(run.error.as_deref().unwrap().starts_with("Missed 5 runs"), "{:?}", run.error);
        assert!(run.error.as_deref().unwrap().contains("Mimir wasn't running"));
        let a = d.run_start(1, 8, "routine", Some(r.id), "claude -p").unwrap();
        d.run_finish(a, Ok(&Usage { model: None, tokens_in: 900, tokens_out: 100, cost_usd: Some(0.02), ..Default::default() })).unwrap();
        let b = d.run_start(1, 8, "routine", Some(r.id), "claude -p").unwrap();
        d.run_finish(b, Err("usage limit")).unwrap();

        let s = d.routine_stats(8, now() + 6 * 3600).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].id, s[0].runs, s[0].tokens, s[0].failed, s[0].missed), (r.id, 2, 1000, 1, 1));
        assert_eq!(s[0].last.first().map(|m| m.status.as_str()), Some("missed"), "newest first: the missed slot is latest in time");
        // A missed run is in Needs you, but isn't use.
        assert!(d.runs_failed(0).unwrap().iter().any(|x| x.id == late));
        assert_eq!(d.run_totals(Some(8), 0).unwrap().runs, 2);
        // A one-off misses its one time only.
        let o = d.routine_once(1, 9, now() + 60, "call mum", false).unwrap();
        let m = d.routine_missed(&o, o.next_fire + 3 * 86400, ASLEEP).unwrap().unwrap();
        assert!(d.runs(Some(9), 1).unwrap()[0].error.as_deref().unwrap().starts_with("Didn't run at"), "{m}");
    }

    #[test]
    fn a_usage_limit_holds_everyone_on_that_login() {
        let d = db();
        let mut b = d.bot(1).unwrap();
        b.provider = "claude".into();
        d.bot_save(&b).unwrap();
        for (name, provider) in [("Ada", "claude"), ("Scout", "codex"), ("Ollie", "local")] {
            let mut x = b.clone();
            (x.id, x.name, x.provider) = (0, name.into(), provider.into());
            d.bot_save(&x).unwrap();
        }
        let h = Hold { until: now() + 600, why: "limit".into(), waive: None };
        assert_eq!(d.hold_account(1, "claude", &h).unwrap().len(), 2, "Mimir and Ada share the Claude login");
        assert_eq!(d.holds(now()).unwrap().len(), 2);
        let ollie = d.bots().unwrap().into_iter().find(|x| x.name == "Ollie").unwrap().id;
        assert_eq!(d.hold_account(ollie, "api", &h).unwrap(), vec![ollie]);
    }

    #[test]
    fn watched_quiet_checks() {
        let d = db();
        let r = d.routine_create(1, 8, "*/15 * * * *", "tell me if the price drops", true).unwrap();
        assert!(d.routine_set_watch(8, r.id, "http://localhost:8080/").is_err());
        assert!(d.routine_set_watch(8, r.id, "https://shop.example/widget").unwrap());
        d.routine_set_seen(r.id, "Price: 40").unwrap();
        assert_eq!(d.routine_seen(r.id).unwrap().as_deref(), Some("Price: 40"));
        // The same address again keeps what was seen; another one starts from a fresh look.
        assert!(!d.routine_set_watch(8, r.id, "https://shop.example/widget").unwrap());
        assert!(d.routine_seen(r.id).unwrap().is_some());
        d.routine_set_watch(8, r.id, "mail").unwrap();
        assert_eq!(d.routine_seen(r.id).unwrap(), None);
        let r = d.routines(8).unwrap().remove(0);
        assert_eq!(r.watch, "mail");
        // Nothing changed: in the log and the digest's "nothing needed you", but not use.
        d.run_unchanged(&r).unwrap();
        let s = d.routine_stats(8, now()).unwrap();
        assert_eq!((s[0].runs, s[0].unchanged, s[0].last[0].status.as_str()), (0, 1, UNCHANGED));
        d.routine_add_noise(r.id, &["Views: #".to_string()]).unwrap();
        assert_eq!(d.routine_stats(8, now()).unwrap()[0].ignored, vec!["Views: #".to_string()]);
        assert!(d.routine_reset_noise(8, r.id).unwrap() && d.routine_noise(r.id).unwrap().is_empty());
        assert_eq!(d.run_totals(Some(8), 0).unwrap().runs, 0);
        d.routine_set_delivery(8, r.id, "digest").unwrap();
        assert_eq!(d.quiet_nothing_since(1, 8, 0).unwrap(), 1);
        assert!(d.runs_failed(0).unwrap().is_empty());
        // A deleted routine takes its pause with it.
        d.hold_set(&format!("hold:routine:{}", r.id), &Hold { until: now() + 60, why: "cap".into(), waive: None }).unwrap();
        d.routine_delete(8, r.id).unwrap();
        assert!(d.hold(&format!("hold:routine:{}", r.id)).unwrap().is_none());
    }

    #[test]
    fn run_log_and_totals() {
        let d = db();
        let a = d.run_start(1, 3, "message", None, "claude -p").unwrap();
        d.tool_call_log(Some(a), 3, "memory_store", "Remember x", "allow", true, "stored #1").unwrap();
        d.tool_call_log(Some(a), 3, "memory_forget", "Forget #1", "ask", false, "denied").unwrap();
        d.run_finish(a, Ok(&Usage { model: Some("sonnet".into()), tokens_in: 100, tokens_out: 20, cost_usd: Some(0.01), ..Default::default() })).unwrap();
        let b = d.run_start(2, 3, "routine", Some(1), "codex exec").unwrap();
        d.run_finish(b, Err("usage limit")).unwrap();
        d.run_start(1, 4, "message", None, "x").unwrap();

        let runs = d.runs(Some(3), 10).unwrap();
        assert_eq!(runs.iter().map(|r| (r.id, r.status.as_str(), r.tool_calls)).collect::<Vec<_>>(), vec![(b, "error", 0), (a, "ok", 2)]);
        assert_eq!(runs[0].error.as_deref(), Some("usage limit"));
        assert_eq!(d.runs(None, 10).unwrap().len(), 3);
        let t = d.run_totals(Some(3), 0).unwrap();
        assert_eq!((t.runs, t.errors, t.tokens_in, t.tokens_out), (2, 1, 100, 20));
        assert!((t.cost_usd - 0.01).abs() < 1e-9);
        let calls = d.tool_calls(a).unwrap();
        assert_eq!((calls[1].decision.as_str(), calls[1].ok), ("ask", false));
    }

    #[test]
    fn bots_sessions_messages() {
        let d = db();
        let first = d.bots().unwrap();
        assert_eq!((first.len(), first[0].id, first[0].name.as_str()), (1, 1, "Mimir"), "bot 1 seeded");
        let mut b = first[0].clone();
        b.id = 0;
        b.name = "Researcher".into();
        b.tools = "web_fetch, memory_search".into();
        let id = d.bot_save(&b).unwrap();
        let r = d.bot(id).unwrap();
        assert!(r.allows("web_fetch") && !r.allows("memory_forget") && d.bot(1).unwrap().allows("memory_forget"));
        assert_eq!(super::min_gap_minutes("0 9 * * *").unwrap(), 1440);
        assert_eq!(super::min_gap_minutes("*/5 * * * *").unwrap(), 5);
        assert_eq!(super::min_gap_minutes("* * * * * *").unwrap(), 0, "every second");
        // Deleting a chat's history, erasing a memory's copies, exporting.
        let hc = 4242;
        d.log(1, hc, "user", "my secret plan", None).unwrap();
        let run = d.run_start(1, hc, "message", None, "test").unwrap();
        d.tool_call_log(Some(run), hc, "memory_store", "Remember \"Plan\": my secret plan", "allow", true, "stored #1").unwrap();
        d.run_finish(run, Ok(&Default::default())).unwrap();
        let mid = d.store(&format!("chat:{hc}"), "knowledge", "Plan", "my secret plan", "", None).unwrap();
        assert!(d.export_all().unwrap()["messages"].as_array().unwrap().iter().any(|m| m["content"] == "my secret plan"));
        d.erase(&format!("chat:{hc}"), mid).unwrap();
        assert!(d.c().query_row("SELECT count(*) FROM tool_calls WHERE instr(summary, 'secret') > 0", [], |r| r.get::<_, i64>(0)).unwrap() == 0, "the tool log's copy is gone");
        assert_eq!(d.chat_history_clear(hc).unwrap(), 1);
        assert!(d.messages(hc, 10).unwrap().is_empty() && d.runs(Some(hc), 10).unwrap().is_empty());
        let rt = d.routine_create(1, 0, "0 9 * * *", "check the news", false).unwrap();
        assert!(!rt.tainted);
        d.routine_taint(rt.id).unwrap();
        assert!(d.routines(0).unwrap().iter().any(|x| x.id == rt.id && x.tainted), "a routine made after outside content stays marked");
        assert!(super::valid_look("") && super::valid_look(r##"{"shape":"dome","eyes":"wide","extra":"","color":"#4A6FC4"}"##));
        for bad in [r#"{"color":"red"}"#, r#"{"shape":"<b>"}"#, r#"{"other":"x"}"#, "[1]", "nope"] {
            assert!(!super::valid_look(bad), "{bad}");
        }
        let w = Bot { tools: "memory_search, browser__*".into(), ..r.clone() };
        assert!(w.allows("browser__browser_click") && !w.allows("mail__send") && !Bot { tools: "*".into(), ..r.clone() }.allows("x"));

        d.set_session(1, 5, Some("s1")).unwrap();
        d.set_session(id, 5, Some("s2")).unwrap();
        assert_eq!((d.session(1, 5).unwrap().as_deref(), d.session(id, 5).unwrap().as_deref()), (Some("s1"), Some("s2")));
        d.set_session(1, 5, None).unwrap();
        assert_eq!(d.session(1, 5).unwrap(), None);

        d.log(1, 5, "user", "hi A", None).unwrap();
        d.log(id, 5, "user", "hi B", None).unwrap();
        assert_eq!(d.recent(id, 5, 10).unwrap(), vec![("user".to_string(), "hi B".to_string())]);
        assert_eq!(d.messages(5, 10).unwrap().iter().map(|m| m.bot_id).collect::<Vec<_>>(), vec![1, id]);

        d.routine_create(id, 5, "0 9 * * *", "x", false).unwrap();
        d.bot_delete(id).unwrap();
        assert!(d.routines(5).unwrap().is_empty(), "bot's routines go with it");
        assert!(d.bot_delete(1).is_err(), "keep at least one bot");
    }

    #[test]
    fn external_chats() {
        let d = db();
        assert_eq!(d.chat_lookup("slack", "D01").unwrap(), None, "unpaired");
        let s = d.pair_external("slack", "D01", "Slack Willem").unwrap();
        let g = d.pair_external("discord", "1234567890123", "Discord DM").unwrap();
        let t = d.pair_external("telegram", "555", "TG").unwrap();
        assert_eq!((s, g, t), (9_000_000_000_000_000, 9_000_000_000_000_001, 555));
        assert_eq!(d.pair_external("slack", "D01", "renamed").unwrap(), s, "re-pairing is stable");
        assert_eq!(d.chat_lookup("slack", "D01").unwrap(), Some(s));
        assert_eq!(d.chat_lookup("discord", "D01").unwrap(), None, "ids are per channel");
        assert_eq!(d.chat_lookup("telegram", "555").unwrap(), Some(555));
        assert_eq!(d.chat_address(s).unwrap(), Some(("slack".into(), "D01".into())));
        assert_eq!(d.chat_address(555).unwrap(), Some(("telegram".into(), "555".into())));
        assert_eq!(d.chat_address(0).unwrap(), None);
        assert_eq!(d.chat_address(987654).unwrap(), None, "an unpaired chat has no address");
        d.pair(-100555, "Family group").unwrap();
        assert_eq!(d.chat_address(-100555).unwrap(), Some(("telegram".into(), "-100555".into())), "older Telegram chats: the id is the address");
        let chats = d.chats().unwrap();
        assert_eq!(chats.iter().map(|c| c.channel.as_str()).collect::<Vec<_>>(), vec!["telegram", "telegram", "slack", "discord"]);
    }

    #[test]
    fn suggestions_flow() {
        let d = db();
        let sc = "chat:1";
        let a = d.store(sc, "knowledge", "Gym", "Virgin Active Claremont", "", None).unwrap();
        let add = Suggestion { id: 0, action: "add".into(), entry_id: None, kind: "convention".into(), title: "News".into(), body: "3 bullets with links".into(), reason: "asked twice".into(), check: None, tainted: false };
        let s1 = d.suggestion_add(sc, &add).unwrap().unwrap();
        assert_eq!(d.suggestion_add(sc, &add).unwrap(), None, "no duplicate pending suggestions");
        assert_eq!(d.suggestion_add(sc, &Suggestion { title: "news".into(), body: "other".into(), ..add.clone() }).unwrap(), None, "same-title add");
        let upd = Suggestion { action: "update".into(), entry_id: Some(a), kind: "knowledge".into(), title: "Gym".into(), body: "Cancelled gym membership".into(), ..add.clone() };
        let s2 = d.suggestion_add(sc, &upd).unwrap().unwrap();
        let fgt = Suggestion { action: "forget".into(), entry_id: Some(a), ..add.clone() };
        assert_eq!(d.suggestion_add(sc, &fgt).unwrap(), None, "one pending suggestion per memory");
        assert!(d.suggestion_add(sc, &Suggestion { action: "forget".into(), entry_id: Some(999), ..add.clone() }).is_err(), "unknown target");
        assert!(d.suggestion_add(sc, &Suggestion { action: "drop".into(), ..add.clone() }).is_err());
        assert!(d.suggestion_add("chat:2", &upd).is_err(), "can't target another person's memory");
        // Dismissed: not proposed again (same title for adds; same memory + action otherwise).
        let other = Suggestion { title: "Gym".into(), body: "Goes Mondays".into(), ..add.clone() };
        let g = d.suggestion_add(sc, &other).unwrap().unwrap();
        d.suggestion_decide(sc, g, false).unwrap();
        assert_eq!(d.suggestion_add(sc, &other).unwrap(), None, "dismissed stays dismissed");

        assert_eq!(d.suggestion_decide(sc, s1, false).unwrap(), "dismissed");
        assert!(d.suggestion_decide(sc, s2, true).unwrap().starts_with(&format!("#{a} replaced by")));
        let active: Vec<_> = d.entries(sc).unwrap().into_iter().filter(|e| e.status == "active").map(|e| e.body).collect();
        assert_eq!(active, vec!["Cancelled gym membership"]);
        assert!(d.suggestions(sc).unwrap().is_empty());
        assert!(d.suggestion_decide(sc, s2, true).is_err(), "decided once");
    }

    #[test]
    fn lookalikes_of_dismissed_suggestions() {
        let d = db();
        let sc = "chat:1";
        let sug = |title: &str, body: &str, reason: &str| Suggestion { id: 0, action: "add".into(), entry_id: None, kind: "convention".into(), title: title.into(), body: body.into(), reason: reason.into(), check: None, tainted: false };
        let dismiss = |s: &Suggestion| {
            let id = d.suggestion_add(sc, s).unwrap().unwrap();
            d.suggestion_decide(sc, id, false).unwrap();
        };
        dismiss(&sug("Emoji in replies", "When replying, add a few emoji", "seen twice"));
        // Once: shown, and the card says so.
        let again = d.suggestion_add(sc, &sug("Use emoji", "When replying casually, add emoji", "again")).unwrap().unwrap();
        let reason = d.suggestions(sc).unwrap().into_iter().find(|s| s.id == again).unwrap().reason;
        assert!(reason.contains("You dismissed a similar one on") && reason.contains("“Emoji in replies”"), "{reason}");
        d.suggestion_decide(sc, again, false).unwrap();
        // Twice: not shown at all.
        assert_eq!(d.suggestion_add(sc, &sug("Emoji please", "Add emoji when replying", "third time")).unwrap(), None);
        // Something else, or something you imported yourself, still comes through.
        assert!(d.suggestion_add(sc, &sug("Metric units", "Use Celsius and kilometres", "said so")).unwrap().is_some());
        assert!(d.suggestion_add(sc, &sug("Emoji ok", "Add emoji when replying to friends", "Imported: emoji")).unwrap().is_some());
        // The same change, worded twice in one tidy-up (seen live with Sonnet): one card.
        let sc = "chat:2";
        assert!(d.suggestion_add(sc, &sug("Use metric units only", "User wants metric units only in responses (meters, Celsius, km, etc.) — no feet, miles, or Fahrenheit.", "x")).unwrap().is_some());
        assert_eq!(d.suggestion_add(sc, &sug("Metric units only", "When answering questions with measurements, use metric units only (no feet, miles, or Fahrenheit).", "y")).unwrap(), None);
    }

    #[test]
    fn cron_and_routines() {
        let t0 = Local.with_ymd_and_hms(2026, 10, 2, 9, 0, 0).unwrap().timestamp(); // Friday 09:00
        let next = next_fire("0 8 * * Mon-Fri", t0).unwrap();
        assert_eq!(Local.timestamp_opt(next, 0).unwrap().format("%a %H:%M").to_string(), "Mon 08:00");
        assert!(next_fire("not a cron", t0).is_err());

        let d = db();
        let r = d.routine_create(2, 7, "*/5 * * * *", "ping", true).unwrap();
        assert!(d.routines(7).unwrap()[0].quiet);
        assert!(d.routine_create(2, 7, "* * * * *", "  ", false).is_err());
        assert!(d.due(2, now()).unwrap().is_empty());
        assert_eq!(d.due(2, r.next_fire).unwrap().len(), 1);
        assert!(d.due(1, r.next_fire).unwrap().is_empty(), "other bot doesn't fire it");
        d.routine_advance(&r).unwrap();
        assert!(d.routines(7).unwrap()[0].next_fire > r.next_fire);
        // Paused routines aren't due; resuming schedules from now.
        assert!(d.routine_update(7, r.id, "*/5 * * * *", "ping", true, true).unwrap());
        assert!(d.due(2, i64::MAX).unwrap().is_empty());
        assert!(d.routine_update(7, r.id, "0 9 * * *", "pong", false, false).unwrap());
        let r2 = &d.routines(7).unwrap()[0];
        assert_eq!((r2.cron.as_str(), r2.prompt.as_str(), r2.quiet, r2.paused), ("0 9 * * *", "pong", false, false));
        assert!(r2.next_fire > now() && d.due(2, i64::MAX).unwrap().len() == 1);
        assert!(!d.routine_update(8, r.id, "0 9 * * *", "x", false, false).unwrap(), "other chat");
        assert!(d.routine_update(7, r.id, "nonsense", "x", false, false).is_err());
        assert!(!d.routine_delete(8, r.id).unwrap(), "other chat can't delete");
        assert!(d.routine_delete(7, r.id).unwrap());
    }

    #[test]
    fn conversation_taint() {
        let d = db();
        assert!(!d.conv_tainted(1, 5).unwrap());
        d.conv_taint(1, 5).unwrap();
        d.conv_taint(1, 5).unwrap();
        assert!(d.conv_tainted(1, 5).unwrap() && !d.conv_tainted(2, 5).unwrap() && !d.conv_tainted(1, 6).unwrap());
        d.set_session(1, 5, Some("s1")).unwrap();
        assert!(d.conv_tainted(1, 5).unwrap(), "continuing the conversation keeps it");
        d.set_session(1, 5, None).unwrap();
        assert!(!d.conv_tainted(1, 5).unwrap(), "a fresh conversation clears it");
    }

    #[test]
    fn busy_lists_runs_in_progress() {
        let d = db();
        let r = d.run_start(1, 3, "routine", Some(1), "claude").unwrap();
        d.run_start(2, 3, "tidy", None, "claude").unwrap();
        let done = d.run_start(1, 4, "message", None, "claude").unwrap();
        d.run_finish(done, Ok(&Usage::default())).unwrap();
        assert_eq!(d.busy().unwrap(), vec![(2, 3, "tidy".to_string()), (1, 3, "routine".to_string())]);
        d.run_finish(r, Err("x")).unwrap();
        assert_eq!(d.busy().unwrap().len(), 1);
    }

}
