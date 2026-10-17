//! WhatsApp through Meta's Cloud API. Replies go out through the Graph API; messages come in on a webhook Meta calls
//! (`POST /whatsapp` on `mimir server`, signed with the Meta app's secret), so it needs the server at an https address.
//! The bot is a WhatsApp Business number of its own. Direct chats only. Up to three short choices are buttons; more, or
//! longer ones, are numbered as on Signal. WhatsApp lets a business write only within 24 hours of the person's last
//! message; after that it wants an approved template, which Mimir doesn't send.
// ponytail: no streaming drafts (WhatsApp can't edit a sent message); typing shows instead
// ponytail: no templates, so a routine's result more than 24 hours after you last wrote isn't delivered

use crate::db::Approval;
use crate::runtime::{CATCH_UP_SECS, Chan, Core, File, Row, Sender, Slot, card_text};
use crate::signal::{Picks, choose, numbered};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// WhatsApp's own cap on a text message.
const MAX_TEXT: usize = 4096;
/// How long after the person's last message a business may still write to them.
const WINDOW_SECS: i64 = 86_400;

#[derive(Clone)]
pub(crate) struct WhatsApp {
    http: reqwest::Client,
    /// The Graph API, with its version.
    api: String,
    /// The bot's phone number id (Meta's, not the number itself).
    id: String,
    token: String,
    /// The Meta app's secret, which signs the webhook's calls.
    secret: String,
    picks: Picks,
    /// Chat → when they last wrote and that message's id (the typing indicator goes on it).
    heard: Arc<Mutex<HashMap<String, (i64, String)>>>,
    /// Message ids already taken: Meta delivers a message again when the webhook was slow to answer.
    seen: Arc<Mutex<VecDeque<String>>>,
}

/// Set by `mimir server`, which takes the webhook. Anywhere else nothing reaches the bot.
static SERVED: AtomicBool = AtomicBool::new(false);
/// A listening bot's app secret, and where its part of a webhook call goes.
type Hook = (String, mpsc::UnboundedSender<Value>);
/// The bots listening, by phone number id.
static HOOKS: LazyLock<Mutex<HashMap<String, Hook>>> = LazyLock::new(Default::default);

/// `mimir server` takes WhatsApp's webhook at /whatsapp.
pub fn serve_webhook() {
    SERVED.store(true, Ordering::Relaxed);
}

/// Meta's check when the webhook is set up: the challenge back, if the verify token is a listening bot's phone number id.
// ponytail: the phone number id as the verify token. The handshake guards nothing; every call after it is signed.
pub fn verify(mode: &str, token: &str, challenge: &str) -> Option<String> {
    (mode == "subscribe" && HOOKS.lock().unwrap().contains_key(token)).then(|| challenge.to_string())
}

/// A call to the webhook (`sig`: X-Hub-Signature-256). Each change for a listening bot's number goes to it if the call
/// is signed with that bot's app secret; false if one wasn't.
pub fn webhook(sig: Option<&str>, body: &[u8]) -> bool {
    let Ok(v) = serde_json::from_slice::<Value>(body) else { return false };
    let sig = sig.unwrap_or("").trim().trim_start_matches("sha256=").to_ascii_lowercase();
    let hooks = HOOKS.lock().unwrap();
    let mut ok = true;
    for value in v["entry"].as_array().into_iter().flatten().flat_map(|e| e["changes"].as_array().into_iter().flatten()).map(|c| &c["value"]) {
        let Some((secret, to)) = value["metadata"]["phone_number_id"].as_str().and_then(|id| hooks.get(id)) else { continue };
        if crate::events::same(&sig, &crate::events::hex(&crate::events::hmac(secret.as_bytes(), body))) {
            let _ = to.send(value.clone());
        } else {
            ok = false;
        }
    }
    ok
}

/// A message to the bot, from the webhook's `value`.
#[derive(Debug, PartialEq, Default)]
pub(crate) struct Incoming {
    /// Their WhatsApp id (their number, without +): also the chat.
    from: String,
    name: String,
    id: String,
    text: String,
    /// The bot's message they replied to.
    quoted: Option<String>,
    /// Forwarded: someone else's words.
    forwarded: bool,
    /// A button they tapped: its data.
    tapped: Option<String>,
    /// (name, mime, media id, recorded in WhatsApp as a voice note)
    file: Option<(String, String, String, bool)>,
    sent: Option<i64>,
}

/// The direct messages in a webhook `value`: text, a button tapped, or a file. Not statuses, reactions or groups'.
pub(crate) fn messages_of(value: &Value) -> Vec<Incoming> {
    let name_of = |from: &str| value["contacts"].as_array().into_iter().flatten().find(|c| c["wa_id"] == from).and_then(|c| c["profile"]["name"].as_str()).unwrap_or("").to_string();
    let mut out = vec![];
    for m in value["messages"].as_array().into_iter().flatten() {
        let (Some(from), Some(id)) = (m["from"].as_str(), m["id"].as_str()) else { continue };
        if m["group_id"].is_string() {
            continue;
        }
        let kind = m["type"].as_str().unwrap_or("");
        let mut i = Incoming { from: from.into(), name: name_of(from), id: id.into(), ..Default::default() };
        match kind {
            "text" => i.text = m["text"]["body"].as_str().unwrap_or("").trim().into(),
            "interactive" => {
                let r = &m["interactive"];
                i.tapped = r["button_reply"]["id"].as_str().or(r["list_reply"]["id"].as_str()).map(String::from);
            }
            "image" | "audio" | "document" | "video" | "sticker" => {
                let f = &m[kind];
                let Some(media) = f["id"].as_str() else { continue };
                let mime = f["mime_type"].as_str().unwrap_or("").split(';').next().unwrap_or("").trim().to_string();
                let ext = mime.rsplit('/').next().filter(|e| !e.is_empty() && e.chars().all(char::is_alphanumeric)).unwrap_or("bin");
                let name = f["filename"].as_str().filter(|n| !n.is_empty()).map(String::from).unwrap_or_else(|| format!("{media}.{ext}"));
                i.file = Some((name, mime, media.into(), f["voice"] == true));
                i.text = f["caption"].as_str().unwrap_or("").trim().into();
            }
            _ => continue,
        }
        if i.text.is_empty() && i.tapped.is_none() && i.file.is_none() {
            continue;
        }
        let c = &m["context"];
        i.quoted = c["id"].as_str().map(String::from);
        i.forwarded = c["forwarded"] == true || c["frequently_forwarded"] == true;
        i.sent = m["timestamp"].as_str().and_then(|t| t.parse().ok());
        out.push(i);
    }
    out
}

/// Up to three choices of at most 20 characters, on a message of at most 1024: WhatsApp's reply buttons.
fn buttons(text: &str, rows: &[Row]) -> Option<Value> {
    let all: Vec<&(String, String)> = rows.iter().flatten().collect();
    let fits = (1..=3).contains(&all.len()) && text.chars().count() <= 1024 && all.iter().all(|(l, d)| l.chars().count() <= 20 && !d.starts_with("https://"));
    fits.then(|| {
        let b: Vec<Value> = all.iter().map(|(l, d)| json!({"type": "reply", "reply": {"id": d, "title": l}})).collect();
        json!({"type": "interactive", "interactive": {"type": "button", "body": {"text": text}, "action": {"buttons": b}}})
    })
}

impl WhatsApp {
    /// (phone number id, access token, app secret). The Graph API is MIMIR_WHATSAPP_API's, else Meta's.
    pub(crate) fn new(id: &str, token: &str, secret: &str) -> Result<Self> {
        let (id, token, secret) = (id.trim(), token.trim(), secret.trim());
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            bail!("WhatsApp needs the bot's phone number id (digits, from the app's WhatsApp → API Setup page), not the number");
        }
        if token.is_empty() || secret.is_empty() {
            bail!("WhatsApp needs an access token and the Meta app's secret");
        }
        let api = std::env::var("MIMIR_WHATSAPP_API").unwrap_or_else(|_| "https://graph.facebook.com/v23.0".into());
        Ok(WhatsApp {
            http: reqwest::Client::builder().timeout(Duration::from_secs(60)).build()?,
            api: api.trim_end_matches('/').into(),
            id: id.into(),
            token: token.into(),
            secret: secret.into(),
            picks: Default::default(),
            heard: Default::default(),
            seen: Default::default(),
        })
    }

    /// A Graph API call; Meta's own words when it fails.
    async fn call(&self, req: reqwest::RequestBuilder) -> Result<Value> {
        let resp = req.bearer_auth(&self.token).send().await.map_err(|e| e.without_url())?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or_default();
        if !status.is_success() {
            bail!("whatsapp: {status} {}", v["error"]["message"].as_str().unwrap_or(""));
        }
        Ok(v)
    }

    /// Sends a message (`body`: its type and content) to a chat; its id.
    async fn post(&self, to: &str, mut body: Value) -> Result<String> {
        // Past the window Meta takes the message and drops it later: say so now, so it shows as not sent.
        if let Some((at, _)) = self.heard.lock().unwrap().get(to)
            && crate::db::now() - at > WINDOW_SECS
        {
            bail!("WhatsApp lets the bot write only within 24 hours of your last message: send it anything to get this");
        }
        body["messaging_product"] = json!("whatsapp");
        body["to"] = json!(to);
        let v = self.call(self.http.post(format!("{}/{}/messages", self.api, self.id)).json(&body)).await?;
        let msg = v["messages"][0]["id"].as_str().unwrap_or_default().to_string();
        self.picks.sent(to, &msg);
        Ok(msg)
    }

    /// Text, in parts under WhatsApp's cap; the last part's id. `reply`: the message it answers.
    async fn send(&self, to: &str, text: &str, reply: Option<&str>) -> Result<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut msg = String::new();
        for part in chars.chunks(MAX_TEXT) {
            let mut body = json!({"type": "text", "text": {"body": part.iter().collect::<String>()}});
            if let Some(r) = reply {
                body["context"] = json!({"message_id": r});
            }
            msg = self.post(to, body).await?;
        }
        Ok(msg)
    }

    /// Buttons when they fit, else a numbered list.
    async fn send_rows(&self, to: &str, mut text: String, rows: &[Row], card: bool) -> Result<String> {
        if let Some(body) = buttons(&text, rows) {
            return self.post(to, body).await;
        }
        let data = numbered(&mut text, rows, card);
        let msg = self.send(to, &text, None).await?;
        self.picks.offer(to, msg.clone(), card, data);
        Ok(msg)
    }

    pub(crate) async fn upload(&self, chat: &str, name: &str, bytes: Vec<u8>) -> Result<()> {
        let mime = match name.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "pdf" => "application/pdf",
            "txt" => "text/plain",
            _ => "application/octet-stream",
        };
        let part = reqwest::multipart::Part::bytes(bytes).file_name(name.to_string()).mime_str(mime)?;
        let form = reqwest::multipart::Form::new().text("messaging_product", "whatsapp").text("type", mime).part("file", part);
        let v = self.call(self.http.post(format!("{}/{}/media", self.api, self.id)).multipart(form)).await?;
        let media = v["id"].as_str().context("WhatsApp gave no media id")?;
        let body = if mime.starts_with("image/") { json!({"type": "image", "image": {"id": media}}) } else { json!({"type": "document", "document": {"id": media, "filename": name}}) };
        self.post(chat, body).await.map(drop)
    }

    /// Final reply as plain text (WhatsApp's own markup isn't Markdown), with any buttons.
    pub(crate) async fn finish(&self, chat: &str, _draft: Option<&str>, text: &str, rows: &[Row]) -> Result<()> {
        self.send_rows(chat, crate::fmt::plain(text), rows, false).await.map(drop)
    }

    pub(crate) async fn draft_post(&self, _chat: &str, _text: &str) -> Option<String> {
        None
    }

    /// Only a card is ever "edited" (there are no drafts): its outcome comes as a reply to it, and its numbers stop working.
    pub(crate) async fn draft_edit(&self, chat: &str, msg: &str, text: &str) {
        self.picks.drop(chat, msg);
        if let Err(e) = self.send(chat, text, Some(msg).filter(|m| !m.is_empty())).await {
            eprintln!("whatsapp: {e:#}");
        }
    }

    /// WhatsApp shows typing on the message being answered, for up to 25 seconds or until the reply.
    pub(crate) fn typing(&self, chat: &str) -> JoinHandle<()> {
        let (s, msg) = (self.clone(), self.heard.lock().unwrap().get(chat).map(|(_, m)| m.clone()));
        tokio::spawn(async move {
            let Some(msg) = msg else { return };
            let body = json!({"messaging_product": "whatsapp", "status": "read", "message_id": msg, "typing_indicator": {"type": "text"}});
            loop {
                let _ = s.http.post(format!("{}/{}/messages", s.api, s.id)).bearer_auth(&s.token).json(&body).send().await;
                tokio::time::sleep(Duration::from_secs(20)).await;
            }
        })
    }

    pub(crate) async fn card(&self, chat: &str, a: &Approval) -> Result<Option<String>> {
        self.send_rows(chat, card_text(a), &crate::runtime::card_buttons(a), true).await.map(Some)
    }

    /// The token reaches the bot's number: the number.
    pub(crate) async fn check(&self) -> Result<String> {
        let v = self.call(self.http.get(format!("{}/{}?fields=display_phone_number", self.api, self.id))).await.context("can't reach the bot's number on WhatsApp")?;
        Ok(v["display_phone_number"].as_str().unwrap_or(&self.id).to_string())
    }

    pub(crate) async fn listen(&self, me: &Chan, core: Arc<Core>, slot: Slot) -> Result<()> {
        if !SERVED.load(Ordering::Relaxed) {
            bail!("WhatsApp needs Mimir running as a server (mimir server) at an https address Meta can reach");
        }
        let handle = self.check().await?;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _hooked = Hooked::new(&self.id, &self.secret, tx);
        slot.connected(handle);
        while let Some(v) = rx.recv().await {
            for s in v["statuses"].as_array().into_iter().flatten().filter(|s| s["status"] == "failed") {
                eprintln!("whatsapp: a message to {} wasn't delivered: {}", s["recipient_id"], s["errors"][0]["title"]);
            }
            for m in messages_of(&v) {
                if let Err(e) = self.on_message(me, &core, m).await {
                    eprintln!("whatsapp message: {e:#}");
                }
            }
        }
        bail!("the webhook stopped")
    }

    /// A message not taken before, and not one Meta kept from more than a day ago.
    fn fresh(&self, m: &Incoming) -> bool {
        let mut seen = self.seen.lock().unwrap();
        if seen.contains(&m.id) || m.sent.is_some_and(|t| crate::db::now() - t > CATCH_UP_SECS) {
            return false;
        }
        seen.push_back(m.id.clone());
        if seen.len() > 500 {
            seen.pop_front();
        }
        true
    }

    async fn on_message(&self, me: &Chan, core: &Arc<Core>, m: Incoming) -> Result<()> {
        if !self.fresh(&m) {
            return Ok(());
        }
        self.heard.lock().unwrap().insert(m.from.clone(), (m.sent.unwrap_or_else(crate::db::now), m.id.clone()));
        let (chat, from) = (m.from.as_str(), Sender { id: &m.from, dm: true });
        if let Some(data) = &m.tapped {
            return choose(me, core, chat, from, data).await;
        }
        if m.file.is_none()
            && let Some(data) = self.picks.picked(chat, &m.text, m.quoted.as_deref())
        {
            return choose(me, core, chat, from, &data).await;
        }
        // A reply to a card waiting for an answer, not a number: no, and why.
        if m.file.is_none()
            && let Some(card) = m.quoted.as_deref()
            && core.deny_by_reply(me, chat, from, card, &m.text)?
        {
            return Ok(());
        }
        if let Some((name, mime, media, voice)) = m.file {
            if m.forwarded {
                core.taint(me, chat, from)?;
            }
            // A forwarded voice note is someone else's words, like an audio file.
            let files = [File { name, mime, size: 0, url: media, voice: voice && !m.forwarded }];
            return core.incoming_files(me, chat, from, &m.text, &files, m.sent, |f| self.download(f.url.clone())).await;
        }
        let title = format!("WhatsApp {}", m.name);
        if m.forwarded {
            let said = crate::with_document("(I forwarded you a message.)", "a forwarded message", &m.text);
            return core.incoming_outside(me, chat, from, title.trim_end(), &said, m.sent).await;
        }
        core.incoming(me, chat, from, title.trim_end(), &m.text, m.sent).await
    }

    /// A file someone sent: Meta gives a short-lived address for it, fetched with the token.
    async fn download(&self, media: String) -> Result<Vec<u8>> {
        if media.is_empty() || !media.chars().all(|c| c.is_ascii_digit()) {
            bail!("that isn't a WhatsApp file");
        }
        let v = self.call(self.http.get(format!("{}/{media}", self.api))).await?;
        let url = v["url"].as_str().context("WhatsApp gave no address for the file")?;
        Ok(self.http.get(url).bearer_auth(&self.token).send().await?.error_for_status()?.bytes().await?.to_vec())
    }
}

/// A bot's place in HOOKS while it listens; gone when it stops (unless a newer listener for its number took it).
struct Hooked(String, mpsc::UnboundedSender<Value>);

impl Hooked {
    fn new(id: &str, secret: &str, tx: mpsc::UnboundedSender<Value>) -> Self {
        HOOKS.lock().unwrap().insert(id.into(), (secret.into(), tx.clone()));
        Hooked(id.into(), tx)
    }
}

impl Drop for Hooked {
    fn drop(&mut self) {
        let mut hooks = HOOKS.lock().unwrap();
        if hooks.get(&self.0).is_some_and(|(_, tx)| tx.same_channel(&self.1)) {
            hooks.remove(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(messages: Value) -> Value {
        json!({"metadata": {"phone_number_id": "100"}, "contacts": [{"wa_id": "3161", "profile": {"name": "Ana"}}], "messages": messages})
    }

    #[test]
    fn messages_taps_files_and_forwards() {
        let v = value(json!([
            {"from": "3161", "id": "w1", "timestamp": "1700000000", "type": "text", "text": {"body": " hi "}},
            {"from": "3161", "id": "w2", "type": "interactive", "interactive": {"type": "button_reply", "button_reply": {"id": "ap:7:y", "title": "Approve"}}},
            {"from": "3161", "id": "w3", "type": "audio", "audio": {"id": "555", "mime_type": "audio/ogg; codecs=opus", "voice": true}},
            {"from": "3161", "id": "w4", "type": "document", "document": {"id": "556", "mime_type": "application/pdf", "filename": "trip.pdf", "caption": "read"}},
            {"from": "3161", "id": "w5", "type": "text", "text": {"body": "look"}, "context": {"forwarded": true}},
            {"from": "3161", "id": "w6", "type": "text", "text": {"body": "no"}, "context": {"from": "15550000", "id": "card-1"}},
            {"from": "3161", "id": "w7", "type": "reaction", "reaction": {"emoji": "👍"}},
            {"from": "3161", "id": "w8", "type": "text", "text": {"body": "hi all"}, "group_id": "g1"}
        ]));
        let m = messages_of(&v);
        assert_eq!(m.len(), 6, "reactions and groups are ignored");
        assert_eq!((m[0].from.as_str(), m[0].name.as_str(), m[0].text.as_str(), m[0].sent), ("3161", "Ana", "hi", Some(1_700_000_000)));
        assert_eq!(m[1].tapped.as_deref(), Some("ap:7:y"));
        assert_eq!(m[2].file, Some(("555.ogg".into(), "audio/ogg".into(), "555".into(), true)), "a voice note named for its kind of audio");
        assert_eq!((m[3].file.as_ref().unwrap().0.as_str(), m[3].text.as_str()), ("trip.pdf", "read"));
        assert!(m[4].forwarded && !m[0].forwarded);
        assert_eq!(m[5].quoted.as_deref(), Some("card-1"));
        assert!(messages_of(&json!({"statuses": [{"status": "read"}]})).is_empty());
    }

    #[test]
    fn buttons_when_they_fit() {
        let rows: Vec<Row> = vec![vec![("Approve".into(), "ap:7:y".into()), ("Deny".into(), "ap:7:n".into())]];
        let b = buttons("Send it?", &rows).unwrap();
        assert_eq!(b["interactive"]["action"]["buttons"][1], json!({"type": "reply", "reply": {"id": "ap:7:n", "title": "Deny"}}));
        let long: Vec<Row> = vec![vec![("Approve all like this".into(), "ap:7:a".into())]];
        assert!(buttons("x", &long).is_none(), "a label over 20 characters: numbered");
        let link: Vec<Row> = vec![vec![("Edit in Mimir".into(), "https://x/#approval-7".into())]];
        assert!(buttons("x", &link).is_none(), "a link isn't a button");
        assert!(buttons(&"x".repeat(1025), &rows).is_none());
        assert!(buttons("x", &[]).is_none());
    }

    #[test]
    fn only_signed_calls_reach_a_bot() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _h = Hooked::new("100", "s3cret", tx);
        let body = json!({"entry": [{"changes": [{"value": value(json!([]))}, {"value": {"metadata": {"phone_number_id": "999"}}}]}]}).to_string();
        let sig = format!("sha256={}", crate::events::hex(&crate::events::hmac(b"s3cret", body.as_bytes())));
        assert!(!webhook(Some("sha256=00"), body.as_bytes()));
        assert!(rx.try_recv().is_err(), "a wrong signature delivers nothing");
        assert!(webhook(Some(&sig), body.as_bytes()));
        assert_eq!(rx.try_recv().unwrap()["metadata"]["phone_number_id"], "100");
        assert!(rx.try_recv().is_err(), "another number's part stays out");
        assert_eq!(verify("subscribe", "100", "c"), Some("c".into()));
        assert_eq!(verify("subscribe", "101", "c"), None);
        drop(_h);
        assert_eq!(verify("subscribe", "100", "c"), None, "gone when it stops listening");
    }

    #[test]
    fn needs_the_number_id_and_keys() {
        assert!(WhatsApp::new("+31612345678", "t", "s").is_err());
        assert!(WhatsApp::new("100", "", "s").is_err());
        let w = WhatsApp::new(" 100 ", "t", "s").unwrap();
        let m = |id: &str, sent| Incoming { id: id.into(), sent, ..Default::default() };
        assert!(w.fresh(&m("w1", Some(crate::db::now()))));
        assert!(!w.fresh(&m("w1", Some(crate::db::now()))), "delivered again");
        assert!(!w.fresh(&m("w2", Some(crate::db::now() - 2 * 86_400))), "kept by Meta from days ago");
    }
}
