use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct Chat {
    pub chat_id: i64,
    pub title: String,
    /// telegram | slack | discord | signal | local
    pub channel: String,
    pub memories: i64,
    /// Paired by someone other than you (their own memory and routines; none of your email, calendar, meetings…).
    pub guest: bool,
    /// Reads (and adds to) the household's memories: your own direct chats always, a guest's or a group when you say so.
    pub household: bool,
    /// A group where every household member can talk to the bots (it's a guest's chat, so none of your private things).
    pub family: bool,
    /// A direct chat (only one person in it), not a group.
    pub private: bool,
}

impl Db {
    // ---------- chats & messages ----------

    pub fn is_paired(&self, chat: i64) -> Result<bool> {
        Ok(self.c().query_row("SELECT 1 FROM chats WHERE chat_id=?1", [chat], |_| Ok(())).optional()?.is_some())
    }

    pub fn pair(&self, chat: i64, title: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO chats(chat_id, title) VALUES (?1, ?2) ON CONFLICT(chat_id) DO UPDATE SET title = excluded.title",
            params![chat, title],
        )?;
        Ok(())
    }

    /// Paired chats with their live memory counts (for UIs).
    pub fn chats(&self) -> Result<Vec<Chat>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT c.chat_id, c.title,
               (SELECT count(*) FROM entries e WHERE e.scope = 'chat:' || c.chat_id AND e.superseded_by IS NULL AND e.invalid = 0),
               CASE WHEN c.chat_id = 0 THEN 'local' ELSE c.channel END, c.guest, c.family
             FROM chats c ORDER BY c.chat_id",
        )?;
        let rows: Vec<Chat> = st
            .query_map([], |r| Ok(Chat { chat_id: r.get(0)?, title: r.get(1)?, memories: r.get(2)?, channel: r.get(3)?, guest: r.get(4)?, household: false, family: r.get(5)?, private: false }))?
            .collect::<Result<_, _>>()?;
        drop(st);
        drop(c);
        Ok(rows.into_iter().map(|c| Chat { household: self.in_household(c.chat_id), private: self.chat_private(c.chat_id), ..c }).collect())
    }

    // ---------- the household ----------

    /// Whether a chat reads the household's memories (and can add to them): the app's own chat and your direct chats
    /// always; a guest's chat or a group once you add it. Never the household itself or an incognito chat.
    pub fn in_household(&self, chat: i64) -> bool {
        if chat == crate::INCOGNITO_CHAT || chat == crate::HOUSEHOLD_CHAT {
            return false;
        }
        let added = self.c().query_row("SELECT household FROM chats WHERE chat_id=?1", [chat], |r| r.get::<_, bool>(0)).unwrap_or(false);
        added || (!self.chat_guest(chat) && self.chat_private(chat))
    }

    /// The household's memories live in a chat of their own (`HOUSEHOLD_CHAT`), so the Memory tab, suggestions and
    /// Needs you work for it as for any chat. Made the first time it's needed.
    pub fn household_ensure(&self) -> Result<()> {
        self.c().execute(
            "INSERT OR IGNORE INTO chats(chat_id, title, channel) VALUES (?1, 'Household', 'household')",
            [crate::HOUSEHOLD_CHAT],
        )?;
        Ok(())
    }

    /// Add a guest's chat or a group to the household, or take it out. Your own direct chats are always in it.
    pub fn chat_set_household(&self, chat: i64, on: bool) -> Result<bool> {
        if chat == crate::HOUSEHOLD_CHAT || chat == crate::INCOGNITO_CHAT {
            return Ok(false);
        }
        let n = self.c().execute("UPDATE chats SET household=?2, family=family AND ?2 WHERE chat_id=?1", params![chat, on])?;
        if n > 0 && on {
            self.household_ensure()?;
        }
        Ok(n > 0)
    }

    /// A family group: any household member can talk in it. It's in the household, and a guest's chat (several people
    /// are in it, so none of your email, calendar or logins), whoever paired it.
    pub fn chat_set_family(&self, chat: i64, on: bool) -> Result<bool> {
        if self.chat_private(chat) || chat == crate::HOUSEHOLD_CHAT {
            bail!("only a group chat can be a family group");
        }
        let sql = if on { "UPDATE chats SET family=1, household=1, guest=1 WHERE chat_id=?1" } else { "UPDATE chats SET family=0 WHERE chat_id=?1" };
        let n = self.c().execute(sql, [chat])?;
        if n > 0 && on {
            self.household_ensure()?;
        }
        Ok(n > 0)
    }

    /// Someone in the household writing in a family group (not the person who paired it, who's let in anyway).
    pub fn family_member(&self, chat: i64, person: &str) -> Result<bool> {
        if person.is_empty() || !self.chat_family(chat) {
            return Ok(false);
        }
        let channel: String = self.c().query_row("SELECT coalesce(channel, 'telegram') FROM chats WHERE chat_id=?1", [chat], |r| r.get(0))?;
        Ok(self.household_people(&channel)?.iter().any(|p| p.0 == person))
    }

    /// Who wrote in a family group, for the bot: the title of their own chat ("Sam"); None outside family groups.
    pub fn family_speaker(&self, chat: i64, person: &str) -> Result<Option<String>> {
        if !self.chat_family(chat) {
            return Ok(None);
        }
        let channel: String = self.c().query_row("SELECT coalesce(channel, 'telegram') FROM chats WHERE chat_id=?1", [chat], |r| r.get(0))?;
        let name = self.household_people(&channel)?.into_iter().find(|p| p.0 == person).map(|p| p.2).filter(|t| !t.trim().is_empty());
        // ‹›: a name can't pass for a block of the prompt.
        Ok(Some(name.unwrap_or_else(|| "Someone".into()).replace('<', "‹").replace('>', "›").replace(':', "")))
    }

    pub fn chat_family(&self, chat: i64) -> bool {
        self.c().query_row("SELECT family FROM chats WHERE chat_id=?1", [chat], |r| r.get::<_, bool>(0)).unwrap_or(false)
    }

    /// The household's people on one app: (their id there, their direct chat, its title). You, and each guest whose
    /// direct chat you added to the household.
    pub fn household_people(&self, channel: &str) -> Result<Vec<(String, i64, String)>> {
        let rows: Vec<(String, i64, String)> = {
            let c = self.c();
            let mut st = c.prepare("SELECT owner, chat_id, title FROM chats WHERE coalesce(channel, 'telegram')=?1 AND owner != '' AND family=0 ORDER BY chat_id")?;
            st.query_map([channel], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?
        };
        Ok(rows.into_iter().filter(|(_, chat, _)| self.chat_private(*chat) && self.in_household(*chat)).collect())
    }

    /// The direct chat of the household member called `name` (their chat's title, ignoring case; a first name will do),
    /// other than `not` (the chat asking). Err names who there is when nobody, or more than one, matches.
    pub fn household_member(&self, name: &str, not: i64) -> Result<(i64, String)> {
        let want = name.trim().to_lowercase();
        let mut all: Vec<(i64, String)> = vec![];
        for channel in ["telegram", "slack", "discord", "signal"] {
            all.extend(self.household_people(channel)?.into_iter().map(|(_, chat, title)| (chat, title)).filter(|(chat, _)| *chat != not));
        }
        let first = |t: &str| t.split_whitespace().next().unwrap_or("").to_lowercase();
        let hits: Vec<_> = all.iter().filter(|(_, t)| t.to_lowercase() == want || first(t) == want).collect();
        match hits.as_slice() {
            [one] => Ok((*one).clone()),
            _ => {
                let names = all.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(", ");
                bail!(
                    "{} household member called {name}. In the household: {}",
                    if hits.is_empty() { "no" } else { "more than one" },
                    if names.is_empty() { "nobody yet (add a guest's chat to the household in the app)" } else { &names }
                )
            }
        }
    }

    /// The chat id for an inbound message, if that chat was paired. Telegram ids are used as-is ("chat:topic" is
    /// that chat: a topic is its own conversation, see `topic_conv`, in the same chat).
    pub fn chat_lookup(&self, channel: &str, external: &str) -> Result<Option<i64>> {
        if channel == "telegram" {
            let id: i64 = tg_chat(external).parse()?;
            return Ok(self.is_paired(id)?.then_some(id));
        }
        Ok(self
            .c()
            .query_row("SELECT chat_id FROM chats WHERE channel=?1 AND external=?2", params![channel, external], |r| r.get(0))
            .optional()?)
    }

    /// Paired chats on one app (not Telegram): (its id there, a direct chat?).
    pub fn chats_on(&self, channel: &str) -> Result<Vec<(String, bool)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT external, dm FROM chats WHERE channel=?1 AND external IS NOT NULL")?;
        let v = st.query_map([channel], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Pair a chat from any channel; returns its chat id. Non-Telegram chats get ids from a range
    /// (9e15+) above Telegram's, so no external id can collide with another.
    /// The person (their id in the messaging app) who paired this chat; "" for chats paired before it was kept.
    pub fn chat_title(&self, chat: i64) -> Result<String> {
        Ok(self.c().query_row("SELECT title FROM chats WHERE chat_id=?1", [chat], |r| r.get(0)).optional()?.unwrap_or_default())
    }

    pub fn chat_owner(&self, chat: i64) -> Result<String> {
        Ok(self.c().query_row("SELECT owner FROM chats WHERE chat_id=?1", [chat], |r| r.get(0)).optional()?.unwrap_or_default())
    }

    /// At startup: an incognito chat that ended in a crash leaves nothing behind either. (Not on every open: the
    /// tools server each turn opens the database, and a live incognito chat may be waiting on an approval.)
    pub fn incognito_leftovers(&self) -> Result<()> {
        self.c().execute("DELETE FROM approvals WHERE chat_id = -1", [])?;
        self.c().execute("DELETE FROM conv_taint WHERE chat_id = -1", [])?;
        self.c().execute("DELETE FROM settings WHERE key LIKE 'private:%:-1'", [])?;
        Ok(())
    }

    pub fn chat_set_dm(&self, chat: i64, dm: bool) -> Result<()> {
        self.c().execute("UPDATE chats SET dm=?2 WHERE chat_id=?1", params![chat, dm])?;
        Ok(())
    }

    /// A conversation only you are in: the app's own chats, or a direct chat in a messaging app.
    pub fn chat_private(&self, chat: i64) -> bool {
        chat == crate::host::LOCAL_CHAT
            || chat == crate::INCOGNITO_CHAT
            // Telegram's private chats have positive ids (a group's are negative); others say so when paired.
            || self.c().query_row("SELECT dm OR (coalesce(channel, 'telegram')='telegram' AND chat_id > 0 AND chat_id < 9000000000000000) FROM chats WHERE chat_id=?1", [chat], |r| r.get::<_, bool>(0)).unwrap_or(false)
    }

    /// A chat someone else paired: see `Chat::guest`. The app's own chats are always yours.
    pub fn chat_guest(&self, chat: i64) -> bool {
        chat != crate::host::LOCAL_CHAT && self.c().query_row("SELECT guest FROM chats WHERE chat_id=?1", [chat], |r| r.get::<_, bool>(0)).unwrap_or(false)
    }

    pub fn chat_set_guest(&self, chat: i64, guest: bool) -> Result<bool> {
        // Everyone in the household talks in a family group: it can't have your email, calendar or logins.
        if !guest && self.chat_family(chat) {
            bail!("a family group is always a guest's chat; turn off Family group first");
        }
        Ok(chat != crate::host::LOCAL_CHAT && self.c().execute("UPDATE chats SET guest=?2 WHERE chat_id=?1", params![chat, guest])? > 0)
    }

    /// Whether a person pairing a chat on this app is someone else: you are whoever paired your chats there before
    /// (the first person to pair on an app is you). Telegram chats from before channels were kept count as Telegram.
    pub fn pairs_as_guest(&self, channel: &str, person: &str) -> Result<bool> {
        let c = self.c();
        let mut st = c.prepare("SELECT DISTINCT owner FROM chats WHERE coalesce(channel, 'telegram')=?1 AND guest=0 AND owner != '' AND chat_id != 0")?;
        let you: Vec<String> = st.query_map([channel], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(!you.is_empty() && !you.iter().any(|o| o == person))
    }

    pub fn chat_set_owner(&self, chat: i64, owner: &str) -> Result<()> {
        self.c().execute("UPDATE chats SET owner=?2 WHERE chat_id=?1", params![chat, owner])?;
        Ok(())
    }

    pub fn pair_external(&self, channel: &str, external: &str, title: &str) -> Result<i64> {
        if channel == "telegram" {
            let id: i64 = tg_chat(external).parse()?;
            self.pair(id, title)?;
            return Ok(id);
        }
        if let Some(id) = self.chat_lookup(channel, external)? {
            self.c().execute("UPDATE chats SET title=?2 WHERE chat_id=?1", params![id, title])?;
            return Ok(id);
        }
        const BASE: i64 = 9_000_000_000_000_000;
        let c = self.c();
        let id: i64 = c.query_row("SELECT max(?1, coalesce(max(chat_id) + 1, ?1)) FROM chats WHERE chat_id >= ?1", [BASE], |r| r.get(0))?;
        c.execute("INSERT INTO chats(chat_id, title, channel, external) VALUES (?1,?2,?3,?4)", params![id, title, channel, external])?;
        Ok(id)
    }

    /// A Telegram topic's own conversation (session, taint, history; memory and routines stay the chat's): an id
    /// from a range no chat uses (Telegram's are below 2^52, other apps' from 9e15), made when the topic is first used.
    pub fn topic_conv(&self, chat: i64, topic: i64) -> Result<i64> {
        const BASE: i64 = 8_000_000_000_000_000;
        let c = self.c();
        let key = format!("topic:{chat}:{topic}");
        c.execute(
            "INSERT OR IGNORE INTO settings(key, value) SELECT ?1, max(?2, coalesce(max(CAST(value AS INTEGER)) + 1, ?2)) FROM settings WHERE key LIKE 'topic:%'",
            params![key, BASE],
        )?;
        Ok(c.query_row("SELECT CAST(value AS INTEGER) FROM settings WHERE key=?1", [key], |r| r.get(0))?)
    }

    /// Telegram guest mode's conversation in your private chat (you typed @bot in some other chat): a topic no real one
    /// can be (their ids are message ids), started afresh each time.
    pub fn guest_mode_conv(&self, chat: i64) -> Result<i64> {
        self.topic_conv(chat, -1)
    }

    /// Whether a conversation is guest mode's: its reply is posted where other people see it.
    pub fn is_guest_mode(&self, conv: i64) -> bool {
        self.c().prepare("SELECT 1 FROM settings WHERE key LIKE 'topic:%:-1' AND CAST(value AS INTEGER)=?1").and_then(|mut s| s.exists([conv])).unwrap_or(false)
    }

    /// Where to send messages for a chat: (channel, external id). None for the local chat.
    pub fn chat_address(&self, chat: i64) -> Result<Option<(String, String)>> {
        if chat == 0 || chat == crate::INCOGNITO_CHAT {
            return Ok(None);
        }
        let row: Option<(String, Option<String>)> =
            self.c().query_row("SELECT channel, external FROM chats WHERE chat_id=?1", [chat], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        // Older Telegram chats use the Telegram id as chat id (no `external`). A chat that isn't paired has no address.
        Ok(row.map(|(ch, ext)| (ch, ext.unwrap_or_else(|| chat.to_string()))))
    }

    pub fn unpair(&self, chat: i64) -> Result<()> {
        // Where catch-up left off goes too: paired again later, the chat starts afresh (not with what came between).
        self.c().execute(
            "DELETE FROM settings WHERE key LIKE 'seen:%' AND key LIKE (SELECT 'seen:%:' || channel || ':' || external FROM chats WHERE chat_id=?1)",
            [chat],
        )?;
        self.c().execute("DELETE FROM chats WHERE chat_id=?1", [chat])?;
        Ok(())
    }

    /// CLI provider session (claude/codex) for a bot's conversation with a chat.
    /// Every conversation a bot has open (to delete the AI CLIs' copies when it goes).
    pub fn sessions_of(&self, bot: i64) -> Result<Vec<String>> {
        let c = self.c();
        let mut st = c.prepare("SELECT session FROM sessions WHERE bot_id=?1")?;
        let v = st.query_map([bot], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn session(&self, bot: i64, chat: i64) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT session FROM sessions WHERE bot_id=?1 AND chat_id=?2", [bot, chat], |r| r.get(0)).optional()?)
    }

    /// What the chat's current session already holds: the memories sent in it, and a hash of its instructions.
    pub fn session_held(&self, bot: i64, chat: i64) -> Result<(Vec<i64>, String)> {
        let (s, p): (String, String) = self
            .c()
            .query_row("SELECT sent, prompt FROM sessions WHERE bot_id=?1 AND chat_id=?2", [bot, chat], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .unwrap_or_default();
        Ok((s.split(',').filter_map(|s| s.parse().ok()).collect(), p))
    }

    /// The memories and instructions the chat's current session holds (call after set_session).
    pub fn set_session_held(&self, bot: i64, chat: i64, ids: &[i64], prompt: &str) -> Result<()> {
        let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        self.c().execute("UPDATE sessions SET sent=?3, prompt=?4 WHERE bot_id=?1 AND chat_id=?2", params![bot, chat, list, prompt])?;
        Ok(())
    }

    pub fn set_session(&self, bot: i64, chat: i64, sid: Option<&str>) -> Result<()> {
        let c = self.c();
        match sid {
            Some(s) => c.execute(
                "INSERT INTO sessions(bot_id, chat_id, session) VALUES (?1,?2,?3) ON CONFLICT(bot_id, chat_id) DO UPDATE SET session = excluded.session",
                params![bot, chat, s],
            )?,
            // A fresh conversation also leaves behind whatever untrusted content the old one read; accounts that are
            // handed the history again (API keys) start from here too.
            None => {
                let last: i64 = c.query_row("SELECT coalesce(max(id), 0) FROM messages WHERE bot_id=?1 AND chat_id=?2", [bot, chat], |r| r.get(0))?;
                c.execute(
                    "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![format!("fresh:{bot}:{chat}"), last.to_string()],
                )?;
                c.execute("DELETE FROM sessions WHERE bot_id=?1 AND chat_id=?2", [bot, chat])? + c.execute("DELETE FROM conv_taint WHERE bot_id=?1 AND chat_id=?2", [bot, chat])?
                    + c.execute("DELETE FROM settings WHERE key = ?1", [crate::tools::private_key(bot, chat)])?
            }
        };
        Ok(())
    }
}
