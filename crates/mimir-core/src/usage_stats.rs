//! Usage stats: once a day, an anonymous count to Meterbird (our analytics) so we know how many installs there are, on
//! which systems and with which AI accounts. A random id for this install, the version, OS and, per bot, its provider and
//! model, and how many pages the server's browser opened and how many of those a site blocked (#160: does a bot need
//! your own browser?). Never names, messages, memories, keys or addresses. Off in Settings → Your data, or with MIMIR_USAGE_STATS=off.
use anyhow::Result;
use serde_json::{Value, json};
use std::time::Duration;

use crate::db::Db;

/// The Meterbird app "Mimir app": its public site key and the domain it accepts (Meterbird checks the Origin against it).
// ponytail: a public key in the source, like the website's tracker; anyone can send fake pings, so read it as a rough count.
const SITE_KEY: &str = "mb_d3d8b667ed2285f6";
const ORIGIN: &str = "https://app.heymimir.app";
const URL: &str = "https://app.meterbird.com/api/batch";
const EVERY: i64 = 24 * 3600;

/// Phrases of the pages sites show a browser they won't serve: Cloudflare, Akamai, PerimeterX, Google's /sorry.
// ponytail: phrase match on the page's text, so a page that talks about captchas counts too; a rough share is all #160 needs.
const BLOCKED: &[&str] = &["just a moment...", "attention required! | cloudflare", "verify you are human", "checking your browser",
    "unusual traffic from your computer network", "page title: access denied", "page title: 403 forbidden", "are you a robot", "press & hold"];

pub fn blocked(page: &str) -> bool {
    let page = page.to_lowercase();
    BLOCKED.iter().any(|b| page.contains(b))
}

/// Count a page the server's browser opened (what the browser tool returned for it).
pub fn count_page(db: &Db, page: &str) {
    let _ = db.bump_setting("browser_pages");
    if blocked(page) {
        let _ = db.bump_setting("browser_blocked");
    }
}

fn counted(db: &Db, key: &str) -> i64 {
    db.setting(key).ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0)
}

pub fn on(db: &Db) -> bool {
    std::env::var("MIMIR_USAGE_STATS").map_or(true, |v| v != "off" && v != "0")
        && db.setting("usage_stats").ok().flatten().as_deref() != Some("off")
}

/// Sends today's count if it's due (any bot's loop may call it; the first one takes it).
pub async fn tick(db: &Db) {
    let sent: i64 = db.setting("usage_sent_at").ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0);
    if SITE_KEY.is_empty() || !on(db) || crate::db::now() - sent < EVERY {
        return;
    }
    let _ = db.set_setting("usage_sent_at", &crate::db::now().to_string());
    if let Err(e) = send(db).await {
        eprintln!("usage stats: {e:#}");
    }
}

fn install_id(db: &Db) -> Result<String> {
    if let Some(id) = db.setting("install_id")? {
        return Ok(id);
    }
    let id = crate::random_hex(16)?;
    db.set_setting("install_id", &id)?;
    Ok(id)
}

/// What's sent: one "Active" event for the install, one "Bot" event per bot. Meterbird counts each property value's
/// distinct installs (the visitor), so "how many installs use Claude" is Goals → Bot → provider.
pub fn batch(db: &Db) -> Result<Value> {
    let (visitor, session) = (install_id(db)?, crate::random_hex(8)?);
    let kind = if !crate::host::is_server() {
        "desktop"
    } else if std::path::Path::new("/.dockerenv").exists() {
        "docker"
    } else {
        "server"
    };
    let os = os_info::get();
    let bots = db.bots()?;
    let short = |s: &str| s.chars().take(100).collect::<String>();
    let mut events = vec![json!({"type": "custom", "name": "Active", "visitorId": visitor, "sessionId": session, "path": "/app",
        "properties": {"version": crate::server_update::CURRENT, "kind": kind, "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
            "osVersion": short(&format!("{} {}", os.os_type(), os.version())), "bots": bots.len(),
            "pages": counted(db, "browser_pages"), "blockedPages": counted(db, "browser_blocked")}})];
    // Meterbird takes up to 50 events a batch.
    events.extend(bots.iter().take(49).map(|b| json!({"type": "custom", "name": "Bot", "visitorId": visitor, "sessionId": session, "path": "/app",
        "properties": {"provider": short(&b.provider), "model": short(if b.model.is_empty() { "default" } else { &b.model })}})));
    Ok(json!({"siteKey": SITE_KEY, "events": events}))
}

async fn send(db: &Db) -> Result<()> {
    let body = batch(db)?.to_string();
    reqwest::Client::builder().timeout(Duration::from_secs(20)).build()?.post(URL)
        // text/plain like Meterbird's browser tracker; it reads the body as JSON either way.
        .header("content-type", "text/plain").header("origin", ORIGIN)
        .header("user-agent", format!("Mimir/{} ({})", crate::server_update::CURRENT, std::env::consts::OS))
        .body(body).send().await?.error_for_status()?;
    // Pages since the last count. ponytail: a page opened while this was sending is dropped; fine for a share.
    db.set_setting("browser_pages", "0")?;
    db.set_setting("browser_blocked", "0")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_counts_not_content() {
        let p = std::env::temp_dir().join(format!("mimir-usage-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let b = batch(&db).unwrap();
        let ev = b["events"].as_array().unwrap();
        assert_eq!(ev[0]["name"], "Active");
        assert_eq!(ev.len() as u64, 1 + ev[0]["properties"]["bots"].as_u64().unwrap(), "one Bot event per bot");
        assert_eq!(batch(&db).unwrap()["events"][0]["visitorId"], ev[0]["visitorId"], "the install keeps its id");
        // Only these keys leave: no bot name, persona, address or tools.
        for e in ev.iter().skip(1) {
            let mut keys: Vec<_> = e["properties"].as_object().unwrap().keys().cloned().collect();
            keys.sort();
            assert_eq!(keys, ["model", "provider"]);
        }
        count_page(&db, "### Page\n- Page URL: https://github.com/\n- Page Title: GitHub");
        count_page(&db, "### Page\n- Page URL: https://shop.example/\n- Page Title: Just a moment...");
        let p = &batch(&db).unwrap()["events"][0]["properties"];
        assert_eq!((p["pages"].as_i64(), p["blockedPages"].as_i64()), (Some(2), Some(1)));
        assert!(blocked("- Page Title: Access Denied") && !blocked("Access denied for this repo? Ask the owner."));
        assert!(on(&db));
        db.set_setting("usage_stats", "off").unwrap();
        assert!(!on(&db), "switched off");
    }
}
