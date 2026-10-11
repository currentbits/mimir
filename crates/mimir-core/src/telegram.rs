//! Telegram via the raw Bot API (long-poll getUpdates).
// ponytail: plain reqwest, no teloxide; adopt it if we need media/inline queries

use crate::db::Approval;
use crate::runtime::{Chan, Core, Row, Sender, Slot, card_text, parse_decision, verdict_for};
use crate::say::tr;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

#[derive(Clone)]
pub(crate) struct Tg {
    http: reqwest::Client,
    base: String,
    /// Where files sent to the bot are downloaded from.
    files: String,
    /// The next update to fetch, kept across reconnects: an update that made handling fail isn't fetched again.
    offset: std::sync::Arc<std::sync::atomic::AtomicI64>,
}

impl Tg {
    pub(crate) fn new(token: &str) -> Result<Self> {
        // MIMIR_TELEGRAM_API: a self-hosted Bot API server (or a test double).
        let api = std::env::var("MIMIR_TELEGRAM_API").unwrap_or_else(|_| "https://api.telegram.org".into());
        let api = api.trim_end_matches('/');
        Ok(Tg {
            http: reqwest::Client::builder().timeout(Duration::from_secs(70)).build()?,
            base: format!("{api}/bot{}", token.trim()),
            files: format!("{api}/file/bot{}", token.trim()),
            offset: Default::default(),
        })
    }

    async fn call(&self, method: &str, mut body: Value) -> Result<Value> {
        // No link previews: Telegram fetches them itself, so a link a misled bot wrote could carry data out unclicked.
        if matches!(method, "sendMessage" | "editMessageText") && body.get("link_preview_options").is_none() {
            body["link_preview_options"] = json!({"is_disabled": true});
        }
        // The token is in the URL: errors drop it (they reach logs and the bot's status in the app).
        let v: Value = self.http.post(format!("{}/{method}", self.base)).json(&body).send().await.map_err(|e| e.without_url())?.json().await.map_err(|e| e.without_url())?;
        if v["ok"].as_bool() != Some(true) {
            bail!("telegram {method}: {}", v["description"]);
        }
        Ok(v["result"].clone())
    }

    /// Markdown → Telegram HTML, split between blocks under the 4096-char cap. If Telegram rejects the markup, that piece
    /// goes out as plain text instead of being lost. Buttons go under the last part. With a streamed draft, the draft is replaced by the first part.
    pub(crate) async fn finish(&self, chat: &str, draft: Option<&str>, text: &str, rows: &[Row]) -> Result<()> {
        let draft = draft.and_then(|d| d.parse::<i64>().ok());
        let parts = crate::fmt::telegram(text, 3800);
        let last = parts.len().saturating_sub(1);
        for (i, html) in parts.into_iter().enumerate() {
            let mut msg = json!({"chat_id": chat, "text": html, "parse_mode": "HTML", "link_preview_options": {"is_disabled": true}});
            let buttons = (i == last && !rows.is_empty()).then(|| keyboard(rows));
            if let Some(k) = &buttons {
                msg["reply_markup"] = k.clone();
            }
            let method = match draft.filter(|_| i == 0) {
                Some(mid) => {
                    msg["message_id"] = json!(mid);
                    "editMessageText"
                }
                None => "sendMessage",
            };
            if self.call(method, msg.clone()).await.is_err() {
                // Rejected markup: the same text, plain, rather than nothing.
                let plain: Vec<char> = crate::fmt::html_to_plain(&html).chars().collect();
                let chunks = plain.chunks(4000).count();
                for (j, chunk) in plain.chunks(4000).enumerate() {
                    msg["text"] = json!(chunk.iter().collect::<String>());
                    msg.as_object_mut().unwrap().remove("parse_mode");
                    if j > 0 {
                        msg.as_object_mut().unwrap().remove("message_id");
                    }
                    // The buttons stay under the very last piece.
                    match (&buttons, j + 1 == chunks) {
                        (Some(k), true) => msg["reply_markup"] = k.clone(),
                        _ => drop(msg.as_object_mut().unwrap().remove("reply_markup")),
                    }
                    self.call(if j == 0 { method } else { "sendMessage" }, msg.clone()).await?;
                }
            }
        }
        Ok(())
    }

    /// Streaming drafts are plain text (half-written Markdown can be unbalanced); `finish` formats.
    /// Private chats get Telegram's native streaming draft (Bot API 10), with its own stop button; the final
    /// reply is then sent as a new message (`finish` sees "draft:N" isn't a message id). Groups, and any
    /// error, fall back to posting a message and editing it.
    pub(crate) async fn draft_post(&self, chat: &str, text: &str) -> Option<String> {
        if chat.parse::<i64>().is_ok_and(|c| c > 0) {
            let id = crate::random_hex(4).ok().and_then(|h| u32::from_str_radix(&h, 16).ok()).map_or(1, |n| (n & 0x7fff_ffff).max(1));
            if self.draft(chat, id, text).await {
                return Some(format!("draft:{id}"));
            }
        }
        self.call("sendMessage", json!({"chat_id": chat, "text": text})).await.ok().and_then(|m| m["message_id"].as_i64()).map(|id| id.to_string())
    }

    pub(crate) async fn draft_edit(&self, chat: &str, id: &str, text: &str) {
        if let Some(d) = id.strip_prefix("draft:").and_then(|d| d.parse().ok()) {
            self.draft(chat, d, text).await;
            return;
        }
        let _ = self.call("editMessageText", json!({"chat_id": chat, "message_id": id.parse::<i64>().unwrap_or(0), "text": text})).await;
    }

    async fn draft(&self, chat: &str, id: u32, text: &str) -> bool {
        // Drafts can't turn link previews off (sendMessageDraft has no such option), so links in one aren't left
        // recognisable as links while it's written: an invisible word joiner inside "://". The final message has
        // previews off and its links whole.
        let text = text.replace("://", ":/\u{2060}/");
        self.call("sendMessageDraft", json!({"chat_id": chat, "draft_id": id, "text": text, "can_stop": true})).await.is_ok()
    }

    pub(crate) fn typing(&self, chat: &str) -> JoinHandle<()> {
        let (tg, chat) = (self.clone(), chat.to_string());
        tokio::spawn(async move {
            loop {
                let _ = tg.call("sendChatAction", json!({"chat_id": chat, "action": "typing"})).await;
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        })
    }

    pub(crate) async fn card(&self, chat: &str, a: &Approval) -> Result<Option<String>> {
        let m = self.call(
            "sendMessage",
            json!({
                "chat_id": chat,
                "text": card_text(a),
                "reply_markup": keyboard(&crate::runtime::card_buttons(a))
            }),
        )
        .await?;
        Ok(m["message_id"].as_i64().map(|id| id.to_string()))
    }

    pub(crate) async fn listen(&self, me: &Chan, core: Arc<Core>, slot: Slot) -> Result<()> {
        let bot = self.call("getMe", json!({})).await.context("Telegram rejected the bot token")?;
        let me_id = bot["id"].as_i64();
        slot.connected(format!("@{}", bot["username"].as_str().unwrap_or("?")));
        // The "/" menu in Telegram shows these.
        let base = json!([
            {"command": "memory", "description": tr("What I remember about you")},
            {"command": "routines", "description": tr("What I do on a schedule")},
            {"command": "why", "description": tr("What my last reply used")},
            {"command": "new", "description": tr("Start a fresh conversation (memories kept)")},
            {"command": "stop", "description": tr("Stop the reply I'm writing")}
        ]);
        let _ = self.call("setMyCommands", json!({"commands": base})).await;
        // Each chat's menu also lists the bot's skills there ("/book_padel"), refreshed every 10 minutes when it changed.
        let mut menus: std::collections::HashMap<i64, Value> = Default::default();
        let mut menus_at: Option<std::time::Instant> = None;
        let mut offset: i64 = self.offset.load(std::sync::atomic::Ordering::Relaxed);
        loop {
            if menus_at.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(600)) {
                menus_at = Some(std::time::Instant::now());
                let mut now: std::collections::HashMap<i64, Value> = core
                    .skill_menus()
                    .into_iter()
                    .map(|(chat, skills)| {
                        let mut cmds = base.as_array().cloned().unwrap_or_default();
                        cmds.extend(skills.into_iter().take(90).map(|(c, d)| json!({"command": c, "description": d})));
                        (chat, Value::Array(cmds))
                    })
                    .collect();
                // A chat whose last skill went: back to the plain menu.
                for chat in menus.keys().filter(|c| !now.contains_key(c)).copied().collect::<Vec<_>>() {
                    now.insert(chat, base.clone());
                }
                for (chat, cmds) in &now {
                    if menus.get(chat) != Some(cmds) {
                        let scope = json!({"type": "chat", "chat_id": chat});
                        if let Err(e) = self.call("setMyCommands", json!({"commands": cmds, "scope": scope})).await {
                            eprintln!("telegram menu: {e:#}");
                        }
                    }
                }
                now.retain(|_, v| *v != base);
                menus = now;
            }
            let updates = self.call("getUpdates", json!({"offset": offset, "timeout": 50, "allowed_updates": ["message", "callback_query", "stopped_message_generation"]})).await?;
            for u in updates.as_array().into_iter().flatten() {
                offset = offset.max(u["update_id"].as_i64().unwrap_or(0) + 1);
                self.offset.store(offset, std::sync::atomic::Ordering::Relaxed);
                let r = if u["callback_query"].is_object() {
                    self.on_button(me, &core, &u["callback_query"]).await
                } else if let Some(chat) = u["stopped_message_generation"]["chat"]["id"].as_i64() {
                    // The stop button on a streaming draft: same as /stop.
                    let chat = chat.to_string();
                    core.stop(me, &chat, Sender { id: &chat, dm: true }).map(drop)
                } else if let (Some(chat), Some(text)) = (u["message"]["chat"]["id"].as_i64(), u["message"]["text"].as_str()) {
                    let c = &u["message"]["chat"];
                    let title = c["title"].as_str().or(c["username"].as_str()).or(c["first_name"].as_str()).unwrap_or("");
                    let from = u["message"]["from"]["id"].as_i64().map(|i| i.to_string()).unwrap_or_default();
                    let who = Sender { id: &from, dm: c["type"] == "private" };
                    if u["message"]["forward_origin"].is_object() {
                        // A forwarded message is someone else's words: outside content, like a file.
                        let said = crate::with_document("(I forwarded you a message.)", "a forwarded message", text);
                        core.incoming_outside(me, &chat.to_string(), who, title, &said, u["message"]["date"].as_i64()).await
                    } else if let Some(quoted) = replied_to(&u["message"], me_id) {
                        core.incoming(me, &chat.to_string(), who, title, &crate::with_reply(text, &quoted), u["message"]["date"].as_i64()).await
                    } else {
                        core.incoming(me, &chat.to_string(), who, title, text, u["message"]["date"].as_i64()).await
                    }
                } else if let Some(chat) = u["message"]["chat"]["id"].as_i64().filter(|_| u["message"]["chat"]["type"] == "private") {
                    self.on_media(me, &core, &chat.to_string(), &u["message"]).await
                } else {
                    Ok(())
                };
                if let Err(e) = r {
                    eprintln!("telegram update: {e:#}");
                }
            }
        }
    }

    /// Bot API files are capped at 20 MB for bots.
    async fn download(&self, file_id: &str) -> Result<(Vec<u8>, String)> {
        let f = self.call("getFile", json!({"file_id": file_id})).await?;
        let path = f["file_path"].as_str().context("Telegram gave no file path")?.to_string();
        let bytes = self.http.get(format!("{}/{path}", self.files)).send().await.and_then(|r| r.error_for_status()).map_err(|e| e.without_url())?.bytes().await.map_err(|e| e.without_url())?;
        Ok((bytes.to_vec(), path))
    }

    /// Photos go to the bot (its AI looks at them); voice notes are transcribed first; PDFs and text files are read.
    /// Anything else gets an honest "can't read this yet". Only in paired private chats.
    async fn on_media(&self, me: &Chan, core: &Arc<Core>, chat: &str, m: &Value) -> Result<()> {
        if !core.is_paired(me, chat)? {
            return Ok(());
        }
        let from_id = m["from"]["id"].as_i64().map(|i| i.to_string()).unwrap_or_default();
        let from = Sender { id: &from_id, dm: true };
        // A forwarded voice note or photo is someone else's: outside content, like a forwarded message.
        let forwarded = m["forward_origin"].is_object();
        let caption = m["caption"].as_str().unwrap_or("").to_string();
        let image_doc = m["document"]["mime_type"].as_str().is_some_and(|t| t == "image/jpeg" || t == "image/png");
        let photo_id = m["photo"].as_array().and_then(|sizes| sizes.last()).and_then(|p| p["file_id"].as_str()).or_else(|| image_doc.then(|| m["document"]["file_id"].as_str()).flatten());
        if let Some(id) = photo_id {
            let (bytes, path) = self.download(id).await?;
            let saved = core.save_media(&bytes, if path.ends_with(".png") { "png" } else { "jpg" })?;
            let text = if caption.trim().is_empty() { "(I sent you a photo.)".to_string() } else { caption };
            if forwarded {
                let said = crate::with_document("(I forwarded you a photo.)", "a forwarded photo's caption", &text);
                return core.incoming_outside_with(me, chat, from, "", &said, vec![saved], m["date"].as_i64()).await;
            }
            return core.incoming_with(me, chat, from, "", &text, vec![saved], m["date"].as_i64()).await;
        }
        if let Some(id) = m["voice"]["file_id"].as_str().or(m["audio"]["file_id"].as_str()) {
            let (bytes, path) = self.download(id).await?;
            let name = path.rsplit('/').next().unwrap_or("voice.ogg");
            return core.incoming_voice(me, chat, from, bytes, name, forwarded, m["date"].as_i64()).await;
        }
        if let Some(id) = m["document"]["file_id"].as_str() {
            // Telegram's bots can download files up to 20 MB.
            if m["document"]["file_size"].as_i64().unwrap_or(0) > 20_000_000 {
                me.send(chat, tr("That file is over 20 MB, more than Telegram lets me download. Could you send a smaller one?")).await;
                return Ok(());
            }
            let (name, mime) = (m["document"]["file_name"].as_str().unwrap_or("file"), m["document"]["mime_type"].as_str().unwrap_or(""));
            if crate::readable_document(name, mime) {
                let (bytes, _) = self.download(id).await?;
                return core.incoming_document(me, chat, from, name, mime, &bytes, &caption, m["date"].as_i64()).await;
            }
        }
        core.incoming_media(me, chat).await
    }

    async fn on_button(&self, me: &Chan, core: &Arc<Core>, q: &Value) -> Result<()> {
        let chat = q["message"]["chat"]["id"].as_i64().map(|c| c.to_string()).unwrap_or_default();
        let from = q["from"]["id"].as_i64().map(|i| i.to_string()).unwrap_or_default();
        let who = Sender { id: &from, dm: q["message"]["chat"]["type"] == "private" };
        let data = q["data"].as_str().unwrap_or("");
        let mid = q["message"]["message_id"].as_i64();
        let reply = match parse_decision(data) {
            Some((id, d)) => match core.decide(me, &chat, who, id, d) {
                Err(e) => e.to_string(),
                Ok(Some(a)) => {
                    if let Some(mid) = mid {
                        let _ = self.call("editMessageText", json!({"chat_id": chat, "message_id": mid, "text": verdict_for(&a, d)})).await;
                    }
                    (if a.status == "approved" { tr("✅ Approved") } else { tr("❌ Denied") }).into()
                }
                Ok(None) => tr("This request is no longer pending.").into(),
            },
            None => match core.tap(me, &chat, who, data).await {
                Err(e) => e.to_string(),
                Ok(Some(did)) => {
                    // The buttons go; the message stays as it was.
                    if let Some(mid) = mid {
                        let _ = self.call("editMessageReplyMarkup", json!({"chat_id": chat, "message_id": mid, "reply_markup": {"inline_keyboard": []}})).await;
                    }
                    did
                }
                Ok(None) => tr("Unknown button.").into(),
            },
        };
        self.call("answerCallbackQuery", json!({"callback_query_id": q["id"], "text": crate::tools::truncate(&reply, 190)})).await?;
        Ok(())
    }
}

fn keyboard(rows: &[Row]) -> Value {
    json!({"inline_keyboard": rows.iter().map(|row| row.iter().map(|(text, data)| json!({"text": text, "callback_data": data})).collect::<Vec<_>>()).collect::<Vec<_>>()})
}

/// The bot's own message this one replies to (swipe-to-reply): the part the person selected, else the whole message.
/// Not a topic's opening message (every message in a topic replies to it), nor anyone else's.
fn replied_to(m: &Value, me_id: Option<i64>) -> Option<String> {
    let r = &m["reply_to_message"];
    if me_id.is_none() || !r.is_object() || r["forum_topic_created"].is_object() || r["from"]["id"].as_i64() != me_id {
        return None;
    }
    m["quote"]["text"].as_str().or(r["text"].as_str()).or(r["caption"].as_str()).filter(|q| !q.trim().is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn a_reply_says_which_of_the_bots_messages() {
        let reply = |r: serde_json::Value| json!({"text": "move this one", "reply_to_message": r});
        let mine = json!({"from": {"id": 7}, "text": "1. Dentist\n2. Call Sam"});
        assert_eq!(super::replied_to(&reply(mine.clone()), Some(7)).as_deref(), Some("1. Dentist\n2. Call Sam"));
        let mut quoted = reply(mine);
        quoted["quote"] = json!({"text": "2. Call Sam"});
        assert_eq!(super::replied_to(&quoted, Some(7)).as_deref(), Some("2. Call Sam"), "the part they selected");
        assert!(super::replied_to(&reply(json!({"from": {"id": 9}, "text": "someone else's"})), Some(7)).is_none());
        assert!(super::replied_to(&reply(json!({"from": {"id": 7}, "forum_topic_created": {"name": "Trip"}})), Some(7)).is_none());
        assert!(super::replied_to(&json!({"text": "hi"}), Some(7)).is_none());
        let t = crate::with_reply("move this one", "2. Call <b>Sam</b>");
        assert_eq!(crate::own_part(&t), "move this one");
        assert!(t.contains("‹b›Sam‹/b›"), "the quote can't close the tag");
        assert_eq!(crate::own_part("plain"), "plain");
    }
}
