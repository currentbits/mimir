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

mod approvals;
mod bots;
mod cards;
mod chats;
mod goals;
mod meetings;
mod memory;
mod messages;
mod outbox;
mod routines;
mod runs;
mod settings;
mod skills;
mod suggestions;

pub use approvals::*;
pub use bots::*;
pub use cards::*;
pub use chats::*;
pub use goals::*;
pub use memory::*;
pub use messages::*;
pub use outbox::*;
pub use routines::*;
pub use runs::*;
pub use skills::*;
pub use suggestions::*;

pub const TYPES: [&str; 3] = ["convention", "knowledge", "task"];

#[derive(Clone)]
/// Connection + the data directory it lives in.
pub struct Db(Arc<Mutex<Connection>>, Arc<std::path::PathBuf>);

/// Set on every open (outside the migration's transaction, where journal_mode can't change).
const PRAGMAS: &str = "
PRAGMA journal_mode=WAL;
-- Deleted rows are overwritten, not left in free pages (what incognito chats asked, erased memories).
PRAGMA secure_delete=ON;
";

/// The schema version this build writes (`PRAGMA user_version`). A schema change bumps it and adds a step to
/// `migrate`; each step runs once per database.
pub(crate) const VERSION: i64 = 13;

const SCHEMA: &str = "
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

/// The chat part of a Telegram address ("chat" or "chat:topic").
pub(crate) fn tg_chat(external: &str) -> &str {
    external.split(':').next().unwrap_or(external)
}

pub fn now() -> i64 {
    Local::now().timestamp()
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

pub(crate) fn user_version(c: &Connection) -> Result<i64> {
    Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// A database a newer Mimir wrote: refused whole, never half-migrated by an older build.
pub(crate) fn check_not_newer(c: &Connection) -> Result<()> {
    if user_version(c)? > VERSION {
        let by: Option<String> =
            c.query_row("SELECT value FROM settings WHERE key='schema_written_by'", [], |r| r.get(0)).optional().ok().flatten();
        let by = by.map(|v| format!("Mimir {v}")).unwrap_or_else(|| "a newer Mimir".into());
        bail!("This data was written by {by}, and this app is Mimir {}. Update the app to open it.", env!("CARGO_PKG_VERSION"));
    }
    Ok(())
}

/// Copies the database aside before it's migrated (`mimir.db.before-<version>`), keeping the last 2.
fn backup_before(c: &Connection, path: &Path) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let prefix = format!("{}.before-", path.file_name().and_then(|n| n.to_str()).unwrap_or("mimir.db"));
    let target = dir.join(format!("{prefix}{VERSION}"));
    // An existing copy is the older one: a second process racing this upgrade would copy the upgraded file over it.
    if !target.exists() {
        c.execute("VACUUM INTO ?1", [target.to_string_lossy()]).context("copying the database aside before upgrading it")?;
    }
    let mut old: Vec<(i64, std::path::PathBuf)> = std::fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| Some((e.file_name().to_str()?.strip_prefix(&prefix)?.parse().ok()?, e.path())))
        .collect();
    old.sort();
    for (_, p) in old.iter().rev().skip(2) {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

/// Brings a database up to `VERSION`, one step per version it's behind. Runs inside one transaction.
fn migrate(c: &Connection) -> Result<()> {
    // Another process may have migrated it while this one waited for the lock.
    check_not_newer(c)?;
    let v = user_version(c)?;
    if v < 1 {
        migrate_v1(c)?;
    }
    if v < 2 {
        migrate_v2(c)?;
    }
    if v < 3 {
        migrate_v3(c)?;
    }
    if v < 4 {
        migrate_v4(c)?;
    }
    if v < 5 {
        migrate_v5(c)?;
    }
    if v < 6 {
        migrate_v6(c)?;
    }
    if v < 7 {
        migrate_v7(c)?;
    }
    if v < 8 {
        migrate_v8(c)?;
    }
    if v < 9 {
        migrate_v9(c)?;
    }
    if v < 10 {
        migrate_v10(c)?;
    }
    if v < 11 {
        migrate_v11(c)?;
    }
    if v < 12 {
        migrate_v12(c)?;
    }
    if v < 13 {
        migrate_v13(c)?;
    }
    c.execute(
        "INSERT INTO settings(key, value) VALUES ('schema_written_by', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [env!("CARGO_PKG_VERSION")],
    )?;
    c.execute_batch(&format!("PRAGMA user_version = {VERSION}"))?;
    Ok(())
}

/// Version 1: the schema as of Mimir 0.1.5, from an empty file or any older build (they kept no version and
/// checked every column on each open).
fn migrate_v1(c: &Connection) -> Result<()> {
    c.execute_batch(SCHEMA)?;
    // Additive columns on DBs created by older builds.
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
            "ALTER TABLE suggestions RENAME TO suggestions_old; {new}; INSERT INTO suggestions SELECT * FROM suggestions_old; DROP TABLE suggestions_old;"
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
    Ok(())
}

/// Version 2: indexes for the messages poll (a chat's actions since a run) and a chat's latest messages.
fn migrate_v2(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE INDEX IF NOT EXISTS tool_calls_chat ON tool_calls(chat_id, run_id);
         CREATE INDEX IF NOT EXISTS messages_chat ON messages(chat_id, id);",
    )?;
    Ok(())
}

/// Version 3: memories one bot keeps to itself (0: all bots, like skills); the household (memories you and chosen
/// guests share) and family groups any household member can talk in.
fn migrate_v3(c: &Connection) -> Result<()> {
    // A database a branch build already gave these columns (or a re-run ladder) keeps them.
    for (table, col) in [("entries", "for_bot"), ("chats", "household"), ("chats", "family")] {
        if !c.prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name='{col}'"))?.exists([])? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} INTEGER NOT NULL DEFAULT 0"))?;
        }
    }
    Ok(())
}

/// Version 4: the person's "come through right away if …" for a routine that would otherwise wait, and the runs
/// it let through (counted for the daily cap).
fn migrate_v4(c: &Connection) -> Result<()> {
    for (table, col, ty) in [("routines", "urgent", "TEXT NOT NULL DEFAULT ''"), ("runs", "urgent", "INTEGER NOT NULL DEFAULT 0")] {
        if !c.prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name='{col}'"))?.exists([])? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {ty}"))?;
        }
    }
    Ok(())
}

/// Version 5: a routine's ask also sent to the decisions chat: that chat, and its card there until it shows the outcome.
fn migrate_v5(c: &Connection) -> Result<()> {
    for (col, decl) in [("mirror_chat", "INTEGER"), ("mirror_ref", "TEXT")] {
        if !c.prepare(&format!("SELECT 1 FROM pragma_table_info('approvals') WHERE name='{col}'"))?.exists([])? {
            c.execute_batch(&format!("ALTER TABLE approvals ADD COLUMN {col} {decl}"))?;
        }
    }
    Ok(())
}

/// Version 6: pinned cards a bot keeps up to date (a shopping list, bills due), written by a tool or a routine.
fn migrate_v6(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS cards(id INTEGER PRIMARY KEY, bot_id INTEGER NOT NULL, chat_id INTEGER NOT NULL, routine_id INTEGER,
           title TEXT NOT NULL, body TEXT NOT NULL, by_you INTEGER NOT NULL DEFAULT 0, tainted INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS cards_chat ON cards(chat_id);",
    )?;
    Ok(())
}

/// Version 7: what came for a routine started by an event (a matching email, a webhook call), until a run has seen it.
fn migrate_v7(c: &Connection) -> Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS routine_events(id INTEGER PRIMARY KEY, routine_id INTEGER NOT NULL, text TEXT NOT NULL, ts INTEGER NOT NULL)")?;
    Ok(())
}

/// Version 8: messages to chat apps kept until they're sent, so a crash or an update in between doesn't lose them.
fn migrate_v8(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS outbox(id INTEGER PRIMARY KEY, bot_id INTEGER NOT NULL, chat_id INTEGER NOT NULL, kind TEXT NOT NULL,
           ext TEXT NOT NULL, draft TEXT, text TEXT NOT NULL, rows TEXT NOT NULL DEFAULT '[]', files TEXT NOT NULL DEFAULT '[]', run_id INTEGER,
           created_at INTEGER NOT NULL, claimed_by TEXT, claimed_at INTEGER, tries INTEGER NOT NULL DEFAULT 0, error TEXT);
         CREATE INDEX IF NOT EXISTS outbox_bot ON outbox(bot_id, id);",
    )?;
    Ok(())
}

/// Version 9: when a repeating routine ends (0 = never), and a one-off the bot booked on its own to check back.
fn migrate_v9(c: &Connection) -> Result<()> {
    for col in ["ends", "followup"] {
        if !c.prepare(&format!("SELECT 1 FROM pragma_table_info('routines') WHERE name='{col}'"))?.exists([])? {
            c.execute_batch(&format!("ALTER TABLE routines ADD COLUMN {col} INTEGER NOT NULL DEFAULT 0"))?;
        }
    }
    Ok(())
}

/// Version 10: tasks handed to another bot that report back later (ask_bot's `later`), until the result is told.
fn migrate_v10(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS delegations(id INTEGER PRIMARY KEY, bot_id INTEGER NOT NULL, to_bot INTEGER NOT NULL, chat_id INTEGER NOT NULL,
           conv INTEGER NOT NULL, run_id INTEGER, task TEXT NOT NULL, unattended INTEGER NOT NULL DEFAULT 0, tainted INTEGER NOT NULL DEFAULT 0,
           status TEXT NOT NULL DEFAULT 'pending', reply TEXT, failed INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL)",
    )?;
    Ok(())
}

/// Version 11: a chat's goal, and the routines, follow-ups, handed-over tasks and runs that belong to it.
fn migrate_v11(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS goals(id INTEGER PRIMARY KEY, bot_id INTEGER NOT NULL, chat_id INTEGER NOT NULL UNIQUE, text TEXT NOT NULL,
           done_when TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','paused','blocked','done')),
           progress TEXT NOT NULL DEFAULT '', why TEXT NOT NULL DEFAULT '', budget INTEGER NOT NULL DEFAULT 0, budget_from INTEGER NOT NULL DEFAULT 0,
           tainted INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
    )?;
    for table in ["routines", "delegations", "runs"] {
        if !c.prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name='goal_id'"))?.exists([])? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN goal_id INTEGER"))?;
        }
    }
    Ok(())
}

/// Version 12: an approval card's text you can edit before it goes (an email's subject and body), and why you said no.
fn migrate_v12(c: &Connection) -> Result<()> {
    for col in ["draft", "reason"] {
        if !c.prepare(&format!("SELECT 1 FROM pragma_table_info('approvals') WHERE name='{col}'"))?.exists([])? {
            c.execute_batch(&format!("ALTER TABLE approvals ADD COLUMN {col} TEXT"))?;
        }
    }
    Ok(())
}

/// Version 13: cache writes per run (#10), and what a CLI session already holds (#218): the memories recalled into
/// it (comma-separated ids, so they aren't sent again) and a hash of its instructions ('' = unknown, from before).
fn migrate_v13(c: &Connection) -> Result<()> {
    for (table, col, ty) in [("runs", "cache_write", "INTEGER NOT NULL DEFAULT 0"), ("sessions", "sent", "TEXT NOT NULL DEFAULT ''"), ("sessions", "prompt", "TEXT NOT NULL DEFAULT ''")] {
        if !c.prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name='{col}'"))?.exists([])? {
            c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {ty}"))?;
        }
    }
    Ok(())
}

impl Db {
    /// Opens the database, migrating it first if an older Mimir wrote it. An up-to-date database only reads its
    /// version: the tools process opens it on every turn, and mustn't compete with the app for the write lock.
    pub fn open(path: &Path) -> Result<Self> {
        let c = Connection::open(path)?;
        c.busy_timeout(Duration::from_secs(5))?;
        c.execute_batch(PRAGMAS)?;
        check_not_newer(&c)?;
        if user_version(&c)? < VERSION {
            if c.prepare("SELECT 1 FROM sqlite_master WHERE name='entries'")?.exists([])? {
                backup_before(&c, path)?;
            }
            c.execute_batch("BEGIN IMMEDIATE")?;
            if let Err(e) = migrate(&c) {
                let _ = c.execute_batch("ROLLBACK");
                return Err(e.context(format!("upgrading the database {}", path.display())));
            }
            c.execute_batch("COMMIT")?;
        }
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| ".".into());
        Ok(Db(Arc::new(Mutex::new(c)), Arc::new(dir)))
    }

    /// Closes out runs a crash left 'running'. Once at startup, before the scheduler's first pass, not on every open.
    pub fn close_interrupted(&self) -> Result<()> {
        self.c().execute("UPDATE runs SET status='error', error='interrupted', ended_at=started_at WHERE status='running'", [])?;
        // A one-off still parked at IDLE (routine_advance) was cut off mid-run. It may have done part of it (sent an
        // email), so it isn't run again on its own: it waits in Needs you for the person to say run it again.
        let cut: Vec<i64> = {
            let c = self.c();
            let mut st = c.prepare("SELECT id FROM routines WHERE once=1 AND cron='' AND next_fire >= ?1")?;
            st.query_map([IDLE], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        for id in cut {
            let key = format!("hold:routine:{id}");
            // One that failed already waits for the person, and says so.
            if self.hold(&key)?.is_none() {
                self.hold_set(&key, &Hold { until: IDLE, why: "it was interrupted, and may have done part of this already".into(), waive: None })?;
            }
        }
        // ponytail: a handed-over task Mimir stopped in the middle isn't run again (it may have sent half of something):
        // the bot that asked says it didn't finish. Assumes one Mimir per data folder, like the rest.
        self.c().execute(
            "UPDATE delegations SET status='done', failed=1, reply='Mimir stopped while it was on this task, so it didn''t finish.' WHERE status='running'",
            [],
        )?;
        Ok(())
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

    pub(crate) fn c(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
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
    fn a_reused_routine_id_starts_without_the_old_sensitive_mark() {
        let d = db();
        // Made after a sensitive read, then gone before it ever ran (its bot deleted): its id is given out again.
        let r = d.routine_create(1, 9, "0 9 * * *", "check the bank", false).unwrap();
        d.set_setting(&crate::tools::routine_private_key(r.id), "bank").unwrap();
        d.c().execute("DELETE FROM routines WHERE id=?1", [r.id]).unwrap();
        let next = d.routine_once(1, 9, now() + 600, "water the plants", false).unwrap();
        assert_eq!(next.id, r.id);
        assert_eq!(d.setting(&crate::tools::routine_private_key(next.id)).unwrap(), None);
    }

    #[test]
    fn schema_version_ladder() {
        let dir = std::env::temp_dir().join(format!("mimir-ladder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("mimir.db");
        // A database as 0.1.4 left it: no version, a column it didn't have yet.
        drop(Db::open(&p).unwrap());
        Connection::open(&p).unwrap().execute_batch("PRAGMA user_version = 0; ALTER TABLE runs DROP COLUMN estimated; DELETE FROM settings;").unwrap();

        let d = Db::open(&p).unwrap();
        assert_eq!(user_version(&d.c()).unwrap(), VERSION);
        assert!(d.c().prepare("SELECT estimated FROM runs").is_ok(), "migrated");
        assert!(dir.join(format!("mimir.db.before-{VERSION}")).exists(), "copied aside first");
        drop(d);

        // Up to date: opens while another process holds the write lock, so it wrote nothing.
        let other = Connection::open(&p).unwrap();
        other.execute_batch("BEGIN IMMEDIATE").unwrap();
        let t = std::time::Instant::now();
        drop(Db::open(&p).unwrap());
        assert!(t.elapsed() < Duration::from_secs(1), "waited for the write lock");
        other.execute_batch("ROLLBACK").unwrap();

        // Only the last 2 copies are kept.
        for v in [-3, -2, -1] {
            std::fs::write(dir.join(format!("mimir.db.before-{v}")), "").unwrap();
        }
        Connection::open(&p).unwrap().execute_batch("PRAGMA user_version = 0").unwrap();
        drop(Db::open(&p).unwrap());
        let mut kept: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.contains(".before-")).collect();
        kept.sort();
        assert_eq!(kept, vec!["mimir.db.before--1".to_string(), format!("mimir.db.before-{VERSION}")]);

        // Version 1 → 2 adds the indexes.
        Connection::open(&p).unwrap().execute_batch("DROP INDEX tool_calls_chat; DROP INDEX messages_chat; PRAGMA user_version = 1").unwrap();
        let d = Db::open(&p).unwrap();
        let n: i64 = d.c().query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('tool_calls_chat', 'messages_chat')", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
        drop(d);

        // Version 10 → 13: the goals (11), an approval's draft and reason (12), cache writes and what a session holds (13).
        Connection::open(&p)
            .unwrap()
            .execute_batch(
                "DROP TABLE goals; ALTER TABLE routines DROP COLUMN goal_id; ALTER TABLE delegations DROP COLUMN goal_id; ALTER TABLE runs DROP COLUMN goal_id;
                 ALTER TABLE approvals DROP COLUMN draft; ALTER TABLE approvals DROP COLUMN reason;
                 ALTER TABLE runs DROP COLUMN cache_write; ALTER TABLE sessions DROP COLUMN sent; ALTER TABLE sessions DROP COLUMN prompt; PRAGMA user_version = 10",
            )
            .unwrap();
        let d = Db::open(&p).unwrap();
        assert_eq!(user_version(&d.c()).unwrap(), 13);
        assert!(d.c().prepare("SELECT goal_id FROM runs").is_ok() && d.c().prepare("SELECT id FROM goals").is_ok(), "v11 ran");
        assert!(d.c().prepare("SELECT draft, reason FROM approvals").is_ok(), "v12 ran");
        assert!(d.c().prepare("SELECT cache_write FROM runs").is_ok() && d.c().prepare("SELECT sent, prompt FROM sessions").is_ok(), "v13 ran");
        drop(d);
        // A new database starts at the latest.
        let fresh = dir.join("fresh.db");
        assert_eq!(user_version(&Db::open(&fresh).unwrap().c()).unwrap(), 13);

        // Written by a newer Mimir: refused, untouched.
        Connection::open(&p).unwrap().execute_batch("PRAGMA user_version = 99; UPDATE settings SET value='9.9.9' WHERE key='schema_written_by'").unwrap();
        let err = Db::open(&p).err().unwrap().to_string();
        assert!(err.contains("Mimir 9.9.9") && err.contains("Update the app"), "{err}");
        assert_eq!(user_version(&Connection::open(&p).unwrap()).unwrap(), 99);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn memory_roundtrip_supersede_forget_and_scope() {
        let d = db();
        let a = d.store("chat:1", "convention", "Coffee", "User drinks oat flat whites", "message", None).unwrap();
        d.store("chat:1", "knowledge", "Dentist", "Dentist is Dr. Lee in Cape Town", "", None).unwrap();
        d.store("chat:2", "knowledge", "Other user", "Dentist is Dr. Who", "", None).unwrap();
        assert_eq!(d.store("chat:1", "convention", "Coffee", "User drinks oat flat whites", "x", None).unwrap(), a, "dedup");
        assert!(d.store("chat:1", "bogus", "t", "b", "", None).is_err());

        let hits = d.search("chat:1", 0, "who is my dentist?", 5).unwrap();
        assert_eq!(hits.len(), 1, "scoped + relevance gated");
        assert!(hits[0].body.contains("Dr. Lee"));

        let b = d.store("chat:1", "convention", "Coffee", "User switched to black coffee", "message", Some(a)).unwrap();
        let hits = d.search("chat:1", 0, "coffee", 5).unwrap();
        assert_eq!(hits.iter().map(|e| e.id).collect::<Vec<_>>(), vec![b], "superseded entry hidden");
        assert!(d.store("chat:1", "knowledge", "x", "y", "", Some(a)).is_err(), "can't supersede twice");

        assert!(d.forget("chat:1", b).unwrap());
        assert!(d.search("chat:1", 0, "coffee", 5).unwrap().is_empty());
        assert!(d.search("chat:1", 0, "\"); DROP TABLE entries; --", 5).is_ok(), "FTS input is sanitized");
        assert_eq!(d.bootstrap("chat:1", 0).unwrap().len(), 1);
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
        assert!(!d.bootstrap(s, 0).unwrap().iter().any(|e| e.id == old));
        assert!(d.pin(s, old, true).unwrap());
        assert_eq!(d.bootstrap(s, 0).unwrap()[0].id, old, "pinned comes first");
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
        assert!(d.search(s, 0, "negative", 5).unwrap().is_empty());
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
        assert!(d.bootstrap(s, 0).unwrap().is_empty(), "out of standing memory");
        assert_eq!(d.search(s, 0, "Observatory", 5).unwrap()[0].id, id, "still found by search");
        assert!(d.suggestion_add(s, &sug("demote")).is_err(), "already in the background");
        let fixed = d.store(s, "knowledge", "Old flat", "Lived in Woodstock in 2019", "", Some(id)).unwrap();
        assert!(d.entries(s).unwrap().iter().any(|e| e.id == fixed && e.background), "a correction stays in the background");
        assert!(d.pin(s, fixed, true).unwrap());
        assert_eq!(d.bootstrap(s, 0).unwrap()[0].id, fixed, "pinning brings it back");
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
        d.search(s, 0, "wifi hint", 5).unwrap();
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
    fn earlier_messages_page_back() {
        let d = db();
        for i in 0..5 {
            d.log(1, 7, "user", &format!("m{i}"), None).unwrap();
        }
        let text = |v: Vec<Message>| v.into_iter().map(|m| m.content).collect::<Vec<_>>().join(" ");
        let last = d.messages_of(7, None, None, 0, 2).unwrap();
        assert_eq!(text(last.clone()), "m3 m4");
        assert_eq!(text(d.messages_of(7, None, Some(last[0].id), 0, 2).unwrap()), "m1 m2", "the page before");
        // A search hit's id: everything from it on.
        let hit = d.chat_search(7, "m1", None, (0, i64::MAX), 1).unwrap()[0].id;
        assert_eq!(text(d.messages_of(7, None, None, hit, 100).unwrap()), "m1 m2 m3 m4");
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
        assert!(d.last_said(1, 11).unwrap().is_some_and(|t| now() - t < 5));
        assert_eq!(d.last_said(2, 11).unwrap(), None, "another bot's");
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
        assert_eq!(d.blame_last_reply(1, 7, 7, "no").unwrap(), 0, "no reply yet");
        let run = d.run_start(1, 7, "message", None, "test").unwrap();
        d.run_recalled(run, &[used, other]).unwrap();
        d.log(1, 7, "assistant", "See you at the Paarl office", Some(run)).unwrap();
        assert_eq!(d.blame_last_reply(1, 7, 7, "no, wrong").unwrap(), 1, "only this chat's memory");
        let count = |scope: &str, id| d.entries(scope).unwrap().into_iter().find(|e| e.id == id).unwrap().missteps;
        assert_eq!((count("chat:7", used), count("chat:8", other)), (1, 0));
        // What the reply used, under it and for /why: this chat's memory only, and what happened to it since.
        let skill = d.skill_propose("chat:7", 1, "Book padel", "When booking padel", "1. Open the site", None).unwrap().unwrap();
        d.skill_used(skill, Some(run)).unwrap();
        d.tool_call_log(Some(run), 7, "web_fetch", "Read https://paarl.example", "allow", true, "ok").unwrap();
        d.tool_call_log(Some(run), 7, "web_fetch", "Read https://down.example", "allow", false, "404").unwrap();
        let u = d.messages(7, 1).unwrap().pop().unwrap().used.unwrap();
        assert_eq!((u.memories.len(), u.memories[0].id, u.skills.len(), u.pages.clone()), (1, used, 1, vec!["Read https://paarl.example".to_string()]));
        d.forget("chat:7", used).unwrap();
        assert_eq!(d.last_reply_used(1, 7).unwrap().unwrap().memories[0].status, "forgotten");
        assert!(d.last_reply_used(2, 7).unwrap().is_none(), "another bot's reply");
        // A standing preference the correction is about gets one too (it's in every reply, never "recalled").
        let bullets = d.store("chat:7", "convention", "News format", "Give tech news as three bullet points", "", None).unwrap();
        let tone = d.store("chat:7", "convention", "Tone", "Keep it friendly", "", None).unwrap();
        assert_eq!(d.blame_last_reply(1, 7, 7, "Please, no bullet points for this").unwrap(), 2, "the recalled memory and the bullets rule");
        assert_eq!((count("chat:7", bullets), count("chat:7", tone)), (1, 0));
        // Lines in a prompt carry the date they were saved.
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert!(d.bootstrap("chat:7", 0).unwrap().iter().all(|e| e.line().contains(&format!(", saved {today}]"))));
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
        let b = d.bootstrap(s, 0).unwrap();
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

        // A sign-in card: done once, by its form (not an Approve), and not after it ran out.
        let s = d.approval_create(1, 5, "computer_signin", "Sign in to github.com", 300, "signin", Some("github.com")).unwrap();
        assert!(d.approval_decide(s, true, None).unwrap().is_none(), "approving isn't signing in");
        assert_eq!(d.approval_claim(s).unwrap().unwrap().site.as_deref(), Some("github.com"));
        assert!(d.approval_claim(s).unwrap().is_none(), "the link works once");
        assert!(d.approval_decide(s, false, None).unwrap().is_none(), "nor can it be turned down while signing in");
        d.approval_signed_in(s, true).unwrap();
        assert_eq!(d.approval_status(s).unwrap(), "approved");
        assert!(d.approval_claim(a).unwrap().is_none(), "only sign-in cards");
        let late = d.approval_create(1, 5, "computer_signin", "Sign in", 0, "signin", Some("github.com")).unwrap();
        assert!(d.approval_claim(late).unwrap().is_none(), "not once it ran out");
        let no = d.approval_create(1, 5, "computer_signin", "Sign in", 300, "signin", Some("github.com")).unwrap();
        assert_eq!(d.approval_decide(no, false, None).unwrap().unwrap().status, "denied", "Can't now still works");

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
    fn a_fallback_runs_while_the_login_is_held() {
        let d = db();
        let mut b = d.bot(1).unwrap();
        b.provider = "claude".into();
        d.bot_save(&b).unwrap();
        assert!(d.set_fallback(1, "claude", "").is_err(), "the same login is at the same limit");
        d.set_fallback(1, "openrouter", " openrouter/auto ").unwrap();
        assert_eq!(d.fallback(1).unwrap(), Some(("openrouter".into(), "openrouter/auto".into())));
        assert_eq!(d.on_fallback(1, now()).unwrap(), None, "not held: its own account");
        let until = now() + 600;
        d.hold_account(1, "claude", &Hold { until, why: "limit".into(), waive: None }).unwrap();
        assert_eq!(d.on_fallback(1, now()).unwrap(), Some(("openrouter".into(), "openrouter/auto".into(), until)));
        assert_eq!(d.on_fallback(1, until).unwrap(), None, "the limit reset: back on its own");
        // A daily cap applies whatever the account.
        d.hold_set("hold:1", &Hold { until, why: "cap".into(), waive: Some("cap_waived:bot:1".into()) }).unwrap();
        assert_eq!(d.on_fallback(1, now()).unwrap(), None);
        // The fallback at its own limit: wait for the bot's own again.
        d.hold_account(1, "claude", &Hold { until, why: "limit".into(), waive: None }).unwrap();
        d.fallback_spent(1, until).unwrap();
        assert_eq!(d.on_fallback(1, now()).unwrap(), None);
        d.set_fallback(1, "", "").unwrap();
        assert_eq!(d.fallback(1).unwrap(), None);
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
    fn a_routine_sees_its_last_results() {
        let d = db();
        let r = d.routine_create(1, 8, "0 8 * * *", "brief me", false).unwrap();
        let u = Usage::default();
        for (i, (said, outside)) in [("Mon news", false), ("", false), ("Tue news", true), ("Wed news", false)].into_iter().enumerate() {
            let run = d.run_start(1, 8, "routine", Some(r.id), "x").unwrap();
            d.run_reply(run, said).unwrap();
            if outside {
                d.run_taint(run).unwrap();
            }
            d.run_finish(run, if i == 3 { Err("failed") } else { Ok(&u) }).unwrap();
        }
        // A rehearsal isn't one of its results.
        let run = d.run_start(1, 8, "rehearsal", Some(r.id), "x").unwrap();
        d.run_reply(run, "rehearsed").unwrap();
        d.run_finish(run, Ok(&u)).unwrap();
        // Newest first; not the empty one, the failed one or the rehearsal; the outside one says so.
        let v: Vec<_> = d.routine_results(r.id, 3).unwrap().into_iter().map(|(_, s, t)| (s, t)).collect();
        assert_eq!(v, vec![("Tue news".to_string(), true), ("Mon news".to_string(), false)]);
        assert_eq!(d.routine_results(r.id, 1).unwrap().len(), 1);
        // Deleted, the next routine (another chat's) doesn't get its id, or its results.
        d.routine_delete(8, r.id).unwrap();
        let n = d.routine_create(1, 9, "0 8 * * *", "brief me", false).unwrap();
        assert!(n.id > r.id);
        assert!(d.routine_results(n.id, 3).unwrap().is_empty());
        // One a database from before handed the same id: another chat's runs still aren't its own.
        d.c().execute("UPDATE runs SET routine_id=?1", [n.id]).unwrap();
        assert!(d.routine_results(n.id, 3).unwrap().is_empty());
    }

    #[test]
    fn urgent_rule_and_daily_cap() {
        let d = db();
        let r = d.routine_create(1, 8, "0 * * * *", "check my flight", true).unwrap();
        assert!(d.routine_set_urgent(8, r.id, "  the flight\n is <b>delayed</b> ").unwrap());
        assert_eq!(d.routines(8).unwrap()[0].urgent, "the flight is bdelayed/b", "one line, no tags");
        assert!(d.routine_set_urgent(8, r.id, &"x".repeat(201)).is_err());
        assert!(!d.routine_set_urgent(9, r.id, "x").unwrap(), "another chat's routine");
        let runs: Vec<i64> = (0..=URGENT_PER_DAY).map(|_| d.run_start(1, 8, "routine", Some(r.id), "claude -p").unwrap()).collect();
        let day = now() - 60;
        for &run in &runs[..URGENT_PER_DAY as usize] {
            assert!(d.run_take_urgent(run, day).unwrap() && d.run_urgent(run).unwrap());
        }
        let last = *runs.last().unwrap();
        assert!(!d.run_take_urgent(last, day).unwrap() && !d.run_urgent(last).unwrap(), "over the day's cap");
        assert!(d.run_take_urgent(last, now() + 60).unwrap(), "a new day");
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
        d.set_session_held(1, 5, &[3, 7], "h1").unwrap();
        d.set_session(1, 5, Some("s1b")).unwrap();
        assert_eq!(d.session_held(1, 5).unwrap(), (vec![3, 7], "h1".into()), "kept when the CLI names the resumed session anew");
        d.set_session(1, 5, None).unwrap();
        assert_eq!(d.session(1, 5).unwrap(), None);
        assert_eq!(d.session_held(1, 5).unwrap(), (vec![], String::new()), "a fresh conversation starts with nothing held");
        // API history: its start moves 10 messages at a time.
        let lens: Vec<usize> = (0..25).map(|i| { d.log(1, 77, "user", &i.to_string(), None).unwrap(); d.recent_stepped(1, 77).unwrap().len() }).collect();
        assert_eq!(&lens[..12], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        assert_eq!((lens[18], lens[19], lens[24]), (19, 10, 15));
        assert_eq!(d.recent_stepped(1, 77).unwrap()[0].1, "10");

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
        assert!(d.routine_update(7, r.id, "*/5 * * * *", "ping", true, true, None).unwrap());
        assert!(d.due(2, i64::MAX).unwrap().is_empty());
        assert!(d.routine_update(7, r.id, "0 9 * * *", "pong", false, false, None).unwrap());
        let r2 = &d.routines(7).unwrap()[0];
        assert_eq!((r2.cron.as_str(), r2.prompt.as_str(), r2.quiet, r2.paused), ("0 9 * * *", "pong", false, false));
        assert!(r2.next_fire > now() && d.due(2, i64::MAX).unwrap().len() == 1);
        assert!(!d.routine_update(8, r.id, "0 9 * * *", "x", false, false, None).unwrap(), "other chat");
        assert!(d.routine_update(7, r.id, "nonsense", "x", false, false, None).is_err());
        assert!(!d.routine_delete(8, r.id).unwrap(), "other chat can't delete");
        assert!(d.routine_delete(7, r.id).unwrap());
        // Timed off the calendar: it waits until the scheduler has read the calendar, and only where it's private.
        assert!(d.routine_create(2, 7, "calendar:60:outside", "brief me", false).unwrap_err().to_string().contains("private"));
        let c = d.routine_create(2, crate::host::LOCAL_CHAT, "calendar:60:outside", "brief me", false).unwrap();
        assert_eq!(c.next_fire, IDLE);
        assert!(d.routine_create(2, crate::host::LOCAL_CHAT, "calendar:60:all", "x", false).is_err());
        assert_eq!(d.calendar_routines(2, now()).unwrap().len(), 1);
        d.routine_set_next(c.id, now() + 60).unwrap();
        assert!(d.due(2, now() + 60).unwrap().iter().any(|r| r.id == c.id));
        d.routine_advance(&d.routines(crate::host::LOCAL_CHAT).unwrap()[0]).unwrap();
        assert_eq!(d.routines(crate::host::LOCAL_CHAT).unwrap()[0].next_fire, IDLE, "the next event is the scheduler's to find");
    }

    #[test]
    fn pause_and_resume() {
        let d = db();
        let r = d.routine_create(2, 7, "0 9 * * *", "brief", false).unwrap();
        let o = d.routine_once(2, 7, now() + 600, "call mum", false).unwrap();
        let other = d.routine_create(3, 7, "0 9 * * *", "news", false).unwrap();
        // Slots came due while it was paused.
        d.c().execute("UPDATE routines SET next_fire=?1", [now() - 3 * 86400]).unwrap();
        d.pause_set(Some(2), 0).unwrap();
        assert_eq!(d.paused(2, now()).unwrap(), Some(0), "until resumed");
        assert_eq!(d.paused(3, now()).unwrap(), None, "only that bot");
        // Everyone's pause on top: the later end wins (0 = no end). One clock read: a slow runner crosses a second.
        let end = now() + 600;
        d.pause_set(None, end).unwrap();
        assert_eq!(d.paused(2, now()).unwrap(), Some(0));
        assert_eq!(d.paused(3, now()).unwrap(), Some(end));
        assert_eq!(d.pauses(now()).unwrap().len(), 2);
        // Resumed: the routine moves on to its next time instead of catching up (or showing as missed); the
        // one-off still runs once. The other bot is still held by everyone's pause.
        d.pause_end(Some(2)).unwrap();
        assert_eq!(d.paused(2, now()).unwrap(), Some(end));
        let due: Vec<i64> = d.due(2, now()).unwrap().iter().map(|x| x.id).collect();
        assert_eq!(due, vec![o.id]);
        assert!(d.routines(7).unwrap().iter().any(|x| x.id == r.id && x.next_fire > now()));
        assert!(d.routine_missed(&d.routines(7).unwrap().into_iter().find(|x| x.id == r.id).unwrap(), now(), ASLEEP).unwrap().is_none());
        // A timed pause whose time has come ends by itself, the same way.
        d.pause_set(None, now() - 1).unwrap();
        assert_eq!(d.paused(3, now()).unwrap(), None);
        assert!(d.pauses(now()).unwrap().is_empty());
        assert!(d.routines(7).unwrap().iter().any(|x| x.id == other.id && x.next_fire > now()));
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

    #[test]
    fn cards_write_tick_and_routine() {
        let d = db();
        assert!(d.card_write(1, 5, "Shopping", "<b>milk</b>", false).is_err(), "no HTML");
        let k = d.card_write(1, 5, "Shopping", "- [ ] milk\n- [ ] eggs", false).unwrap();
        // Same title (any case) is the same card, rewritten whole.
        assert_eq!(d.card_write(1, 5, "shopping", "- [ ] milk\n- [ ] eggs\n- [ ] bread", false).unwrap().id, k.id);
        let t = d.card_tick(5, k.id, 1, "- [ ] eggs").unwrap();
        assert!(t.by_you && t.body.contains("- [x] eggs") && t.body.contains("- [ ] milk"));
        assert!(d.card_tick(5, k.id, 1, "- [ ] eggs").is_err(), "the line changed since");
        assert!(d.card_tick(5, k.id, 0, "- [ ] milk").is_ok() && d.card_tick(6, k.id, 0, "- [x] milk").is_err(), "another chat's card");
        assert_eq!(cards::toggle_box("  * [X] done"), Some("  * [ ] done".into()));
        assert_eq!(cards::toggle_box("plain"), None);
        // A routine's card: named by its "# " line, then kept, and the bot's rewrite clears "by you".
        let r = d.routine_create(1, 5, "0 8 * * *", "List bills due this week", false).unwrap();
        let c = d.card_from_routine(&r, "# Bills due\n| Bill | Due |\n|---|---|\n| Rent | 1 Nov |", true).unwrap();
        assert_eq!((c.title.as_str(), c.tainted, c.body.starts_with('|')), ("Bills due", true, true));
        assert_eq!(d.card_from_routine(&r, "Nothing due.", false).unwrap().id, c.id);
        assert_eq!(d.routine_card(r.id).unwrap().unwrap().title, "Bills due");
        assert_eq!(d.cards(5).unwrap().len(), 2);
        d.routine_set_delivery(5, r.id, "card").unwrap();
        assert!(d.card_delete(5, c.id).unwrap() && d.cards(5).unwrap().len() == 1);
    }

    #[test]
    fn pausing_a_goal_holds_its_followups_and_deleting_removes_them() {
        let d = db();
        let g = d.goal_set(1, 5, "Find a plumber who can come this week", "a plumber is booked", 0, false).unwrap();
        assert_eq!(g.status, "active");
        assert!(d.goal_set(1, 5, "Another", "x", 0, false).is_err(), "one goal per chat");
        // A follow-up booked while the goal is on is the goal's; one in another chat isn't.
        let f = d.routine_once(1, 5, now() + 60, "Check whether the plumber replied", false).unwrap();
        d.routine_set_followup(f.id).unwrap();
        d.goal_claim_routine(f.id, 5).unwrap();
        let other = d.routine_once(1, 6, now() + 60, "Unrelated", false).unwrap();
        d.goal_claim_routine(other.id, 6).unwrap();
        let later = now() + 120;
        assert_eq!(d.due(1, later).unwrap().len(), 2);
        // Paused: its follow-up doesn't run, though it's due; the other chat's still does.
        d.goal_status(5, "paused", "").unwrap();
        assert_eq!(d.due(1, later).unwrap().iter().map(|r| r.id).collect::<Vec<_>>(), vec![other.id]);
        assert!(d.routine(f.id).unwrap().unwrap().goal_held && !d.routine(other.id).unwrap().unwrap().goal_held, "shown as paused with its goal");
        assert!(d.goal_progress(5, "x", None, "", false).is_err(), "a paused goal is the person's to resume");
        d.goal_status(5, "active", "").unwrap();
        assert_eq!(d.due(1, later).unwrap().len(), 2, "resumed, it runs again");
        // A task handed over for the goal: pausing stops it mid-run (and reports it), deleting stops a waiting one.
        let t1 = d.delegation_add(&Delegation { bot_id: 1, to_bot: 2, chat_id: 5, task: "call plumbers".into(), ..Default::default() }).unwrap();
        d.goal_claim_delegation(t1, 5).unwrap();
        d.delegation_status(t1, "running").unwrap();
        assert_eq!(d.goal_status(5, "paused", "").unwrap(), vec![t1]);
        d.delegation_done(t1, "found one", false, false).unwrap();
        assert!(d.delegations(1, "done").unwrap().is_empty(), "a stopped task's result isn't told");
        d.goal_status(5, "active", "").unwrap();
        let t2 = d.delegation_add(&Delegation { bot_id: 1, to_bot: 2, chat_id: 5, task: "again".into(), ..Default::default() }).unwrap();
        d.goal_claim_delegation(t2, 5).unwrap();
        d.goal_delete(5).unwrap();
        assert!(d.goal(5).unwrap().is_none());
        assert!(d.routine(f.id).unwrap().is_none(), "its follow-up went with it");
        assert!(d.routine(other.id).unwrap().is_some());
        assert!(d.delegations(1, "pending").unwrap().is_empty(), "its waiting task too");
    }

    #[test]
    fn a_goal_over_budget_pauses_and_asks() {
        let d = db();
        let g = d.goal_set(1, 5, "Get the insurance refund paid", "the refund is in the account", 1000, false).unwrap();
        assert!(!d.goal_check_budget(&g).unwrap());
        // Only its own runs count: a turn in its chat while it's on, not a routine of the chat that isn't its.
        let other = d.routine_create(1, 5, "0 8 * * *", "news", false).unwrap();
        let o = d.run_start(1, 5, "routine", Some(other.id), "x").unwrap();
        d.goal_claim_run(o, 5, Some(other.id), None).unwrap();
        let r = d.run_start(1, 5, "message", None, "x").unwrap();
        d.goal_claim_run(r, 5, None, None).unwrap();
        d.c().execute("UPDATE runs SET tokens_in=1500 WHERE id IN (?1, ?2)", [r, o]).unwrap();
        let g = d.goal(5).unwrap().unwrap();
        assert_eq!(g.spent, 1500);
        assert!(d.goal_check_budget(&g).unwrap());
        let g = d.goal(5).unwrap().unwrap();
        assert!(g.status == "paused" && g.why.contains("budget"), "{g:?}");
        assert_eq!(d.goals_waiting().unwrap().len(), 1, "Needs you asks");
        // Going on starts the budget over.
        d.goal_status(5, "active", "").unwrap();
        let g = d.goal(5).unwrap().unwrap();
        assert!(g.status == "active" && g.why.is_empty() && !d.goal_check_budget(&g).unwrap());
        assert!(d.goals_waiting().unwrap().is_empty());
        // Done is done: removed or replaced, never taken up again (its routines are gone).
        d.goal_status(5, "done", "").unwrap();
        assert!(d.goal_status(5, "active", "").is_err() && d.goal_status(5, "paused", "").is_err());
        assert_eq!(d.goal_set(1, 5, "Next thing", "x", 0, false).unwrap().status, "active", "a new goal replaces it");
    }
}
