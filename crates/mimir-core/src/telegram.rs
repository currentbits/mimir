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
        Self::at(&std::env::var("MIMIR_TELEGRAM_API").unwrap_or_else(|_| "https://api.telegram.org".into()), token)
    }

    pub(crate) fn at(api: &str, token: &str) -> Result<Self> {
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
        // Rich messages have no previews (nor the option); what would fetch there is media, which `fmt::telegram_rich` never writes.
        if matches!(method, "sendMessage" | "editMessageText") && body.get("link_preview_options").is_none() && body.get("rich_message").is_none() {
            body["link_preview_options"] = json!({"is_disabled": true});
        }
        // "chat:topic" (a topic in a private chat, see `listen`): the chat, and the topic new messages go to.
        if let Some((chat, topic)) = body["chat_id"].as_str().and_then(|c| c.split_once(':')).map(|(c, t)| (c.to_string(), t.parse::<i64>().unwrap_or(0))) {
            body["chat_id"] = json!(chat);
            if matches!(method, "sendMessage" | "sendMessageDraft" | "sendRichMessage" | "sendRichMessageDraft" | "sendChatAction") {
                body["message_thread_id"] = json!(topic);
            }
        }
        // The token is in the URL: errors drop it (they reach logs and the bot's status in the app).
        let v: Value = self.http.post(format!("{}/{method}", self.base)).json(&body).send().await.map_err(|e| e.without_url())?.json().await.map_err(|e| e.without_url())?;
        if v["ok"].as_bool() != Some(true) {
            bail!("telegram {method}: {}", v["description"]);
        }
        Ok(v["result"].clone())
    }

    /// A new topic in a private chat (Bot API 9.3): its thread id.
    pub(crate) async fn new_topic(&self, chat: &str, name: &str) -> Result<i64> {
        self.call("createForumTopic", json!({"chat_id": chat, "name": name})).await?["message_thread_id"].as_i64().context("telegram createForumTopic: no topic id")
    }

    /// A file the bot sends: a photo (shown inline) or a document.
    pub(crate) async fn upload(&self, chat: &str, name: &str, bytes: Vec<u8>) -> Result<()> {
        let (method, field) = if crate::is_photo(name) { ("sendPhoto", "photo") } else { ("sendDocument", "document") };
        let form = reqwest::multipart::Form::new().text("chat_id", chat.to_string()).part(field, reqwest::multipart::Part::bytes(bytes).file_name(name.to_string()));
        let v: Value = self.http.post(format!("{}/{method}", self.base)).multipart(form).send().await.map_err(|e| e.without_url())?.json().await.map_err(|e| e.without_url())?;
        if v["ok"].as_bool() != Some(true) {
            bail!("telegram {method}: {}", v["description"]);
        }
        Ok(())
    }

    /// One rich message (Bot API 10.1: tables, headings, task lists, up to 32768 characters). Failing that (too long, an
    /// older Bot API server, rejected markup): Markdown → Telegram HTML, split between blocks under the 4096-char cap. If
    /// Telegram rejects the markup, that piece goes out as plain text instead of being lost. Buttons go under the last part.
    /// With a streamed draft, the draft is replaced by the first part.
    pub(crate) async fn finish(&self, chat: &str, draft: Option<&str>, text: &str, rows: &[Row]) -> Result<()> {
        let draft = draft.and_then(|d| d.parse::<i64>().ok());
        let html = crate::fmt::telegram_rich(text);
        if crate::fmt::html_to_plain(&html).chars().count() <= 32768 {
            let mut msg = json!({"chat_id": chat, "rich_message": {"html": html}});
            if !rows.is_empty() {
                msg["reply_markup"] = keyboard(rows);
            }
            let method = match draft {
                Some(mid) => {
                    msg["message_id"] = json!(mid);
                    "editMessageText"
                }
                None => "sendRichMessage",
            };
            if self.call(method, msg).await.is_ok() {
                return Ok(());
            }
        }
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

    /// Private chats get Telegram's native streaming draft (Bot API 10), with its own stop button: a rich one where the
    /// server has it (10.1; our converter leaves half-written Markdown literal), else plain text. The final
    /// reply is then sent as a new message (`finish` sees "draft:N" isn't a message id). Groups, and any
    /// error, fall back to posting a plain message and editing it; `finish` formats.
    pub(crate) async fn draft_post(&self, chat: &str, text: &str) -> Option<String> {
        if crate::db::tg_chat(chat).parse::<i64>().is_ok_and(|c| c > 0) {
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
        // Drafts can't turn link previews off (neither draft call has that option), so links in one aren't left
        // recognisable as links while it's written: an invisible word joiner inside "://" and after each dot inside a
        // word (Telegram links a bare "evil.com/x?d=…" too). The final message has previews off and its links whole.
        let text = unlinked(text);
        // ponytail: a server without rich drafts gets two calls a tick; remember that it lacks them if that matters
        let rich = json!({"chat_id": chat, "draft_id": id, "rich_message": {"html": crate::fmt::telegram_rich(&text)}, "can_stop": true});
        if self.call("sendRichMessageDraft", rich).await.is_ok() {
            return true;
        }
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

    /// For `mimir doctor`: the token works. The bot's @name.
    pub(crate) async fn check(&self) -> Result<String> {
        let bot = self.call("getMe", json!({})).await.context("Telegram rejected the bot token")?;
        Ok(format!("@{}", bot["username"].as_str().unwrap_or("?")))
    }

    pub(crate) async fn listen(&self, me: &Chan, core: Arc<Core>, slot: Slot) -> Result<()> {
        let bot = self.call("getMe", json!({})).await.context("Telegram rejected the bot token")?;
        let me_id = bot["id"].as_i64();
        let my_name = bot["username"].as_str().unwrap_or("?").to_string();
        slot.connected(format!("@{my_name}"));
        // The "/" menu in Telegram shows these.
        let base = json!([
            {"command": "memory", "description": tr("What I remember about you")},
            {"command": "routines", "description": tr("What I do on a schedule")},
            {"command": "why", "description": tr("What my last reply used")},
            {"command": "new", "description": tr("Start a fresh conversation (memories kept)")},
            {"command": "branch", "description": tr("Branch off a new conversation from here (reply to a message to branch from it)")},
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
            let updates = self.call("getUpdates", json!({"offset": offset, "timeout": 50, "allowed_updates": ["message", "callback_query", "stopped_message_generation", "guest_message"]})).await?;
            for u in updates.as_array().into_iter().flatten() {
                offset = offset.max(u["update_id"].as_i64().unwrap_or(0) + 1);
                self.offset.store(offset, std::sync::atomic::Ordering::Relaxed);
                let r = if u["callback_query"].is_object() {
                    self.on_button(me, &core, &u["callback_query"]).await
                } else if u["guest_message"].is_object() {
                    self.on_guest(me, &core, &u["guest_message"])
                } else if let Some(chat) = u["stopped_message_generation"]["chat"]["id"].as_i64() {
                    // The stop button on a streaming draft: same as /stop.
                    let chat = chat.to_string();
                    core.stop(me, &chat, Sender { id: &chat, dm: true }).map(drop)
                } else if let (Some(_), Some(text)) = (
                    u["message"]["chat"]["id"].as_i64(),
                    u["message"]["text"].as_str().and_then(|t| for_bot(&u["message"], t, me_id, &my_name, || core.is_family(me, &address(&u["message"], false)))),
                ) {
                    let text = text.as_str();
                    let chat = address(&u["message"], core.topics());
                    let c = &u["message"]["chat"];
                    let title = c["title"].as_str().or(c["username"].as_str()).or(c["first_name"].as_str()).unwrap_or("");
                    let from = u["message"]["from"]["id"].as_i64().map(|i| i.to_string()).unwrap_or_default();
                    let who = Sender { id: &from, dm: c["type"] == "private" };
                    if u["message"]["forward_origin"].is_object() {
                        // A forwarded message is someone else's words: outside content, like a file.
                        let said = crate::with_document("(I forwarded you a message.)", "a forwarded message", text);
                        core.incoming_outside(me, &chat, who, title, &said, u["message"]["date"].as_i64()).await
                    } else if let Some(card) = replied_to(&u["message"], me_id).and(u["message"]["reply_to_message"]["message_id"].as_i64())
                        && core.deny_by_reply(me, &chat, who, &card.to_string(), text)?
                    {
                        Ok(())
                    } else if let Some(quoted) = replied_to(&u["message"], me_id) {
                        core.incoming(me, &chat, who, title, &crate::with_reply(text, &quoted), u["message"]["date"].as_i64()).await
                    } else {
                        core.incoming(me, &chat, who, title, text, u["message"]["date"].as_i64()).await
                    }
                } else if let (Some(_), Some(caption)) = (
                    u["message"]["chat"]["id"].as_i64(),
                    for_bot(&u["message"], u["message"]["caption"].as_str().unwrap_or(""), me_id, &my_name, || core.is_family(me, &address(&u["message"], false))),
                ) {
                    // In a group, a photo or file addressed to the bot (or any in a family group); `on_media` checks who sent it.
                    let chat = address(&u["message"], core.topics());
                    match self.on_media(me, &core, &chat, &u["message"], caption).await {
                        // No file_size to check up front: getFile is where Telegram says it's over 20 MB.
                        Err(e) if format!("{e:#}").contains("file is too big") => {
                            me.send(&chat, tr("That file is over 20 MB, more than Telegram lets me download. Could you send a smaller one?")).await;
                            Ok(())
                        }
                        r => r,
                    }
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
    /// Anything else gets an honest "can't read this yet". Only in paired chats, from the person who paired it (or a
    /// family group's members).
    async fn on_media(&self, me: &Chan, core: &Arc<Core>, chat: &str, m: &Value, caption: String) -> Result<()> {
        let from_id = m["from"]["id"].as_i64().map(|i| i.to_string()).unwrap_or_default();
        let from = Sender { id: &from_id, dm: m["chat"]["type"] == "private" };
        if !core.files_ok(me, chat, from)? {
            return Ok(());
        }
        // Telegram's bots can download files up to 20 MB: a photo sent as a file, a voice note or an audio file too.
        let photo = m["photo"].as_array().and_then(|sizes| sizes.last());
        if [photo.unwrap_or(&Value::Null), &m["document"], &m["voice"], &m["audio"]].iter().any(|f| f["file_size"].as_i64().unwrap_or(0) > 20_000_000) {
            me.send(chat, tr("That file is over 20 MB, more than Telegram lets me download. Could you send a smaller one?")).await;
            return Ok(());
        }
        // A forwarded voice note or photo is someone else's: outside content, like a forwarded message.
        let forwarded = m["forward_origin"].is_object();
        let image_doc = m["document"]["mime_type"].as_str().is_some_and(|t| t == "image/jpeg" || t == "image/png");
        let photo_id = photo.and_then(|p| p["file_id"].as_str()).or_else(|| image_doc.then(|| m["document"]["file_id"].as_str()).flatten());
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
        // A voice note is recorded in the app; an audio file (a song, a recording someone made) is outside content.
        let audio = m["voice"]["file_id"].as_str().map(|id| (id, forwarded.then_some("a forwarded voice note"))).or_else(|| m["audio"]["file_id"].as_str().map(|id| (id, Some("an audio file"))));
        if let Some((id, outside)) = audio {
            let (bytes, path) = self.download(id).await?;
            let name = path.rsplit('/').next().unwrap_or("voice.ogg");
            return core.incoming_voice(me, chat, from, bytes, name, outside, m["date"].as_i64()).await;
        }
        if let Some(id) = m["document"]["file_id"].as_str() {
            let (name, mime) = (m["document"]["file_name"].as_str().unwrap_or("file"), m["document"]["mime_type"].as_str().unwrap_or(""));
            if crate::readable_document(name, mime) {
                let (bytes, _) = self.download(id).await?;
                return core.incoming_document(me, chat, from, name, mime, &bytes, &caption, m["date"].as_i64()).await;
            }
        }
        core.incoming_media(me, chat).await
    }

    /// Guest mode (Bot API 10.0): @bot typed in a chat it isn't in. It gets that message and the one it replies to, and
    /// one reply: "…" at once (a long reply can't lose the query), edited into the answer once it's written.
    fn on_guest(&self, me: &Chan, core: &Arc<Core>, m: &Value) -> Result<()> {
        let from = m["from"]["id"].as_i64().map(|i| i.to_string()).unwrap_or_default();
        let (Some(query), Some(chat)) = (m["guest_query_id"].as_str().map(str::to_string), core.guest_mode_chat(me, &from)?) else { return Ok(()) };
        let text = m["text"].as_str().or(m["caption"].as_str()).unwrap_or("").to_string();
        let referenced = quoted(m);
        let (tg, core) = (self.clone(), core.clone());
        tokio::spawn(async move {
            let article = json!({"type": "article", "id": "1", "title": "…", "input_message_content": {"message_text": "…"}});
            let sent = match tg.call("answerGuestQuery", json!({"guest_query_id": query, "result": article})).await {
                Ok(s) => s["inline_message_id"].as_str().unwrap_or("").to_string(),
                Err(e) => return eprintln!("telegram guest reply: {e:#}"),
            };
            let reply = core.guest_mode(chat, &text, referenced.as_deref()).await.unwrap_or_else(|e| crate::friendly_error(&format!("{e:#}")));
            let reply = if reply.trim().is_empty() { "(no reply)".to_string() } else { reply };
            // Rich, else Telegram HTML, else plain: one message, so a long reply is cut.
            let parts = crate::fmt::telegram(&reply, 3800);
            let mut html = parts.first().cloned().unwrap_or_default();
            if parts.len() > 1 {
                html.push_str(&format!("…\n\n{}", tr("(Ask me in our private chat for the rest.)")));
            }
            let tries = [
                json!({"inline_message_id": sent, "rich_message": {"html": crate::fmt::telegram_rich(&reply)}}),
                json!({"inline_message_id": sent, "text": html, "parse_mode": "HTML"}),
                json!({"inline_message_id": sent, "text": crate::tools::truncate(&crate::fmt::html_to_plain(&html), 4000)}),
            ];
            for t in tries {
                match tg.call("editMessageText", t).await {
                    Ok(_) => return,
                    Err(e) => eprintln!("telegram guest reply: {e:#}"),
                }
            }
        });
        Ok(())
    }

    async fn on_button(&self, me: &Chan, core: &Arc<Core>, q: &Value) -> Result<()> {
        // A button in a topic: its answer (a tapped choice, say) belongs to that topic's conversation.
        let chat = address(&q["message"], core.topics());
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

/// Where a message came from: its chat, or with topics on, "chat:topic" for a message in a private chat's topic (Bot
/// API 9.3), so each topic is its own conversation and replies go back into it.
fn address(m: &Value, topics: bool) -> String {
    let chat = m["chat"]["id"].as_i64().unwrap_or(0);
    match m["message_thread_id"].as_i64().filter(|_| topics && m["is_topic_message"] == true && m["chat"]["type"] == "private") {
        Some(topic) => format!("{chat}:{topic}"),
        None => chat.to_string(),
    }
}

/// A text message as the bot should take it, or None when it isn't for the bot. Direct chats and family groups: every
/// message. Other groups: only one that @-mentions the bot (the mention taken out), replies to one of its messages, is a
/// /command or pairs. (Telegram never shows a bot other bots' messages in a group.)
fn for_bot(m: &Value, text: &str, me_id: Option<i64>, name: &str, family: impl FnOnce() -> bool) -> Option<String> {
    if !matches!(m["chat"]["type"].as_str(), Some("group" | "supergroup")) {
        return Some(text.to_string());
    }
    // Usernames are ASCII and case-insensitive; "@mybot2" isn't "@mybot".
    let (tag, lower) = (format!("@{}", name.to_ascii_lowercase()), text.to_ascii_lowercase());
    let at = lower.match_indices(&tag).map(|(i, _)| i).find(|&i| !lower[i + tag.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'));
    let said = match at {
        Some(i) => format!("{}{}", &text[..i], &text[i + tag.len()..]).trim().to_string(),
        None => text.to_string(),
    };
    let r = &m["reply_to_message"];
    let to_me = me_id.is_some() && r["from"]["id"].as_i64() == me_id && !r["forum_topic_created"].is_object();
    let addressed = at.is_some() || to_me || said.starts_with('/') || matches!(crate::runtime::parse_command(&said), crate::runtime::Command::Pair(_));
    (addressed || family()).then_some(said)
}

fn keyboard(rows: &[Row]) -> Value {
    let button = |text: &str, data: &str| if data.starts_with("https://") { json!({"text": text, "url": data}) } else { json!({"text": text, "callback_data": data}) };
    json!({"inline_keyboard": rows.iter().map(|row| row.iter().map(|(text, data)| button(text, data)).collect::<Vec<_>>()).collect::<Vec<_>>()})
}

/// The bot's own message this one replies to (swipe-to-reply): the part the person selected, else the whole message.
/// Not a topic's opening message (every message in a topic replies to it), nor anyone else's.
fn replied_to(m: &Value, me_id: Option<i64>) -> Option<String> {
    let r = &m["reply_to_message"];
    if me_id.is_none() || !r.is_object() || r["forum_topic_created"].is_object() || r["from"]["id"].as_i64() != me_id {
        return None;
    }
    quoted(m)
}

/// The message this one replies to, anyone's: the part the person selected, else its whole text.
fn quoted(m: &Value) -> Option<String> {
    let r = &m["reply_to_message"];
    let rich = r["rich_message"]["blocks"].is_array().then(|| rich_plain(&r["rich_message"]["blocks"]));
    m["quote"]["text"].as_str().or(r["text"].as_str()).or(r["caption"].as_str()).or(rich.as_deref()).filter(|q| !q.trim().is_empty()).map(str::to_string)
}

/// A received rich message's text: a line per block (paragraph, heading, list item, table cell…).
fn rich_plain(blocks: &Value) -> String {
    // RichText: a string, an array of RichText, or a span ({"type": "bold", "text": RichText, …}).
    fn text(v: &Value) -> String {
        match v {
            Value::String(s) => s.clone(),
            Value::Array(a) => a.iter().map(text).collect(),
            _ => text(&v["text"]),
        }
    }
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => match o.get("text") {
                Some(t) => out.push(text(t)),
                None => o.values().for_each(|x| walk(x, out)),
            },
            _ => {}
        }
    }
    let mut out = vec![];
    walk(blocks, &mut out);
    out.join("\n")
}

/// Text with a word joiner inside "://" and after every dot that has a letter or digit after it, so Telegram finds no
/// address in it to link (and preview).
fn unlinked(text: &str) -> String {
    let text = text.replace("://", ":/\u{2060}/");
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '.' && chars.peek().is_some_and(|n| n.is_alphanumeric()) {
            out.push('\u{2060}');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn drafts_have_no_recognisable_links() {
        let t = super::unlinked("see https://evil.com/x and evil.com/x?d=1. Done");
        assert!(!t.contains("://") && !t.contains("evil.com"), "{t}");
        assert_eq!(t.replace('\u{2060}', ""), "see https://evil.com/x and evil.com/x?d=1. Done");
    }

    #[test]
    fn a_topic_in_a_private_chat_is_its_own_address_when_on() {
        let m = json!({"chat": {"id": 5, "type": "private"}, "message_thread_id": 9, "is_topic_message": true});
        assert_eq!(super::address(&m, true), "5:9");
        assert_eq!(super::address(&m, false), "5", "opt-in");
        assert_eq!(super::address(&json!({"chat": {"id": 5, "type": "private"}}), true), "5", "no topic");
        let group = json!({"chat": {"id": -100, "type": "supergroup"}, "message_thread_id": 9, "is_topic_message": true});
        assert_eq!(super::address(&group, true), "-100", "private chats only");
        assert_eq!(crate::db::tg_chat("5:9"), "5");
    }

    #[test]
    fn a_group_message_reaches_the_bot_only_when_meant_for_it() {
        let group = |extra: serde_json::Value| {
            let mut m = json!({"chat": {"id": -100, "type": "supergroup"}});
            m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            m
        };
        let take = |m: &serde_json::Value, t: &str, family: bool| super::for_bot(m, t, Some(7), "MyBot", || family);
        assert_eq!(take(&group(json!({})), "lunch anyone?", false), None, "not addressed");
        assert_eq!(take(&group(json!({})), "lunch anyone?", true).as_deref(), Some("lunch anyone?"), "a family group hears all");
        assert_eq!(take(&group(json!({})), "@mybot hi", false).as_deref(), Some("hi"));
        assert_eq!(take(&group(json!({})), "ask @mybot2 instead", false), None, "another bot's name");
        assert_eq!(take(&group(json!({})), "/new@MyBot", false).as_deref(), Some("/new"));
        assert_eq!(take(&group(json!({})), "/memory", false).as_deref(), Some("/memory"));
        assert_eq!(take(&group(json!({})), "pair ab12", false).as_deref(), Some("pair ab12"));
        assert_eq!(take(&group(json!({"reply_to_message": {"from": {"id": 7}}})), "and tomorrow?", false).as_deref(), Some("and tomorrow?"));
        assert_eq!(take(&group(json!({"reply_to_message": {"from": {"id": 9}}})), "and tomorrow?", false), None, "someone else's message");
        let private = json!({"chat": {"id": 5, "type": "private"}});
        assert_eq!(take(&private, "hi", false).as_deref(), Some("hi"), "direct chats hear all");
    }

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
        let rich = json!({"from": {"id": 7}, "rich_message": {"blocks": [
            {"type": "heading", "size": 2, "text": "Today"},
            {"type": "list", "items": [{"label": "1.", "blocks": [{"type": "paragraph", "text": ["Call ", {"type": "bold", "text": "Sam"}]}]}]}
        ]}});
        assert_eq!(super::replied_to(&reply(rich), Some(7)).as_deref(), Some("Today\nCall Sam"), "a rich message's text");
        let t = crate::with_reply("move this one", "2. Call <b>Sam</b>");
        assert_eq!(crate::own_part(&t), "move this one");
        assert!(t.contains("‹b›Sam‹/b›"), "the quote can't close the tag");
        assert_eq!(crate::own_part("plain"), "plain");
    }
}
