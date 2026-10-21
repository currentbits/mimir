//! iMessage through Messages on this Mac, in the desktop app: one conversation you choose (a note to yourself, or a
//! chat with a second Apple ID), and only that one, is read from ~/Library/Messages/chat.db (needs Full Disk Access)
//! every two seconds. Replies go out through Messages with AppleScript (needs Automation → Messages). Only while the
//! Mac is awake and Messages is signed in. Like Signal, there are no buttons: choices are numbered.
// ponytail: no files from the bot (Messages' sandbox only sends from a few folders), no typing, no drafts

use crate::db::Approval;
use crate::runtime::{Chan, Core, File, Row, Sender, Slot, card_text};
use crate::signal::{Picks, choose, numbered};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;

const POLL: Duration = Duration::from_secs(2);
/// Ends every message the bot sends: in a note to yourself, your messages and the bot's are both "from me", and this
/// tells them apart. Invisible (U+2060 WORD JOINER).
// ponytail: a bot message you copy and send back is taken for the bot's and skipped
const MARK: char = '\u{2060}';
/// Seconds from 1970 to 2001, where Messages' clock starts.
const APPLE_EPOCH: i64 = 978_307_200;

#[derive(Clone)]
pub(crate) struct IMessage {
    /// The conversation: the other side's phone number (+…) or email, your own for a note to yourself.
    handle: String,
    db: PathBuf,
    picks: Picks,
    sent: Arc<Mutex<Sent>>,
}

#[derive(Default)]
struct Sent {
    /// The bot's latest messages, (the id `send` gave it, its text), until they show up in chat.db.
    recent: Vec<(String, String)>,
    /// A message's Messages guid → (its id, its text), so a reply to it says which.
    by_guid: HashMap<String, (String, String)>,
    /// The person's last message (its text, when): Messages can store one sent to yourself twice.
    last: (String, i64),
}

/// A message in the conversation, as chat.db has it.
#[derive(Debug, PartialEq)]
struct Msg {
    rowid: i64,
    guid: String,
    text: String,
    /// The message it's an inline reply to.
    reply_to: Option<String>,
    /// Unix seconds.
    sent: i64,
    /// (name, mime, size, path on disk or "" before it's downloaded, a recorded voice message)
    files: Vec<(String, String, u64, String, bool)>,
}

/// The text in a message's attributedBody (an NSAttributedString in typedstream form): macOS leaves `text` empty on
/// many newer messages. The NSString's bytes follow its class name and five bytes, behind a typedstream length.
fn body_text(b: &[u8]) -> Option<String> {
    let at = b.windows(8).position(|w| w == b"NSString")? + 8;
    let rest = b.get(at + 5..)?;
    let (len, from): (usize, usize) = match *rest.first()? {
        0x81 => (u16::from_le_bytes(rest.get(1..3)?.try_into().ok()?) as usize, 3),
        0x82 => (u32::from_le_bytes(rest.get(1..5)?.try_into().ok()?) as usize, 5),
        n => (n as usize, 1),
    };
    String::from_utf8(rest.get(from..from.checked_add(len)?)?.to_vec()).ok()
}

/// chat.db's dates: nanoseconds since 2001 (seconds before macOS 10.13).
fn unix(date: i64) -> i64 {
    (if date > 1_000_000_000_000 { date / 1_000_000_000 } else { date }) + APPLE_EPOCH
}

fn open(db: &Path) -> Result<Connection> {
    Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| {
        anyhow::anyhow!("can't read Messages ({e}): give Mimir Full Disk Access in System Settings → Privacy & Security → Full Disk Access, then quit and reopen it")
    })
}

/// The conversation's messages after `after` (a chat.db row id), oldest first; tapbacks and the like left out. Never
/// another conversation's.
fn fetch(db: &Path, handle: &str, after: i64) -> Result<Vec<Msg>> {
    let c = open(db)?;
    let mut q = c.prepare_cached(
        "SELECT m.ROWID, m.guid, m.text, m.attributedBody, m.date, m.thread_originator_guid, m.cache_has_attachments, m.is_audio_message
         FROM message m JOIN chat_message_join j ON j.message_id = m.ROWID JOIN chat c ON c.ROWID = j.chat_id
         WHERE c.chat_identifier = ?1 COLLATE NOCASE AND m.ROWID > ?2 AND m.associated_message_type = 0 AND m.item_type = 0
         ORDER BY m.ROWID",
    )?;
    let rows = q.query_map(params![handle, after], |r| {
        let text: Option<String> = r.get(2)?;
        let body: Option<Vec<u8>> = r.get(3)?;
        let text = text.filter(|t| !t.is_empty()).or_else(|| body.as_deref().and_then(body_text)).unwrap_or_default();
        Ok((
            Msg { rowid: r.get(0)?, guid: r.get(1)?, text: text.replace('\u{FFFC}', "").trim().to_string(), reply_to: r.get(5)?, sent: unix(r.get(4)?), files: vec![] },
            r.get::<_, Option<bool>>(6)?.unwrap_or(false),
            r.get::<_, Option<bool>>(7)?.unwrap_or(false),
        ))
    })?;
    let mut out = vec![];
    for row in rows {
        let (mut m, attached, voice) = row?;
        if attached {
            let mut f = c.prepare_cached(
                "SELECT a.transfer_name, a.mime_type, a.total_bytes, a.filename FROM attachment a
                 JOIN message_attachment_join j ON j.attachment_id = a.ROWID WHERE j.message_id = ?1",
            )?;
            m.files = f
                .query_map([m.rowid], |r| {
                    let name: String = r.get::<_, Option<String>>(0)?.unwrap_or_else(|| "file".into());
                    Ok((name, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, Option<i64>>(2)?.unwrap_or(0).max(0) as u64, r.get::<_, Option<String>>(3)?.unwrap_or_default(), voice))
                })?
                .collect::<rusqlite::Result<_>>()?;
        }
        out.push(m);
    }
    Ok(out)
}

impl IMessage {
    /// `handle`: who the conversation is with, as Messages shows it (+31…, or an email). MIMIR_IMESSAGE_DB stands in
    /// for chat.db in tests.
    pub(crate) fn new(handle: &str) -> Result<Self> {
        let handle: String = handle.chars().filter(|c| !c.is_whitespace() && !"()-".contains(*c)).collect();
        let handle = if handle.contains('@') { handle.to_lowercase() } else { handle };
        if !(handle.contains('@') || handle.starts_with('+') && handle.len() > 4 && handle[1..].chars().all(|c| c.is_ascii_digit())) {
            bail!("iMessage needs the conversation's phone number with its country code (+…) or its email");
        }
        let db = match std::env::var_os("MIMIR_IMESSAGE_DB") {
            Some(p) => p.into(),
            None => crate::user_home().context("no home folder")?.join("Library/Messages/chat.db"),
        };
        Ok(IMessage { handle, db, picks: Default::default(), sent: Default::default() })
    }

    /// Messages can be read and has the conversation: where chat.db ends now.
    async fn start(&self) -> Result<i64> {
        if !cfg!(target_os = "macos") && std::env::var_os("MIMIR_IMESSAGE_DB").is_none() {
            bail!("iMessage works only in the Mac app");
        }
        let (db, handle) = (self.db.clone(), self.handle.clone());
        tokio::task::spawn_blocking(move || {
            let c = open(&db)?;
            let found: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM chat WHERE chat_identifier = ?1 COLLATE NOCASE)", [&handle], |r| r.get(0))?;
            if !found {
                bail!("there's no conversation with {handle} in Messages on this Mac: send one message there first");
            }
            Ok(c.query_row("SELECT IFNULL(MAX(ROWID), 0) FROM message", [], |r| r.get(0))?)
        })
        .await?
    }

    pub(crate) async fn check(&self) -> Result<String> {
        self.start().await.map(|_| self.handle.clone())
    }

    /// Sends through Messages; the id it's known by here (Messages gives none back).
    async fn send(&self, text: &str) -> Result<String> {
        let id = {
            let mut s = self.sent.lock().unwrap();
            // Milliseconds, so a card's id from before a restart isn't given out again.
            let mut id = chrono::Utc::now().timestamp_millis();
            if let Some(last) = s.recent.last().and_then(|(i, _)| i.parse::<i64>().ok()) {
                id = id.max(last + 1);
            }
            s.recent.push((id.to_string(), text.trim().to_string()));
            let over = s.recent.len().saturating_sub(50);
            s.recent.drain(..over);
            id.to_string()
        };
        let script = r#"on run argv
  tell application "Messages" to send (item 1 of argv) to participant (item 2 of argv) of (1st account whose service type = iMessage)
end run"#;
        // The text goes in as an argument: nothing in it is read as AppleScript.
        let out = tokio::process::Command::new("osascript").args(["-e", script, &format!("{text}{MARK}"), &self.handle]).output().await.context("can't run osascript")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("-1743") {
                bail!("Mimir may not control Messages: turn it on in System Settings → Privacy & Security → Automation → Mimir");
            }
            bail!("Messages didn't send it: {}", crate::tools::truncate(err.trim(), 200));
        }
        self.picks.sent(&self.handle, &id);
        Ok(id)
    }

    pub(crate) async fn upload(&self, _chat: &str, _name: &str, _bytes: Vec<u8>) -> Result<()> {
        bail!("files can't be sent on iMessage yet")
    }

    /// Final reply as plain text, with any buttons as a numbered list.
    pub(crate) async fn finish(&self, chat: &str, _draft: Option<&str>, text: &str, rows: &[Row]) -> Result<()> {
        let mut text = crate::fmt::plain(text);
        let data = numbered(&mut text, rows, false);
        let msg = self.send(&text).await?;
        self.picks.offer(chat, msg, false, data);
        Ok(())
    }

    pub(crate) async fn draft_post(&self, _chat: &str, _text: &str) -> Option<String> {
        None
    }

    /// Only a card is ever "edited": its outcome comes as a new message, and its numbers stop working.
    pub(crate) async fn draft_edit(&self, chat: &str, msg: &str, text: &str) {
        self.picks.drop(chat, msg);
        if let Err(e) = self.send(text).await {
            eprintln!("imessage: {e:#}");
        }
    }

    pub(crate) fn typing(&self, _chat: &str) -> Option<JoinHandle<()>> {
        None
    }

    pub(crate) async fn card(&self, chat: &str, a: &Approval) -> Result<Option<String>> {
        let mut text = card_text(a);
        let data = numbered(&mut text, &crate::runtime::card_buttons(a), true);
        let msg = self.send(&text).await?;
        self.picks.offer(chat, msg.clone(), true, data);
        Ok(Some(msg))
    }

    pub(crate) async fn listen(&self, me: &Chan, core: Arc<Core>, slot: Slot) -> Result<()> {
        let mut after = self.start().await?;
        slot.connected(self.handle.clone());
        loop {
            tokio::time::sleep(POLL).await;
            let (db, handle) = (self.db.clone(), self.handle.clone());
            // Opened afresh each time: Messages is left alone between looks.
            for m in tokio::task::spawn_blocking(move || fetch(&db, &handle, after)).await?? {
                after = m.rowid;
                if let Err(e) = self.on_message(me, &core, m).await {
                    eprintln!("imessage message: {e:#}");
                }
            }
        }
    }

    /// The person's message, or None for the bot's own (it learns their guids for replies) and a stored-twice copy.
    fn theirs(&self, m: &Msg) -> Option<(Option<String>, Option<String>)> {
        let mut s = self.sent.lock().unwrap();
        let marked = m.text.strip_suffix(MARK);
        let text = marked.unwrap_or(&m.text).trim();
        let hit = s.recent.iter().rev().find(|(_, t)| t == text).cloned();
        // Unmarked but word for word what it just sent: still the bot's, so it can never answer itself.
        if marked.is_some() || hit.is_some() {
            if let Some(hit) = hit {
                if s.by_guid.len() > 1000 {
                    s.by_guid.clear();
                }
                s.by_guid.insert(m.guid.clone(), hit);
            }
            return None;
        }
        if m.files.is_empty() && s.last.0 == m.text && (m.sent - s.last.1).abs() <= 3 {
            return None;
        }
        s.last = (m.text.clone(), m.sent);
        let quoted = m.reply_to.as_ref().and_then(|g| s.by_guid.get(g)).cloned();
        Some((quoted.as_ref().map(|(id, _)| id.clone()), quoted.map(|(_, t)| t)))
    }

    async fn on_message(&self, me: &Chan, core: &Arc<Core>, m: Msg) -> Result<()> {
        let Some((quoted, quoted_text)) = self.theirs(&m) else { return Ok(()) };
        if m.text.is_empty() && m.files.is_empty() {
            return Ok(());
        }
        let (chat, from) = (self.handle.as_str(), Sender { id: &self.handle, dm: true });
        if m.files.is_empty()
            && let Some(data) = self.picks.picked(chat, &m.text, quoted.as_deref())
        {
            return choose(me, core, chat, from, &data).await;
        }
        // A reply to a card waiting for an answer, not a number: no, and why.
        if m.files.is_empty()
            && let Some(card) = quoted.as_deref()
            && core.deny_by_reply(me, chat, from, card, &m.text)?
        {
            return Ok(());
        }
        let text = match quoted_text.filter(|t| !t.is_empty()) {
            Some(q) => crate::with_reply(&m.text, &q),
            None => m.text,
        };
        if !m.files.is_empty() {
            let files: Vec<File> = m.files.iter().map(|(name, mime, size, path, voice)| File { name: name.clone(), mime: mime.clone(), size: *size, url: path.clone(), voice: *voice }).collect();
            return core.incoming_files(me, chat, from, &text, &files, Some(m.sent), |f| download(f.url.clone())).await;
        }
        core.incoming(me, chat, from, "iMessage", &text, Some(m.sent)).await
    }
}

/// An attachment as Messages saved it; only from its own attachments folder.
async fn download(path: String) -> Result<Vec<u8>> {
    let home = crate::user_home().context("no home folder")?;
    let path = match path.strip_prefix("~/") {
        Some(p) => home.join(p),
        None => PathBuf::from(&path),
    };
    let real = tokio::fs::canonicalize(&path).await.context("that file isn't on this Mac yet")?;
    let root = tokio::fs::canonicalize(home.join("Library/Messages/Attachments")).await?;
    if !real.starts_with(&root) {
        bail!("that isn't a Messages attachment");
    }
    Ok(tokio::fs::read(real).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An attributedBody as Messages writes it, around `text`.
    fn body(text: &str) -> Vec<u8> {
        let mut b = b"\x04\x0bstreamtyped\x81\xe8\x03\x84\x01@\x84\x84\x84\x12NSAttributedString\x00\x84\x84\x08NSObject\x00\x85\x92\x84\x84\x84\x08NSString\x01\x94\x84\x01+".to_vec();
        match text.len() {
            n if n < 0x80 => b.push(n as u8),
            n => {
                b.push(0x81);
                b.extend((n as u16).to_le_bytes());
            }
        }
        b.extend(text.as_bytes());
        b.extend(b"\x86\x84\x02iI\x01");
        b
    }

    #[test]
    fn attributed_bodies() {
        assert_eq!(body_text(&body("hi there")).as_deref(), Some("hi there"));
        let long = "é".repeat(200);
        assert_eq!(body_text(&body(&long)), Some(long), "a long one's length takes two bytes");
        assert_eq!(body_text(b"no string here"), None);
        assert_eq!(body_text(&body("hi")[..body("hi").len() - 8]), None, "cut short");
        assert_eq!(unix(0), APPLE_EPOCH);
        assert_eq!(unix(700_000_000_000_000_000), 700_000_000 + APPLE_EPOCH);
    }

    #[test]
    fn handles() {
        assert_eq!(IMessage::new("+31 (6) 12-34").unwrap().handle, "+3161234");
        assert_eq!(IMessage::new(" Me@iCloud.com ").unwrap().handle, "me@icloud.com");
        assert!(IMessage::new("0612345678").is_err(), "needs the country code");
        assert!(IMessage::new("").is_err());
    }

    #[test]
    fn reads_only_its_conversation_and_tells_the_bot_apart() {
        let dir = std::env::temp_dir().join(format!("mimir-imessage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chat.db");
        let _ = std::fs::remove_file(&path);
        let c = Connection::open(&path).unwrap();
        c.execute_batch(
            "CREATE TABLE message (ROWID INTEGER PRIMARY KEY, guid TEXT, text TEXT, attributedBody BLOB, date INTEGER, is_from_me INTEGER,
               thread_originator_guid TEXT, cache_has_attachments INTEGER, is_audio_message INTEGER, associated_message_type INTEGER DEFAULT 0, item_type INTEGER DEFAULT 0);
             CREATE TABLE chat (ROWID INTEGER PRIMARY KEY, chat_identifier TEXT);
             CREATE TABLE chat_message_join (chat_id INTEGER, message_id INTEGER);
             CREATE TABLE attachment (ROWID INTEGER PRIMARY KEY, filename TEXT, mime_type TEXT, transfer_name TEXT, total_bytes INTEGER);
             CREATE TABLE message_attachment_join (message_id INTEGER, attachment_id INTEGER);
             INSERT INTO chat VALUES (1, 'me@icloud.com'), (2, '+3161');
             INSERT INTO message (ROWID, guid, text, date, cache_has_attachments, is_audio_message) VALUES
               (1, 'g1', 'old', 1000000000000000000, 0, 0), (2, 'g2', 'someone else', 1000000000000000000, 0, 0);
             INSERT INTO message (ROWID, guid, text, date, associated_message_type) VALUES (4, 'g4', 'Loved “hi”', 1000000000000000000, 2000);
             INSERT INTO message (ROWID, guid, text, date, cache_has_attachments, is_audio_message) VALUES (5, 'g5', '￼', 1000000000000000000, 1, 1);
             INSERT INTO attachment VALUES (1, '~/Library/Messages/Attachments/a/Audio Message.caf', 'audio/x-caf', 'Audio Message.caf', 900);
             INSERT INTO message_attachment_join VALUES (5, 1);
             INSERT INTO chat_message_join VALUES (1, 1), (2, 2), (1, 3), (1, 4), (1, 5);",
        )
        .unwrap();
        c.execute("INSERT INTO message (ROWID, guid, text, attributedBody, date, thread_originator_guid) VALUES (3, 'g3', NULL, ?1, 1000000000000000000, 'b1')", [body("hello")]).unwrap();

        let got = fetch(&path, "Me@iCloud.com", 1).unwrap();
        assert_eq!(got.iter().map(|m| m.rowid).collect::<Vec<_>>(), [3, 5], "not the older one, not another chat's, not a tapback");
        assert_eq!((got[0].text.as_str(), got[0].reply_to.as_deref(), got[0].sent), ("hello", Some("b1"), 1_000_000_000 + APPLE_EPOCH));
        assert_eq!((got[1].text.as_str(), &got[1].files[0]), ("", &("Audio Message.caf".into(), "audio/x-caf".into(), 900, "~/Library/Messages/Attachments/a/Audio Message.caf".into(), true)));

        // In a note to yourself everything is "from me": the bot's own come back marked, and a reply to one says which.
        let im = IMessage::new("me@icloud.com").unwrap();
        im.sent.lock().unwrap().recent.push(("77".into(), "1. Approve\n2. Deny".into()));
        let msg = |guid: &str, text: &str, reply_to: Option<&str>, sent| Msg { rowid: 0, guid: guid.into(), text: text.into(), reply_to: reply_to.map(Into::into), sent, files: vec![] };
        assert_eq!(im.theirs(&msg("b1", &format!("1. Approve\n2. Deny{MARK}"), None, 10)), None, "the bot's own");
        assert_eq!(im.theirs(&msg("u1", "2", Some("b1"), 20)), Some((Some("77".into()), Some("1. Approve\n2. Deny".into()))));
        assert_eq!(im.theirs(&msg("u2", "2", None, 22)), None, "stored twice");
        assert_eq!(im.theirs(&msg("u3", "2", None, 60)), Some((None, None)), "said again later");
        std::fs::remove_dir_all(&dir).ok();
    }
}
