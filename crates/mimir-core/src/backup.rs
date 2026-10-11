//! Backups you can restore: once a day, after SQLite's quick_check says the database is sound, a copy taken with its
//! online backup API goes to `backups/` in the data folder. 7 daily and 4 weekly copies are kept. Damage found by the
//! check is one item in Needs you, and no copy is taken (the good ones stay).
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::db::Db;

const DAILY: usize = 7;
const WEEKLY: usize = 4;
/// What the check found, while the database reports damage (Needs you shows it).
pub const DAMAGE: &str = "db_damage";
const LAST: &str = "backup_at";

pub fn dir(db: &Db) -> PathBuf {
    db.dir().join("backups")
}

#[derive(Serialize)]
pub struct Snapshot {
    pub name: String,
    pub size: u64,
    pub taken_at: i64,
}

/// What SQLite's quick_check reports: None when the database is sound.
fn damage(c: &rusqlite::Connection) -> Result<Option<String>> {
    let lines: Vec<String> = c.prepare("PRAGMA quick_check(5)")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok((lines != ["ok"]).then(|| lines.join("; ")))
}

/// Checks the database and, when it's sound, takes today's copy (`suffix` names an extra one, like before a restore).
/// Err when it reports damage: that's in Needs you until a check passes again.
pub fn snapshot(db: &Db, suffix: &str) -> Result<String> {
    let dir = dir(db);
    std::fs::create_dir_all(&dir)?;
    let name = format!("mimir-{}{suffix}.db", chrono::Local::now().format("%Y-%m-%d"));
    let tmp = dir.join(format!("{name}.part"));
    let _ = std::fs::remove_file(&tmp);
    {
        let c = db.c();
        if let Some(bad) = damage(&c).unwrap_or_else(|e| Some(format!("{e:#}"))) {
            let _ = c.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?1, ?2)", [DAMAGE, &bad]);
            bail!("the database reports damage: {bad}");
        }
        c.execute("DELETE FROM settings WHERE key=?1", [DAMAGE])?;
        // ponytail: holds the one connection for the whole copy (a few MB, well under a second); step it if that grows
        c.backup(rusqlite::MAIN_DB, &tmp, None)?;
    }
    // One file on its own (no -wal beside it), so it can be copied anywhere.
    rusqlite::Connection::open(&tmp)?.pragma_update(None, "journal_mode", "DELETE")?;
    std::fs::rename(&tmp, dir.join(&name))?;
    db.set_setting(LAST, &crate::db::now().to_string())?;
    prune(&dir);
    Ok(name)
}

/// Copies, newest first.
pub fn list(db: &Db) -> Vec<Snapshot> {
    let mut out: Vec<Snapshot> = std::fs::read_dir(dir(db))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let meta = e.metadata().ok()?;
            let taken_at = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
            (name.starts_with("mimir-") && name.ends_with(".db") && meta.is_file()).then_some(Snapshot { name, size: meta.len(), taken_at })
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// The copies to delete: everything but the 7 newest days and the newest copy of each of the 4 weeks before them.
fn to_prune(names: &[String]) -> Vec<String> {
    let mut names: Vec<&String> = names.iter().collect();
    names.sort_by(|a, b| b.cmp(a));
    let (mut days, mut weeks, mut weekly) = (Vec::new(), Vec::new(), 0);
    let mut out = Vec::new();
    for n in names {
        let Some(day) = n.get(6..16).and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) else { continue };
        let week = chrono::Datelike::iso_week(&day);
        if days.len() < DAILY && !days.contains(&day) {
            days.push(day);
            weeks.push(week);
        } else if days.contains(&day) {
            // Another copy of a kept day (before a restore): kept with it.
        } else if weekly < WEEKLY && !weeks.contains(&week) {
            weeks.push(week);
            weekly += 1;
        } else {
            out.push(n.clone());
        }
    }
    out
}

fn prune(dir: &Path) {
    let names: Vec<String> = std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.starts_with("mimir-") && n.ends_with(".db")).collect();
    for n in to_prune(&names) {
        let _ = std::fs::remove_file(dir.join(n));
    }
}

/// A day since the last copy (23 hours, so it doesn't creep later each day).
pub fn due(db: &Db) -> bool {
    let last: i64 = db.setting(LAST).ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0);
    crate::db::now() - last >= 23 * 3600
}

/// Puts a copy back in place of the database, after checking the copy is sound and keeping one of now, so the
/// restore itself can be undone. The bots should be stopped around it.
pub fn restore(db: &Db, name: &str) -> Result<()> {
    let file = list(db).into_iter().find(|s| s.name == name).with_context(|| format!("no backup called {name}"))?;
    let path = dir(db).join(&file.name);
    if let Some(bad) = damage(&rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?)? {
        bail!("that backup is damaged too ({bad}); pick an older one");
    }
    snapshot(db, &format!("-{}-before-restore", chrono::Local::now().format("%H%M%S"))).context("couldn't keep a copy of now first")?;
    let live = {
        let mut c = db.c();
        c.restore(rusqlite::MAIN_DB, &path, None::<fn(rusqlite::backup::Progress)>).context("the database is busy; try again in a moment")?;
        c.path().map(PathBuf::from)
    };
    // A copy from an older version: bring its tables up to this one's (what opening the database does).
    if let Some(live) = live {
        drop(Db::open(&live)?);
    }
    db.c().execute("DELETE FROM settings WHERE key=?1", [DAMAGE])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_seven_days_and_four_weeks() {
        let day0 = chrono::NaiveDate::from_ymd_opt(2026, 10, 11).unwrap();
        let names: Vec<String> = (0..60).map(|i| format!("mimir-{}.db", day0 - chrono::Days::new(i))).collect();
        let gone = to_prune(&names);
        let kept: Vec<&String> = names.iter().filter(|n| !gone.contains(n)).collect();
        assert_eq!(kept.len(), DAILY + WEEKLY, "{kept:?}");
        assert!(kept[..DAILY].iter().zip(&names).all(|(a, b)| *a == b), "the 7 newest days");
        // 2026-10-05 is the Monday of the newest week; the weekly copies are each earlier week's Sunday.
        assert_eq!(kept[DAILY..].iter().map(|s| s.as_str()).collect::<Vec<_>>(), ["mimir-2026-10-04.db", "mimir-2026-09-27.db", "mimir-2026-09-20.db", "mimir-2026-09-13.db"]);
        let mut more = names.clone();
        more.push("mimir-2026-10-11-120000-before-restore.db".into());
        assert!(!to_prune(&more).contains(&more[60]), "an extra copy of a kept day stays");
    }

    #[test]
    fn snapshot_restore_and_damage() {
        let home = std::env::temp_dir().join(format!("mimir-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let db = Db::open(&home.join("mimir.db")).unwrap();
        db.set_setting("color", "blue").unwrap();
        assert!(due(&db));
        let name = snapshot(&db, "").unwrap();
        assert!(!due(&db) && db.setting(DAMAGE).unwrap().is_none());
        assert!(!dir(&db).join(format!("{name}-wal")).exists(), "one file");
        db.set_setting("color", "red").unwrap();
        let other = Db::open(&home.join("mimir.db")).unwrap();
        assert_eq!(other.setting("color").unwrap().as_deref(), Some("red"));
        assert!(restore(&db, "../mimir.db").is_err(), "only a copy in backups/");
        restore(&db, &name).unwrap();
        assert_eq!(db.setting("color").unwrap().as_deref(), Some("blue"), "back as it was");
        // Another process's connection (a tools process) sees it too, and the database stays in WAL mode.
        assert_eq!(other.setting("color").unwrap().as_deref(), Some("blue"));
        let mode: String = db.c().query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
        assert!(list(&db).iter().any(|s| s.name.contains("before-restore")), "now was kept first");
        // A damaged copy: refused, and the database is left as it is.
        let bad = dir(&db).join("mimir-2020-01-01.db");
        std::fs::write(&bad, b"SQLite format 3\0 not really").unwrap();
        assert!(restore(&db, "mimir-2020-01-01.db").is_err());
        assert_eq!(db.setting("color").unwrap().as_deref(), Some("blue"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
