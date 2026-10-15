use super::*;

/// One lasting objective a bot keeps working on in a chat until it's done ("find a plumber who can come this week").
/// The routines and follow-ups it books while the goal is on belong to it, and so do tasks it hands to other bots:
/// pausing the goal holds them, deleting it (or finishing it) removes them. All of that is local, never up to the AI.
#[derive(Debug, Clone, Serialize)]
pub struct Goal {
    pub id: i64,
    pub bot_id: i64,
    pub chat_id: i64,
    /// The goal in the person's words.
    pub text: String,
    /// What has to be true for it to be done.
    pub done_when: String,
    /// active | paused | blocked (waiting on the person) | done
    pub status: String,
    /// Where it stands, one line, written by the bot.
    pub progress: String,
    /// Why it's paused (its budget ran out; "" when you paused it) or blocked.
    pub why: String,
    /// Tokens it may use (0: no budget of its own, only the daily caps), counted after run `budget_from`.
    pub budget: i64,
    #[serde(skip)]
    pub budget_from: i64,
    /// Set or updated by a run that had read outside content: its words may be someone else's.
    pub tainted: bool,
    /// Tokens its own runs used since it was set (or last resumed).
    pub spent: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A goal's routines run and its handed-over tasks start only in these states.
const LIVE: &str = "('active','blocked')";

impl Db {
    /// This chat's goal (a finished one too, until a new one replaces it).
    pub fn goal(&self, chat: i64) -> Result<Option<Goal>> {
        let c = self.c();
        let g = c
            .query_row(
                "SELECT id, bot_id, chat_id, text, done_when, status, progress, why, budget, budget_from, created_at, updated_at, tainted FROM goals WHERE chat_id=?1",
                [chat],
                |r| {
                    Ok(Goal {
                        id: r.get(0)?,
                        bot_id: r.get(1)?,
                        chat_id: r.get(2)?,
                        text: r.get(3)?,
                        done_when: r.get(4)?,
                        status: r.get(5)?,
                        progress: r.get(6)?,
                        why: r.get(7)?,
                        budget: r.get(8)?,
                        budget_from: r.get(9)?,
                        spent: 0,
                        created_at: r.get(10)?,
                        updated_at: r.get(11)?,
                        tainted: r.get(12)?,
                    })
                },
            )
            .optional()?;
        let Some(mut g) = g else { return Ok(None) };
        // Its own runs: its routines and follow-ups, tasks it handed over and their reports, and turns in its chat while it was on.
        g.spent = c.query_row(
            &format!("SELECT coalesce(sum({WEIGHED}), 0) FROM runs WHERE goal_id=?1 AND id > ?2"),
            params![g.id, g.budget_from],
            |r| r.get(0),
        )?;
        Ok(Some(g))
    }

    /// A bot sets the chat's goal. One per chat: a finished one is replaced, one still going has to be finished or
    /// removed by the person first.
    pub fn goal_set(&self, bot: i64, chat: i64, text: &str, done_when: &str, budget: i64, tainted: bool) -> Result<Goal> {
        let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let (text, done_when) = (flat(text), flat(done_when));
        if text.is_empty() || text.chars().count() > 300 {
            bail!("a goal needs to be said in up to 300 characters");
        }
        if done_when.is_empty() || done_when.chars().count() > 300 {
            bail!("say when it's done, in up to 300 characters");
        }
        if budget < 0 {
            bail!("a budget is a number of tokens (0 for none)");
        }
        if let Some(g) = self.goal(chat)? {
            if g.status != "done" {
                bail!("this chat already has a goal (\"{}\"); update its progress, or the person can remove it first", g.text);
            }
            self.goal_delete(chat)?;
        }
        let t = now();
        self.c().execute(
            "INSERT INTO goals(bot_id, chat_id, text, done_when, status, budget, budget_from, tainted, created_at, updated_at) VALUES (?1,?2,?3,?4,'active',?5,(SELECT coalesce(max(id), 0) FROM runs),?6,?7,?7)",
            params![bot, chat, text, done_when, budget, tainted, t],
        )?;
        self.goal(chat)?.context("goal gone")
    }

    /// The bot's progress line, and optionally a new state (active or blocked; done goes through goal_status after its
    /// check). Not on a paused or finished goal: that's the person's to change.
    /// `tainted`: written by a run that had read outside content; the goal stays marked from then on.
    pub fn goal_progress(&self, chat: i64, progress: &str, status: Option<&str>, why: &str, tainted: bool) -> Result<Goal> {
        let g = self.goal(chat)?.context("this chat has no goal")?;
        if !["active", "blocked"].contains(&g.status.as_str()) {
            bail!("the goal is {}; only the person can change it now", g.status);
        }
        let progress = progress.split_whitespace().collect::<Vec<_>>().join(" ");
        if progress.chars().count() > 300 {
            bail!("keep progress to one line (up to 300 characters)");
        }
        let status = status.unwrap_or(&g.status);
        if !["active", "blocked"].contains(&status) {
            bail!("status is active, blocked or done");
        }
        let why = if status == "blocked" { why.split_whitespace().collect::<Vec<_>>().join(" ") } else { String::new() };
        self.c().execute("UPDATE goals SET progress=?2, status=?3, why=?4, updated_at=?5, tainted = tainted OR ?6 WHERE id=?1", params![g.id, progress, status, why, now(), tainted])?;
        self.goal(chat)?.context("goal gone")
    }

    /// Pause, resume or finish a goal (the person in the app, or a done the run log backs). Paused, its routines don't
    /// run and its handed-over tasks don't start; done, they're removed. Resuming starts its budget over, and moves
    /// routines that came due meanwhile on to their next time (like the end of a pause). Returns the handed-over
    /// tasks it stopped mid-run, for the runtime to cut off.
    pub fn goal_status(&self, chat: i64, status: &str, why: &str) -> Result<Vec<i64>> {
        if !["active", "paused", "done"].contains(&status) {
            bail!("a goal is active, paused or done");
        }
        let g = self.goal(chat)?.context("this chat has no goal")?;
        // Its routines are gone: a finished goal is removed, or replaced by a new one, not taken up again.
        if g.status == "done" {
            bail!("this goal is done; remove it, or set a new one");
        }
        let t = now();
        if status == "active" && g.status == "paused" {
            for r in self.routines_where("goal_id = ?1 AND next_fire <= ?2 AND once = 0", params![g.id, t])? {
                self.c().execute("UPDATE routines SET next_fire=?2, retries=0 WHERE id=?1", params![r.id, next_fire(&r.cron, t)?])?;
            }
            self.c().execute("UPDATE goals SET budget_from=(SELECT coalesce(max(id), 0) FROM runs) WHERE id=?1", [g.id])?;
        }
        if status == "done" {
            for r in self.routines_where("goal_id = ?1", [g.id])? {
                self.routine_delete(r.chat_id, r.id)?;
            }
        }
        self.c().execute("UPDATE goals SET status=?2, why=?3, updated_at=?4 WHERE id=?1", params![g.id, status, why, t])?;
        match status {
            "paused" => self.goal_stop_tasks(g.id, false),
            "done" => self.goal_stop_tasks(g.id, true),
            _ => Ok(vec![]),
        }
    }

    /// Remove a chat's goal, and the routines, follow-ups and handed-over tasks it started. Returns the tasks stopped
    /// mid-run.
    pub fn goal_delete(&self, chat: i64) -> Result<Vec<i64>> {
        let Some(g) = self.goal(chat)? else { return Ok(vec![]) };
        for r in self.routines_where("goal_id = ?1", [g.id])? {
            self.routine_delete(r.chat_id, r.id)?;
        }
        let stopped = self.goal_stop_tasks(g.id, true)?;
        self.c().execute("DELETE FROM goals WHERE id=?1", [g.id])?;
        Ok(stopped)
    }

    /// Stop the goal's handed-over tasks that are running (their result is never told), and those waiting to start
    /// too when `all` (paused, they just wait). Returns the running ones.
    fn goal_stop_tasks(&self, goal: i64, all: bool) -> Result<Vec<i64>> {
        let c = self.c();
        let running: Vec<i64> = c.prepare("SELECT id FROM delegations WHERE goal_id=?1 AND status='running'")?.query_map([goal], |r| r.get(0))?.collect::<Result<_, _>>()?;
        c.execute(&format!("UPDATE delegations SET status='stopped' WHERE goal_id=?1 AND status IN ({})", if all { "'pending','running'" } else { "'running'" }), [goal])?;
        Ok(running)
    }

    /// A run counts for a goal: a routine's run for the routine's goal, a handed-over task's (or its report's) for the
    /// asking run's, and a turn in a chat for the chat's goal while it's on.
    pub fn goal_claim_run(&self, run: i64, chat: i64, routine: Option<i64>, parent: Option<i64>) -> Result<()> {
        let c = self.c();
        match (routine, parent) {
            (Some(r), _) => c.execute("UPDATE runs SET goal_id=(SELECT goal_id FROM routines WHERE id=?2) WHERE id=?1", [run, r])?,
            (None, Some(p)) => c.execute("UPDATE runs SET goal_id=(SELECT goal_id FROM runs WHERE id=?2) WHERE id=?1", [run, p])?,
            (None, None) => c.execute(&format!("UPDATE runs SET goal_id=(SELECT id FROM goals WHERE chat_id=?2 AND status IN {LIVE}) WHERE id=?1"), [run, chat])?,
        };
        Ok(())
    }

    /// A routine or follow-up booked in a chat whose goal is on belongs to that goal.
    pub fn goal_claim_routine(&self, routine: i64, chat: i64) -> Result<()> {
        self.c().execute(&format!("UPDATE routines SET goal_id=(SELECT id FROM goals WHERE chat_id=?2 AND status IN {LIVE}) WHERE id=?1"), [routine, chat])?;
        Ok(())
    }

    /// Likewise a task handed to another bot.
    pub fn goal_claim_delegation(&self, delegation: i64, chat: i64) -> Result<()> {
        self.c().execute(&format!("UPDATE delegations SET goal_id=(SELECT id FROM goals WHERE chat_id=?2 AND status IN {LIVE}) WHERE id=?1"), [delegation, chat])?;
        Ok(())
    }

    /// The goal a routine or a handed-over task belongs to, if any.
    pub fn goal_of(&self, table: &str, id: i64) -> Result<Option<Goal>> {
        let table = if table == "routines" { "routines" } else { "delegations" };
        let chat: Option<i64> = self.c().query_row(&format!("SELECT g.chat_id FROM {table} x JOIN goals g ON g.id = x.goal_id WHERE x.id=?1"), [id], |r| r.get(0)).optional()?;
        match chat {
            Some(c) => self.goal(c),
            None => Ok(None),
        }
    }

    /// A goal over its own budget pauses, and Needs you asks whether to go on. True when it just did.
    pub fn goal_check_budget(&self, g: &Goal) -> Result<bool> {
        if g.budget <= 0 || g.spent < g.budget || g.status == "paused" {
            return Ok(false);
        }
        let k = |n: i64| if n >= 10_000 { format!("{}k", n / 1000) } else { n.to_string() };
        self.goal_status(g.chat_id, "paused", &format!("it used {} of its {}-token budget", k(g.spent), k(g.budget)))?;
        Ok(true)
    }

    /// Goals that need the person: paused by their budget, or blocked on them.
    pub fn goals_waiting(&self) -> Result<Vec<Goal>> {
        let chats: Vec<i64> = {
            let c = self.c();
            let mut st = c.prepare("SELECT chat_id FROM goals WHERE status='blocked' OR (status='paused' AND why != '') ORDER BY updated_at")?;
            st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        Ok(chats.into_iter().filter_map(|c| self.goal(c).ok().flatten()).collect())
    }
}
