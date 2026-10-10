//! A self-hosted server doesn't update itself (the desktop app does): it looks for a newer release twice a day, and
//! says so once per version, in your own chat and in Needs you, with the commands for how it was installed.
use anyhow::Result;
use std::time::Duration;

use crate::db::Db;

/// Where the newest release says its version (the same file the desktop app's updater reads).
fn feed() -> String {
    std::env::var("MIMIR_UPDATE_URL").unwrap_or_else(|_| "https://github.com/currentbits/mimir/releases/latest/download/latest.json".into())
}

const EVERY: i64 = 12 * 3600;
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Whether `a` is a newer version than `b` (numbers compared part by part: 0.1.10 > 0.1.9).
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.trim_start_matches('v').split(['.', '-']).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

/// Running from the Docker image (/.dockerenv), or from a build of the source.
fn in_docker() -> bool {
    std::path::Path::new("/.dockerenv").exists()
}

/// How to update this server, in words for a chat message or the app.
pub fn how() -> &'static str {
    if in_docker() {
        "In the folder with docker-compose.yml: `docker compose pull && docker compose up -d`. (Built it yourself from the source? `git pull && docker compose up -d --build`.) Your data stays."
    } else {
        "In the source folder: `git pull`, `cargo build --release -p mimir`, then restart `mimir server`. Your data stays."
    }
}

/// The newest released version, looked up at most twice a day (kept in settings, shared by every bot's loop).
pub async fn latest(db: &Db) -> Option<String> {
    let checked: i64 = db.setting("update_checked_at").ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0);
    if crate::db::now() - checked >= EVERY {
        // Taken first, so the other bots' loops don't all fetch at the same moment.
        let _ = db.set_setting("update_checked_at", &crate::db::now().to_string());
        match fetch().await {
            Ok(v) => {
                let _ = db.set_setting("update_latest", &v);
            }
            Err(e) => eprintln!("update check: {e:#}"),
        }
    }
    db.setting("update_latest").ok().flatten().filter(|v| newer(v, CURRENT))
}

async fn fetch() -> Result<String> {
    let v: serde_json::Value = reqwest::Client::builder().timeout(Duration::from_secs(20)).build()?.get(feed()).send().await?.error_for_status()?.json().await?;
    v["version"].as_str().map(str::to_string).ok_or_else(|| anyhow::anyhow!("no version in the release feed"))
}

/// The message, once per version: None when it was said already, or you switched it off.
pub fn to_tell(db: &Db, latest: &str) -> Option<String> {
    if db.setting("update_notice").ok().flatten().as_deref() == Some("off") || db.setting("update_told").ok().flatten().as_deref() == Some(latest) {
        return None;
    }
    Some(format!(
        "⬆️ **Mimir {latest} is out** (this server runs {CURRENT}).\n{}\nWhat's new: https://github.com/currentbits/mimir/releases/tag/v{latest}",
        how()
    ))
}

/// Marks the version as said; false when another bot's loop got there first (each bot runs its own loop).
pub fn told(db: &Db, latest: &str) -> bool {
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    if db.setting("update_told").ok().flatten().as_deref() == Some(latest) {
        return false;
    }
    db.set_setting("update_told", latest).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_once_per_version() {
        assert!(newer("0.1.10", "0.1.9") && newer("v0.2.0", "0.1.3") && !newer("0.1.3", "0.1.3") && !newer("0.1.2", "0.1.3"));
        let p = std::env::temp_dir().join(format!("mimir-srvupd-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let msg = to_tell(&db, "9.9.9").unwrap();
        assert!(msg.contains("Mimir 9.9.9 is out") && msg.contains(CURRENT) && msg.contains("releases/tag/v9.9.9"), "{msg}");
        assert!(told(&db, "9.9.9") && !told(&db, "9.9.9"), "one bot says it");
        assert!(to_tell(&db, "9.9.9").is_none(), "said once");
        assert!(to_tell(&db, "9.9.10").is_some(), "a later version is said again");
        db.set_setting("update_notice", "off").unwrap();
        assert!(to_tell(&db, "9.9.10").is_none(), "switched off");
    }
}
