//! `mimir doctor`: is everything Mimir needs working? The database, its backups, disk space, the bots' computer, each
//! bot's AI account and channels, and the mailboxes. Nothing here spends tokens or sends a message.
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

use crate::db::Db;

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Ok,
    Warn,
    Fail,
}

/// One line of the report: what was checked and, when it isn't fine, what to do.
#[derive(Serialize, Debug)]
pub struct Check {
    pub state: State,
    pub what: String,
    pub fix: String,
}

fn ok(what: impl Into<String>) -> Check {
    Check { state: State::Ok, what: what.into(), fix: String::new() }
}

fn bad(state: State, what: impl Into<String>, fix: impl Into<String>) -> Check {
    Check { state, what: what.into(), fix: fix.into() }
}

impl std::fmt::Display for Check {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mark = match self.state {
            State::Ok => "✓",
            State::Warn => "!",
            State::Fail => "✗",
        };
        write!(f, "{mark} {}", self.what)?;
        if !self.fix.is_empty() {
            write!(f, "\n    {}", self.fix)?;
        }
        Ok(())
    }
}

/// Everything, for the data folder `home`.
pub async fn run(home: &Path) -> Vec<Check> {
    let path = home.join("mimir.db");
    let p = path.clone();
    let mut out = vec![tokio::task::spawn_blocking(move || database(&p)).await.unwrap_or_else(|e| bad(State::Fail, "Database", e.to_string()))];
    let Ok(db) = Db::open(&path) else { return out };
    // The server's secrets (in the database). The app's own store is already installed, and kept.
    crate::host::install_secrets(std::sync::Arc::new(crate::host::DbSecrets(db.clone())));
    out.push(backup(&crate::backup::list(&db), crate::db::now()));
    out.push(disk(free_bytes(home)));
    if let Some(r) = tokio::task::spawn_blocking(crate::computer::reach).await.ok().flatten() {
        out.push(match r {
            Ok(()) => ok("The bots' computer answers"),
            Err(e) => bad(State::Fail, format!("The bots' computer: {e:#}"), "Check the computer service is running and MIMIR_COMPUTER_TOKEN is the same on both sides."),
        });
    }
    let bots = db.bots().unwrap_or_default();
    let mut seen = std::collections::HashMap::new();
    for b in &bots {
        if !seen.contains_key(&b.provider) {
            seen.insert(b.provider.clone(), crate::provider::detect(&b.provider, home).await);
        }
        out.push(account(&b.name, &b.provider, &seen[&b.provider]));
    }
    for b in &bots {
        out.extend(channels(&b.name, crate::host::channels_of(b.id)).await);
    }
    for a in crate::email::accounts(&db) {
        out.push(match crate::email::check(&a).await {
            Ok(n) => ok(format!("Email {}: signed in, {n} in the inbox", a.user)),
            Err(e) => bad(State::Fail, format!("Email {}: {e:#}", a.user), "Check the address, password (an app password for Gmail or iCloud) and server in Settings → Email."),
        });
    }
    out
}

/// The database file: written by this version or an older one, and sound (SQLite's full integrity check).
fn database(path: &Path) -> Check {
    if !path.exists() {
        return bad(State::Warn, "No database yet", "Mimir makes one the first time it starts.");
    }
    let check = || -> anyhow::Result<Option<String>> {
        let c = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        c.busy_timeout(Duration::from_secs(5))?;
        crate::db::check_not_newer(&c)?;
        let lines: Vec<String> = c.prepare("PRAGMA integrity_check(5)")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok((lines != ["ok"]).then(|| lines.join("; ")))
    };
    match check() {
        Ok(None) => ok("Database is sound"),
        Ok(Some(damage)) => bad(State::Fail, format!("The database reports damage: {damage}"), "Restore the latest backup: Settings → Your data → Restore a backup…"),
        Err(e) => bad(State::Fail, format!("Database: {e:#}"), "Update Mimir, or check the data folder can be read."),
    }
}

/// A copy from the last two days (they're taken daily while the app or server runs).
fn backup(list: &[crate::backup::Snapshot], now: i64) -> Check {
    match list.iter().map(|s| s.taken_at).max() {
        Some(t) if now - t < 2 * 86400 => ok(format!("Last backup {} hours ago", (now - t) / 3600)),
        Some(t) => bad(State::Warn, format!("The last backup is {} days old", (now - t) / 86400), "Copies are taken daily while the app or `mimir server` runs; Settings → Your data → Daily backup takes one now."),
        None => bad(State::Warn, "No backups yet", "Copies are taken daily while the app or `mimir server` runs; Settings → Your data → Daily backup takes one now."),
    }
}

const LOW_DISK: u64 = 1 << 30;

fn disk(free: Option<u64>) -> Check {
    match free {
        None => bad(State::Warn, "Couldn't tell how much disk space is free", ""),
        Some(b) if b < LOW_DISK => bad(State::Warn, format!("Only {} MB free for the data folder", b >> 20), "Free up space: backups and the database stop being written when the disk is full."),
        Some(b) => ok(format!("{:.1} GB free for the data folder", b as f64 / (1u64 << 30) as f64)),
    }
}

/// Free space where `dir` is, from `df`.
// ponytail: df, so no free-space line on Windows; GetDiskFreeSpaceExW if the desktop app shows this there
fn free_bytes(dir: &Path) -> Option<u64> {
    let out = std::process::Command::new("df").arg("-Pk").arg(dir).stdin(std::process::Stdio::null()).output().ok()?;
    let kb: u64 = String::from_utf8_lossy(&out.stdout).lines().nth(1)?.split_whitespace().nth(3)?.parse().ok()?;
    Some(kb * 1024)
}

/// What `provider::detect` says about a bot's AI account (no tokens spent: the CLI's own sign-in status, or a saved key).
fn account(bot: &str, kind: &str, v: &serde_json::Value) -> Check {
    let fix = "Sign in under Settings → AI accounts, or pick another account for this bot.";
    if v["installed"] == false {
        bad(State::Fail, format!("{bot}: {kind} isn't installed here"), fix)
    } else if v["signed_in"] == false {
        bad(State::Fail, format!("{bot}: {kind} isn't signed in"), fix)
    } else if v["signed_in"].is_null() {
        bad(State::Warn, format!("{bot}: {kind} is installed, but couldn't say whether it's signed in"), "")
    } else {
        ok(format!("{bot}: {kind} is signed in"))
    }
}

/// Each connected channel answers with its token (who the bot is there).
async fn channels(bot: &str, ch: crate::runtime::Channels) -> Vec<Check> {
    async fn within(f: impl std::future::Future<Output = anyhow::Result<String>>) -> anyhow::Result<String> {
        tokio::time::timeout(Duration::from_secs(20), f).await.unwrap_or_else(|_| Err(anyhow::anyhow!("no answer in 20 seconds")))
    }
    let mut out = vec![];
    if let Some(t) = ch.telegram {
        let r = match crate::telegram::Tg::new(&t) { Ok(c) => within(c.check()).await, Err(e) => Err(e) };
        out.push(("Telegram", r, "Paste a new token from @BotFather in the bot's settings."));
    }
    if let Some(t) = ch.discord {
        let r = match crate::discord::Discord::new(&t) { Ok(c) => within(c.check()).await, Err(e) => Err(e) };
        out.push(("Discord", r, "Paste a new token from the Discord Developer Portal in the bot's settings."));
    }
    if let Some((b, a)) = ch.slack {
        let r = match crate::slack::Slack::new(&b, &a) { Ok(c) => within(c.check()).await, Err(e) => Err(e) };
        out.push(("Slack", r, "Check both tokens (xoxb-… and xapp-…) in the bot's settings."));
    }
    if let Some((api, n)) = ch.signal {
        let r = match crate::signal::Signal::new(&api, &n) { Ok(c) => within(c.check()).await, Err(e) => Err(e) };
        out.push(("Signal", r, "Check that signal-cli-rest-api is running (MODE=json-rpc) and the bot's number is linked to it."));
    }
    out.into_iter()
        .map(|(name, r, fix)| match r {
            Ok(me) => ok(format!("{bot}: {name} reaches {me}")),
            Err(e) => bad(State::Fail, format!("{bot}: {name}: {e:#}"), fix),
        })
        .collect()
}

/// True when nothing failed (warnings pass).
pub fn passed(checks: &[Check]) -> bool {
    checks.iter().all(|c| c.state != State::Fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_warns_under_a_gigabyte() {
        assert_eq!(disk(Some(500 << 20)).state, State::Warn);
        assert_eq!(disk(Some(5 << 30)).state, State::Ok);
        if cfg!(unix) {
            assert!(free_bytes(&std::env::temp_dir()).is_some_and(|b| b > 0));
        }
    }

    #[test]
    fn backups_go_stale_after_two_days() {
        let snap = |taken_at| crate::backup::Snapshot { name: "mimir-x.db".into(), size: 1, taken_at };
        assert_eq!(backup(&[], 1000).state, State::Warn);
        assert_eq!(backup(&[snap(0), snap(90_000)], 100_000).state, State::Ok);
        assert_eq!(backup(&[snap(0)], 3 * 86400).state, State::Warn);
    }

    #[test]
    fn database_sound_newer_and_damaged() {
        let home = std::env::temp_dir().join(format!("mimir-doctor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let path = home.join("mimir.db");
        assert_eq!(database(&path).state, State::Warn, "none yet");
        let db = Db::open(&path).unwrap();
        assert_eq!(database(&path).state, State::Ok);
        db.c().execute_batch("PRAGMA user_version = 9999").unwrap();
        let c = database(&path);
        assert!(c.state == State::Fail && c.what.contains("Update the app"), "{c}");
        drop(db);
        let bad_file = home.join("bad").join("mimir.db");
        std::fs::create_dir_all(bad_file.parent().unwrap()).unwrap();
        std::fs::write(&bad_file, b"SQLite format 3\0 not really").unwrap();
        assert_eq!(database(&bad_file).state, State::Fail);
        assert!(!passed(&[database(&bad_file), ok("x")]) && passed(&[ok("x"), disk(Some(1))]));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn account_states() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        assert_eq!(account("Ava", "claude", &v(r#"{"installed":false}"#)).state, State::Fail);
        assert_eq!(account("Ava", "codex", &v(r#"{"installed":true,"signed_in":false}"#)).state, State::Fail);
        assert_eq!(account("Ava", "claude", &v(r#"{"installed":true,"signed_in":null}"#)).state, State::Warn);
        assert_eq!(account("Ava", "openai", &v(r#"{"installed":true,"signed_in":true}"#)).state, State::Ok);
    }
}
