//! Webhooks out (Settings → Webhooks out): a small signed JSON to one address when something happens, for Home
//! Assistant, ntfy or n8n. Signed like the calls Mimir takes (`X-Hub-Signature-256: sha256=<hmac>`), sent through the
//! outbox (kind "webhook", the address as its `ext`), so a failed send is tried again. Message text only if asked.

use crate::db::{Db, Outgoing};
use serde_json::{Value, json};
use std::time::Duration;

/// The keychain entry the bodies are signed with.
pub const SECRET: &str = "webhook_out_secret";
/// Every event, in the order Settings shows them.
pub const EVENTS: [&str; 4] = ["routine_finished", "routine_failed", "approval_waiting", "reply_sent"];

pub enum Event<'a> {
    RoutineFinished { routine: i64, text: &'a str },
    RoutineFailed { routine: i64, error: &'a str },
    Approval { id: i64, tool: &'a str, summary: &'a str },
    Reply { text: &'a str },
}

/// Settings: the address, which events, whether the text goes too.
pub fn settings(db: &Db) -> (Option<String>, Vec<String>, bool) {
    let get = |k| db.setting(k).ok().flatten().filter(|v| !v.is_empty());
    let events = get("webhook_out_events").map(|v| v.split(',').map(String::from).collect()).unwrap_or_default();
    (get("webhook_out_url"), events, get("webhook_out_text").as_deref() == Some("on"))
}

/// The body for an event, or None when it isn't one to send.
pub fn body(db: &Db, bot: i64, chat: i64, ev: &Event) -> Option<(String, String)> {
    let (_, events, with_text) = settings(db);
    let (name, routine, summary, text, hash) = match ev {
        Event::RoutineFinished { routine, text } => ("routine_finished", Some(*routine), format!("Routine #{routine} finished"), *text, "#inbox".to_string()),
        Event::RoutineFailed { routine, error } => ("routine_failed", Some(*routine), format!("Routine #{routine} failed"), *error, "#inbox".to_string()),
        Event::Approval { id, tool, summary } => ("approval_waiting", None, format!("An approval is waiting: {tool}"), *summary, format!("#approval-{id}")),
        Event::Reply { text } => ("reply_sent", None, "A reply was sent".to_string(), *text, "#inbox".to_string()),
    };
    if !events.iter().any(|e| e == name) {
        return None;
    }
    let bot_name = db.bot(bot).map(|b| b.name).unwrap_or_default();
    let chat_title = db.chats().unwrap_or_default().into_iter().find(|c| c.chat_id == chat).map(|c| c.title);
    let link = db.setting("public_url").ok().flatten().map(|u| format!("{}/{hash}", u.trim_end_matches('/')));
    let mut b = json!({
        "event": name, "at": crate::db::now(), "summary": summary, "link": link, "routine": routine,
        "bot": {"id": bot, "name": bot_name}, "chat": {"id": chat, "title": chat_title},
    });
    if with_text {
        b["text"] = Value::from(crate::tools::truncate(text, 4_000));
    }
    Some((name.into(), b.to_string()))
}

/// Kept in the outbox and sent now (in the background) if it's one of the chosen events. Never fails what called it.
pub fn fire(db: &Db, bot: i64, chat: i64, ev: Event) {
    let Some(url) = settings(db).0 else { return };
    let Some((_, body)) = body(db, bot, chat, &ev) else { return };
    let id = match db.outbox_add(bot, chat, "webhook", &url, None, &body, "[]", "[]", None) {
        Ok(id) => id,
        Err(e) => return eprintln!("webhook out: {e:#}"),
    };
    // Not in an async runtime (a test, a CLI): the scheduler's next pass sends it.
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        let (db, o) = (db.clone(), Outgoing { id, chat_id: chat, kind: "webhook".into(), ext: url, draft: None, text: body, rows: "[]".into(), files: "[]".into(), run_id: None });
        rt.spawn(async move { send(&db, &o).await });
    }
}

/// The signature header's value for a body, if there's a secret yet.
pub fn signature(body: &str) -> Option<String> {
    let s = crate::host::secret_get(SECRET)?;
    Some(format!("sha256={}", crate::events::hex(&crate::events::hmac(s.as_bytes(), body.as_bytes()))))
}

pub async fn post(url: &str, body: &str) -> anyhow::Result<()> {
    let http = reqwest::Client::builder().timeout(Duration::from_secs(15)).redirect(reqwest::redirect::Policy::none()).build()?;
    let event = serde_json::from_str::<Value>(body).ok().and_then(|v| v["event"].as_str().map(String::from)).unwrap_or_default();
    let mut req = http.post(url).header("content-type", "application/json").header("x-mimir-event", event).body(body.to_string());
    if let Some(sig) = signature(body) {
        req = req.header("x-hub-signature-256", sig);
    }
    req.send().await?.error_for_status()?;
    Ok(())
}

/// One from the outbox: gone once the address took it (any 2xx), else tried again later.
pub async fn send(db: &Db, o: &Outgoing) {
    let r = match post(&o.ext, &o.text).await {
        Ok(()) => db.outbox_sent(o.id),
        Err(e) => {
            eprintln!("webhook out to {} failed: {e:#}", o.ext);
            db.outbox_failed(o.id, &format!("{e:#}")).map(|_| ())
        }
    };
    if let Err(e) = r {
        eprintln!("webhook out: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_chosen_events_and_no_text_unless_asked() {
        let p = std::env::temp_dir().join(format!("mimir-hooks-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        let ev = Event::RoutineFailed { routine: 7, error: "the secret plan" };
        assert!(body(&db, 1, 0, &ev).is_none(), "nothing chosen");
        db.set_setting("webhook_out_events", "routine_failed,approval_waiting").unwrap();
        db.set_setting("public_url", "https://bots.example/").unwrap();
        let (name, b) = body(&db, 1, 0, &ev).unwrap();
        let v: Value = serde_json::from_str(&b).unwrap();
        assert_eq!((name.as_str(), v["routine"].as_i64(), v["link"].as_str()), ("routine_failed", Some(7), Some("https://bots.example/#inbox")));
        assert!(!b.contains("secret plan"), "no text unless asked: {b}");
        assert!(body(&db, 1, 0, &Event::Reply { text: "hi" }).is_none());
        db.set_setting("webhook_out_text", "on").unwrap();
        let (_, b) = body(&db, 1, 0, &Event::Approval { id: 3, tool: "email_send", summary: "Send it" }).unwrap();
        let v: Value = serde_json::from_str(&b).unwrap();
        assert_eq!((v["text"].as_str(), v["link"].as_str()), (Some("Send it"), Some("https://bots.example/#approval-3")));
        // Queued for the scheduler when there's no async runtime to send it now.
        db.set_setting("webhook_out_url", "http://127.0.0.1:9/hook").unwrap();
        fire(&db, 1, 0, Event::RoutineFailed { routine: 7, error: "x" });
        let left = db.outbox_take(1, crate::db::now() + crate::db::OUTBOX_STALE + 1).unwrap();
        assert!(left.iter().any(|o| o.kind == "webhook" && o.ext == "http://127.0.0.1:9/hook"));
    }
}
