//! Discord via the Gateway (outbound WebSocket) + REST.
//! Bot setup: enable the Message Content intent in the Developer Portal, invite the bot with
//! Send Messages + Read Message History + Attach Files (+ Send Messages in Threads). It answers DMs, and in servers only when
//! @-mentioned; a thread under a paired channel counts as that channel, and is answered in the thread.
// ponytail: no RESUME — on any gateway hiccup we reconnect and re-IDENTIFY

use crate::db::Approval;
use crate::runtime::{Chan, Core, File, Row, Sender, Slot, card_text, parse_decision, verdict_for};
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

const API: &str = "https://discord.com/api/v10";
const GATEWAY: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
/// GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT
const INTENTS: u64 = (1 << 9) | (1 << 12) | (1 << 15);

#[derive(Clone)]
pub(crate) struct Discord {
    http: reqwest::Client,
    token: String,
    /// Channel → the @-mention waiting for an answer: the bot's next message there replies to it.
    mentions: Arc<Mutex<HashMap<String, String>>>,
    /// Paired channel → the thread of the last mention there, and when: replies go into it, not the channel.
    threads: Arc<Mutex<HashMap<String, (String, Instant)>>>,
    /// Channel id → its parent channel if it's a thread (None if it isn't), so a thread uses its channel's pairing.
    parents: Arc<Mutex<HashMap<String, Option<String>>>>,
}

/// How long a mention's thread stays the place to answer (later posts, e.g. routines, go to the channel).
const THREAD_FOR: Duration = Duration::from_secs(30 * 60);

/// Text of a MESSAGE_CREATE meant for the bot: DMs, or server messages that @-mention it.
pub(crate) fn message_for_bot(d: &Value, my_id: &str) -> Option<String> {
    if d["author"]["bot"].as_bool() == Some(true) {
        return None;
    }
    let text = d["content"].as_str()?;
    if d["guild_id"].is_null() {
        return Some(text.trim().to_string());
    }
    let mentioned = d["mentions"].as_array()?.iter().any(|m| m["id"] == my_id);
    mentioned.then(|| text.replace(&format!("<@{my_id}>"), "").replace(&format!("<@!{my_id}>"), "").trim().to_string())
}

impl Discord {
    pub(crate) fn new(token: &str) -> Result<Self> {
        Ok(Discord { http: reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?, token: token.trim().into(), mentions: Default::default(), threads: Default::default(), parents: Default::default() })
    }

    /// A new message in `channel`; the first one after an @-mention is a reply to it, so it's clear who it answers.
    fn post(&self, channel: &str, mut body: Value) -> Value {
        if let Some(id) = self.mentions.lock().unwrap().remove(channel) {
            body["message_reference"] = json!({"message_id": id, "fail_if_not_exists": false});
        }
        body
    }

    /// Where a message for a paired channel goes: the thread of a recent mention there, if any.
    fn dest(&self, channel: &str) -> String {
        let threads = self.threads.lock().unwrap();
        threads.get(channel).filter(|(_, at)| at.elapsed() < THREAD_FOR).map_or(channel, |(t, _)| t.as_str()).to_string()
    }

    /// The paired chat a message or button press in `raw` belongs to: `raw` itself, or for a thread under a paired
    /// channel, that channel.
    async fn home(&self, me: &Chan, core: &Core, raw: &str) -> String {
        if core.is_paired(me, raw).unwrap_or(false) {
            return raw.to_string();
        }
        let known = self.parents.lock().unwrap().get(raw).cloned();
        let parent = match known {
            Some(p) => p,
            None => {
                // 10, 11, 12: announcement, public and private threads.
                let path = format!("/channels/{raw}");
                let mut got = self.api(reqwest::Method::GET, &path, None).await;
                if got.is_err() {
                    // Once more (a rate limit, a hiccup): otherwise the owner's message in a thread goes unanswered.
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    got = self.api(reqwest::Method::GET, &path, None).await;
                }
                let parent = match got {
                    Ok(c) if matches!(c["type"].as_u64(), Some(10..=12)) => c["parent_id"].as_str().map(String::from),
                    Ok(_) => None,
                    Err(e) => {
                        eprintln!("discord: can't tell if {raw} is a thread: {e:#}");
                        return raw.to_string();
                    }
                };
                self.parents.lock().unwrap().insert(raw.to_string(), parent.clone());
                parent
            }
        };
        parent.filter(|p| core.is_paired(me, p).unwrap_or(false)).unwrap_or_else(|| raw.to_string())
    }

    async fn api(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut req = self.http.request(method, format!("{API}{path}")).header("Authorization", format!("Bot {}", self.token));
        if let Some(mut b) = body {
            // Every message the bot writes: no pings (@everyone, roles, people), and no link previews, which Discord
            // fetches on its own (a link written by a misled bot could carry data out without anyone clicking it).
            let quiet = |m: &mut Value| {
                if m.is_object() {
                    m["allowed_mentions"] = json!({"parse": []});
                    m["flags"] = json!(m["flags"].as_u64().unwrap_or(0) | 4);
                }
            };
            if path.starts_with("/interactions/") {
                quiet(&mut b["data"]);
            } else if path.contains("/messages") {
                quiet(&mut b);
            }
            req = req.json(&b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            bail!("discord {path}: {status} {}", crate::tools::truncate(&text, 200));
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// A file the bot sends (needs Attach Files), as an attachment in the same place as a reply.
    pub(crate) async fn upload(&self, channel: &str, name: &str, bytes: Vec<u8>) -> Result<()> {
        let to = self.dest(channel);
        let payload = json!({"attachments": [{"id": 0, "filename": name}], "allowed_mentions": {"parse": []}});
        let form = reqwest::multipart::Form::new().text("payload_json", payload.to_string()).part("files[0]", reqwest::multipart::Part::bytes(bytes).file_name(name.to_string()));
        let resp = self.http.post(format!("{API}/channels/{to}/messages")).header("Authorization", format!("Bot {}", self.token)).multipart(form).send().await?;
        let status = resp.status();
        if !status.is_success() {
            bail!("discord upload: {status} {}", crate::tools::truncate(&resp.text().await.unwrap_or_default(), 200));
        }
        Ok(())
    }

    /// Final reply (Discord speaks Markdown), with buttons under its last part; with a streamed draft, the draft
    /// becomes its first part.
    pub(crate) async fn finish(&self, channel: &str, draft: Option<&str>, text: &str, rows: &[Row]) -> Result<()> {
        // Discord caps messages at 2000 chars. The rest follows the draft into its thread, if it's in one.
        let mut to = self.dest(channel);
        let chars: Vec<char> = text.chars().collect();
        let last = chars.len().saturating_sub(1) / 1900;
        for (i, chunk) in chars.chunks(1900).enumerate() {
            let mut body = json!({"content": chunk.iter().collect::<String>()});
            if i == last && !rows.is_empty() {
                // A row of grey buttons per row (Discord takes five rows of five, labels up to 80 characters).
                let row = |r: &Row| json!({"type": 1, "components": r.iter().map(|(l, d)| json!({"type": 2, "style": 2, "label": crate::tools::truncate(l, 80), "custom_id": d})).collect::<Vec<_>>()});
                body["components"] = json!(rows.iter().take(5).map(row).collect::<Vec<_>>());
            }
            let body = Some(body);
            match draft.filter(|_| i == 0) {
                Some(d) => {
                    let (at, id) = self.at(channel, d);
                    to = at;
                    self.api(reqwest::Method::PATCH, &format!("/channels/{to}/messages/{id}"), body).await?
                }
                None => self.api(reqwest::Method::POST, &format!("/channels/{to}/messages"), body.map(|b| self.post(channel, b))).await?,
            };
        }
        Ok(())
    }

    /// A message the bot posted and edits later (a draft, a card), as "<where it is>/<id>": a thread is a channel of
    /// its own, and where replies go can change while it's being written.
    fn at<'a>(&self, channel: &str, posted: &'a str) -> (String, &'a str) {
        match posted.split_once('/') {
            Some((to, id)) => (to.to_string(), id),
            None => (self.dest(channel), posted),
        }
    }

    pub(crate) async fn draft_post(&self, channel: &str, text: &str) -> Option<String> {
        let to = self.dest(channel);
        let m = self.api(reqwest::Method::POST, &format!("/channels/{to}/messages"), Some(self.post(channel, json!({"content": text})))).await.ok()?;
        m["id"].as_str().map(|id| format!("{to}/{id}"))
    }

    pub(crate) async fn draft_edit(&self, channel: &str, posted: &str, text: &str) {
        let (to, id) = self.at(channel, posted);
        let _ = self.api(reqwest::Method::PATCH, &format!("/channels/{to}/messages/{id}"), Some(json!({"content": text, "components": []}))).await;
    }

    pub(crate) fn typing(&self, channel: &str) -> JoinHandle<()> {
        let (d, path) = (self.clone(), format!("/channels/{}/typing", self.dest(channel)));
        tokio::spawn(async move {
            loop {
                let _ = d.api(reqwest::Method::POST, &path, None).await;
                tokio::time::sleep(Duration::from_secs(8)).await;
            }
        })
    }

    pub(crate) async fn card(&self, channel: &str, a: &Approval) -> Result<Option<String>> {
        // One row (Discord allows five buttons in it): green Approve, red Deny, grey for the rest.
        let buttons: Vec<Value> = crate::runtime::card_buttons(a)
            .into_iter()
            .flatten()
            .map(|(label, data)| {
                let style = if data.ends_with(":y") { 3 } else if data.ends_with(":n") { 4 } else { 2 };
                json!({"type": 2, "style": style, "label": label, "custom_id": data})
            })
            .collect();
        let body = json!({"content": card_text(a), "components": [{"type": 1, "components": buttons}]});
        let to = self.dest(channel);
        let m = self.api(reqwest::Method::POST, &format!("/channels/{to}/messages"), Some(self.post(channel, body))).await?;
        Ok(m["id"].as_str().map(|id| format!("{to}/{id}")))
    }

    /// For `mimir doctor`: the token works. The bot's name.
    pub(crate) async fn check(&self) -> Result<String> {
        let me = self.api(reqwest::Method::GET, "/users/@me", None).await.context("Discord rejected the bot token")?;
        Ok(me["username"].as_str().unwrap_or("?").to_string())
    }

    pub(crate) async fn listen(&self, me: &Chan, core: Arc<Core>, slot: Slot) -> Result<()> {
        self.api(reqwest::Method::GET, "/users/@me", None).await.context("Discord rejected the bot token")?;
        let (mut ws, _) = tokio_tungstenite::connect_async(GATEWAY).await?;
        let hello = next_json(&mut ws).await?;
        let every = hello["d"]["heartbeat_interval"].as_u64().context("no HELLO from gateway")?;
        ws.send(Message::text(
            json!({"op": 2, "d": {"token": self.token, "intents": INTENTS, "properties": {"os": std::env::consts::OS, "browser": "mimir", "device": "mimir"}}})
                .to_string(),
        ))
        .await?;

        let mut heartbeat = tokio::time::interval(Duration::from_millis(every));
        heartbeat.tick().await;
        let (mut seq, mut my_id) = (Value::Null, String::new());
        loop {
            tokio::select! {
                _ = heartbeat.tick() => ws.send(Message::text(json!({"op": 1, "d": seq}).to_string())).await?,
                msg = ws.next() => {
                    let text = match msg.context("gateway closed")?? {
                        Message::Text(t) => t,
                        Message::Close(Some(f)) if u16::from(f.code) == 4014 =>
                            bail!("Discord refused the Message Content intent: enable it under Bot → Privileged Gateway Intents"),
                        Message::Close(f) => bail!("gateway closed: {f:?}"),
                        _ => continue,
                    };
                    let v: Value = serde_json::from_str(&text)?;
                    if !v["s"].is_null() {
                        seq = v["s"].clone();
                    }
                    match v["op"].as_i64() {
                        Some(1) => ws.send(Message::text(json!({"op": 1, "d": seq}).to_string())).await?,
                        Some(7) | Some(9) => bail!("gateway asked to reconnect"),
                        Some(0) => {
                            let d = &v["d"];
                            let r = match v["t"].as_str().unwrap_or("") {
                                "READY" => {
                                    my_id = d["user"]["id"].as_str().unwrap_or("").to_string();
                                    slot.connected(format!("@{}", d["user"]["username"].as_str().unwrap_or("bot")));
                                    // ponytail: inline, delaying heartbeats; a long catch-up may get us reconnected, and
                                    // mark_seen keeps that from answering twice
                                    self.catch_up(me, &core, &my_id).await;
                                    Ok(())
                                }
                                "MESSAGE_CREATE" => self.on_message(me, &core, d, &my_id).await,
                                "INTERACTION_CREATE" => self.on_button(me, &core, d).await,
                                _ => Ok(()),
                            };
                            if let Err(e) = r {
                                eprintln!("discord event: {e:#}");
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// A later message from the paired person in the thread the bot was last mentioned in: for the bot, without
    /// another @-mention. Anyone else's stays ignored.
    async fn followed(&self, me: &Chan, core: &Arc<Core>, d: &Value) -> Option<String> {
        let raw = d["channel_id"].as_str()?;
        if d["guild_id"].is_null() || d["author"]["bot"].as_bool() == Some(true) || !self.threads.lock().unwrap().values().any(|(t, _)| t == raw) {
            return None;
        }
        let home = self.home(me, core, raw).await;
        let mut threads = self.threads.lock().unwrap();
        let t = threads.get_mut(&home).filter(|(t, at)| t == raw && at.elapsed() < THREAD_FOR)?;
        if !core.is_owner(me, &home, d["author"]["id"].as_str().unwrap_or("")) {
            return None;
        }
        t.1 = Instant::now();
        d["content"].as_str().map(|c| c.trim().to_string()).filter(|c| !c.is_empty())
    }

    /// A message (live, or fetched on reconnect): text, or attachments with it.
    async fn on_message(&self, me: &Chan, core: &Arc<Core>, d: &Value, my_id: &str) -> Result<()> {
        let Some(t) = (match message_for_bot(d, my_id) {
            Some(t) => Some(t),
            None => self.followed(me, core, d).await,
        }) else {
            return Ok(());
        };
        let raw = d["channel_id"].as_str().unwrap_or("");
        let home = if d["guild_id"].is_null() { raw.to_string() } else { self.home(me, core, raw).await };
        let ch = home.as_str();
        if let Some(id) = d["id"].as_str().and_then(|i| i.parse().ok())
            && !core.mark_seen(me, ch, id)
        {
            return Ok(());
        }
        if !d["guild_id"].is_null()
            && let Some(id) = d["id"].as_str()
            && core.is_owner(me, ch, d["author"]["id"].as_str().unwrap_or(""))
        {
            self.mentions.lock().unwrap().insert(ch.to_string(), id.to_string());
            // Asked in a thread under the paired channel: answer there.
            if raw != ch {
                self.threads.lock().unwrap().insert(ch.to_string(), (raw.to_string(), Instant::now()));
            } else {
                self.threads.lock().unwrap().remove(ch);
            }
        }
        let title = if d["guild_id"].is_null() { format!("Discord DM {}", d["author"]["username"].as_str().unwrap_or("")) } else { format!("Discord #{ch}") };
        let sent = d["timestamp"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.timestamp());
        let from = Sender { id: d["author"]["id"].as_str().unwrap_or(""), dm: d["guild_id"].is_null() };
        let files = files_of(d);
        if !files.is_empty() {
            return core.incoming_files(me, ch, from, &t, &files, sent, |f| self.download(f.url.clone())).await;
        }
        // A reply to one of the bot's messages: which one.
        let r = &d["referenced_message"];
        let t = match r["content"].as_str().filter(|q| r["author"]["id"] == my_id && !q.trim().is_empty()) {
            Some(quoted) => crate::with_reply(&t, quoted),
            None => t,
        };
        core.incoming(me, ch, from, &title, &t, sent).await
    }

    /// What came in the paired chats while Mimir was away (the gateway doesn't replay it), up to a day back.
    async fn catch_up(&self, me: &Chan, core: &Arc<Core>, my_id: &str) {
        let day_ago = snowflake_at(crate::db::now() - crate::runtime::CATCH_UP_SECS);
        for (ch, dm) in core.paired_on(me) {
            let Some(seen) = core.seen(me, &ch) else { continue };
            let mut after = seen.max(day_ago);
            // Pages of 50 after the cursor, oldest page first; busy channels count chatter too.
            // ponytail: at most 20 pages (1000 messages) per channel
            for _ in 0..20 {
                let page = match self.api(reqwest::Method::GET, &format!("/channels/{ch}/messages?after={after}&limit=50"), None).await {
                    Ok(v) => v.as_array().cloned().unwrap_or_default(),
                    Err(e) => {
                        eprintln!("discord catch-up {ch}: {e:#}");
                        break;
                    }
                };
                let full = page.len() == 50;
                // Newest first within a page; answer in the order they were written.
                for mut m in page.into_iter().rev() {
                    let Some(id) = m["id"].as_str().and_then(|i| i.parse::<u128>().ok()) else { continue };
                    // Fetched messages don't say which server they're in; the gateway's do, and that's what tells a
                    // DM from a server message that needs a mention.
                    if !dm {
                        m["guild_id"] = json!("?");
                    }
                    if let Err(e) = self.on_message(me, core, &m, my_id).await {
                        eprintln!("discord catch-up {ch}: {e:#}");
                    }
                    // Chatter not meant for the bot is seen too, so the next catch-up starts after it.
                    core.mark_seen(me, &ch, id);
                    after = after.max(id);
                }
                if !full {
                    break;
                }
            }
        }
    }

    /// An attachment someone sent, from Discord's own file hosts.
    async fn download(&self, url: String) -> Result<Vec<u8>> {
        let u = url::Url::parse(&url)?;
        if u.scheme() != "https" || !matches!(u.host_str(), Some("cdn.discordapp.com" | "media.discordapp.net")) {
            bail!("that file isn't on Discord's file host");
        }
        Ok(self.http.get(u).send().await?.error_for_status()?.bytes().await?.to_vec())
    }

    async fn on_button(&self, me: &Chan, core: &Arc<Core>, d: &Value) -> Result<()> {
        let data = d["data"]["custom_id"].as_str().unwrap_or("");
        if parse_decision(data).is_none() && !data.starts_with("tp:") {
            return Ok(());
        }
        let raw = d["channel_id"].as_str().unwrap_or("");
        let channel = &if d["guild_id"].is_null() { raw.to_string() } else { self.home(me, core, raw).await };
        // 7 = update the message the button is on; 4 + ephemeral flag = private reply.
        let from = Sender { id: d["member"]["user"]["id"].as_str().or(d["user"]["id"].as_str()).unwrap_or(""), dm: d["guild_id"].is_null() };
        let private = |text: String| json!({"type": 4, "data": {"content": text, "flags": 64}});
        let body = match parse_decision(data) {
            Some((id, choice)) => match core.decide(me, channel, from, id, choice) {
                Ok(Some(a)) => json!({"type": 7, "data": {"content": verdict_for(&a, choice), "components": []}}),
                Ok(None) => private(crate::say::tr("This request is no longer pending.").into()),
                Err(e) => private(e.to_string()),
            },
            // The buttons give way to a small line saying what the tap did; the message above them stays.
            None => match core.tap(me, channel, from, data).await {
                Ok(Some(did)) => json!({"type": 7, "data": {"content": format!("{}\n-# {}", d["message"]["content"].as_str().unwrap_or(""), crate::tools::truncate(&did, 90)), "components": []}}),
                Ok(None) => return Ok(()),
                Err(e) => private(e.to_string()),
            },
        };
        let path = format!("/interactions/{}/{}/callback", d["id"].as_str().unwrap_or(""), d["token"].as_str().unwrap_or(""));
        self.api(reqwest::Method::POST, &path, Some(body)).await?;
        Ok(())
    }
}

/// The smallest Discord id at a unix time (ids start with milliseconds since 2015).
fn snowflake_at(unix: i64) -> u128 {
    ((unix * 1000 - 1_420_070_400_000).max(0) as u128) << 22
}

/// A message's attachments (a voice message is one, audio/ogg).
fn files_of(d: &Value) -> Vec<File> {
    let files = d["attachments"].as_array().map(Vec::as_slice).unwrap_or_default();
    files
        .iter()
        .map(|f| File {
            name: f["filename"].as_str().unwrap_or("file").into(),
            mime: f["content_type"].as_str().unwrap_or("").split(';').next().unwrap_or("").trim().into(),
            size: f["size"].as_u64().unwrap_or(0),
            url: f["url"].as_str().unwrap_or("").into(),
            // A voice message recorded in Discord carries its waveform; an uploaded audio file doesn't.
            voice: f["waveform"].is_string(),
        })
        .collect()
}

async fn next_json<S>(ws: &mut S) -> Result<Value>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Some(m) = ws.next().await {
        if let Message::Text(t) = m? {
            return Ok(serde_json::from_str(&t)?);
        }
    }
    bail!("gateway closed before HELLO")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_message_is_yours_an_audio_file_isnt() {
        let d = json!({"attachments": [
            {"filename": "voice-message.ogg", "content_type": "audio/ogg", "size": 9, "url": "u", "waveform": "AAAA", "duration_secs": 2.0},
            {"filename": "song.mp3", "content_type": "audio/mpeg", "size": 9, "url": "u"}
        ]});
        let f = files_of(&d);
        assert!(f[0].voice && !f[1].voice);
    }

    /// Network: proves the rustls WebSocket path and HELLO parsing against the real gateway (no token needed).
    #[tokio::test]
    #[ignore]
    async fn gateway_hello() {
        let (mut ws, _) = tokio_tungstenite::connect_async(GATEWAY).await.unwrap();
        let hello = next_json(&mut ws).await.unwrap();
        assert_eq!(hello["op"], 10);
        assert!(hello["d"]["heartbeat_interval"].as_u64().unwrap() > 1000);
    }

    #[test]
    fn first_post_after_a_mention_replies_to_it() {
        let d = Discord::new("t").unwrap();
        assert!(d.post("C1", json!({"content": "x"}))["message_reference"].is_null());
        d.mentions.lock().unwrap().insert("C1".into(), "99".into());
        assert_eq!(d.post("C1", json!({"content": "draft"}))["message_reference"]["message_id"], "99");
        assert!(d.post("C1", json!({"content": "part 2"}))["message_reference"].is_null(), "only once");
    }

    #[test]
    fn replies_go_in_the_mention_thread() {
        let d = Discord::new("t").unwrap();
        assert_eq!(d.dest("C1"), "C1");
        d.threads.lock().unwrap().insert("C1".into(), ("T7".into(), Instant::now()));
        assert_eq!(d.dest("C1"), "T7");
        assert_eq!(d.dest("C2"), "C2", "other channels unaffected");
        d.threads.lock().unwrap().insert("C1".into(), ("T7".into(), Instant::now() - THREAD_FOR));
        assert_eq!(d.dest("C1"), "C1", "an old mention: back to the channel");
        assert_eq!(d.at("C1", "T7/55"), ("T7".to_string(), "55"), "a draft stays where it was posted");
        assert_eq!(d.at("C1", "55"), ("C1".to_string(), "55"));
        // Discord's own example id: 175928847299117063 was made at 1462015105796 ms.
        assert!(snowflake_at(1_462_015_105) <= 175928847299117063 && 175928847299117063 < snowflake_at(1_462_015_106));
    }

    #[test]
    fn filters_messages() {
        let dm = json!({"content":" hey ","author":{"id":"1","username":"w"},"channel_id":"9"});
        assert_eq!(message_for_bot(&dm, "B").as_deref(), Some("hey"));
        let guild_mention = json!({"content":"<@B> status?","guild_id":"G","author":{"id":"1"},"mentions":[{"id":"B"}]});
        assert_eq!(message_for_bot(&guild_mention, "B").as_deref(), Some("status?"));
        let nick_mention = json!({"content":"<@!B> hi","guild_id":"G","author":{"id":"1"},"mentions":[{"id":"B"}]});
        assert_eq!(message_for_bot(&nick_mention, "B").as_deref(), Some("hi"));
        let chatter = json!({"content":"lunch?","guild_id":"G","author":{"id":"1"},"mentions":[]});
        assert_eq!(message_for_bot(&chatter, "B"), None);
        let voice = json!({"content":"","author":{"id":"1"},"attachments":[{"filename":"voice-message.ogg","content_type":"audio/ogg; codecs=opus","size":9,"url":"https://cdn.discordapp.com/v.ogg"}]});
        assert_eq!(message_for_bot(&voice, "B").as_deref(), Some(""));
        assert_eq!(files_of(&voice)[0].mime, "audio/ogg");
        let bot = json!({"content":"beep","author":{"id":"2","bot":true}});
        assert_eq!(message_for_bot(&bot, "B"), None);
    }
}
