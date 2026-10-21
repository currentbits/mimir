//! Signal through signal-cli-rest-api (github.com/bbernhard/signal-cli-rest-api) in json-rpc mode: messages arrive on
//! its WebSocket, replies go out with /v2/send. The bot is its own Signal number, linked to signal-cli as a device.
//! Direct chats only: group messages are ignored. Signal has no buttons, so a card's or a question's choices are
//! numbered, and a reply that is just a number picks one.
// ponytail: no streaming drafts (Signal allows only a few edits per message); typing shows instead

use crate::db::Approval;
use crate::runtime::{Chan, Core, File, Row, Sender, Slot, card_text, parse_decision, verdict_for};
use crate::say::tr;
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone)]
pub(crate) struct Signal {
    http: reqwest::Client,
    api: String,
    /// The bot's own number, as signal-cli knows it ("+31…").
    number: String,
    picks: Picks,
}

/// Chat → the bot's latest message there and the numbered choices still open (Signal, and WhatsApp when its buttons
/// won't fit).
#[derive(Clone, Default)]
pub(crate) struct Picks(Arc<Mutex<HashMap<String, Numbers>>>);

#[derive(Default)]
struct Numbers {
    /// The bot's latest message in the chat (its Signal timestamp): a bare number answers only that one.
    last: String,
    open: Vec<Choices>,
}

struct Choices {
    /// The message they're under.
    msg: String,
    /// An approval card's stay open until it's decided (answered later by replying to the card); a question's go with
    /// the bot's next message.
    card: bool,
    /// Each choice's button data, in order.
    data: Vec<String>,
}

/// A direct message to the bot, from a signal-cli envelope.
#[derive(Debug, PartialEq)]
pub(crate) struct Incoming {
    /// Who wrote it (their Signal account id, else their number): also the chat.
    from: String,
    name: String,
    text: String,
    /// Their own words, without the message they replied to.
    said: String,
    /// The bot's message they replied to (its Signal timestamp).
    quoted: Option<String>,
    /// Attachments; `url` is signal-cli's attachment id.
    /// (name, mime, size, id, recorded in Signal as a voice note)
    files: Vec<(String, String, u64, String, bool)>,
    sent: Option<i64>,
}

/// A message in a direct chat, not a group's, the bot's own, a receipt or a typing note. A reply to one of the bot's
/// messages says which.
pub(crate) fn message_of(env: &Value, me: &str) -> Option<Incoming> {
    let d = &env["dataMessage"];
    if !d.is_object() || d["groupInfo"].is_object() || d["groupV2"].is_object() || env["sourceNumber"] == me || env["source"] == me {
        return None;
    }
    let from = env["sourceUuid"].as_str().or(env["sourceNumber"].as_str()).or(env["source"].as_str()).filter(|s| !s.is_empty())?;
    let files: Vec<_> = d["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let (id, mime) = (a["id"].as_str()?, a["contentType"].as_str().unwrap_or(""));
            // A voice note has no file name: its id, with the kind of audio it is, for the transcriber.
            let named = a["filename"].as_str().filter(|n| !n.is_empty());
            let name = match named {
                Some(n) => n.to_string(),
                None if id.contains('.') => id.to_string(),
                None => format!("{id}.{}", mime.rsplit('/').next().filter(|e| e.chars().all(char::is_alphanumeric) && !e.is_empty()).unwrap_or("bin")),
            };
            // A voice note recorded in Signal is the person's own words; an audio file someone made isn't.
            let voice = a["voiceNote"] == true || (named.is_none() && mime.starts_with("audio/"));
            Some((name, mime.to_string(), a["size"].as_u64().unwrap_or(0), id.to_string(), voice))
        })
        .collect();
    let text = d["message"].as_str().unwrap_or("").trim().to_string();
    if text.is_empty() && files.is_empty() {
        return None;
    }
    let q = &d["quote"];
    let mine = q["authorNumber"] == me || q["author"] == me;
    let quoted = q["id"].as_i64().filter(|_| mine).map(|id| id.to_string());
    let said = text.clone();
    let text = match q["text"].as_str().filter(|t| mine && !t.trim().is_empty()) {
        Some(quoted) => crate::with_reply(&text, quoted),
        None => text,
    };
    let name = env["sourceName"].as_str().unwrap_or("").to_string();
    Some(Incoming { from: from.into(), name, text, said, quoted, files, sent: env["timestamp"].as_i64().map(|t| t / 1000) })
}

/// Buttons as a numbered list under the text (a card's say how to answer it later); returns each number's button data.
pub(crate) fn numbered(text: &mut String, rows: &[Row], card: bool) -> Vec<String> {
    let all: Vec<&(String, String)> = rows.iter().flatten().collect();
    if all.is_empty() {
        return vec![];
    }
    // A link isn't a choice: it goes on its own line, after them.
    let (links, all): (Vec<_>, Vec<_>) = all.into_iter().partition(|(_, d)| d.starts_with("https://"));
    text.push_str("\n\n");
    for (n, (label, _)) in all.iter().enumerate() {
        text.push_str(&format!("{}. {label}\n", n + 1));
    }
    for (label, url) in links {
        text.push_str(&format!("{label}: {url}\n"));
    }
    text.push_str(if card { tr("(Reply with a number. If I've written since, reply to this message with it.)") } else { tr("(Reply with a number.)") });
    all.into_iter().map(|(_, d)| d.clone()).collect()
}

/// "2", "2." or " 2 " → 2.
pub(crate) fn pick(text: &str) -> Option<usize> {
    text.trim().trim_end_matches('.').parse().ok().filter(|n| *n > 0)
}

impl Picks {
    /// The bot wrote in a chat: a bare number there now answers only this message, and a question's numbers are gone.
    pub(crate) fn sent(&self, chat: &str, msg: &str) {
        let mut choices = self.0.lock().unwrap();
        let n = choices.entry(chat.into()).or_default();
        n.last = msg.into();
        n.open.retain(|c| c.card);
    }

    /// The numbered choices under a message just sent.
    pub(crate) fn offer(&self, chat: &str, msg: String, card: bool, data: Vec<String>) {
        if !data.is_empty() {
            self.0.lock().unwrap().entry(chat.into()).or_default().open.push(Choices { msg, card, data });
        }
    }

    /// The button a reply of just a number picks: on the message it replies to, else on the bot's latest message. So a
    /// number meant for something the bot wrote later can't answer an older card.
    pub(crate) fn picked(&self, chat: &str, text: &str, quoted: Option<&str>) -> Option<String> {
        let n = pick(text)?;
        let mut choices = self.0.lock().unwrap();
        let c = choices.get_mut(chat)?;
        let under = quoted.unwrap_or(&c.last).to_string();
        let i = c.open.iter().position(|o| o.msg == under)?;
        let data = c.open[i].data.get(n - 1)?.clone();
        c.open.remove(i);
        Some(data)
    }

    /// A card was decided: its numbers stop working.
    pub(crate) fn drop(&self, chat: &str, msg: &str) {
        if let Some(c) = self.0.lock().unwrap().get_mut(chat) {
            c.open.retain(|o| o.msg != msg);
        }
    }
}

/// A number picked from a card or a question (or a button tapped on WhatsApp): what pressing that button does
/// elsewhere, said back.
pub(crate) async fn choose(me: &Chan, core: &Arc<Core>, chat: &str, from: Sender<'_>, data: &str) -> Result<()> {
    let said = match parse_decision(data) {
        Some((id, d)) => match core.decide(me, chat, from, id, d) {
            Ok(Some(a)) => verdict_for(&a, d),
            Ok(None) => tr("This request is no longer pending.").into(),
            Err(e) => e.to_string(),
        },
        None => match core.tap(me, chat, from, data).await {
            // "Something else" has already asked them to type it.
            Ok(Some(did)) if did != tr("Type your answer.") => did,
            Ok(_) => return Ok(()),
            Err(e) => e.to_string(),
        },
    };
    me.send(chat, &said).await;
    Ok(())
}

impl Signal {
    /// `api` empty: MIMIR_SIGNAL_API, else port 8081 on this computer (8080 is `mimir server`'s own).
    pub(crate) fn new(api: &str, number: &str) -> Result<Self> {
        let api = match api.trim() {
            "" => std::env::var("MIMIR_SIGNAL_API").unwrap_or_else(|_| "http://127.0.0.1:8081".into()),
            a => a.into(),
        };
        let number: String = number.chars().filter(|c| !c.is_whitespace()).collect();
        if !number.starts_with('+') {
            bail!("Signal needs the bot's number with its country code (+…)");
        }
        Ok(Signal { http: reqwest::Client::builder().timeout(Duration::from_secs(60)).build()?, api: api.trim_end_matches('/').into(), number, picks: Default::default() })
    }

    /// Sends a message (and attachments as data: URIs); its Signal timestamp, which is its id.
    async fn send(&self, to: &str, text: &str, files: Vec<String>) -> Result<String> {
        let body = json!({"number": self.number, "recipients": [to], "message": text, "base64_attachments": files});
        let resp = self.http.post(format!("{}/v2/send", self.api)).json(&body).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            bail!("signal send: {status} {}", crate::tools::truncate(&text, 200));
        }
        let v: Value = serde_json::from_str(&text).unwrap_or_default();
        let msg = v["timestamp"].as_str().map(String::from).or(v["timestamp"].as_i64().map(|t| t.to_string())).unwrap_or_default();
        self.picks.sent(to, &msg);
        Ok(msg)
    }

    pub(crate) async fn upload(&self, chat: &str, name: &str, bytes: Vec<u8>) -> Result<()> {
        use base64::Engine;
        let mime = match name.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "pdf" => "application/pdf",
            _ => "application/octet-stream",
        };
        let name = name.replace([';', ','], "_");
        let uri = format!("data:{mime};filename={name};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
        self.send(chat, "", vec![uri]).await.map(drop)
    }

    /// Final reply as plain text (Signal shows no Markdown), with any buttons as a numbered list.
    pub(crate) async fn finish(&self, chat: &str, _draft: Option<&str>, text: &str, rows: &[Row]) -> Result<()> {
        let mut text = crate::fmt::plain(text);
        let data = numbered(&mut text, rows, false);
        let msg = self.send(chat, &text, vec![]).await?;
        self.picks.offer(chat, msg, false, data);
        Ok(())
    }

    pub(crate) async fn draft_post(&self, _chat: &str, _text: &str) -> Option<String> {
        None
    }

    /// Only a card is ever "edited" (there are no drafts): its outcome comes as a new message, and its numbers stop working.
    pub(crate) async fn draft_edit(&self, chat: &str, msg: &str, text: &str) {
        self.picks.drop(chat, msg);
        if let Err(e) = self.send(chat, text, vec![]).await {
            eprintln!("signal: {e:#}");
        }
    }

    pub(crate) fn typing(&self, chat: &str) -> JoinHandle<()> {
        let (s, body) = (self.clone(), json!({"recipient": chat}));
        tokio::spawn(async move {
            loop {
                let _ = s.http.put(format!("{}/v1/typing-indicator/{}", s.api, s.number)).json(&body).send().await;
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        })
    }

    pub(crate) async fn card(&self, chat: &str, a: &Approval) -> Result<Option<String>> {
        let mut text = card_text(a);
        let data = numbered(&mut text, &crate::runtime::card_buttons(a), true);
        let msg = self.send(chat, &text, vec![]).await?;
        self.picks.offer(chat, msg.clone(), true, data);
        Ok(Some(msg))
    }

    /// signal-cli-rest-api answers, and has the bot's number linked: the number.
    pub(crate) async fn check(&self) -> Result<String> {
        let accounts: Value = self.http.get(format!("{}/v1/accounts", self.api)).send().await.with_context(|| format!("can't reach signal-cli-rest-api at {}", self.api))?.json().await?;
        if !accounts.as_array().is_some_and(|a| a.iter().any(|n| n == self.number.as_str())) {
            bail!("{} isn't linked to signal-cli yet: open {}/v1/qrcodelink?device_name=mimir and scan it in Signal (Settings → Linked devices)", self.number, self.api);
        }
        Ok(self.number.clone())
    }

    pub(crate) async fn listen(&self, me: &Chan, core: Arc<Core>, slot: Slot) -> Result<()> {
        self.check().await?;
        let url = format!("{}/v1/receive/{}", self.api.replacen("http", "ws", 1), self.number);
        let (mut ws, _) = tokio_tungstenite::connect_async(url.as_str()).await.context("signal-cli-rest-api has to run with MODE=json-rpc")?;
        slot.connected(self.number.clone());
        // A connection that died quietly shows up when a ping can't go out.
        let mut ping = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                _ = ping.tick() => ws.send(Message::Ping(Default::default())).await?,
                msg = ws.next() => {
                    let text = match msg.context("signal-cli closed the connection")?? {
                        Message::Text(t) => t,
                        Message::Close(f) => bail!("signal-cli closed the connection: {f:?}"),
                        _ => continue,
                    };
                    let v: Value = serde_json::from_str(&text).unwrap_or_default();
                    if let Err(e) = self.on_message(me, &core, &v["envelope"]).await {
                        eprintln!("signal message: {e:#}");
                    }
                }
            }
        }
    }

    async fn on_message(&self, me: &Chan, core: &Arc<Core>, env: &Value) -> Result<()> {
        let Some(m) = message_of(env, &self.number) else { return Ok(()) };
        let (chat, from) = (m.from.as_str(), Sender { id: &m.from, dm: true });
        if m.files.is_empty()
            && let Some(data) = self.picks.picked(chat, &m.said, m.quoted.as_deref())
        {
            return choose(me, core, chat, from, &data).await;
        }
        if m.files.is_empty()
            && let Some(card) = m.quoted.as_deref()
            && let Some(data) = core.card_pick(me, chat, card, &m.said)?
        {
            return choose(me, core, chat, from, &data).await;
        }
        // A reply to a card waiting for an answer, not a number: no, and why.
        if m.files.is_empty()
            && let Some(card) = m.quoted.as_deref()
            && core.deny_by_reply(me, chat, from, card, &m.said)?
        {
            return Ok(());
        }
        if !m.files.is_empty() {
            let files: Vec<File> = m.files.iter().map(|(name, mime, size, id, voice)| File { name: name.clone(), mime: mime.clone(), size: *size, url: id.clone(), voice: *voice }).collect();
            return core.incoming_files(me, chat, from, &m.text, &files, m.sent, |f| self.download(f.url.clone())).await;
        }
        core.incoming(me, chat, from, format!("Signal {}", m.name).trim_end(), &m.text, m.sent).await
    }

    /// An attachment someone sent, as signal-cli saved it.
    async fn download(&self, id: String) -> Result<Vec<u8>> {
        if id.is_empty() || id.contains(['/', '\\', '?', '#']) || id.contains("..") {
            bail!("that isn't a Signal attachment");
        }
        Ok(self.http.get(format!("{}/v1/attachments/{id}", self.api)).send().await?.error_for_status()?.bytes().await?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "+15550000";

    #[test]
    fn direct_messages_only() {
        let env = |d: Value| json!({"sourceUuid": "u-1", "sourceNumber": "+3161", "sourceName": "Ana", "timestamp": 1_700_000_000_123i64, "dataMessage": d});
        let m = message_of(&env(json!({"message": " hi "})), ME).unwrap();
        assert_eq!((m.from.as_str(), m.name.as_str(), m.text.as_str(), m.sent), ("u-1", "Ana", "hi", Some(1_700_000_000)));
        assert!(message_of(&env(json!({"message": "hi", "groupInfo": {"groupId": "g"}})), ME).is_none(), "groups are ignored");
        assert!(message_of(&json!({"sourceNumber": "+3161", "typingMessage": {"action": "STARTED"}}), ME).is_none());
        assert!(message_of(&json!({"sourceNumber": "+3161", "receiptMessage": {"isRead": true}}), ME).is_none());
        assert!(message_of(&env(json!({"message": null})), ME).is_none(), "a reaction, a deletion: nothing to answer");
        let own = json!({"sourceNumber": ME, "source": ME, "dataMessage": {"message": "echo"}});
        assert!(message_of(&own, ME).is_none(), "the bot's own");
        let old = json!({"source": "+3161", "dataMessage": {"message": "hey"}});
        assert_eq!(message_of(&old, ME).unwrap().from, "+3161", "no account id: the number");
    }

    #[test]
    fn voice_notes_files_and_replies() {
        let env = json!({"sourceUuid": "u-1", "dataMessage": {"message": null, "attachments": [
            {"contentType": "audio/aac", "filename": null, "id": "Ab12", "size": 900},
            {"contentType": "application/pdf", "filename": "trip.pdf", "id": "Cd34.pdf", "size": 5}]}});
        let m = message_of(&env, ME).unwrap();
        assert_eq!(m.text, "");
        assert_eq!(m.files[0], ("Ab12.aac".into(), "audio/aac".into(), 900, "Ab12".into(), true), "a voice note named for its kind of audio");
        assert_eq!(m.files[1].0, "trip.pdf");
        assert!(!m.files[1].4);
        let song = json!({"sourceUuid": "u-1", "dataMessage": {"attachments": [{"contentType": "audio/mpeg", "filename": "song.mp3", "id": "Ef56.mp3", "size": 9}]}});
        assert!(!message_of(&song, ME).unwrap().files[0].4, "an audio file is outside content");
        let reply = json!({"sourceUuid": "u-1", "dataMessage": {"message": "move it", "quote": {"id": 1_700_000_000_999i64, "author": ME, "authorNumber": ME, "text": "2. Call Sam"}}});
        let m = message_of(&reply, ME).unwrap();
        assert_eq!((m.text, m.said.as_str(), m.quoted.as_deref()), (crate::with_reply("move it", "2. Call Sam"), "move it", Some("1700000000999")));
        let theirs = json!({"sourceUuid": "u-1", "dataMessage": {"message": "this", "quote": {"author": "+3162", "text": "x"}}});
        let m = message_of(&theirs, ME).unwrap();
        assert_eq!((m.text.as_str(), m.quoted), ("this", None), "only the bot's own messages are quoted");
    }

    #[test]
    fn choices_are_numbered_and_a_number_picks_one() {
        let rows: Vec<Row> = vec![vec![("Approve".into(), "ap:7:y".into()), ("Deny".into(), "ap:7:n".into())], vec![("Always".into(), "ap:7:w".into())]];
        let mut text = "🔐 Approve this action?".to_string();
        let data = numbered(&mut text, &rows, true);
        assert_eq!(text, "🔐 Approve this action?\n\n1. Approve\n2. Deny\n3. Always\n(Reply with a number. If I've written since, reply to this message with it.)");
        assert_eq!(data, ["ap:7:y", "ap:7:n", "ap:7:w"]);
        assert!(numbered(&mut String::new(), &rows, false).len() == 3);
        assert_eq!((pick(" 2 "), pick("3."), pick("0"), pick("2 please")), (Some(2), Some(3), None, None));

        let s = Signal::new("http://x", "+1 555 0000").unwrap();
        assert_eq!(s.number, "+15550000");
        assert!(Signal::new("", "0612").is_err(), "needs the country code");
        // What `send` then `finish` / `card` do for a message.
        let post = |msg: &str, card: bool, data: Vec<String>| {
            s.picks.sent("u-1", msg);
            s.picks.offer("u-1", msg.into(), card, data);
        };
        post("100", true, data.clone());
        assert_eq!(s.picks.picked("u-1", "4", None), None, "out of range");
        assert_eq!(s.picks.picked("u-2", "1", None), None, "another chat's");
        assert_eq!(s.picks.picked("u-1", "2", None).as_deref(), Some("ap:7:n"), "the card is the latest message");
        assert_eq!(s.picks.picked("u-1", "2", None), None, "used up");

        // Hours later the bot writes something else, its own list in plain text: "1" is an answer to that, not to the
        // card. Replying to the card still answers it.
        post("200", true, data);
        post("201", false, vec![]);
        assert_eq!(s.picks.picked("u-1", "1", None), None, "a bare number can't reach an older card");
        assert_eq!(s.picks.picked("u-1", "1", Some("201")), None);
        assert_eq!(s.picks.picked("u-1", "1", Some("200")).as_deref(), Some("ap:7:y"), "replying to the card");

        // A question's numbers go with the bot's next message, even when that message is replied to.
        post("300", false, vec!["tp:a:0".into()]);
        post("301", false, vec![]);
        assert_eq!(s.picks.picked("u-1", "1", Some("300")), None);
    }
}
