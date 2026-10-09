//! Slack via Socket Mode (outbound WebSocket, no public URL needed).
//! App setup: enable Socket Mode, create an app-level token (connections:write), bot scopes
//! chat:write, im:history, app_mentions:read, files:read (+ channels:history, groups:history to catch up channel
//! mentions sent while Mimir was off); subscribe to message.im and app_mention events;
//! enable Interactivity (for Approve/Deny buttons).

use crate::db::Approval;
use crate::runtime::{Chan, Core, File, Sender, Slot, card_text, parse_decision, verdict_for};
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone)]
pub(crate) struct Slack {
    http: reqwest::Client,
    bot_token: String,
    app_token: String,
    /// Channel → the thread of the last @-mention there, and when. Replies go into it, not the channel.
    threads: Arc<Mutex<HashMap<String, (String, Instant)>>>,
}

/// How long a mention's thread stays the place to answer (later posts, e.g. routines, go to the channel).
// ponytail: per-channel, so two people mentioning the bot at once share the newest thread; key by turn if that bites
const THREAD_FOR: Duration = Duration::from_secs(30 * 60);

/// Text of a message event meant for the bot: DMs, or channel messages that @-mention it.
/// Bot messages and edits/joins (subtypes) are ignored; a message with files (file_share) counts.
pub(crate) fn message_for_bot(event: &Value, my_user: &str) -> Option<String> {
    if !event["bot_id"].is_null() || !(event["subtype"].is_null() || event["subtype"] == "file_share") {
        return None;
    }
    let text = event["text"].as_str()?;
    match event["type"].as_str()? {
        "message" if event["channel_type"] == "im" => Some(text.trim().to_string()),
        "app_mention" => Some(text.replace(&format!("<@{my_user}>"), "").trim().to_string()),
        _ => None,
    }
}

impl Slack {
    pub(crate) fn new(bot_token: &str, app_token: &str) -> Result<Self> {
        if !bot_token.trim().starts_with("xoxb-") || !app_token.trim().starts_with("xapp-") {
            bail!("Slack needs a bot token (xoxb-…) and an app-level token (xapp-…)");
        }
        Ok(Slack {
            http: reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?,
            bot_token: bot_token.trim().into(),
            app_token: app_token.trim().into(),
            threads: Default::default(),
        })
    }

    async fn api(&self, method: &str, token: &str, body: Value) -> Result<Value> {
        let v: Value = self.http.post(format!("https://slack.com/api/{method}")).bearer_auth(token).json(&body).send().await?.json().await?;
        if v["ok"].as_bool() != Some(true) {
            bail!("slack {method}: {}", v["error"].as_str().unwrap_or("unknown error"));
        }
        Ok(v)
    }

    /// A new message in `channel`: in the thread of a recent @-mention there, if any.
    fn post(&self, channel: &str, mut body: Value) -> Value {
        body["channel"] = json!(channel);
        // No link previews (Slack fetches them itself: a link a misled bot wrote could carry data out unclicked).
        body["unfurl_links"] = json!(false);
        body["unfurl_media"] = json!(false);
        let threads = self.threads.lock().unwrap();
        if let Some((ts, _)) = threads.get(channel).filter(|(_, at)| at.elapsed() < THREAD_FOR) {
            body["thread_ts"] = json!(ts);
        }
        body
    }

    pub(crate) async fn send(&self, channel: &str, text: &str) -> Result<()> {
        self.finish(channel, None, text).await
    }

    /// Final reply in Slack mrkdwn; with a streamed draft, the draft becomes its first part.
    pub(crate) async fn finish(&self, channel: &str, draft: Option<&str>, text: &str) -> Result<()> {
        let mrkdwn = crate::fmt::slack(text);
        let chars: Vec<char> = mrkdwn.chars().collect();
        for (i, chunk) in chars.chunks(3900).enumerate() {
            let text = chunk.iter().collect::<String>();
            match draft.filter(|_| i == 0) {
                Some(ts) => self.api("chat.update", &self.bot_token, json!({"channel": channel, "ts": ts, "text": text})).await?,
                None => self.api("chat.postMessage", &self.bot_token, self.post(channel, json!({"text": text}))).await?,
            };
        }
        Ok(())
    }

    pub(crate) async fn draft_post(&self, channel: &str, text: &str) -> Option<String> {
        // Escaped like the final text, so a draft can't ping the channel (<!channel>, <!here>).
        self.api("chat.postMessage", &self.bot_token, self.post(channel, json!({"text": crate::fmt::esc(text)}))).await.ok()?["ts"].as_str().map(String::from)
    }

    pub(crate) async fn draft_edit(&self, channel: &str, ts: &str, text: &str) {
        let _ = self.api("chat.update", &self.bot_token, json!({"channel": channel, "ts": ts, "text": crate::fmt::esc(text), "blocks": []})).await;
    }

    pub(crate) async fn card(&self, channel: &str, a: &Approval) -> Result<Option<String>> {
        let text = card_text(a);
        let buttons: Vec<Value> = crate::runtime::card_buttons(a)
            .into_iter()
            .flatten()
            .map(|(label, data)| {
                // action_id is unique per card: the decision letter.
                let d = data.rsplit(':').next().unwrap_or("y").to_string();
                let mut b = json!({"type": "button", "action_id": format!("ap_{d}"), "value": data, "text": {"type": "plain_text", "text": label}});
                match d.as_str() {
                    "y" => b["style"] = json!("primary"),
                    "n" => b["style"] = json!("danger"),
                    _ => {}
                }
                b
            })
            .collect();
        let blocks = json!([
            {"type": "section", "text": {"type": "plain_text", "text": text}},
            {"type": "actions", "elements": buttons}
        ]);
        let m = self.api("chat.postMessage", &self.bot_token, self.post(channel, json!({"text": text, "blocks": blocks}))).await?;
        Ok(m["ts"].as_str().map(String::from))
    }

    pub(crate) async fn listen(&self, me: &Chan, core: Arc<Core>, slot: Slot) -> Result<()> {
        let auth = self.api("auth.test", &self.bot_token, json!({})).await.context("Slack rejected the bot token")?;
        let my_user = auth["user_id"].as_str().unwrap_or("").to_string();
        let url = self.api("apps.connections.open", &self.app_token, json!({})).await.context("Slack rejected the app-level token")?["url"]
            .as_str()
            .context("no socket url")?
            .to_string();
        let (mut ws, _) = tokio_tungstenite::connect_async(url.as_str()).await?;
        slot.connected(format!("{} in {}", auth["user"].as_str().unwrap_or("bot"), auth["team"].as_str().unwrap_or("Slack")));
        // ponytail: inline, so events queue meanwhile and Slack may resend them (mark_seen drops those)
        self.catch_up(me, &core, &my_user).await;

        while let Some(msg) = ws.next().await {
            let text = match msg? {
                Message::Text(t) => t,
                Message::Close(f) => bail!("socket closed: {f:?}"),
                _ => continue,
            };
            let v: Value = serde_json::from_str(&text)?;
            // Every envelope must be acked within 3s or Slack retries it.
            if let Some(id) = v["envelope_id"].as_str() {
                ws.send(Message::text(json!({"envelope_id": id}).to_string())).await?;
            }
            let r = match v["type"].as_str().unwrap_or("") {
                "disconnect" => bail!("Slack asked to reconnect"),
                "events_api" => self.on_message(me, &core, &v["payload"]["event"], &my_user).await,
                "interactive" => self.on_button(me, &core, &v["payload"]).await,
                _ => Ok(()),
            };
            if let Err(e) = r {
                eprintln!("slack event: {e:#}");
            }
        }
        bail!("socket ended")
    }

    /// A message event (live, or fetched on reconnect): text, or files with it.
    async fn on_message(&self, me: &Chan, core: &Arc<Core>, ev: &Value, my_user: &str) -> Result<()> {
        let Some(t) = message_for_bot(ev, my_user) else { return Ok(()) };
        let ch = ev["channel"].as_str().unwrap_or("");
        if let Some(k) = ev["ts"].as_str().and_then(ts_key)
            && !core.mark_seen(me, ch, k)
        {
            return Ok(());
        }
        // Only the paired person's mention decides where replies go (a stranger's would make the owner's next answer
        // land in their thread).
        if ev["type"] == "app_mention" && core.is_owner(me, ch, ev["user"].as_str().unwrap_or("")) {
            // Answer in the thread: the one it was asked in, or a new one under the mention.
            let ts = ev["thread_ts"].as_str().or(ev["ts"].as_str()).unwrap_or("").to_string();
            self.threads.lock().unwrap().insert(ch.to_string(), (ts, Instant::now()));
        }
        let title = if ev["channel_type"] == "im" { format!("Slack DM {}", ev["user"].as_str().unwrap_or("")) } else { format!("Slack {ch}") };
        let sent = ev["ts"].as_str().and_then(|ts| ts.split('.').next()?.parse().ok());
        let from = Sender { id: ev["user"].as_str().unwrap_or(""), dm: ev["channel_type"] == "im" };
        let files = files_of(ev);
        if !files.is_empty() {
            return core.incoming_files(me, ch, from, &t, &files, sent, |f| self.download(f.url.clone(), f.mime.clone())).await;
        }
        core.incoming(me, ch, from, &title, &t, sent).await
    }

    /// What came in the paired chats while Mimir was away (Socket Mode doesn't replay it), up to a day back: DMs, and
    /// @-mentions in channels. (Replies inside threads aren't in a channel's history.)
    async fn catch_up(&self, me: &Chan, core: &Arc<Core>, my_user: &str) {
        let day_ago = (crate::db::now() - crate::runtime::CATCH_UP_SECS).max(0) as u128 * 1_000_000;
        for (ch, dm) in core.paired_on(me) {
            let Some(after) = core.seen(me, &ch) else { continue };
            let oldest = after.max(day_ago);
            let oldest = format!("{}.{:06}", oldest / 1_000_000, oldest % 1_000_000);
            let (mut msgs, mut cursor) = (vec![], String::new());
            // Slack gives the newest first, page by page: gather them all, then answer the oldest first.
            // ponytail: at most 10 pages (1000 messages) per chat
            for _ in 0..10 {
                let mut q = vec![("channel", ch.as_str()), ("oldest", oldest.as_str()), ("limit", "100")];
                if !cursor.is_empty() {
                    q.push(("cursor", cursor.as_str()));
                }
                let got: Result<Value> = async {
                    let url = url::Url::parse_with_params("https://slack.com/api/conversations.history", &q)?;
                    let v: Value = self.http.get(url).bearer_auth(&self.bot_token).send().await?.json().await?;
                    if v["ok"].as_bool() != Some(true) {
                        bail!("{}", v["error"].as_str().unwrap_or("unknown error"));
                    }
                    Ok(v)
                }
                .await;
                match got {
                    Ok(v) => {
                        msgs.extend(v["messages"].as_array().cloned().unwrap_or_default());
                        cursor = v["response_metadata"]["next_cursor"].as_str().unwrap_or("").to_string();
                        if v["has_more"] != true || cursor.is_empty() {
                            break;
                        }
                    }
                    Err(e) => {
                        // Channels need channels:history (groups:history for private ones); without it, DMs still work.
                        let hint = if e.to_string() == "missing_scope" { " (add channels:history and groups:history to the Slack app)" } else { "" };
                        eprintln!("slack catch-up {ch}: {e:#}{hint}");
                        break;
                    }
                }
            }
            msgs.sort_by_key(|m| m["ts"].as_str().and_then(ts_key));
            for mut m in msgs {
                m["channel"] = json!(ch);
                if dm {
                    m["channel_type"] = json!("im");
                } else if m["text"].as_str().is_some_and(|t| t.contains(&format!("<@{my_user}>"))) {
                    // Live, a channel message reaches the bot only as an app_mention event; fetched, it's a plain
                    // message, so one that mentions the bot is treated as that event.
                    m["type"] = json!("app_mention");
                    m["channel_type"] = json!("channel");
                }
                if let Err(e) = self.on_message(me, core, &m, my_user).await {
                    eprintln!("slack catch-up {ch}: {e:#}");
                }
                // Chatter not meant for the bot is seen too, so the next catch-up starts after it.
                if let Some(k) = m["ts"].as_str().and_then(ts_key) {
                    core.mark_seen(me, &ch, k);
                }
            }
        }
    }

    /// A file someone sent. The bot token goes only to Slack's own file host.
    async fn download(&self, url: String, mime: String) -> Result<Vec<u8>> {
        let u = url::Url::parse(&url)?;
        if u.scheme() != "https" || u.host_str() != Some("files.slack.com") {
            bail!("that file isn't on Slack's file host");
        }
        let r = self.http.get(u).bearer_auth(&self.bot_token).send().await?.error_for_status()?;
        // Without the files:read scope Slack answers with its sign-in page instead of the file.
        let html = r.headers().get(reqwest::header::CONTENT_TYPE).and_then(|t| t.to_str().ok()).is_some_and(|t| t.starts_with("text/html"));
        if html && !mime.contains("html") {
            bail!("Slack didn't hand over the file: add the files:read scope to the Slack app and reinstall it");
        }
        Ok(r.bytes().await?.to_vec())
    }

    async fn on_button(&self, me: &Chan, core: &Core, p: &Value) -> Result<()> {
        if p["type"] != "block_actions" {
            return Ok(());
        }
        let Some((id, d)) = parse_decision(p["actions"][0]["value"].as_str().unwrap_or("")) else { return Ok(()) };
        let channel = p["channel"]["id"].as_str().unwrap_or("");
        let who = Sender { id: p["user"]["id"].as_str().unwrap_or(""), dm: channel.starts_with('D') };
        let text = match core.decide(me, channel, who, id, d) {
            Ok(Some(a)) => verdict_for(&a, d),
            Ok(None) => "This request is no longer pending.".into(),
            // Someone else in the channel pressed it: tell only them, leave the card as it is.
            Err(e) => {
                self.api("chat.postEphemeral", &self.bot_token, json!({"channel": channel, "user": who.id, "text": e.to_string()})).await?;
                return Ok(());
            }
        };
        self.api("chat.update", &self.bot_token, json!({"channel": channel, "ts": p["message"]["ts"], "text": text, "blocks": []})).await?;
        Ok(())
    }
}

/// Slack's message ts ("1712345678.123456") as microseconds, to order messages.
fn ts_key(ts: &str) -> Option<u128> {
    let (secs, micros) = ts.split_once('.')?;
    Some(secs.parse::<u128>().ok()? * 1_000_000 + micros.parse::<u128>().ok()?)
}

/// The files shared with a message (external ones, like Google Drive links, can't be fetched: no url).
fn files_of(ev: &Value) -> Vec<File> {
    let files = ev["files"].as_array().map(Vec::as_slice).unwrap_or_default();
    files
        .iter()
        .map(|f| File {
            name: f["name"].as_str().unwrap_or("file").into(),
            mime: f["mimetype"].as_str().unwrap_or("").into(),
            size: f["size"].as_u64().unwrap_or(0),
            url: f["url_private_download"].as_str().unwrap_or("").into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_events() {
        let dm = json!({"type":"message","channel_type":"im","text":" hi ","user":"U1","channel":"D1"});
        assert_eq!(message_for_bot(&dm, "UBOT").as_deref(), Some("hi"));
        let mention = json!({"type":"app_mention","text":"<@UBOT> what's up","channel":"C1"});
        assert_eq!(message_for_bot(&mention, "UBOT").as_deref(), Some("what's up"));
        let chatter = json!({"type":"message","channel_type":"channel","text":"hello all"});
        assert_eq!(message_for_bot(&chatter, "UBOT"), None, "channel chatter without a mention");
        let own = json!({"type":"message","channel_type":"im","text":"echo","bot_id":"B1"});
        assert_eq!(message_for_bot(&own, "UBOT"), None, "bot messages (incl. our own) ignored");
        let edit = json!({"type":"message","channel_type":"im","text":"x","subtype":"message_changed"});
        assert_eq!(message_for_bot(&edit, "UBOT"), None);
        let file = json!({"type":"message","channel_type":"im","text":"what's this?","subtype":"file_share","files":[
            {"name":"a.pdf","mimetype":"application/pdf","size":5,"url_private_download":"https://files.slack.com/a.pdf"},
            {"name":"Doc","mimetype":"application/vnd.google-apps.document","mode":"external"}]});
        assert_eq!(message_for_bot(&file, "UBOT").as_deref(), Some("what's this?"), "a message with a file counts");
        let f = files_of(&file);
        assert_eq!((f[0].name.as_str(), f[0].mime.as_str(), f[0].size, f[0].url.as_str()), ("a.pdf", "application/pdf", 5, "https://files.slack.com/a.pdf"));
        assert_eq!(f[1].url, "", "an external file can't be fetched");
        assert!(ts_key("1712345678.000200") < ts_key("1712345678.001000"));
        assert_eq!(ts_key("1712345678.123456"), Some(1_712_345_678_123_456));
    }

    #[test]
    fn replies_go_in_the_mention_thread() {
        let s = Slack::new("xoxb-1", "xapp-1").unwrap();
        assert!(s.post("C1", json!({"text": "hi"}))["thread_ts"].is_null(), "no mention yet: top level");
        s.threads.lock().unwrap().insert("C1".into(), ("171.5".into(), Instant::now()));
        assert_eq!(s.post("C1", json!({"text": "hi"}))["thread_ts"], "171.5");
        assert!(s.post("D9", json!({"text": "hi"}))["thread_ts"].is_null(), "other channels unaffected");
        s.threads.lock().unwrap().insert("C1".into(), ("171.5".into(), Instant::now() - THREAD_FOR));
        assert!(s.post("C1", json!({"text": "routine"}))["thread_ts"].is_null(), "an old mention: back to the channel");
    }
}
