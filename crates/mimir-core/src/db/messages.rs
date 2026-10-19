use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: i64,
    pub bot_id: i64,
    pub role: String,
    pub content: String,
    pub ts: i64,
    /// Replies: what the bot did on the way (the run's tool calls, without results).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    /// Replies: the run that wrote it (to open what another bot did for it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<i64>,
    /// Replies: what the reply drew on ("2 memories · 1 skill · read 3 pages").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<Used>,
}

/// What one run drew on: the memories recalled for it, the skills it read, the pages it opened.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Used {
    pub memories: Vec<UsedMemory>,
    /// (id, name)
    pub skills: Vec<(i64, String)>,
    /// What each page read says ("Read https://…").
    pub pages: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsedMemory {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub body: String,
    /// "active", or "superseded"/"forgotten" since the reply.
    pub status: &'static str,
}

impl Used {
    pub fn is_empty(&self) -> bool {
        self.memories.is_empty() && self.skills.is_empty() && self.pages.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Action {
    pub tool: String,
    pub summary: String,
    pub decision: String,
    pub ok: bool,
    /// What the tool said (short), e.g. "routine #5 created", so the UI can link to what was made.
    pub result: String,
}

/// A past message found by chat_search.
#[derive(Debug, Clone)]
pub struct ChatHit {
    pub id: i64,
    pub ts: i64,
    pub bot_id: i64,
    pub role: String,
    pub excerpt: String,
    /// A bot message whose run read outside content, or whose run isn't known (a digest, a stopped reply): not the
    /// person's words, and not trusted.
    pub tainted: bool,
    /// A routine's scheduled prompt (logged as the user's turn, but a bot may have written it).
    pub routine: bool,
}

impl Db {
    // ---------- taint (untrusted content read) ----------

    /// Has this run read untrusted content? (A tools server started mid-run must not start clean.)
    pub fn run_tainted(&self, run: i64) -> Result<bool> {
        Ok(self.c().query_row("SELECT tainted FROM runs WHERE id=?1", [run], |r| r.get::<_, bool>(0)).optional()?.unwrap_or(false))
    }

    pub fn run_taint(&self, run: i64) -> Result<()> {
        self.c().execute("UPDATE runs SET tainted=1 WHERE id=?1", [run])?;
        Ok(())
    }

    pub fn conv_taint(&self, bot: i64, chat: i64) -> Result<()> {
        self.c().execute("INSERT OR IGNORE INTO conv_taint(bot_id, chat_id, since) VALUES (?1,?2,?3)", params![bot, chat, now()])?;
        Ok(())
    }

    pub fn conv_tainted(&self, bot: i64, chat: i64) -> Result<bool> {
        Ok(self.c().prepare("SELECT 1 FROM conv_taint WHERE bot_id=?1 AND chat_id=?2")?.exists([bot, chat])?)
    }

    /// Whether `bot`'s latest handed-over task in this chat read untrusted content.
    /// The helper's latest handed-over run in this chat; under `parent` (the asking run) when known, so two tasks for the
    /// same helper (its own chat is shared by every asker) don't read each other's.
    pub fn delegated_tainted(&self, bot: i64, chat: i64, parent: Option<i64>) -> Result<bool> {
        Ok(self
            .c()
            .query_row(
                "SELECT tainted FROM runs WHERE bot_id=?1 AND chat_id=?2 AND trigger='delegated' AND (?3 IS NULL OR parent_run=?3) ORDER BY id DESC LIMIT 1",
                params![bot, chat, parent],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    pub fn log(&self, bot: i64, chat: i64, role: &str, content: &str, run: Option<i64>) -> Result<()> {
        self.c().execute(
            "INSERT INTO messages(bot_id, chat_id, role, content, ts, run_id) VALUES (?1,?2,?3,?4,?5,?6)",
            params![bot, chat, role, content, now(), run],
        )?;
        Ok(())
    }

    /// Earlier messages with this chat (all bots) that match `query`, best first: an excerpt around the match.
    /// `tainted`: a bot message from a run that read outside content, or from no known run; a file's text, or a message that
    /// quotes another (the quote isn't the person's words).
    /// `range`: (from, to) in unix seconds, inclusive.
    pub fn chat_search(&self, chat: i64, query: &str, exclude_run: Option<i64>, range: (i64, i64), limit: usize) -> Result<Vec<ChatHit>> {
        let Some(q) = fts_query(query) else { return Ok(vec![]) };
        let c = self.c();
        let mut st = c.prepare(
            "SELECT m.ts, m.bot_id, m.role, snippet(messages_fts, 0, '', '', '…', 40), (m.role = 'assistant' AND coalesce(r.tainted, 1)) OR (m.role = 'user' AND (m.content LIKE '%<document name=%' OR m.content LIKE '<replying_to_%')),
               m.role = 'user' AND m.content LIKE '[Scheduled%', m.id
             FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid LEFT JOIN runs r ON r.id = m.run_id
             WHERE messages_fts MATCH ?1 AND m.chat_id = ?2 AND (?3 IS NULL OR m.run_id IS NOT ?3) AND m.ts BETWEEN ?5 AND ?6
             ORDER BY bm25(messages_fts) LIMIT ?4",
        )?;
        let v = st
            .query_map(params![q, chat, exclude_run, limit as i64, range.0, range.1], |r| Ok(ChatHit { id: r.get(6)?, ts: r.get(0)?, bot_id: r.get(1)?, role: r.get(2)?, excerpt: r.get(3)?, tainted: r.get(4)?, routine: r.get(5)? }))?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// A bot's last `n` messages with a chat, oldest first (provider history).
    /// The history an API account is handed: the last 10 to 19 messages, its start moving 10 at a time (not 2 every
    /// turn), so the provider's cached prefix holds for five turns.
    pub fn recent_stepped(&self, bot: i64, chat: i64) -> Result<Vec<(String, String)>> {
        let n: i64 = self.c().query_row(
            "SELECT count(*) FROM messages WHERE bot_id=?1 AND chat_id=?2
               AND id > coalesce((SELECT CAST(value AS INTEGER) FROM settings WHERE key='fresh:' || ?1 || ':' || ?2), 0)",
            [bot, chat],
            |r| r.get(0),
        )?;
        self.recent(bot, chat, if n < 10 { n as usize } else { 10 + (n % 10) as usize })
    }

    pub fn recent(&self, bot: i64, chat: i64, n: usize) -> Result<Vec<(String, String)>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT role, content FROM messages WHERE bot_id=?1 AND chat_id=?2
               AND id > coalesce((SELECT CAST(value AS INTEGER) FROM settings WHERE key='fresh:' || ?1 || ':' || ?2), 0)
             ORDER BY id DESC LIMIT ?3",
        )?;
        let mut v: Vec<(String, String)> = st.query_map(params![bot, chat, n as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        v.reverse();
        Ok(v)
    }

    /// When a bot and a chat last said anything to each other.
    pub fn last_said(&self, bot: i64, chat: i64) -> Result<Option<i64>> {
        Ok(self.c().query_row("SELECT max(ts) FROM messages WHERE bot_id=?1 AND chat_id=?2", [bot, chat], |r| r.get(0))?)
    }

    /// How many messages a conversation has after message `id` (since it last started fresh), and its newest id.
    pub fn messages_after(&self, bot: i64, chat: i64, id: i64) -> Result<(i64, i64)> {
        Ok(self.c().query_row(
            "SELECT count(*), coalesce(max(id), 0) FROM messages WHERE bot_id=?1 AND chat_id=?2 AND id > ?3
               AND id > coalesce((SELECT CAST(value AS INTEGER) FROM settings WHERE key='fresh:' || ?1 || ':' || ?2), 0)",
            [bot, chat, id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    /// All bots' messages with a chat, oldest first (UI).
    /// Whether the last `n` messages a bot would be given as history hold outside content: a file's or forward's
    /// text, or a reply from a run that had read some.
    pub fn recent_outside(&self, bot: i64, chat: i64, n: usize) -> Result<bool> {
        Ok(self.c().query_row(
            "SELECT EXISTS(SELECT 1 FROM (SELECT role, content, run_id FROM messages WHERE bot_id=?1 AND chat_id=?2
                 AND id > coalesce((SELECT CAST(value AS INTEGER) FROM settings WHERE key='fresh:' || ?1 || ':' || ?2), 0)
                 ORDER BY id DESC LIMIT ?3) m
               WHERE (m.role='user' AND m.content LIKE '%<document name=%')
                  OR (m.role='assistant' AND m.run_id IN (SELECT id FROM runs WHERE tainted=1)))",
            params![bot, chat, n as i64],
            |r| r.get(0),
        )?)
    }

    /// The bot's latest message with a chat, if it was sent outside the chat's CLI session: a routine's result
    /// ("routine result"), a task done in its own chat for another bot, or one from no run (digest, weekly review, a note after an approval: "message").
    pub fn last_outside_session(&self, bot: i64, chat: i64) -> Result<Option<(&'static str, String)>> {
        let last: Option<(String, String, Option<String>)> = self
            .c()
            .query_row(
                "SELECT m.role, m.content, CASE WHEN m.run_id IS NULL THEN NULL ELSE coalesce(r.trigger, '') END
                   FROM messages m LEFT JOIN runs r ON r.id = m.run_id WHERE m.bot_id=?1 AND m.chat_id=?2
                   AND m.id > coalesce((SELECT CAST(value AS INTEGER) FROM settings WHERE key='fresh:' || ?1 || ':' || ?2), 0)
                 ORDER BY m.id DESC LIMIT 1",
                [bot, chat],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match last {
            Some((role, text, trigger)) if role == "assistant" => match trigger.as_deref() {
                None => Some(("message", text)),
                Some("message") => None,
                Some("delegated") => Some(("task another bot handed you", text)),
                Some(_) => Some(("routine result", text)),
            },
            _ => None,
        })
    }

    pub fn messages(&self, chat: i64, n: usize) -> Result<Vec<Message>> {
        self.messages_of(chat, None, None, 0, n)
    }

    /// The last `n` messages of a chat, oldest first; with `bot`, only that bot's conversation in it (what you said to
    /// it and what it said), as when you open a bot from the sidebar. Paging: only ids below `before`, and none below `from`.
    pub fn messages_of(&self, chat: i64, bot: Option<i64>, before: Option<i64>, from: i64, n: usize) -> Result<Vec<Message>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT bot_id, role, content, ts, run_id, id FROM messages WHERE chat_id=?1 AND (?3 IS NULL OR bot_id=?3)
               AND (?4 IS NULL OR id < ?4) AND id >= ?5 ORDER BY id DESC LIMIT ?2",
        )?;
        let mut v: Vec<(Message, Option<i64>)> = st
            .query_map(params![chat, n as i64, bot, before, from], |r| Ok((Message { id: r.get(5)?, bot_id: r.get(0)?, role: r.get(1)?, content: r.get(2)?, ts: r.get(3)?, actions: vec![], run_id: None, used: None }, r.get(4)?)))?
            .collect::<Result<_, _>>()?;
        v.reverse();
        // One query for every reply's actions.
        let first = v.iter().filter_map(|(_, r)| *r).min().unwrap_or(i64::MAX);
        let last = v.iter().filter_map(|(_, r)| *r).max().unwrap_or(0);
        let mut acts = c.prepare("SELECT run_id, tool, summary, decision, ok, substr(result, 1, 200) FROM tool_calls WHERE chat_id=?1 AND run_id BETWEEN ?2 AND ?3 ORDER BY id")?;
        let mut by_run: std::collections::HashMap<i64, Vec<Action>> = Default::default();
        for row in acts.query_map(params![chat, first, last], |r| Ok((r.get::<_, i64>(0)?, Action { tool: r.get(1)?, summary: r.get(2)?, decision: r.get(3)?, ok: r.get(4)?, result: r.get(5)? })))? {
            let (run, a) = row?;
            by_run.entry(run).or_default().push(a);
        }
        for (m, run) in v.iter_mut() {
            if m.role == "assistant" {
                m.run_id = *run;
            }
            if let Some(a) = run.filter(|_| m.role == "assistant").and_then(|r| by_run.remove(&r)) {
                m.actions = a;
            }
        }
        drop(acts);
        drop(st);
        drop(c);
        // ponytail: three small queries per reply; one joined query if long chats load slowly.
        for (m, _) in v.iter_mut() {
            if let Some(run) = m.run_id {
                m.used = Some(self.run_used(run)?).filter(|u| !u.is_empty());
            }
        }
        Ok(v.into_iter().map(|(m, _)| m).collect())
    }
}
