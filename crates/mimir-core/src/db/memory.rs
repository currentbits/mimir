use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub source: String,
    /// A bot saved it in a conversation that had read untrusted content.
    pub tainted: bool,
    /// When it was saved (unix seconds; 0 if unknown), so a bot can tell an old fact from a fresh one.
    pub created_at: i64,
    /// When it stops being true (unix seconds), if it has an end.
    pub valid_until: Option<i64>,
}

impl Entry {
    pub fn line(&self) -> String {
        // One line each, and no angle brackets: a memory's text can't close the block it's in or pass for another entry.
        let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").replace('<', "‹").replace('>', "›");
        let src = if self.source.is_empty() { String::new() } else { format!(" (source: {})", flat(&self.source)) };
        // So a planted "memory" from a web page doesn't read like something the person said: said first, before its
        // text (which could otherwise push the warning out of sight).
        let warn = if self.tainted { " UNCONFIRMED (saved after reading outside content, not by the person)" } else { "" };
        // Dated, so "lives in Utrecht" from two years ago reads as possibly stale next to last week's facts.
        let day = |t: i64| chrono::TimeZone::timestamp_opt(&chrono::Local, t, 0).single().map(|t| t.format("%Y-%m-%d").to_string());
        let saved = (self.created_at > 0).then(|| day(self.created_at)).flatten().map(|d| format!(", saved {d}")).unwrap_or_default();
        // An end date: "until" while it holds; "ENDED" once past it (search still finds it, as history).
        let until = match self.valid_until.and_then(|t| Some((t, day(t)?))) {
            Some((t, d)) if t <= now() => format!(", ENDED {d}"),
            Some((_, d)) => format!(", until {d}"),
            None => String::new(),
        };
        format!("[#{} {}{saved}{until}{warn}] {} — {}{}", self.id, self.kind, flat(&self.title), flat(&self.body), src)
    }
}

/// Who wrote a memory, set by Mimir (never by the model): `you` (in the app), `bot`, `tidy` (an accepted
/// tidy-up suggestion), `import`; empty for memories from older versions.
#[derive(Debug, Clone, Default)]
pub struct Prov {
    pub origin: &'static str,
    pub bot: Option<i64>,
    pub run: Option<i64>,
    /// Written in a conversation that had read untrusted content (web pages, tool results).
    pub tainted: bool,
}

impl Prov {
    pub fn by(origin: &'static str) -> Self {
        Prov { origin, ..Default::default() }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EntryRow {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub source: String,
    pub created_at: i64,
    pub status: &'static str,
    pub superseded_by: Option<i64>,
    pub access_count: i64,
    /// Always in the bot's standing memory, whatever else is there.
    pub pinned: bool,
    /// Moved out of standing memory (not wrong, rarely needed): search still finds it.
    pub background: bool,
    pub origin: String,
    pub bot_id: Option<i64>,
    pub run_id: Option<i64>,
    pub tainted: bool,
    /// Replies that used it (via search) were corrected this many times: a hint it's wrong or outdated.
    pub missteps: i64,
    pub valid_until: Option<i64>,
    /// What the person wrote that the run saving it answered (bot-written memories), so a claim made firmer than
    /// they said it shows.
    pub asked: Option<String>,
    /// Kept for one bot only (the others don't see it); 0 = all bots.
    pub for_bot: i64,
}

fn importance(kind: &str) -> i64 {
    if kind == "convention" { 5 } else { 3 }
}

/// Standing memory's share of every prompt (~2k tokens); a long memory that doesn't fit is left to search.
pub(super) const BOOTSTRAP_CHARS: usize = 8000;
/// Facts and tasks in standing memory: only the newest few (preferences come first).
const BOOTSTRAP_RECENT: usize = 10;

impl Db {
    // ---------- memory ----------

    pub fn store(&self, scope: &str, kind: &str, title: &str, body: &str, source: &str, supersedes: Option<i64>) -> Result<i64> {
        self.store_by(scope, kind, title, body, source, supersedes, &Prov::default())
    }

    /// `store`, recording who wrote it.
    #[allow(clippy::too_many_arguments)]
    pub fn store_by(&self, scope: &str, kind: &str, title: &str, body: &str, source: &str, supersedes: Option<i64>, prov: &Prov) -> Result<i64> {
        if !TYPES.contains(&kind) {
            bail!("type must be one of {TYPES:?}");
        }
        if title.trim().is_empty() || body.trim().is_empty() {
            bail!("title and body are required");
        }
        let mut c = self.c();
        let tx = c.transaction()?;
        let dup: Option<i64> = tx
            .query_row(
                "SELECT id FROM entries WHERE scope=?1 AND title=?2 AND body=?3 AND superseded_by IS NULL AND invalid=0",
                params![scope, title, body],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = dup {
            return Ok(id);
        }
        tx.execute(
            "INSERT INTO entries(scope,type,title,body,source,importance,created_at,origin,bot_id,run_id,tainted) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![scope, kind, title, body, source, importance(kind), now(), prov.origin, prov.bot, prov.run, prov.tainted],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(old) = supersedes {
            let n = tx.execute(
                // id != ?1: the row just inserted can't be the one it replaces (an unknown id could match it).
                "UPDATE entries SET superseded_by=?1 WHERE id=?2 AND id != ?1 AND scope=?3 AND superseded_by IS NULL",
                params![id, old, scope],
            )?;
            if n == 0 {
                bail!("entry #{old} not found or already superseded");
            }
            // A correction keeps the pin (or the background) and the end date; an unconfirmed one doesn't get the pin,
            // which would put it in every prompt.
            tx.execute(
                "UPDATE entries SET (pinned, background, valid_until, for_bot)=(SELECT pinned AND NOT ?3, background, valid_until, for_bot FROM entries WHERE id=?2) WHERE id=?1",
                params![id, old, prov.tainted],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    pub fn forget(&self, scope: &str, id: i64) -> Result<bool> {
        Ok(self.c().execute("UPDATE entries SET invalid=1 WHERE id=?1 AND scope=?2", params![id, scope])? > 0)
    }

    /// Take back something learned: forget it, and if it replaced an earlier version, that one is current again.
    pub fn undo(&self, scope: &str, id: i64) -> Result<bool> {
        let mut c = self.c();
        let tx = c.transaction()?;
        let n = tx.execute("UPDATE entries SET invalid=1, undone_at=?3 WHERE id=?1 AND scope=?2 AND invalid=0 AND superseded_by IS NULL", params![id, scope, now()])?;
        if n > 0 {
            tx.execute("UPDATE entries SET superseded_by=NULL WHERE superseded_by=?1 AND scope=?2", params![id, scope])?;
        }
        tx.commit()?;
        Ok(n > 0)
    }

    /// You checked a memory a bot saved after reading outside content: it's no longer marked unconfirmed.
    pub fn confirm(&self, scope: &str, id: i64) -> Result<bool> {
        Ok(self.c().execute("UPDATE entries SET tainted=0 WHERE id=?1 AND scope=?2", params![id, scope])? > 0)
    }

    /// Pinning brings a background memory back.
    pub fn pin(&self, scope: &str, id: i64, on: bool) -> Result<bool> {
        let sql = "UPDATE entries SET pinned=?3, background=background AND NOT ?3 WHERE id=?1 AND scope=?2 AND superseded_by IS NULL AND invalid=0";
        Ok(self.c().execute(sql, params![id, scope, on])? > 0)
    }

    /// Demote instead of delete: a background memory leaves standing memory (and loses its pin), but
    /// search still finds it when a message needs it.
    /// Give a memory an end date (unix seconds), or take it away (None).
    pub fn set_until(&self, scope: &str, id: i64, until: Option<i64>) -> Result<bool> {
        Ok(self.c().execute("UPDATE entries SET valid_until=?3 WHERE id=?1 AND scope=?2 AND superseded_by IS NULL AND invalid=0", params![id, scope, until])? > 0)
    }

    /// Keep a memory for one bot only, or for all bots again (0).
    pub fn set_for_bot(&self, scope: &str, id: i64, bot: i64) -> Result<bool> {
        Ok(self.c().execute("UPDATE entries SET for_bot=?3 WHERE id=?1 AND scope=?2 AND superseded_by IS NULL AND invalid=0", params![id, scope, bot])? > 0)
    }

    pub fn background(&self, scope: &str, id: i64, on: bool) -> Result<bool> {
        let sql = "UPDATE entries SET background=?3, pinned=pinned AND NOT ?3 WHERE id=?1 AND scope=?2 AND superseded_by IS NULL AND invalid=0";
        Ok(self.c().execute(sql, params![id, scope, on])? > 0)
    }

    /// Delete a memory for good, with every earlier version of it (forget only hides it). Its search
    /// index rows go with it; suggestions about it are dropped. Returns how many entries were erased.
    pub fn erase(&self, scope: &str, id: i64) -> Result<usize> {
        let mut c = self.c();
        let tx = c.transaction()?;
        let mut ids = vec![id];
        let mut next = vec![id];
        while let Some(cur) = next.pop() {
            let mut st = tx.prepare("SELECT id FROM entries WHERE superseded_by=?1 AND scope=?2")?;
            for older in st.query_map(params![cur, scope], |r| r.get::<_, i64>(0))? {
                let older = older?;
                ids.push(older);
                next.push(older);
            }
        }
        let mut n = 0;
        for i in &ids {
            // Copies of its text elsewhere go too: an accepted suggestion that added it, the tool log's lines about it.
            let row: Option<(String, String)> = tx.query_row("SELECT title, body FROM entries WHERE id=?1 AND scope=?2", params![i, scope], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            if let Some((title, body)) = row {
                tx.execute("DELETE FROM suggestions WHERE scope=?1 AND lower(title)=lower(?2) AND body=?3", params![scope, title, body])?;
                if body.len() >= 8 {
                    tx.execute("UPDATE tool_calls SET summary='(an erased memory)', result='' WHERE instr(summary, ?1) > 0 OR instr(result, ?1) > 0", [&body])?;
                }
            }
            n += tx.execute("DELETE FROM entries WHERE id=?1 AND scope=?2", params![i, scope])?;
            tx.execute("DELETE FROM suggestions WHERE entry_id=?1 AND scope=?2", params![i, scope])?;
        }
        tx.commit()?;
        drop(c);
        self.wipe_free_space();
        Ok(n)
    }

    /// bm25 × importance × recency decay (0.995/day, floor 0.05); drops hits under 20% of the top score.
    /// `bot`: leave out the memories other bots keep to themselves (0: all of them).
    pub fn search(&self, scope: &str, bot: i64, query: &str, limit: usize) -> Result<Vec<Entry>> {
        let Some(q) = fts_query(query) else { return Ok(vec![]) };
        let c = self.c();
        let mut st = c.prepare(
            "SELECT e.id, e.type, e.title, e.body, e.source, e.importance, e.created_at, bm25(entries_fts), e.tainted, e.valid_until
             FROM entries_fts JOIN entries e ON e.id = entries_fts.rowid
             WHERE entries_fts MATCH ?1 AND e.scope = ?2 AND e.superseded_by IS NULL AND e.invalid = 0 AND (?3 = 0 OR e.for_bot IN (0, ?3))
             ORDER BY bm25(entries_fts) LIMIT 50",
        )?;
        let t = now();
        let mut hits: Vec<(f64, Entry)> = st
            .query_map(params![q, scope, bot], |r| {
                let imp: i64 = r.get(5)?;
                let created: i64 = r.get(6)?;
                let bm25: f64 = r.get(7)?;
                let days = ((t - created).max(0) as f64) / 86400.0;
                let score = -bm25 * (imp as f64 / 3.0) * 0.995f64.powf(days).max(0.05);
                Ok((score, Entry { id: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, body: r.get(3)?, source: r.get(4)?, tainted: r.get(8)?, created_at: created, valid_until: r.get(9)? }))
            })?
            .collect::<Result<_, _>>()?;
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        let top = hits.first().map(|h| h.0).unwrap_or(0.0);
        let out: Vec<Entry> = hits.into_iter().filter(|h| h.0 >= top * 0.2).take(limit).map(|h| h.1).collect();
        for e in &out {
            c.execute("UPDATE entries SET access_count = access_count + 1, last_recalled = ?2 WHERE id=?1", params![e.id, now()])?;
        }
        Ok(out)
    }

    /// Standing context injected every turn: pinned memories (always), then preferences, then the most recent
    /// facts and tasks, within a size budget. Everything else is still found by search when a message needs it.
    /// `bot`: as in `search`.
    pub fn bootstrap(&self, scope: &str, bot: i64) -> Result<Vec<Entry>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, type, title, body, source, pinned, tainted, created_at, valid_until FROM entries
             WHERE scope=?1 AND superseded_by IS NULL AND invalid=0 AND background=0 AND tainted=0 AND (valid_until IS NULL OR valid_until > ?2)
               AND (?3 = 0 OR for_bot IN (0, ?3))
             ORDER BY pinned DESC, (type='convention') DESC, created_at DESC, id DESC LIMIT 300",
        )?;
        let rows: Vec<(bool, Entry)> = st
            .query_map(params![scope, now(), bot], |r| Ok((r.get(5)?, Entry { id: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, body: r.get(3)?, source: r.get(4)?, tainted: r.get(6)?, created_at: r.get(7)?, valid_until: r.get(8)? })))?
            .collect::<Result<_, _>>()?;
        let (mut used, mut recent, mut out) = (0, 0, vec![]);
        for (pinned, e) in rows {
            let size = e.line().len();
            let fits = used + size <= BOOTSTRAP_CHARS && (e.kind == "convention" || recent < BOOTSTRAP_RECENT);
            // Pinned is a promise: always in, though it still uses up the budget.
            if pinned || fits {
                used += size;
                recent += usize::from(!pinned && e.kind != "convention");
                out.push(e);
            }
        }
        Ok(out)
    }

    /// Memories saved after reading outside content and not confirmed yet (for showing, never for prompts).
    pub fn search_unconfirmed(&self, scope: &str, bot: i64) -> Result<Vec<Entry>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, type, title, body, source, tainted, created_at, valid_until FROM entries WHERE scope=?1 AND superseded_by IS NULL AND invalid=0 AND tainted=1 AND (?2 = 0 OR for_bot IN (0, ?2)) ORDER BY id DESC LIMIT 30")?;
        let v = st.query_map(params![scope, bot], |r| Ok(Entry { id: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, body: r.get(3)?, source: r.get(4)?, tainted: r.get(5)?, created_at: r.get(6)?, valid_until: r.get(7)? }))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// How full standing memory is: (characters used, the budget, memories in it). Pinned ones can push it past 100%.
    pub fn standing_usage(&self, scope: &str) -> Result<(usize, usize, usize)> {
        let b = self.bootstrap(scope, 0)?;
        Ok((b.iter().map(|e| e.line().len()).sum(), BOOTSTRAP_CHARS, b.len()))
    }

    /// Every entry in a scope incl. superseded/forgotten history, newest first (memory browser).
    pub fn entries(&self, scope: &str) -> Result<Vec<EntryRow>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, type, title, body, source, created_at, superseded_by, invalid, access_count, pinned, origin, bot_id, run_id, tainted, missteps, background, valid_until,
               (SELECT asked FROM runs WHERE runs.id = entries.run_id), for_bot
             FROM entries WHERE scope=?1 ORDER BY id DESC LIMIT 1000",
        )?;
        let v = st
            .query_map([scope], |r| {
                let superseded_by: Option<i64> = r.get(6)?;
                let invalid: bool = r.get(7)?;
                Ok(EntryRow {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    title: r.get(2)?,
                    body: r.get(3)?,
                    source: r.get(4)?,
                    created_at: r.get(5)?,
                    status: if invalid { "forgotten" } else if superseded_by.is_some() { "superseded" } else { "active" },
                    superseded_by,
                    access_count: r.get(8)?,
                    pinned: r.get(9)?,
                    origin: r.get(10)?,
                    bot_id: r.get(11)?,
                    run_id: r.get(12)?,
                    tainted: r.get(13)?,
                    missteps: r.get(14)?,
                    background: r.get(15)?,
                    valid_until: r.get(16)?,
                    // What they typed, without a file's text after it.
                    asked: r.get::<_, Option<String>>(17)?.map(|a| a.split("\n\n[Attached:").next().unwrap_or("").to_string()),
                    for_bot: r.get(18)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }
}
