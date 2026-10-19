use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct Approval {
    pub id: i64,
    pub bot_id: i64,
    pub chat_id: i64,
    pub tool: String,
    pub summary: String,
    pub status: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub tg_message_id: Option<i64>,
    /// Why it asks: `rule` (your rules say ask), `guarded` (the conversation read untrusted content), or
    /// `login` (a saved login would be typed). Only `rule` asks can become an "always allow".
    pub kind: String,
    /// The website involved, for a per-site "always allow".
    pub site: Option<String>,
    /// Parked by an unattended run (a routine): the exact call is stored and runs when approved.
    pub parked: bool,
    /// How often you already allowed this same kind of request (tool and site): the card offers "always" after a few.
    pub allowed_before: i64,
    /// The card's text you can change before it goes (an email's subject and body): field → text. Only these fields;
    /// who it's to never is.
    pub draft: Option<serde_json::Value>,
    /// Why you said no, in your words (the bot is told).
    pub reason: Option<String>,
    /// Asked in a guest's chat: the guest answers there, so its chat-app card never offers Always (your rules).
    pub guest: bool,
    /// A link to this card in the app, for chat-app cards you can edit there. Not stored.
    #[serde(skip)]
    pub edit_link: Option<String>,
}

/// (tool, site, times allowed, the bot that asked last).
pub type AllowedOften = (String, Option<String>, i64, i64);

impl Db {
    // ---------- policy rules & approvals ----------

    /// User override for a tool, if any (None = use the built-in default).
    pub fn rule(&self, tool: &str) -> Result<Option<String>> {
        let a: Option<String> = self.c().query_row("SELECT action FROM policy_rules WHERE tool=?1", [tool], |r| r.get(0)).optional()?;
        self.with_always(a, &always_key(tool, None))
    }

    /// "always": an allow that the person marked to never ask, not even after outside content. Kept as "allow" plus a
    /// mark in the settings (no new schema), so an older Mimir reads it as a plain allow.
    fn with_always(&self, action: Option<String>, key: &str) -> Result<Option<String>> {
        Ok(match action {
            Some(a) if a == "allow" && self.setting(key)?.is_some() => Some("always".into()),
            a => a,
        })
    }

    /// action = always | allow | ask | deny, or "default" to remove the override.
    pub fn set_rule(&self, tool: &str, action: &str) -> Result<()> {
        self.set_always(&always_key(tool, None), action)?;
        let action = if action == "always" { "allow" } else { action };
        if action == "default" {
            self.c().execute("DELETE FROM policy_rules WHERE tool=?1", [tool])?;
        } else {
            self.c().execute(
                "INSERT INTO policy_rules(tool, action) VALUES (?1, ?2) ON CONFLICT(tool) DO UPDATE SET action = excluded.action",
                params![tool, action],
            )?;
        }
        Ok(())
    }

    /// A rule for one tool on one website (e.g. open github.com in the browser), checked before the tool's rule.
    pub fn site_rule(&self, tool: &str, host: &str) -> Result<Option<String>> {
        let a: Option<String> = self.c().query_row("SELECT action FROM site_rules WHERE tool=?1 AND host=?2", [tool, host], |r| r.get(0)).optional()?;
        self.with_always(a, &always_key(tool, Some(host)))
    }

    fn set_always(&self, key: &str, action: &str) -> Result<()> {
        if action == "always" {
            self.set_setting(key, "1")
        } else {
            self.setting_take(key).map(|_| ())
        }
    }

    /// action = always | allow | ask | deny, or "default" to remove it.
    pub fn set_site_rule(&self, tool: &str, host: &str, action: &str) -> Result<()> {
        self.set_always(&always_key(tool, Some(host)), action)?;
        let action = if action == "always" { "allow" } else { action };
        if action == "default" {
            self.c().execute("DELETE FROM site_rules WHERE tool=?1 AND host=?2", [tool, host])?;
        } else {
            self.c().execute(
                "INSERT INTO site_rules(tool, host, action) VALUES (?1,?2,?3) ON CONFLICT(tool, host) DO UPDATE SET action = excluded.action",
                params![tool, host, action],
            )?;
        }
        Ok(())
    }

    pub fn site_rules(&self) -> Result<Vec<(String, String, String)>> {
        let v: Vec<(String, String, String)> = {
            let c = self.c();
            let mut st = c.prepare("SELECT tool, host, action FROM site_rules ORDER BY host, tool")?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?
        };
        v.into_iter()
            .map(|(t, h, a)| {
                let a = self.with_always(Some(a), &always_key(&t, Some(&h)))?.unwrap_or_default();
                Ok((t, h, a))
            })
            .collect()
    }

    /// A new incognito chat starts clean: the last one's "read a web page" flag goes.
    pub fn incognito_reset(&self, bot: i64) -> Result<()> {
        self.set_session(bot, crate::INCOGNITO_CHAT, None)?;
        self.incognito_forget_approvals(bot)
    }

    /// An incognito turn's approvals (what it wanted to do) are deleted as soon as the turn ends.
    pub fn incognito_forget_approvals(&self, bot: i64) -> Result<()> {
        self.c().execute("DELETE FROM approvals WHERE bot_id=?1 AND chat_id=?2", params![bot, crate::INCOGNITO_CHAT])?;
        Ok(())
    }

    pub fn approval_set_run(&self, id: i64, run: i64) -> Result<()> {
        self.c().execute("UPDATE approvals SET run_id=?2 WHERE id=?1", [id, run])?;
        Ok(())
    }

    /// The routine whose run asked (a parked call's).
    pub fn approval_routine(&self, id: i64) -> Result<Option<i64>> {
        Ok(self.c().query_row("SELECT r.routine_id FROM approvals a JOIN runs r ON r.id = a.run_id WHERE a.id=?1", [id], |r| r.get(0)).optional()?.flatten())
    }

    /// "Allow all like this": the approval's tool (and site) needs no more OKs for the rest of its run. Call it
    /// before approving, so the plan is in place when the waiting tool goes on. Only for a pending plain ask from a
    /// run that's still going, in `chat` if given; returns false otherwise (and nothing changes).
    pub fn approval_allow_rest_of_run(&self, id: i64, chat: Option<i64>) -> Result<bool> {
        let row: Option<(i64, String, Option<String>)> = self
            .c()
            .query_row(
                "SELECT a.run_id, a.tool, a.site FROM approvals a JOIN runs r ON r.id = a.run_id
                 WHERE a.id=?1 AND a.status='pending' AND a.kind='rule' AND a.parked_args IS NULL AND r.status='running'
                   AND (?2 IS NULL OR a.chat_id=?2)",
                params![id, chat],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((run, tool, site)) = row else { return Ok(false) };
        let key = format!("plan:{run}");
        let mut plan: Vec<(String, Option<String>)> = self.setting(&key)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        if !plan.contains(&(tool.clone(), site.clone())) {
            plan.push((tool, site));
        }
        self.set_setting(&key, &serde_json::to_string(&plan)?)?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn approval_create(&self, bot: i64, chat: i64, tool: &str, summary: &str, ttl_secs: i64, kind: &str, site: Option<&str>) -> Result<i64> {
        let id = {
            let c = self.c();
            let t = now();
            c.execute(
                "INSERT INTO approvals(bot_id, chat_id, tool, summary, created_at, expires_at, kind, site) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![bot, chat, tool, summary, t, t + ttl_secs, kind, site],
            )?;
            c.last_insert_rowid()
        };
        // Every ask goes through here (a rule, a sign-in, taking over the computer): Webhooks out hears of each.
        crate::hooks::fire(self, bot, chat, crate::hooks::Event::Approval { id, tool, summary });
        Ok(id)
    }

    fn expire_stale(c: &Connection) -> Result<()> {
        c.execute("UPDATE approvals SET status='expired' WHERE status='pending' AND expires_at <= ?1", [now()])?;
        Ok(())
    }

    /// Current status; flips overdue pending approvals to 'expired' first.
    pub fn approval_status(&self, id: i64) -> Result<String> {
        let c = self.c();
        Self::expire_stale(&c)?;
        Ok(c.query_row("SELECT status FROM approvals WHERE id=?1", [id], |r| r.get(0))?)
    }

    /// Decide a still-pending approval. `chat` (when given) must match, so one chat can't approve another's request.
    /// A sign-in card can only be turned down here: it's done by signing in (`approval_claim`).
    pub fn approval_decide(&self, id: i64, approve: bool, chat: Option<i64>) -> Result<Option<Approval>> {
        let c = self.c();
        Self::expire_stale(&c)?;
        let n = c.execute(
            "UPDATE approvals SET status=?2 WHERE id=?1 AND status='pending' AND (?3 IS NULL OR chat_id=?3) AND kind != 'signing' AND (?2 = 'denied' OR kind != 'signin')",
            params![id, if approve { "approved" } else { "denied" }, chat],
        )?;
        if n == 0 {
            return Ok(None);
        }
        drop(c);
        Ok(self.approvals_where("id = ?1", id)?.pop())
    }

    /// Take a waiting sign-in card for its form, once: the first submit gets it (kind `signing` while Mimir types,
    /// with two minutes for that), any later one, or one after it ran out, gets None.
    pub fn approval_claim(&self, id: i64) -> Result<Option<Approval>> {
        let c = self.c();
        Self::expire_stale(&c)?;
        let n = c.execute("UPDATE approvals SET kind='signing', expires_at=?2 WHERE id=?1 AND status='pending' AND kind='signin'", [id, now() + 120])?;
        drop(c);
        Ok(if n == 1 { self.approvals_where("id = ?1", id)?.pop() } else { None })
    }

    /// How a claimed sign-in went: approved = signed in.
    pub fn approval_signed_in(&self, id: i64, ok: bool) -> Result<()> {
        let c = self.c();
        c.execute("UPDATE approvals SET status=?2, kind='signin' WHERE id=?1 AND status='pending' AND kind='signing'", params![id, if ok { "approved" } else { "denied" }])?;
        c.execute("DELETE FROM settings WHERE key=?1", [format!("signin:{id}")])?;
        Ok(())
    }

    /// What a sign-in card's form shows besides the site (the full address, warnings), kept while it waits. Ones of
    /// cards no longer waiting go when a new one is made.
    pub fn signin_card_set(&self, id: i64, card: &serde_json::Value) -> Result<()> {
        self.c().execute(
            "DELETE FROM settings WHERE key LIKE 'signin:%' AND CAST(substr(key, 8) AS INTEGER) NOT IN (SELECT id FROM approvals WHERE status='pending')",
            [],
        )?;
        self.set_setting(&format!("signin:{id}"), &card.to_string())
    }

    pub fn signin_card(&self, id: i64) -> Result<serde_json::Value> {
        Ok(self.setting(&format!("signin:{id}"))?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
    }

    /// Sites someone signed in to with a sign-in card before.
    pub fn signin_sites(&self) -> Result<Vec<String>> {
        let c = self.c();
        let mut st = c.prepare("SELECT DISTINCT site FROM approvals WHERE kind='signin' AND status='approved' AND site IS NOT NULL")?;
        let v = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    fn approvals_where(&self, sql: &str, p: i64) -> Result<Vec<Approval>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT id, chat_id, tool, summary, status, created_at, expires_at, tg_message_id, bot_id, kind, site, parked_args IS NOT NULL,
             (SELECT COUNT(*) FROM approvals p WHERE p.tool = approvals.tool AND p.site IS approvals.site AND p.kind = 'rule' AND p.status = 'approved' AND p.id < approvals.id),
             draft, reason, coalesce((SELECT guest FROM chats WHERE chats.chat_id = approvals.chat_id AND approvals.chat_id != 0), 0)
             FROM approvals WHERE {sql} ORDER BY id"
        ))?;
        let v = st
            .query_map([p], |r| {
                Ok(Approval {
                    id: r.get(0)?,
                    chat_id: r.get(1)?,
                    tool: r.get(2)?,
                    summary: r.get(3)?,
                    status: r.get(4)?,
                    created_at: r.get(5)?,
                    expires_at: r.get(6)?,
                    tg_message_id: r.get(7)?,
                    bot_id: r.get(8)?,
                    kind: r.get(9)?,
                    site: r.get(10)?,
                    parked: r.get(11)?,
                    allowed_before: r.get(12)?,
                    draft: r.get::<_, Option<String>>(13)?.and_then(|d| serde_json::from_str(&d).ok()),
                    reason: r.get(14)?,
                    guest: r.get(15)?,
                    edit_link: None,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn approval(&self, id: i64) -> Result<Option<Approval>> {
        Ok(self.approvals_where("id = ?1", id)?.pop())
    }

    /// What a card you can edit holds, as it's made.
    pub fn approval_set_draft(&self, id: i64, draft: &serde_json::Value) -> Result<()> {
        self.c().execute("UPDATE approvals SET draft=?2 WHERE id=?1", params![id, draft.to_string()])?;
        Ok(())
    }

    /// Your edits, while it's still pending, and the card's text with them. False if it isn't pending any more.
    pub fn approval_edit(&self, id: i64, draft: &serde_json::Value, summary: &str) -> Result<bool> {
        let c = self.c();
        Self::expire_stale(&c)?;
        Ok(c.execute("UPDATE approvals SET draft=?2, summary=?3 WHERE id=?1 AND status='pending' AND draft IS NOT NULL", params![id, draft.to_string(), summary])? > 0)
    }

    /// Why you said no (before denying it).
    pub fn approval_set_reason(&self, id: i64, reason: &str) -> Result<()> {
        self.c().execute("UPDATE approvals SET reason=?2 WHERE id=?1 AND status='pending'", params![id, reason])?;
        Ok(())
    }

    /// The pending ask (not a take-over) whose card in `chat` is the message `card`, for a reply to it (the newest if
    /// several). Discord's cards are kept as "channel/message"; a Slack card posted in a thread as "thread/card", so a
    /// reply in that thread (`card` = the thread) finds it.
    pub fn approval_by_card(&self, chat: i64, card: &str) -> Result<Option<i64>> {
        let c = self.c();
        Self::expire_stale(&c)?;
        Ok(c.query_row(
            "SELECT id FROM approvals WHERE status='pending' AND kind != 'takeover' AND (
               chat_id=?1 AND (card_ref=?2 OR card_ref LIKE '%/' || ?2 OR card_ref LIKE ?2 || '/%')
               OR mirror_chat=?1 AND (mirror_ref=?2 OR mirror_ref LIKE '%/' || ?2 OR mirror_ref LIKE ?2 || '/%'))
             ORDER BY id DESC LIMIT 1",
            params![chat, card],
            |r| r.get(0),
        )
        .optional()?)
    }

    /// Park an approval for an unattended run: the call's arguments are kept so it can run once approved.
    /// Keep a call to run once approved, and whether the run that asked had read outside content.
    pub fn approval_park(&self, id: i64, args: &str, tainted: bool) -> Result<()> {
        self.c().execute("UPDATE approvals SET parked_args=?2, tainted=?3 WHERE id=?1", params![id, args, tainted])?;
        Ok(())
    }

    /// A parked call's arguments, and whether the run that asked had read outside content.
    pub fn approval_args(&self, id: i64) -> Result<Option<(String, bool)>> {
        Ok(self
            .c()
            .query_row("SELECT parked_args, tainted FROM approvals WHERE id=?1", [id], |r| Ok(r.get::<_, Option<String>>(0)?.map(|a| (a, r.get::<_, bool>(1).unwrap_or(true)))))
            .optional()?
            .flatten())
    }

    /// Approvals that ran out unanswered since `since` (not yet dismissed from the inbox).
    pub fn approvals_missed(&self, since: i64) -> Result<Vec<Approval>> {
        Self::expire_stale(&self.c())?;
        self.approvals_where("status = 'expired' AND chat_id != -1 AND created_at > ?1 AND id NOT IN (SELECT ref FROM inbox_dismissed WHERE kind='approval')", since)
    }

    /// Failed runs since `since` (not yet dismissed from the inbox), newest first: one item per routine and cause (a
    /// routine failing every 15 minutes is one item, with how often), and results that couldn't be sent.
    pub fn runs_failed(&self, since: i64) -> Result<Vec<Run>> {
        let gone: std::collections::HashSet<i64> = {
            let c = self.c();
            let mut st = c.prepare("SELECT ref FROM inbox_dismissed WHERE kind='run'")?;
            st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        let failed = |r: &Run| r.status == "missed" || r.status == "error" && r.error.as_deref() != Some("Stopped by you") || r.delivered.as_deref().is_some_and(|d| d.starts_with(NOT_SENT));
        let key = |r: &Run| match r.routine_id {
            Some(id) if r.status == "error" => format!("{id}:{:?}", crate::cause(r.error.as_deref().unwrap_or(""))),
            Some(id) if r.status == "missed" => format!("{id}:missed"),
            Some(id) => format!("{id}:delivery"),
            None => format!("run:{}", r.id),
        };
        let mut out: Vec<Run> = vec![];
        let mut seen: std::collections::HashMap<String, usize> = Default::default();
        // Newest first: the first of each kind stands for the rest. Dismissing it dismisses them all (a new one shows again).
        // A rehearsal's failure was seen where it was run.
        for r in self.runs(None, 300)?.into_iter().filter(|r| r.started_at > since && r.trigger != "rehearsal" && failed(r)) {
            match seen.get(&key(&r)) {
                Some(&i) => out[i].repeats += i64::from(!gone.contains(&r.id)),
                None => {
                    seen.insert(key(&r), out.len());
                    out.push(r);
                }
            }
        }
        Ok(out.into_iter().filter(|r| !gone.contains(&r.id)).collect())
    }

    /// Pending memory suggestions per chat.
    /// The last `n` weeks up to `now`, oldest first.
    pub fn weeks(&self, chat: i64, n: i64, now: i64) -> Result<Vec<Week>> {
        let c = self.c();
        let scope = format!("chat:{chat}");
        let count = |sql: &str, p: &dyn rusqlite::ToSql, from: i64, to: i64| -> Result<i64> { Ok(c.query_row(sql, params![p, from, to], |r| r.get(0))?) };
        let signals = "SELECT count(*) FROM signals WHERE chat_id=?1 AND ts>=?2 AND ts<?3 AND kind=";
        let sugg = "SELECT count(*) FROM suggestions WHERE scope=?1 AND created_at>=?2 AND created_at<?3 AND status=";
        (0..n)
            .rev()
            .map(|i| {
                let (from, to) = (now - (i + 1) * 7 * 86400, now - i * 7 * 86400);
                Ok(Week {
                    start: from,
                    turns: count("SELECT count(*) FROM messages WHERE chat_id=?1 AND ts>=?2 AND ts<?3 AND role='user'", &chat, from, to)?,
                    corrections: count(&format!("{signals}'correction'"), &chat, from, to)?,
                    stops: count(&format!("{signals}'stopped'"), &chat, from, to)?,
                    denials: count(&format!("{signals}'denied'"), &chat, from, to)?,
                    tool_calls: count("SELECT count(*) FROM tool_calls WHERE chat_id=?1 AND ts>=?2 AND ts<?3", &chat, from, to)?,
                    tool_errors: count("SELECT count(*) FROM tool_calls WHERE chat_id=?1 AND ts>=?2 AND ts<?3 AND ok=0", &chat, from, to)?,
                    accepted: count(&format!("{sugg}'accepted'"), &scope, from, to)?,
                    dismissed: count(&format!("{sugg}'dismissed'"), &scope, from, to)?,
                    repeats: count(&format!("{signals}'repeated'"), &chat, from, to)?,
                    undone: count("SELECT count(*) FROM entries WHERE scope=?1 AND undone_at>=?2 AND undone_at<?3 AND origin IN ('bot','tidy','import')", &scope, from, to)?,
                    forgot: count(
                        &format!("{signals}'correction' AND lower(detail) LIKE '%they replied: %told you%'"),
                        &chat,
                        from,
                        to,
                    )?,
                })
            })
            .collect()
    }

    /// Memories saved after reading outside content and not confirmed yet, per chat: the bots don't use them until then.
    pub fn unconfirmed_counts(&self) -> Result<Vec<(i64, i64)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT scope, count(*) FROM entries WHERE tainted=1 AND invalid=0 AND superseded_by IS NULL GROUP BY scope")?;
        let v: Vec<(String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v.into_iter().filter_map(|(s, n)| Some((s.strip_prefix("chat:")?.parse().ok()?, n))).collect())
    }

    pub fn suggestion_counts(&self) -> Result<Vec<(i64, i64)>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT scope, count(*) FROM (SELECT scope FROM suggestions WHERE status='pending' UNION ALL SELECT scope FROM skills WHERE status='proposed') GROUP BY scope",
        )?;
        let v: Vec<(String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v.into_iter().filter_map(|(s, n)| Some((s.strip_prefix("chat:")?.parse().ok()?, n))).collect())
    }

    pub fn inbox_dismiss(&self, kind: &str, id: i64) -> Result<()> {
        self.c().execute("INSERT OR IGNORE INTO inbox_dismissed(kind, ref) VALUES (?1,?2)", params![kind, id])?;
        Ok(())
    }

    /// Tool (and site) pairs whose plain asks were allowed at least `min` times since `since` and never denied then (any
    /// card for the pair, after outside content too). Only `rule` asks
    /// count, so never one after outside content, an email, a new bot or a saved login (those have their own kinds).
    pub fn approvals_allowed_often(&self, since: i64, min: i64) -> Result<Vec<AllowedOften>> {
        let c = self.c();
        // SQLite fills the bare bot_id from the row max(id) picked: the newest card's bot.
        let mut st = c.prepare(
            "SELECT tool, site, sum(kind = 'rule' AND status = 'approved') AS n, bot_id, max(id) FROM approvals WHERE created_at > ?1
             GROUP BY tool, site HAVING n >= ?2 AND sum(status = 'denied') = 0 ORDER BY n DESC",
        )?;
        let v = st.query_map(params![since, min], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn approvals_pending(&self) -> Result<Vec<Approval>> {
        Self::expire_stale(&self.c())?;
        self.approvals_where("status = 'pending' AND kind != 'signing' AND ?1 = ?1", 0)
    }

    /// This bot's pending Telegram approvals that haven't been sent as a card yet.
    pub fn approvals_to_notify(&self, bot: i64) -> Result<Vec<Approval>> {
        self.approvals_where("status = 'pending' AND notified = 0 AND chat_id NOT IN (0, -1) AND bot_id = ?1", bot)
    }

    /// The card was sent (`card_ref`: its message, if the channel said).
    pub fn approval_mark_notified(&self, id: i64, card_ref: Option<&str>) -> Result<()> {
        self.c().execute("UPDATE approvals SET notified=1, card_ref=?2 WHERE id=?1", params![id, card_ref])?;
        Ok(())
    }

    /// This bot's cards whose approval was decided elsewhere (app, tray, CLI) or ran out: (approval, card message).
    pub fn approvals_to_close(&self, bot: i64) -> Result<Vec<(Approval, String)>> {
        Self::expire_stale(&self.c())?;
        let list = self.approvals_where("status != 'pending' AND notified = 1 AND card_ref IS NOT NULL AND bot_id = ?1", bot)?;
        let c = self.c();
        let mut out = vec![];
        for a in list {
            let card: String = c.query_row("SELECT card_ref FROM approvals WHERE id=?1", [a.id], |r| r.get(0))?;
            out.push((a, card));
        }
        Ok(out)
    }

    /// The card now shows the outcome; don't edit it again.
    pub fn approval_mark_closed(&self, id: i64) -> Result<()> {
        self.c().execute("UPDATE approvals SET notified=2 WHERE id=?1", [id])?;
        Ok(())
    }

    /// This bot's pending routine asks from other chats (not incognito) not yet sent to the decisions chat `to`.
    pub fn approvals_to_mirror(&self, bot: i64, to: i64) -> Result<Vec<Approval>> {
        self.approvals_where(&format!("status = 'pending' AND parked_args IS NOT NULL AND mirror_chat IS NULL AND chat_id NOT IN (-1, {to}) AND bot_id = ?1"), bot)
    }

    /// The ask was sent to the decisions chat `chat` (`card_ref`: its message, if the channel said).
    pub fn approval_mark_mirrored(&self, id: i64, chat: i64, card_ref: Option<&str>) -> Result<()> {
        self.c().execute("UPDATE approvals SET mirror_chat=?2, mirror_ref=?3 WHERE id=?1", params![id, chat, card_ref])?;
        Ok(())
    }

    /// The decisions chat this ask was also sent to, if any.
    pub fn approval_mirror(&self, id: i64) -> Result<Option<i64>> {
        Ok(self.c().query_row("SELECT mirror_chat FROM approvals WHERE id=?1", [id], |r| r.get(0)).optional()?.flatten())
    }

    /// This bot's decisions-chat cards whose ask was answered elsewhere or ran out: (approval, its chat, card message).
    pub fn mirrors_to_close(&self, bot: i64) -> Result<Vec<(Approval, i64, String)>> {
        Self::expire_stale(&self.c())?;
        let list = self.approvals_where("status != 'pending' AND mirror_ref IS NOT NULL AND bot_id = ?1", bot)?;
        let c = self.c();
        let mut out = vec![];
        for a in list {
            let (chat, card) = c.query_row("SELECT mirror_chat, mirror_ref FROM approvals WHERE id=?1", [a.id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            out.push((a, chat, card));
        }
        Ok(out)
    }

    /// The decisions-chat card now shows the outcome; don't edit it again.
    pub fn mirror_mark_closed(&self, id: i64) -> Result<()> {
        self.c().execute("UPDATE approvals SET mirror_ref=NULL WHERE id=?1", [id])?;
        Ok(())
    }
}

/// The settings mark that makes a tool's (or a tool-on-a-site's) allow an "always".
fn always_key(tool: &str, host: Option<&str>) -> String {
    match host {
        Some(h) => format!("always:{tool}@{h}"),
        None => format!("always:{tool}"),
    }
}
