use super::*;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Usage {
    pub model: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    /// API-list-price equivalent when the provider reports it (claude -p); subscriptions aren't billed per token.
    pub cost_usd: Option<f64>,
    /// Of tokens_in, how many were read back from the prompt cache (billed at about a tenth).
    #[serde(default)]
    pub cache_read: i64,
    /// Of tokens_in, how many were written to the prompt cache (billed at about 1.25×). Many writes and few reads on
    /// a resumed session mean its cached prefix keeps changing.
    #[serde(default)]
    pub cache_write: i64,
    /// The account didn't say what it used: worked out from the text's length.
    #[serde(default)]
    pub estimated: bool,
}

/// A run's tokens as they weigh on a plan (caps, "about N tokens each"): cache reads count a tenth, as they're billed.
pub const WEIGHED: &str = "(tokens_in + tokens_out - cache_read * 9 / 10)";

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub id: i64,
    pub bot_id: i64,
    pub chat_id: i64,
    pub trigger: String,
    pub routine_id: Option<i64>,
    pub provider: String,
    pub model: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: Option<f64>,
    pub tool_calls: i64,
    /// What the run replied (empty for a quiet check that found nothing), for the run log.
    pub reply: Option<String>,
    /// A task handed over by another bot: the run that asked.
    pub parent_run: Option<i64>,
    /// Where a routine's result went: "sent to telegram", "in the digest", "not sent: …" (None: nothing to send).
    pub delivered: Option<String>,
    /// In Needs you: how many times this same failure happened (one item for all of them).
    pub repeats: i64,
    /// Of tokens_in, read back from the prompt cache.
    pub cache_read: i64,
    /// Of tokens_in, written to the prompt cache.
    pub cache_write: i64,
    /// Tokens worked out from the text, the account didn't report them.
    pub estimated: bool,
}

/// A task handed to another bot that reports back later (ask_bot's `later`): pending → running → done → told.
#[derive(Debug, Clone, Default)]
pub struct Delegation {
    pub id: i64,
    /// The bot that asked, and the bot doing it.
    pub bot_id: i64,
    pub to_bot: i64,
    pub chat_id: i64,
    pub conv: i64,
    /// The asking run: the helper's run and the report-back run are linked to it.
    pub run_id: Option<i64>,
    pub task: String,
    pub unattended: bool,
    /// The asker had read outside content, or the helper did: the report-back turn starts tainted.
    pub tainted: bool,
    /// The helper's reply, or why it didn't finish (`failed`).
    pub reply: Option<String>,
    pub failed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCall {
    pub id: i64,
    pub tool: String,
    pub summary: String,
    pub decision: String,
    pub ok: bool,
    pub result: String,
    pub ts: i64,
}

/// One week of how a chat went, from the run log (no model calls): the learning trend metrics.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Week {
    pub start: i64,
    /// Messages the person sent.
    pub turns: i64,
    pub corrections: i64,
    pub stops: i64,
    pub denials: i64,
    pub tool_calls: i64,
    pub tool_errors: i64,
    pub accepted: i64,
    pub dismissed: i64,
    /// Corrections on a request like one corrected before: the same mistake twice, what learning should prevent.
    pub repeats: i64,
    /// "I told you…" corrections: something the bot should have remembered.
    pub forgot: i64,
    /// Memories learned without the person typing them (a bot, a tidy-up, an import) that were taken back with Undo.
    pub undone: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Totals {
    pub runs: i64,
    pub errors: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: f64,
}

impl Db {
    // ---------- run log ----------

    pub fn run_start(&self, bot: i64, chat: i64, trigger: &str, routine_id: Option<i64>, provider: &str) -> Result<i64> {
        let c = self.c();
        c.execute(
            "INSERT INTO runs(bot_id, chat_id, trigger, routine_id, provider, started_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![bot, chat, trigger, routine_id, provider, now()],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// A reply to someone other than you: it counts against their daily allowance (`Db::over_allowance`).
    pub fn run_person(&self, run: i64, person: i64) -> Result<()> {
        self.c().execute("UPDATE runs SET person=?2 WHERE id=?1", params![run, person])?;
        Ok(())
    }

    /// Everything being worked on right now, newest first: (bot, chat, trigger: message | routine | delegated | tidy …).
    /// Not runs older than the longest a turn may take: one a crash left open isn't work (it's closed at the next start).
    pub fn busy(&self) -> Result<Vec<(i64, i64, String)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT bot_id, chat_id, trigger FROM runs WHERE status='running' AND started_at > ?1 ORDER BY id DESC")?;
        let v = st.query_map([now() - 900], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// What started a run: message, routine, rehearsal, delegated…
    pub fn run_trigger(&self, run: i64) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT trigger FROM runs WHERE id=?1", [run], |r| r.get(0)).optional()?)
    }

    /// The routine a run is a run of.
    pub fn run_routine(&self, run: i64) -> Result<Option<i64>> {
        Ok(self.c().query_row("SELECT routine_id FROM runs WHERE id=?1", [run], |r| r.get(0)).optional()?.flatten())
    }

    /// What a rehearsal would have done (the steps it only described) and what it looked at.
    pub fn rehearsal_steps(&self, run: i64) -> Result<(Vec<String>, Vec<String>)> {
        let c = self.c();
        let mut st = c.prepare("SELECT summary, decision FROM tool_calls WHERE run_id=?1 ORDER BY id")?;
        let v: Vec<(String, String)> = st.query_map([run], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        let (would, looked): (Vec<_>, Vec<_>) = v.into_iter().partition(|(_, d)| d == "rehearsal");
        Ok((would.into_iter().map(|w| w.0).collect(), looked.into_iter().map(|l| l.0).collect()))
    }

    /// The run in progress for (bot, chat), if any.
    /// A run's (bot, chat), while it's still running.
    pub fn run_where(&self, run: i64) -> Result<Option<(i64, i64)>> {
        Ok(self.c().query_row("SELECT bot_id, chat_id FROM runs WHERE id=?1 AND status='running'", [run], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
    }

    pub fn running_run(&self, bot: i64, chat: i64) -> Result<Option<i64>> {
        Ok(self.c().query_row("SELECT id FROM runs WHERE bot_id=?1 AND chat_id=?2 AND status='running' ORDER BY id DESC LIMIT 1", [bot, chat], |r| r.get(0)).optional()?)
    }

    /// The user stopped a reply: close its run, and deny anything it was still waiting on.
    /// Stop the reply in progress (`asked`: what it was answering; recorded as a signal for the tidy-up).
    pub fn run_stop(&self, bot: i64, chat: i64, asked: &str) -> Result<()> {
        self.signal_add(bot, chat, "stopped", asked)?;
        let c = self.c();
        c.execute("UPDATE runs SET status='error', error='Stopped by you', ended_at=?3 WHERE bot_id=?1 AND chat_id=?2 AND status='running'", params![bot, chat, now()])?;
        c.execute("UPDATE approvals SET status='denied' WHERE bot_id=?1 AND chat_id=?2 AND status='pending' AND parked_args IS NULL", [bot, chat])?;
        Ok(())
    }

    /// A handed-over task under `parent` was cut off (its goal was paused or removed): its run ends, stopped.
    pub fn run_stopped_under(&self, parent: i64) -> Result<()> {
        self.c().execute("UPDATE runs SET status='error', error='Stopped by you', ended_at=?2 WHERE parent_run=?1 AND status='running'", params![parent, now()])?;
        Ok(())
    }

    pub fn run_asked(&self, run: i64, text: &str) -> Result<()> {
        self.c().execute("UPDATE runs SET asked=?2 WHERE id=?1", params![run, text])?;
        Ok(())
    }

    /// The person's message a run answers; None for routines and tasks from other bots.
    pub fn asked(&self, run: i64) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT asked FROM runs WHERE id=?1", [run], |r| r.get(0)).optional()?.flatten())
    }

    /// What a run was asked to do: the person's message, or the routine's prompt.
    pub fn brief(&self, run: i64) -> Result<Option<String>> {
        let sql = "SELECT COALESCE(r.asked, (SELECT prompt FROM routines WHERE id=r.routine_id)) FROM runs r WHERE r.id=?1";
        Ok(self.c().query_row(sql, [run], |r| r.get(0)).optional()?.flatten())
    }

    pub fn run_recalled(&self, run: i64, ids: &[i64]) -> Result<()> {
        let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        self.c().execute("UPDATE runs SET recalled=?2 WHERE id=?1", params![run, list])?;
        Ok(())
    }

    /// What a run drew on (under each reply in the app, and `/why` in the messaging apps).
    pub fn run_used(&self, run: i64) -> Result<Used> {
        let c = self.c();
        let (chat, recalled): (i64, Option<String>) = c.query_row("SELECT chat_id, recalled FROM runs WHERE id=?1", [run], |r| Ok((r.get(0)?, r.get(1)?))).optional()?.unwrap_or((0, None));
        let mut memories = vec![];
        // Only the run's own chat's (as when blaming a reply).
        let mut st = c.prepare("SELECT id, type, title, body, superseded_by IS NOT NULL, invalid FROM entries WHERE id=?1 AND scope=?2")?;
        for id in recalled.unwrap_or_default().split(',').filter_map(|s| s.parse::<i64>().ok()) {
            if let Some(m) = st
                .query_row(params![id, format!("chat:{chat}")], |r| {
                    let status = if r.get::<_, bool>(5)? { "forgotten" } else if r.get::<_, bool>(4)? { "superseded" } else { "active" };
                    Ok(UsedMemory { id: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, body: r.get(3)?, status })
                })
                .optional()?
            {
                memories.push(m);
            }
        }
        let skills = c
            .prepare("SELECT DISTINCT s.id, s.name FROM skill_uses u JOIN skills s ON s.id = u.skill_id WHERE u.run_id=?1 ORDER BY s.id")?
            .query_map([run], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let pages = c
            .prepare("SELECT summary FROM tool_calls WHERE run_id=?1 AND ok=1 AND tool IN ('web_fetch', 'browser__browser_navigate', 'computer_open') ORDER BY id")?
            .query_map([run], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(Used { memories, skills, pages })
    }

    /// `/why`: what this bot's last reply in the chat drew on.
    pub fn last_reply_used(&self, bot: i64, chat: i64) -> Result<Option<Used>> {
        let run: Option<i64> = self
            .c()
            .query_row("SELECT run_id FROM messages WHERE bot_id=?1 AND chat_id=?2 AND role='assistant' ORDER BY id DESC LIMIT 1", [bot, chat], |r| r.get(0))
            .optional()?
            .flatten();
        run.map(|r| self.run_used(r)).transpose()
    }

    /// The person corrected the bot's last reply here: count it against the memories that reply recalled.
    /// `said`: the correction itself. Besides the memories recalled for the reply, preferences in the standing
    /// instructions that it's about (they shape every reply, so they never show up as "recalled") get a misstep too.
    /// `conv`: the conversation the reply was in (`ToolCtx::conv`).
    pub fn blame_last_reply(&self, bot: i64, chat: i64, conv: i64, said: &str) -> Result<usize> {
        let c = self.c();
        let recalled: Option<String> = c
            .query_row(
                "SELECT r.recalled FROM messages m JOIN runs r ON r.id = m.run_id
                 WHERE m.bot_id=?1 AND m.chat_id=?2 AND m.role='assistant' ORDER BY m.id DESC LIMIT 1",
                [bot, conv],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let mut n = 0;
        let mut blamed: Vec<i64> = recalled.unwrap_or_default().split(',').filter_map(|s| s.parse::<i64>().ok()).collect();
        for id in &blamed {
            n += c.execute("UPDATE entries SET missteps = missteps + 1 WHERE id=?1 AND scope=?2", params![id, format!("chat:{chat}")])?;
        }
        // Words that say what the correction is about (not "please", "that", "your"…).
        // ponytail: word overlap; a wrong match only adds a misstep the tidy-up weighs, and you accept any change
        let words = |s: &str| -> std::collections::HashSet<String> {
            const FILLER: [&str; 24] = [
                "that", "this", "with", "from", "your", "please", "don't", "dont", "just", "like", "what", "when", "then", "than",
                "they", "them", "have", "about", "always", "never", "instead", "should", "would", "said",
            ];
            s.split(|ch: char| !ch.is_alphanumeric() && ch != '\'').map(str::to_lowercase).filter(|w| w.chars().count() > 3 && !FILLER.contains(&w.as_str())).collect()
        };
        let about = words(said);
        let mut st = c.prepare(
            "SELECT id, title, body FROM entries WHERE scope=?1 AND type='convention' AND superseded_by IS NULL AND invalid=0 AND background=0 AND tainted=0",
        )?;
        let hits: Vec<i64> = st
            .query_map([format!("chat:{chat}")], |r| Ok((r.get::<_, i64>(0)?, format!("{} {}", r.get::<_, String>(1)?, r.get::<_, String>(2)?))))?
            .filter_map(|r| r.ok())
            .filter(|(id, text)| !blamed.contains(id) && !words(text).is_disjoint(&about))
            .map(|(id, _)| id)
            .take(3)
            .collect();
        for id in hits {
            n += c.execute("UPDATE entries SET missteps = missteps + 1 WHERE id=?1", [id])?;
            blamed.push(id);
        }
        Ok(n)
    }

    pub fn signal_add(&self, bot: i64, chat: i64, kind: &str, detail: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO signals(bot_id, chat_id, kind, detail, ts) VALUES (?1,?2,?3,?4,?5)",
            params![bot, chat, kind, crate::tools::truncate(detail, 600), now()],
        )?;
        Ok(())
    }

    /// (kind, detail) of a chat's signals since `since`, oldest first.
    pub fn signals_since(&self, chat: i64, since: i64) -> Result<Vec<(String, String)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT kind, detail FROM signals WHERE chat_id=?1 AND ts > ?2 ORDER BY id LIMIT 30")?;
        let v = st.query_map([chat, since], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Corrections, stops and denials since `since`: how much new evidence a tidy-up would have.
    pub fn signal_count(&self, chat: i64, since: i64) -> Result<i64> {
        Ok(self.c().query_row("SELECT count(*) FROM signals WHERE chat_id=?1 AND ts > ?2", [chat, since], |r| r.get(0))?)
    }

    pub fn run_reply(&self, id: i64, text: &str) -> Result<()> {
        self.c().execute("UPDATE runs SET reply=?2 WHERE id=?1", params![id, crate::tools::truncate(text, 4000)])?;
        Ok(())
    }

    pub fn run_finish(&self, id: i64, outcome: Result<&Usage, &str>) -> Result<()> {
        let c = self.c();
        match outcome {
            Ok(u) => c.execute(
                "UPDATE runs SET status='ok', ended_at=?2, model=?3, tokens_in=?4, tokens_out=?5, cost_usd=?6, cache_read=?7, estimated=?8, cache_write=?9 WHERE id=?1",
                params![id, now(), u.model, u.tokens_in, u.tokens_out, u.cost_usd, u.cache_read, u.estimated, u.cache_write],
            )?,
            Err(e) => c.execute("UPDATE runs SET status='error', ended_at=?2, error=?3 WHERE id=?1", params![id, now(), e])?,
        };
        // An approved plan only lasts for its run, and so does its hold on the computer and its files to send.
        c.execute("DELETE FROM settings WHERE key IN (?1, ?2)", [format!("plan:{id}"), format!("outbox:{id}")])?;
        c.execute("DELETE FROM settings WHERE key = 'computer_lease' AND json_extract(value, '$.who') = ?1", [format!("run:{id}")])?;
        Ok(())
    }

    /// Newest first. chat = None for all chats.
    pub fn runs(&self, chat: Option<i64>, limit: usize) -> Result<Vec<Run>> {
        self.runs_where(chat, None, limit)
    }

    /// The runs another bot did for this one (tasks it handed over), oldest first.
    pub fn child_runs(&self, parent: i64) -> Result<Vec<Run>> {
        let mut v = self.runs_where(None, Some(parent), 20)?;
        v.reverse();
        Ok(v)
    }

    pub fn run_set_parent(&self, run: i64, parent: i64) -> Result<()> {
        self.c().execute("UPDATE runs SET parent_run=?2 WHERE id=?1", [run, parent])?;
        Ok(())
    }

    pub fn delegation_add(&self, d: &Delegation) -> Result<i64> {
        let c = self.c();
        c.execute(
            "INSERT INTO delegations(bot_id, to_bot, chat_id, conv, run_id, task, unattended, tainted, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![d.bot_id, d.to_bot, d.chat_id, d.conv, d.run_id, d.task, d.unattended, d.tainted, now()],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// The tasks this bot handed over that are at `status`, oldest first.
    pub fn delegations(&self, bot: i64, status: &str) -> Result<Vec<Delegation>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, bot_id, to_bot, chat_id, conv, run_id, task, unattended, tainted, reply, failed FROM delegations WHERE bot_id=?1 AND status=?2 ORDER BY id")?;
        let v = st
            .query_map(params![bot, status], |r| {
                Ok(Delegation {
                    id: r.get(0)?,
                    bot_id: r.get(1)?,
                    to_bot: r.get(2)?,
                    chat_id: r.get(3)?,
                    conv: r.get(4)?,
                    run_id: r.get(5)?,
                    task: r.get(6)?,
                    unattended: r.get(7)?,
                    tainted: r.get(8)?,
                    reply: r.get(9)?,
                    failed: r.get(10)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn delegation_status(&self, id: i64, status: &str) -> Result<()> {
        self.c().execute("UPDATE delegations SET status=?2 WHERE id=?1", params![id, status])?;
        Ok(())
    }

    /// The helper finished (or failed): what it said waits for the bot that asked to tell the person. Not one its
    /// goal stopped meanwhile.
    pub fn delegation_done(&self, id: i64, reply: &str, failed: bool, tainted: bool) -> Result<()> {
        self.c().execute(
            "UPDATE delegations SET status='done', reply=?2, failed=?3, tainted = tainted OR ?4 WHERE id=?1 AND status='running'",
            params![id, crate::tools::truncate(reply, 8000), failed, tainted],
        )?;
        Ok(())
    }

    fn runs_where(&self, chat: Option<i64>, parent: Option<i64>, limit: usize) -> Result<Vec<Run>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT r.id, r.chat_id, r.trigger, r.routine_id, r.provider, r.model, r.status, r.error, r.started_at, r.ended_at,
                    r.tokens_in, r.tokens_out, r.cost_usd, (SELECT count(*) FROM tool_calls t WHERE t.run_id = r.id), r.bot_id, r.reply, r.parent_run, r.delivered, r.cache_read, r.estimated, r.cache_write
             FROM runs r WHERE (?1 IS NULL OR r.chat_id = ?1) AND (?3 IS NULL OR r.parent_run = ?3) ORDER BY r.id DESC LIMIT ?2",
        )?;
        let v = st
            .query_map(params![chat, limit as i64, parent], |r| {
                Ok(Run {
                    id: r.get(0)?,
                    chat_id: r.get(1)?,
                    trigger: r.get(2)?,
                    routine_id: r.get(3)?,
                    provider: r.get(4)?,
                    model: r.get(5)?,
                    status: r.get(6)?,
                    error: r.get(7)?,
                    started_at: r.get(8)?,
                    ended_at: r.get(9)?,
                    tokens_in: r.get(10)?,
                    tokens_out: r.get(11)?,
                    cost_usd: r.get(12)?,
                    tool_calls: r.get(13)?,
                    bot_id: r.get(14)?,
                    reply: r.get(15)?,
                    parent_run: r.get(16)?,
                    delivered: r.get(17)?,
                    repeats: 1,
                    cache_read: r.get(18)?,
                    estimated: r.get(19)?,
                    cache_write: r.get(20)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn run_totals(&self, chat: Option<i64>, since: i64) -> Result<Totals> {
        Ok(self.c().query_row(
            "SELECT count(*), coalesce(sum(status='error'),0), coalesce(sum(tokens_in),0), coalesce(sum(tokens_out),0), coalesce(sum(cost_usd),0)
             FROM runs WHERE (?1 IS NULL OR chat_id = ?1) AND started_at >= ?2 AND status NOT IN ('missed','unchanged')",
            params![chat, since],
            |r| Ok(Totals { runs: r.get(0)?, errors: r.get(1)?, tokens_in: r.get(2)?, tokens_out: r.get(3)?, cost_usd: r.get(4)? }),
        )?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tool_call_log(&self, run: Option<i64>, chat: i64, tool: &str, summary: &str, decision: &str, ok: bool, result: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO tool_calls(run_id, chat_id, tool, summary, decision, ok, result, ts) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![run, chat, tool, summary, decision, ok, result, now()],
        )?;
        Ok(())
    }

    /// Failed or refused tool calls for a chat since `since` (tidy-up evidence).
    pub fn failed_tool_calls(&self, chat: i64, since: i64) -> Result<Vec<ToolCall>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, tool, summary, decision, ok, result, ts FROM tool_calls WHERE chat_id=?1 AND ts > ?2 AND ok = 0 ORDER BY id DESC LIMIT 30",
        )?;
        let v = st
            .query_map([chat, since], |r| {
                Ok(ToolCall { id: r.get(0)?, tool: r.get(1)?, summary: r.get(2)?, decision: r.get(3)?, ok: r.get(4)?, result: r.get(5)?, ts: r.get(6)? })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Every tool call in a chat since `ts` (any run, a handed-over task's too), oldest first.
    pub fn tool_calls_since(&self, chat: i64, ts: i64) -> Result<Vec<ToolCall>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, tool, summary, decision, ok, result, ts FROM tool_calls WHERE chat_id=?1 AND ts >= ?2 ORDER BY id")?;
        let v = st
            .query_map(params![chat, ts], |r| {
                Ok(ToolCall { id: r.get(0)?, tool: r.get(1)?, summary: r.get(2)?, decision: r.get(3)?, ok: r.get(4)?, result: r.get(5)?, ts: r.get(6)? })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn tool_calls(&self, run: i64) -> Result<Vec<ToolCall>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, tool, summary, decision, ok, result, ts FROM tool_calls WHERE run_id=?1 ORDER BY id")?;
        let v = st
            .query_map([run], |r| {
                Ok(ToolCall { id: r.get(0)?, tool: r.get(1)?, summary: r.get(2)?, decision: r.get(3)?, ok: r.get(4)?, result: r.get(5)?, ts: r.get(6)? })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }
}
