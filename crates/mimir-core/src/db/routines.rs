use super::*;

/// How late a routine may run before it counts as missed: the scheduler checks every 30 s, and a run before it can
/// take minutes.
pub const MISSED_AFTER: i64 = 15 * 60;

/// Routine results that may come through right away on the person's own rule, a day, all bots together: the rule is
/// theirs, but whether it holds is the AI's call on what it read, so a page can't flood them.
pub const URGENT_PER_DAY: i64 = 3;

#[derive(Debug, Clone, Serialize)]
pub struct RunMark {
    pub status: String,
    pub at: i64,
    pub tokens: i64,
}

/// A routine's last 30 days (runs it did, their tokens and cost, how many failed, slots it missed) and its last runs.
#[derive(Debug, Clone, Serialize)]
pub struct RoutineStats {
    pub id: i64,
    pub runs: i64,
    pub tokens: i64,
    pub cost_usd: Option<f64>,
    pub failed: i64,
    pub missed: i64,
    /// Watched quiet checks that found nothing changed (no AI run; not in `runs`).
    pub unchanged: i64,
    /// Kinds of line on the watched page left out because they change on every load (e.g. "Views: #").
    pub ignored: Vec<String>,
    pub last: Vec<RunMark>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Routine {
    pub id: i64,
    pub bot_id: i64,
    pub chat_id: i64,
    pub cron: String,
    pub prompt: String,
    pub next_fire: i64,
    /// A heartbeat check: read-only, and silent unless something needs the user's attention.
    pub quiet: bool,
    /// Kept but not run.
    pub paused: bool,
    /// now (message right away) | digest (in the daily digest) | log (only in the run log).
    pub delivery: String,
    /// Made by a bot in a conversation that had read outside content: every run of it counts as having read it.
    pub tainted: bool,
    /// Runs once, at next_fire, then is deleted (no cron).
    #[serde(default)]
    pub once: bool,
    /// A quiet check's page or feed address, or "mail": Mimir looks first and runs the AI only when it changed.
    #[serde(default)]
    pub watch: String,
    /// Tries since the last run that worked, after failures that may pass (the network, the setup).
    #[serde(default)]
    pub retries: i64,
    /// The person's own rule for coming through right away when it would otherwise wait ("" = never). Only set from
    /// the app, never by a bot, so nothing the bot read can widen it.
    #[serde(default)]
    pub urgent: String,
    /// A repeating routine's end (0 = none): its last run is the last slot before it, then it's gone.
    #[serde(default)]
    pub ends: i64,
    /// A one-off the bot booked on its own ("I'll check back Friday"), not one the person asked for.
    #[serde(default)]
    pub followup: bool,
    /// Its goal is paused (or done): it waits with it.
    #[serde(default)]
    pub goal_held: bool,
    /// Booked for the chat's goal: it goes when the goal is done or removed.
    #[serde(default)]
    pub for_goal: bool,
}

/// Routines waiting: until a usage limit resets, or a daily cap's day is over. `why` finishes "Missed 3 runs, …: ".
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Hold {
    pub until: i64,
    pub why: String,
    /// A daily cap's hold: the setting that lets it go on today anyway when the person says so (None: a usage limit).
    #[serde(default)]
    pub waive: Option<String>,
}

/// Daily token caps (none when unset): each routine, each bot, all bots together.
pub const CAPS: [&str; 3] = ["cap_routine", "cap_bot", "cap_day"];

/// A calendar routine's next_fire while no matching event is coming up (2100-01-01): the scheduler sets the real one.
pub const IDLE: i64 = 4_102_444_800;

/// Accepts standard 5-field cron ("0 8 * * Mon-Fri") or the cron crate's 6/7-field form. A calendar trigger
/// ("calendar:60:outside") waits (IDLE) until the scheduler reads the calendar.
pub fn next_fire(expr: &str, after: i64) -> Result<i64> {
    if crate::calendar::trigger(expr)?.is_some() || crate::events::on(expr)?.is_some() {
        return Ok(IDLE);
    }
    let expr = expr.trim();
    let full = if expr.split_whitespace().count() == 5 { format!("0 {expr}") } else { expr.to_string() };
    let schedule = cron::Schedule::from_str(&full).map_err(|e| anyhow::anyhow!("bad cron '{expr}': {e}"))?;
    let after = Local.timestamp_opt(after, 0).single().unwrap_or_else(Local::now);
    match schedule.after(&after).next() {
        Some(t) => Ok(t.timestamp()),
        None => bail!("cron '{expr}' never fires"),
    }
}

/// The shortest gap between two runs of a schedule over the next couple of days, in minutes.
pub fn min_gap_minutes(expr: &str) -> Result<i64> {
    // ponytail: a calendar or event trigger runs as often as its events come; not counted
    if crate::calendar::trigger(expr)?.is_some() || crate::events::on(expr)?.is_some() {
        return Ok(i64::MAX);
    }
    let expr = expr.trim();
    let full = if expr.split_whitespace().count() == 5 { format!("0 {expr}") } else { expr.to_string() };
    let schedule = cron::Schedule::from_str(&full).map_err(|e| anyhow::anyhow!("bad cron '{expr}': {e}"))?;
    let times: Vec<i64> = schedule.upcoming(chrono::Utc).take(200).map(|t| t.timestamp()).collect();
    Ok(times.windows(2).map(|w| (w[1] - w[0]) / 60).min().unwrap_or(i64::MAX))
}

/// How a routine result that couldn't be sent is marked (`runs.delivered`).
pub const NOT_SENT: &str = "not sent";
/// Why a routine ran late, for its run log line.
pub const AWAY: &str = "Mimir wasn't running then";
pub const ASLEEP: &str = "the computer was asleep, or a long run held it up";

/// A run's status when a watched quiet check found nothing changed, so no AI ran.
pub const UNCHANGED: &str = "unchanged";

/// (id, chat, routine, text) of an unsent digest item.
/// (id, chat, routine, text, from a run that read outside content)
/// A hold that hasn't ended: (its key, the bot, the routine if it's one routine's, the hold).
pub type HoldRow = (String, i64, Option<i64>, Hold);
pub type DigestItem = (i64, i64, Option<i64>, String, bool);

impl Db {
    // ---------- routines ----------

    pub fn routine_create(&self, bot: i64, chat: i64, cron: &str, prompt: &str, quiet: bool) -> Result<Routine> {
        if prompt.trim().is_empty() {
            bail!("a routine needs a prompt");
        }
        let next = next_fire(cron, now())?;
        self.calendar_private(chat, cron)?;
        let c = self.c();
        c.execute(
            "INSERT INTO routines(bot_id, chat_id, cron, prompt, next_fire, quiet) VALUES (?1,?2,?3,?4,?5,?6)",
            params![bot, chat, cron.trim(), prompt.trim(), next, quiet],
        )?;
        Ok(Routine { id: c.last_insert_rowid(), bot_id: bot, chat_id: chat, cron: cron.trim().into(), prompt: prompt.trim().into(), next_fire: next, quiet, paused: false, delivery: "now".into(), tainted: false, once: false, watch: String::new(), retries: 0, urgent: String::new(), ends: 0, followup: false, goal_held: false, for_goal: false })
    }

    /// A one-off at `at` (unix seconds): a reminder, or a task for later. It's deleted once it has run.
    pub fn routine_once(&self, bot: i64, chat: i64, at: i64, prompt: &str, quiet: bool) -> Result<Routine> {
        if prompt.trim().is_empty() {
            bail!("a routine needs a prompt");
        }
        if at <= now() {
            bail!("that time has already passed");
        }
        let c = self.c();
        c.execute(
            "INSERT INTO routines(bot_id, chat_id, cron, prompt, next_fire, quiet, once) VALUES (?1,?2,'',?3,?4,?5,1)",
            params![bot, chat, prompt.trim(), at, quiet],
        )?;
        Ok(Routine { id: c.last_insert_rowid(), bot_id: bot, chat_id: chat, cron: String::new(), prompt: prompt.trim().into(), next_fire: at, quiet, paused: false, delivery: "now".into(), tainted: false, once: true, watch: String::new(), retries: 0, urgent: String::new(), ends: 0, followup: false, goal_held: false, for_goal: false })
    }

    /// A routine a bot made after reading outside content: its prompt may carry someone else's words.
    pub fn routine_taint(&self, id: i64) -> Result<()> {
        self.c().execute("UPDATE routines SET tainted=1 WHERE id=?1", [id])?;
        Ok(())
    }

    pub(super) fn routines_where(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Routine>> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT id, chat_id, cron, prompt, next_fire, bot_id, quiet, paused, delivery, tainted, once, watch, retries, urgent, ends, followup,
                goal_id IS NOT NULL AND goal_id NOT IN (SELECT id FROM goals WHERE status IN ('active','blocked')), goal_id IS NOT NULL FROM routines WHERE {sql} ORDER BY id"))?;
        let v = st
            .query_map(p, |r| {
                Ok(Routine {
                    id: r.get(0)?,
                    chat_id: r.get(1)?,
                    cron: r.get(2)?,
                    prompt: r.get(3)?,
                    next_fire: r.get(4)?,
                    bot_id: r.get(5)?,
                    quiet: r.get(6)?,
                    paused: r.get(7)?,
                    delivery: r.get(8)?,
                    tainted: r.get(9)?,
                    once: r.get(10)?,
                    watch: r.get(11)?,
                    retries: r.get(12)?,
                    urgent: r.get(13)?,
                    ends: r.get(14)?,
                    followup: r.get(15)?,
                    goal_held: r.get(16)?,
                    for_goal: r.get(17)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn routines(&self, chat: i64) -> Result<Vec<Routine>> {
        self.routines_where("chat_id = ?1", [chat])
    }

    /// A bot's repeating routines, in every chat (one-offs are reminders, not part of what it does).
    pub fn routines_of(&self, bot: i64) -> Result<Vec<Routine>> {
        self.routines_where("bot_id = ?1 AND once = 0", [bot])
    }

    /// A bot's routines that are due. Ones past their end (by more than a late run's grace) are removed first: they
    /// don't run late after it.
    pub fn due(&self, bot: i64, t: i64) -> Result<Vec<Routine>> {
        for r in self.routines_where("bot_id = ?1 AND ends > 0 AND ends + ?3 <= ?2", params![bot, t, MISSED_AFTER])? {
            self.routine_delete(r.chat_id, r.id)?;
        }
        // A paused or finished goal's routines wait with it.
        self.routines_where("bot_id = ?1 AND next_fire <= ?2 AND paused = 0 AND (goal_id IS NULL OR goal_id IN (SELECT id FROM goals WHERE status IN ('active','blocked')))", [bot, t])
    }

    /// Edit, pause or resume. The next run is worked out from now, so resuming doesn't fire missed slots.
    pub fn routine_update(&self, chat: i64, id: i64, cron: &str, prompt: &str, quiet: bool, paused: bool) -> Result<bool> {
        if prompt.trim().is_empty() {
            bail!("a routine needs a prompt");
        }
        // A one-off keeps its time (it has no schedule to work one out from).
        let once: bool = self.c().query_row("SELECT once FROM routines WHERE id=?1 AND chat_id=?2", [id, chat], |r| r.get(0)).optional()?.unwrap_or(false);
        if once {
            return Ok(self.c().execute("UPDATE routines SET prompt=?3, quiet=?4, paused=?5 WHERE id=?1 AND chat_id=?2", params![id, chat, prompt.trim(), quiet, paused])? > 0);
        }
        let next = next_fire(cron, now())?;
        self.calendar_private(chat, cron)?;
        let ends: i64 = self.c().query_row("SELECT ends FROM routines WHERE id=?1", [id], |r| r.get(0)).optional()?.unwrap_or(0);
        if !paused && ends > 0 && next < IDLE && next > ends {
            bail!("on that schedule it wouldn't run again before it ends; change or clear its end first");
        }
        Ok(self.c().execute(
            "UPDATE routines SET cron=?3, prompt=?4, quiet=?5, paused=?6, next_fire=?7 WHERE id=?1 AND chat_id=?2",
            params![id, chat, cron.trim(), prompt.trim(), quiet, paused, next],
        )? > 0)
    }

    /// Your calendar is private: a routine timed off it (each run is told the event) only in a chat with you alone.
    /// So is your mail, for a routine started by it.
    fn calendar_private(&self, chat: i64, cron: &str) -> Result<()> {
        if crate::calendar::trigger(cron)?.is_some() && !self.chat_private(chat) {
            bail!("a routine timed off your calendar can only be in a private chat");
        }
        if matches!(crate::events::on(cron)?, Some(crate::events::On::Mail(_))) && (!self.chat_private(chat) || self.chat_guest(chat)) {
            bail!("a routine started by your email can only be in your own private chat");
        }
        Ok(())
    }

    /// A bot's routines started by an event of this kind ("mail:%"), not paused.
    pub fn routines_on(&self, bot: i64, like: &str) -> Result<Vec<Routine>> {
        self.routines_where("bot_id = ?1 AND cron LIKE ?2 AND paused = 0", params![bot, like])
    }

    /// An event routine ends after its first run that worked ("tell me when X arrives"), like a one-off.
    pub fn routine_set_once(&self, id: i64) -> Result<()> {
        self.c().execute("UPDATE routines SET once=1 WHERE id=?1", [id])?;
        Ok(())
    }

    /// Something came for an event routine (a matching email, a webhook call): kept for its run, which is due now.
    pub fn routine_event_add(&self, id: i64, text: &str) -> Result<()> {
        let c = self.c();
        c.execute("INSERT INTO routine_events(routine_id, text, ts) VALUES (?1,?2,?3)", params![id, text, now()])?;
        // ponytail: the 20 newest; a flood of calls isn't worth more than that to one run
        c.execute("DELETE FROM routine_events WHERE routine_id=?1 AND id NOT IN (SELECT id FROM routine_events WHERE routine_id=?1 ORDER BY id DESC LIMIT 20)", [id])?;
        c.execute("UPDATE routines SET next_fire=min(next_fire, ?2) WHERE id=?1", [id, now()])?;
        Ok(())
    }

    /// What came for an event routine that no run has seen yet, oldest first: (id, text).
    pub fn routine_events(&self, id: i64) -> Result<Vec<(i64, String)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, text FROM routine_events WHERE routine_id=?1 ORDER BY id")?;
        let v = st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// A run has seen these (up to `upto`); anything that came during it waits for the next.
    pub fn routine_events_clear(&self, id: i64, upto: i64) -> Result<()> {
        self.c().execute("DELETE FROM routine_events WHERE routine_id=?1 AND id<=?2", [id, upto])?;
        Ok(())
    }

    /// A bot's calendar routines whose next run the scheduler may move: not due, paused, or being tried again.
    pub fn calendar_routines(&self, bot: i64, t: i64) -> Result<Vec<Routine>> {
        self.routines_where("bot_id = ?1 AND cron LIKE 'calendar:%' AND next_fire > ?2 AND paused = 0 AND retries = 0", [bot, t])
    }

    pub fn routine_set_next(&self, id: i64, t: i64) -> Result<()> {
        self.c().execute("UPDATE routines SET next_fire=?2 WHERE id=?1", [id, t])?;
        Ok(())
    }

    pub fn routine_set_delivery(&self, chat: i64, id: i64, delivery: &str) -> Result<bool> {
        if !["now", "digest", "log", "card"].contains(&delivery) {
            bail!("delivery is now, digest, log or card");
        }
        Ok(self.c().execute("UPDATE routines SET delivery=?3 WHERE id=?1 AND chat_id=?2", params![id, chat, delivery])? > 0)
    }

    /// When a repeating routine ends (unix seconds; 0 = never). It must still run at least once before then.
    pub fn routine_set_ends(&self, chat: i64, id: i64, ends: i64) -> Result<bool> {
        let Some(r) = self.routine(id)?.filter(|r| r.chat_id == chat) else { return Ok(false) };
        if ends != 0 {
            if r.once {
                bail!("a one-off has no end: it runs once");
            }
            if ends <= now() || (r.next_fire < IDLE && r.next_fire > ends) {
                bail!("it wouldn't run again before that end");
            }
        }
        Ok(self.c().execute("UPDATE routines SET ends=?3 WHERE id=?1 AND chat_id=?2", params![id, chat, ends])? > 0)
    }

    /// A one-off the bot booked on its own, to check back.
    pub fn routine_set_followup(&self, id: i64) -> Result<()> {
        self.c().execute("UPDATE routines SET followup=1 WHERE id=?1", [id])?;
        Ok(())
    }

    /// The person's "come through right away if …" ("" = never): one line, no angle brackets (it goes into the prompt).
    pub fn routine_set_urgent(&self, chat: i64, id: i64, rule: &str) -> Result<bool> {
        let rule = rule.split_whitespace().collect::<Vec<_>>().join(" ").replace(['<', '>'], "");
        if rule.chars().count() > 200 {
            bail!("keep it under 200 characters");
        }
        Ok(self.c().execute("UPDATE routines SET urgent=?3 WHERE id=?1 AND chat_id=?2", params![id, chat, rule])? > 0)
    }

    /// What a quiet check watches ("" for nothing, a page's address, or "mail"). A new target starts from a fresh look.
    pub fn routine_set_watch(&self, chat: i64, id: i64, watch: &str) -> Result<bool> {
        crate::watch::valid(watch)?;
        Ok(self.c().execute("UPDATE routines SET watch=?3, watch_seen=NULL, watch_noise='' WHERE id=?1 AND chat_id=?2 AND watch != ?3", params![id, chat, watch.trim()])? > 0)
    }

    /// How the watched page or mailbox looked at the last check that ran.
    pub fn routine_seen(&self, id: i64) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT watch_seen FROM routines WHERE id=?1", [id], |r| r.get(0)).optional()?.flatten())
    }

    /// Start noticing every line of the watched page again.
    pub fn routine_reset_noise(&self, chat: i64, id: i64) -> Result<bool> {
        Ok(self.c().execute("UPDATE routines SET watch_noise='' WHERE id=?1 AND chat_id=?2", [id, chat])? > 0)
    }

    /// Line shapes of the watched page that churn on their own.
    pub fn routine_noise(&self, id: i64) -> Result<Vec<String>> {
        let v: Option<String> = self.c().query_row("SELECT watch_noise FROM routines WHERE id=?1", [id], |r| r.get(0)).optional()?;
        Ok(v.unwrap_or_default().lines().filter(|l| !l.is_empty()).map(String::from).collect())
    }

    /// ponytail: the 100 newest shapes; a page with more churn than that is mostly noise anyway
    pub fn routine_add_noise(&self, id: i64, shapes: &[String]) -> Result<()> {
        let mut all = self.routine_noise(id)?;
        all.extend(shapes.iter().map(|s| s.replace('\n', " ")).filter(|s| !s.is_empty()));
        let keep = all.len().saturating_sub(100);
        self.c().execute("UPDATE routines SET watch_noise=?2 WHERE id=?1", params![id, all[keep..].join("\n")])?;
        Ok(())
    }

    pub fn routine_set_seen(&self, id: i64, seen: &str) -> Result<()> {
        self.c().execute("UPDATE routines SET watch_seen=?2 WHERE id=?1", params![id, seen])?;
        Ok(())
    }

    /// A watched quiet check that found nothing changed: a run log row with no AI run (and no tokens).
    pub fn run_unchanged(&self, r: &Routine) -> Result<()> {
        let t = now();
        self.c().execute(
            "INSERT INTO runs(bot_id, chat_id, trigger, routine_id, provider, status, started_at, ended_at, reply) VALUES (?1,?2,'routine',?3,'',?4,?5,?5,'')",
            params![r.bot_id, r.chat_id, r.id, UNCHANGED, t],
        )?;
        Ok(())
    }

    /// After a failure that may pass: run again at `at` (or at the next slot, if that's sooner).
    pub fn routine_retry(&self, r: &Routine, at: i64) -> Result<()> {
        self.c().execute("UPDATE routines SET next_fire=min(next_fire, ?2), retries=retries+1 WHERE id=?1", params![r.id, at])?;
        Ok(())
    }

    /// A routine's run is over and won't be tried again: a one-off is gone, and so is one whose next slot is past its
    /// end. When it worked, its earlier failures leave Needs you (it healed), and the count of tries starts over.
    pub fn routine_done(&self, r: &Routine, worked: bool) -> Result<()> {
        let c = self.c();
        // routine_advance already moved it on; a calendar or event routine waiting (IDLE) goes when its end is past.
        let ended = c.query_row("SELECT ends > 0 AND next_fire > ends AND next_fire < ?2 FROM routines WHERE id=?1", [r.id, IDLE], |x| x.get(0)).optional()?.unwrap_or(false);
        if r.once || ended {
            c.execute("DELETE FROM routines WHERE id=?1", [r.id])?;
            c.execute("DELETE FROM routine_events WHERE routine_id=?1", [r.id])?;
            c.execute("DELETE FROM settings WHERE key=?1", [format!("webhook:{}", r.id)])?;
        } else {
            c.execute("UPDATE routines SET retries=0 WHERE id=?1", [r.id])?;
        }
        if worked {
            c.execute("INSERT OR IGNORE INTO inbox_dismissed(kind, ref) SELECT 'run', id FROM runs WHERE routine_id=?1 AND status='error'", [r.id])?;
        }
        Ok(())
    }

    /// How many times a routine has run and said something: its first few ask what to trim.
    pub fn routine_replies(&self, routine: i64) -> Result<i64> {
        Ok(self.c().query_row("SELECT count(*) FROM runs WHERE routine_id=?1 AND trigger='routine' AND status='ok' AND coalesce(reply, '') != ''", [routine], |r| r.get(0))?)
    }

    /// A routine's last `n` results that said something, newest first: (when it ran, what it said, read outside content).
    pub fn routine_results(&self, routine: i64, n: usize) -> Result<Vec<(i64, String, bool)>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT started_at, reply, tainted FROM runs WHERE routine_id=?1 AND trigger='routine' AND status='ok' AND coalesce(reply, '') != '' ORDER BY id DESC LIMIT ?2",
        )?;
        let v = st.query_map(params![routine, n as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn routine(&self, id: i64) -> Result<Option<Routine>> {
        Ok(self.routines_where("id = ?1", [id])?.pop())
    }

    /// Run a routine at the scheduler's next pass (within half a minute).
    pub fn routine_run_soon(&self, id: i64) -> Result<()> {
        self.c().execute("UPDATE routines SET next_fire=?2 WHERE id=?1", params![id, now()])?;
        Ok(())
    }

    /// The routine a run was for.
    pub fn run_routine_id(&self, run: i64) -> Result<Option<i64>> {
        Ok(self.c().query_row("SELECT routine_id FROM runs WHERE id=?1", [run], |r| r.get(0)).optional()?.flatten())
    }

    pub fn routine_prompt(&self, id: i64) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT prompt FROM routines WHERE id=?1", [id], |r| r.get(0)).optional()?)
    }

    /// A routine's newest run.
    pub fn routine_last_run(&self, routine: i64) -> Result<Option<i64>> {
        Ok(self.c().query_row("SELECT max(id) FROM runs WHERE routine_id=?1 AND trigger='routine'", [routine], |r| r.get(0))?)
    }

    /// Where a routine's result went.
    pub fn run_delivered(&self, run: i64, how: &str) -> Result<()> {
        self.c().execute("UPDATE runs SET delivered=?2 WHERE id=?1", params![run, how])?;
        Ok(())
    }

    /// Let this run come through right away, unless URGENT_PER_DAY runs (all bots together) already did since
    /// `day_start`. One statement, so two bots finishing at once can't both take the last one.
    pub fn run_take_urgent(&self, run: i64, day_start: i64) -> Result<bool> {
        Ok(self.c().execute(
            "UPDATE runs SET urgent=1 WHERE id=?1 AND (SELECT count(*) FROM runs WHERE urgent=1 AND started_at >= ?2) < ?3",
            params![run, day_start, URGENT_PER_DAY],
        )? > 0)
    }

    pub fn run_urgent(&self, run: i64) -> Result<bool> {
        Ok(self.c().query_row("SELECT urgent FROM runs WHERE id=?1", [run], |r| r.get(0))?)
    }

    /// What happens next, after a failed run's error ("Trying again at 10:35.").
    pub fn run_note(&self, run: i64, note: &str) -> Result<()> {
        self.c().execute("UPDATE runs SET error = coalesce(error, '') || ' ' || ?2 WHERE id=?1", params![run, note])?;
        Ok(())
    }

    /// A routine that couldn't start (its AI account isn't set up): a failed run, with nothing spent.
    pub fn run_not_started(&self, r: &Routine, provider: &str, error: &str) -> Result<i64> {
        let id = self.run_start(r.bot_id, r.chat_id, "routine", Some(r.id), provider)?;
        self.run_finish(id, Err(error))?;
        Ok(id)
    }

    /// A bot's routines on hold ("hold:3"), or one routine's ("hold:routine:12"). An expired hold too, so a late run
    /// can say why; hold_clear removes it.
    pub fn hold(&self, key: &str) -> Result<Option<Hold>> {
        Ok(self.setting(key)?.and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub fn hold_set(&self, key: &str, h: &Hold) -> Result<()> {
        self.set_setting(key, &serde_json::to_string(h)?)
    }

    /// A usage limit: this bot's routines wait, and so do those of every bot signed in to the same AI (a Claude,
    /// ChatGPT or Grok login is one for all bots). Bots on an API key may be limited per model, so only this one.
    /// Returns the bots held.
    pub fn hold_account(&self, bot: i64, kind: &str, h: &Hold) -> Result<Vec<i64>> {
        let mut held = vec![bot];
        if ["claude", "codex", "grok"].contains(&kind) {
            held.extend(self.bots()?.into_iter().filter(|b| b.provider == kind && b.id != bot).map(|b| b.id));
        }
        for b in &held {
            self.hold_set(&format!("hold:{b}"), h)?;
        }
        Ok(held)
    }

    /// End a hold. `go_on`: the person said to run anyway, so a daily cap's lets the rest of today go over it, and the
    /// hold is left ended rather than gone (the scheduler removes it, and says why the routine ran late).
    pub fn hold_clear(&self, key: &str, go_on: bool) -> Result<()> {
        match self.hold(key)? {
            Some(mut h) if go_on => {
                if let Some(w) = &h.waive {
                    self.set_setting(w, &Local::now().format("%Y-%m-%d").to_string())?;
                }
                h.until = now() - 1;
                self.hold_set(key, &h)
            }
            _ => {
                self.c().execute("DELETE FROM settings WHERE key=?1", [key])?;
                Ok(())
            }
        }
    }

    /// Every hold that hasn't ended, for Needs you: (its key, the bot, the routine if it's one routine's, the hold).
    pub fn holds(&self, now: i64) -> Result<Vec<HoldRow>> {
        let v: Vec<(String, String)> = {
            let c = self.c();
            let mut st = c.prepare("SELECT key, value FROM settings WHERE key LIKE 'hold:%'")?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
        };
        let mut out = vec![];
        for (k, v) in v {
            let Ok(h) = serde_json::from_str::<Hold>(&v) else { continue };
            if h.until <= now {
                continue;
            }
            let rest = k.strip_prefix("hold:").unwrap_or_default();
            if let Some(id) = rest.strip_prefix("routine:").and_then(|i| i.parse().ok()) {
                let bot: Option<i64> = self.c().query_row("SELECT bot_id FROM routines WHERE id=?1", [id], |r| r.get(0)).optional()?;
                if let Some(bot) = bot {
                    out.push((k, bot, Some(id), h));
                }
            } else if let Ok(bot) = rest.parse() {
                out.push((k, bot, None, h));
            }
        }
        Ok(out)
    }

    /// The daily caps in tokens: each routine, each bot, all bots (None: no cap).
    pub fn caps(&self) -> Result<[Option<i64>; 3]> {
        let get = |k: &str| -> Result<Option<i64>> { Ok(self.setting(k)?.and_then(|v| v.parse().ok()).filter(|n: &i64| *n > 0)) };
        Ok([get(CAPS[0])?, get(CAPS[1])?, get(CAPS[2])?])
    }

    /// The first daily cap a run of this routine would go over, counting tokens since `day_start`: (the hold's key, the
    /// hold). None when it's under all of them, or the person let it go over today.
    pub fn over_cap(&self, r: &Routine, day_start: i64) -> Result<Option<(String, Hold)>> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let tokens = |sql: &str, p: i64| -> Result<i64> {
            Ok(self.c().query_row(&format!("SELECT coalesce(sum({WEIGHED}), 0) FROM runs WHERE started_at >= ?1 AND {sql}"), params![day_start, p], |r| r.get(0))?)
        };
        let [per_routine, per_bot, per_day] = self.caps()?;
        let k = |n: i64| if n >= 10_000 { format!("{}k", n / 1000) } else { n.to_string() };
        let checks = [
            (per_routine, format!("cap_waived:routine:{}", r.id), format!("hold:routine:{}", r.id), tokens("routine_id = ?2", r.id)?, "this routine"),
            (per_bot, format!("cap_waived:bot:{}", r.bot_id), format!("hold:{}", r.bot_id), tokens("bot_id = ?2", r.bot_id)?, "this bot"),
            (per_day, "cap_waived:day".to_string(), format!("hold:{}", r.bot_id), tokens("?2 = ?2", 0)?, "all your bots together"),
        ];
        for (cap, waive, key, used, who) in checks {
            let Some(cap) = cap else { continue };
            if used < cap || self.setting(&waive)?.as_deref() == Some(today.as_str()) {
                continue;
            }
            let why = format!("{who} reached the daily cap of {} tokens ({} used today)", k(cap), k(used));
            let until = (Local::now().date_naive() + chrono::Duration::days(1)).and_hms_opt(0, 0, 0).and_then(|t| Local.from_local_datetime(&t).earliest()).map_or(now() + 86400, |t| t.timestamp());
            return Ok(Some((key, Hold { until, why, waive: Some(waive) })));
        }
        Ok(None)
    }

    // ---------- daily digest ----------

    pub fn digest_add(&self, bot: i64, chat: i64, routine: i64, text: &str, tainted: bool) -> Result<()> {
        self.c().execute(
            "INSERT INTO digest_items(bot_id, chat_id, routine_id, text, ts, tainted) VALUES (?1,?2,?3,?4,?5,?6)",
            params![bot, chat, routine, text, now(), tainted],
        )?;
        Ok(())
    }

    /// A bot's unsent digest items, oldest first.
    pub fn digest_pending(&self, bot: i64) -> Result<Vec<DigestItem>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, chat_id, routine_id, text, tainted FROM digest_items WHERE bot_id=?1 AND sent_at IS NULL ORDER BY id")?;
        let v = st.query_map([bot], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn digest_mark_sent(&self, ids: &[i64]) -> Result<()> {
        let c = self.c();
        for id in ids {
            c.execute("UPDATE digest_items SET sent_at=?2 WHERE id=?1", params![id, now()])?;
        }
        Ok(())
    }

    /// Quiet checks in the digest lane that ran since `since` and found nothing.
    pub fn quiet_nothing_since(&self, bot: i64, chat: i64, since: i64) -> Result<i64> {
        Ok(self.c().query_row(
            "SELECT count(*) FROM runs r JOIN routines o ON o.id = r.routine_id
              WHERE r.bot_id=?1 AND r.chat_id=?2 AND r.trigger='routine' AND r.started_at > ?3 AND r.status IN ('ok','unchanged') AND coalesce(r.reply,'') = ''
                AND o.quiet = 1 AND o.delivery = 'digest'",
            params![bot, chat, since],
            |r| r.get(0),
        )?)
    }

    /// A routine that came due late gets one `missed` row in the run log, for the slots it didn't run at, so its history
    /// and Needs you show it. Then it runs once, now (the scheduler goes on as before). `away`: Mimir wasn't running at
    /// the time (it started after); otherwise the computer slept or a long run held the scheduler up.
    pub fn routine_missed(&self, r: &Routine, now: i64, why: &str) -> Result<Option<i64>> {
        if now - r.next_fire < MISSED_AFTER {
            return Ok(None);
        }
        let (mut slots, mut last) = (1, r.next_fire);
        // ponytail: walks the skipped slots one by one, capped; a 15-minute check off for a week is 672.
        while !r.once && slots < 1000 {
            match next_fire(&r.cron, last) {
                Ok(n) if n <= now - MISSED_AFTER => (slots, last) = (slots + 1, n),
                _ => break,
            }
        }
        let day = |t: i64| Local.timestamp_opt(t, 0).single().map(|d| d.date_naive());
        let at = |t: i64| match Local.timestamp_opt(t, 0).single() {
            Some(d) if day(t) == day(now) => d.format("%H:%M").to_string(),
            Some(d) => d.format("%a %-d %b %H:%M").to_string(),
            None => String::new(),
        };
        let what = if slots == 1 { format!("Didn't run at {}", at(r.next_fire)) } else { format!("Missed {slots} runs, {} to {}", at(r.next_fire), at(last)) };
        let c = self.c();
        c.execute(
            "INSERT INTO runs(bot_id, chat_id, trigger, routine_id, provider, status, error, started_at, ended_at)
             VALUES (?1, ?2, 'routine', ?3, '', 'missed', ?4, ?5, ?6)",
            params![r.bot_id, r.chat_id, r.id, format!("{what}: {why}. It ran at {} instead.", at(now)), r.next_fire, now],
        )?;
        Ok(Some(c.last_insert_rowid()))
    }

    /// Per routine in a chat: its last runs (newest first: ok | error | missed | running, and when) and its use over
    /// 30 days, so a routine that keeps failing, or quietly costs a lot, shows on the routine itself.
    pub fn routine_stats(&self, chat: i64, now: i64) -> Result<Vec<RoutineStats>> {
        let c = self.c();
        let mut st = c.prepare(
            &format!("SELECT routine_id, coalesce(sum(status NOT IN ('missed','unchanged')), 0), coalesce(sum({WEIGHED}), 0), sum(cost_usd), coalesce(sum(status='error'), 0), coalesce(sum(status='missed'), 0), coalesce(sum(status='unchanged'), 0)
               FROM runs WHERE chat_id=?1 AND routine_id IS NOT NULL AND trigger='routine' AND started_at >= ?2 GROUP BY routine_id"),
        )?;
        let mut v: Vec<RoutineStats> = st
            .query_map(params![chat, now - 30 * 86400], |r| {
                Ok(RoutineStats { id: r.get(0)?, runs: r.get(1)?, tokens: r.get(2)?, cost_usd: r.get(3)?, failed: r.get(4)?, missed: r.get(5)?, unchanged: r.get(6)?, ignored: vec![], last: vec![] })
            })?
            .collect::<Result<_, _>>()?;
        let mut last = c.prepare(&format!("SELECT status, started_at, {WEIGHED} FROM runs WHERE routine_id=?1 AND chat_id=?2 AND trigger='routine' ORDER BY started_at DESC, id DESC LIMIT 8"))?;
        for s in &mut v {
            s.last = last.query_map(params![s.id, chat], |r| Ok(RunMark { status: r.get(0)?, at: r.get(1)?, tokens: r.get(2)? }))?.collect::<Result<_, _>>()?;
        }
        // After the connection is let go (routine_noise takes it again).
        drop(last);
        drop(st);
        drop(c);
        for s in &mut v {
            s.ignored = self.routine_noise(s.id)?;
        }
        Ok(v)
    }

    pub fn routine_delete(&self, chat: i64, id: i64) -> Result<bool> {
        let gone = self.c().execute("DELETE FROM routines WHERE id=?1 AND chat_id=?2", params![id, chat])? > 0;
        if gone {
            self.c().execute("DELETE FROM settings WHERE key IN (?1, ?2, ?3)", [format!("hold:routine:{id}"), format!("webhook:{id}"), format!("webhook-seen:{id}")])?;
            self.c().execute("DELETE FROM routine_events WHERE routine_id=?1", [id])?;
        }
        Ok(gone)
    }

    /// Advance before running so a crash mid-run can't fire the same slot twice. A one-off stays until its run is done
    /// (routine_done), with an hour's grace: a crash mid-run tries it once more then.
    pub fn routine_advance(&self, r: &Routine) -> Result<()> {
        // One that ends after its first event has no time to try again at: it waits for its events, like any other.
        if r.once && r.cron.is_empty() {
            self.c().execute("UPDATE routines SET next_fire=?2 WHERE id=?1", params![r.id, now() + 3600])?;
            return Ok(());
        }
        let next = next_fire(&r.cron, now().max(r.next_fire))?;
        self.c().execute("UPDATE routines SET next_fire=?2 WHERE id=?1", params![r.id, next])?;
        Ok(())
    }

    // ---------- pause ----------

    /// The person paused all bots (None) or one, until a time or (0) until they resume. While paused nothing runs on
    /// its own, and messaging apps are told so.
    pub fn pause_set(&self, bot: Option<i64>, until: i64) -> Result<()> {
        self.set_setting(&pause_key(bot), &until.to_string())
    }

    /// End a pause (resumed, or its time came). Routines that came due meanwhile move on to their next time, as if
    /// they'd run: no rush of catch-up runs, and nothing shows as missed. A one-off (a reminder) still runs once.
    // ponytail: a slot due after a timed pause ended, while the computer slept, is skipped too, not run late
    pub fn pause_end(&self, bot: Option<i64>) -> Result<()> {
        let t = now();
        for r in self.routines_where("(?1 IS NULL OR bot_id = ?1) AND next_fire <= ?2 AND once = 0", params![bot, t])? {
            self.c().execute("UPDATE routines SET next_fire=?2, retries=0 WHERE id=?1", params![r.id, next_fire(&r.cron, t)?])?;
        }
        // Moved on first, then unpaused: a scheduler pass in between still sees the pause.
        self.c().execute("DELETE FROM settings WHERE key=?1", [pause_key(bot)])?;
        Ok(())
    }

    /// Pauses that haven't ended: (the bot, or None for all; until, 0 for until resumed).
    pub fn pauses(&self, now: i64) -> Result<Vec<(Option<i64>, i64)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT key, value FROM settings WHERE key LIKE 'pause:%'")?;
        let v: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v.into_iter()
            .filter_map(|(k, v)| Some((k.strip_prefix("pause:")?.parse().ok(), v.parse::<i64>().ok()?)))
            .filter(|&(_, until)| until == 0 || until > now)
            .collect())
    }

    /// Whether this bot is paused (its own pause or everyone's): Some(until), 0 for until resumed; the later end wins.
    /// A pause whose time has come is ended here.
    pub fn paused(&self, bot: i64, now: i64) -> Result<Option<i64>> {
        let mut out: Option<i64> = None;
        for who in [None, Some(bot)] {
            let Some(until) = self.setting(&pause_key(who))?.and_then(|v| v.parse::<i64>().ok()) else { continue };
            if until != 0 && until <= now {
                self.pause_end(who)?;
            } else {
                out = Some(match out {
                    Some(0) => 0,
                    Some(o) if until != 0 => o.max(until),
                    _ => until,
                });
            }
        }
        Ok(out)
    }
}


fn pause_key(bot: Option<i64>) -> String {
    bot.map_or("pause:all".into(), |b| format!("pause:{b}"))
}
