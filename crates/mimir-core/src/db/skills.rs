use super::*;

/// A procedure distilled from a run that worked. The tidy-up proposes it; bots use it only once you accept it.
/// Each belongs to one bot in one chat (skills aren't shared between bots).
#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    pub id: i64,
    pub bot_id: i64,
    pub name: String,
    /// When to use it: the only part that sits in the bot's prompt.
    pub description: String,
    /// The steps, read on demand with skill_view.
    pub body: String,
    /// The run it was learned from.
    pub run_id: Option<i64>,
    /// proposed | active | dismissed | removed
    pub status: String,
    pub created_at: i64,
    /// What a proposal does: learned (new, from a run that went well), patch (a fix after trouble), merge, retire.
    pub kind: String,
    /// Why it's proposed (the evidence), shown on the card.
    pub reason: String,
    /// Skills it takes out of use when accepted (merge, retire).
    pub replaces: Vec<i64>,
    /// How many runs read it, and when last.
    pub uses: i64,
    pub last_used: Option<i64>,
}

/// A run that used a skill and then went wrong: what was asked, what it did, and what went wrong.
pub struct SkillTrouble {
    pub skill: Skill,
    /// The run had read outside content (its evidence may carry a page's text).
    pub tainted: bool,
    pub run_id: i64,
    pub asked: String,
    pub steps: Vec<String>,
    pub trouble: String,
}

/// A run that could teach a skill: what was asked and the actions it took.
pub struct GoodRun {
    pub id: i64,
    pub bot_id: i64,
    pub asked: String,
    pub calls: Vec<ToolCall>,
}

/// A chat and its skills (name, description), for a messaging app's "/" menu.
pub type SkillMenu = (i64, Vec<(String, String)>);

impl Db {
    // ---------- skills ----------

    /// Runs since `since` that worked cleanly enough to learn from: finished, never read outside content, took at
    /// least two actions besides memory, none failed or was denied, and the person's next message wasn't a correction.
    pub fn good_runs(&self, chat: i64, since: i64) -> Result<Vec<GoodRun>> {
        let ids: Vec<(i64, i64, Option<String>, Option<String>)> = {
            let c = self.c();
            let mut st = c.prepare(
                "SELECT r.id, r.bot_id, r.asked,
                   (SELECT u.content FROM messages u WHERE u.chat_id=r.chat_id AND u.bot_id=r.bot_id AND u.role='user'
                      AND u.id > (SELECT max(a.id) FROM messages a WHERE a.run_id=r.id) ORDER BY u.id LIMIT 1)
                 FROM runs r WHERE r.chat_id=?1 AND r.started_at>?2 AND r.status='ok' AND r.tainted=0 AND r.trigger IN ('message','routine')
                   AND NOT EXISTS (SELECT 1 FROM skills s WHERE s.run_id=r.id)
                 ORDER BY r.id DESC LIMIT 50",
            )?;
            st.query_map(params![chat, since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?
        };
        let mut out = vec![];
        for (id, bot_id, asked, next) in ids {
            let calls = self.tool_calls(id)?;
            let actions = calls.iter().filter(|c| !c.tool.starts_with("memory_")).count();
            let clean = calls.iter().all(|c| c.ok && c.decision != "deny");
            if actions >= 2 && clean && !next.as_deref().is_some_and(crate::is_correction) {
                out.push(GoodRun { id, bot_id, asked: asked.unwrap_or_default(), calls });
            }
        }
        Ok(out.into_iter().take(10).collect())
    }

    /// Queue a proposed skill; one with the name of an accepted skill is a new version of it (the old one stays in use
    /// until you accept). Skipped (None) when one by that name is already waiting, the bot already has these exact
    /// steps, you turned one down by that name in the last 90 days, or that run already taught one.
    pub fn skill_propose(&self, scope: &str, bot: i64, name: &str, description: &str, body: &str, run: Option<i64>) -> Result<Option<i64>> {
        self.skill_insert(scope, bot, name, description, body, run, "learned", "", &[])
    }

    #[allow(clippy::too_many_arguments)]
    fn skill_insert(&self, scope: &str, bot: i64, name: &str, description: &str, body: &str, run: Option<i64>, kind: &str, reason: &str, replaces: &[i64]) -> Result<Option<i64>> {
        let (name, description, body) = (name.trim(), description.trim(), body.trim());
        if name.is_empty() || description.is_empty() || body.is_empty() {
            bail!("a skill needs a name, a description and steps");
        }
        let c = self.c();
        // A skill with the name of one shared with all bots is a new version of that one.
        let shared = c.prepare("SELECT 1 FROM skills WHERE scope=?1 AND bot_id=0 AND status='active' AND lower(name)=lower(?2)")?.exists(params![scope, name])?;
        let bot = if shared { 0 } else { bot };
        let dup = c
            .prepare(
                "SELECT 1 FROM skills WHERE scope=?1 AND kind != 'retire' AND ((bot_id=?2 AND lower(name)=lower(?3) AND (status='proposed'
                   OR (status='active' AND body=?6) OR (status='dismissed' AND created_at > ?5 AND kind='learned' AND ?7='learned')))
                   OR (?4 IS NOT NULL AND run_id=?4))",
            )?
            .exists(params![scope, bot, name, run, now() - 90 * 86400, body, kind])?;
        if dup {
            return Ok(None);
        }
        // A new skill whose "when to use" reads like one the bot already has: two skills for one job make a bot pick
        // the wrong one, so the card says so (merges and new versions are about that already).
        let similar: Option<String> = if replaces.is_empty() {
            let mut st = c.prepare("SELECT name, description FROM skills WHERE scope=?1 AND bot_id IN (?2, 0) AND status='active' AND lower(name) != lower(?3)")?;
            let rows: Vec<(String, String)> = st.query_map(params![scope, bot, name], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            rows.into_iter().find(|(_, d)| alike(description, d)).map(|(n, _)| n)
        } else {
            None
        };
        let reason = match similar {
            Some(n) => format!("{} Similar to your skill “{n}”: if they do the same job, keep one.", reason.trim()),
            None => reason.to_string(),
        };
        let replaces = replaces.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        c.execute(
            "INSERT INTO skills(scope, bot_id, name, description, body, run_id, created_at, kind, reason, replaces) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![scope, bot, name, description, body, run, now(), kind, reason.trim(), replaces],
        )?;
        Ok(Some(c.last_insert_rowid()))
    }

    fn skills_where(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Skill>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT id, bot_id, name, description, body, run_id, status, created_at, kind, reason, replaces,
               (SELECT count(*) FROM skill_uses u WHERE u.skill_id = skills.id), (SELECT max(ts) FROM skill_uses u WHERE u.skill_id = skills.id)
             FROM skills WHERE {sql} ORDER BY id"
        ))?;
        let v = st
            .query_map(p, |r| {
                let replaces: String = r.get(10)?;
                Ok(Skill {
                    id: r.get(0)?,
                    bot_id: r.get(1)?,
                    name: r.get(2)?,
                    description: r.get(3)?,
                    body: r.get(4)?,
                    run_id: r.get(5)?,
                    status: r.get(6)?,
                    created_at: r.get(7)?,
                    kind: r.get(8)?,
                    reason: r.get(9)?,
                    replaces: replaces.split(',').filter_map(|x| x.trim().parse().ok()).collect(),
                    uses: r.get(11)?,
                    last_used: r.get(12)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Proposed and accepted skills in a chat (the Memory tab).
    pub fn skills(&self, scope: &str) -> Result<Vec<Skill>> {
        self.skills_where("scope=?1 AND status IN ('proposed','active')", [scope])
    }

    /// The accepted skills a bot may use, in every chat: its own and those shared with all bots.
    pub fn skills_of(&self, bot: i64) -> Result<Vec<Skill>> {
        self.skills_where("bot_id IN (?1, 0) AND status='active'", [bot])
    }

    /// The skills one bot may use here: its own, and those shared with all bots (bot_id 0).
    pub fn skills_active(&self, scope: &str, bot: i64) -> Result<Vec<Skill>> {
        self.skills_where("scope=?1 AND bot_id IN (?2, 0) AND status='active'", params![scope, bot])
    }

    /// Give an accepted skill to one bot, or to all bots in this chat (`bot` 0). Its name must stay unique for the bots
    /// that would see it.
    pub fn skill_share(&self, scope: &str, id: i64, bot: i64) -> Result<bool> {
        let k = self.skills_where("id=?1 AND scope=?2 AND status='active'", params![id, scope])?.pop().context("no such skill")?;
        let clash = self
            .skills_where("scope=?1 AND status='active' AND id != ?2 AND lower(name)=lower(?3)", params![scope, id, k.name])?
            .into_iter()
            .any(|o| bot == 0 || o.bot_id == 0 || o.bot_id == bot);
        if clash {
            bail!("another skill here is already called \"{}\"; rename or remove one first", k.name);
        }
        Ok(self.c().execute("UPDATE skills SET bot_id=?3 WHERE id=?1 AND scope=?2", params![id, scope, bot])? > 0)
    }

    /// Accept or dismiss a proposed skill. Accepting one with the name of a skill the bot already has replaces it.
    pub fn skill_decide(&self, scope: &str, id: i64, accept: bool) -> Result<bool> {
        let proposal = self.skills_where("id=?1 AND scope=?2 AND status='proposed'", params![id, scope])?.pop();
        // A new version goes to whoever has the skill now: it may have been shared with all bots, or given to one bot,
        // since it was proposed.
        let owner = match &proposal {
            Some(p) if accept && p.kind != "retire" => self
                .skills_where("scope=?1 AND status='active' AND lower(name)=lower(?2) AND id != ?3", params![scope, p.name, id])?
                .into_iter()
                .find(|o| o.bot_id == p.bot_id || o.bot_id == 0 || p.bot_id == 0)
                .map(|o| o.bot_id),
            _ => None,
        };
        let mut c = self.c();
        let tx = c.transaction()?;
        if let Some(bot) = owner {
            tx.execute("UPDATE skills SET bot_id=?2 WHERE id=?1", params![id, bot])?;
        }
        // A merge or retirement takes the skills it names out of use (kept as removed, so they can come back).
        if accept && let Some(p) = &proposal {
            for old in &p.replaces {
                tx.execute("UPDATE skills SET status='removed' WHERE id=?1 AND scope=?2 AND status='active'", params![old, scope])?;
            }
            if p.kind == "retire" {
                let n = tx.execute("UPDATE skills SET status='removed' WHERE id=?1 AND scope=?2", params![id, scope])?;
                tx.commit()?;
                return Ok(n > 0);
            }
        }
        if accept {
            tx.execute(
                "UPDATE skills SET status='removed' WHERE status='active' AND id != ?1 AND scope=?2
                   AND (bot_id, lower(name)) = (SELECT bot_id, lower(name) FROM skills WHERE id=?1 AND scope=?2 AND status='proposed')",
                params![id, scope],
            )?;
        }
        let n = tx.execute("UPDATE skills SET status=?3 WHERE id=?1 AND scope=?2 AND status='proposed'", params![id, scope, if accept { "active" } else { "dismissed" }])?;
        tx.commit()?;
        Ok(n > 0)
    }

    /// A bot read this skill in this run.
    pub fn skill_used(&self, skill: i64, run: Option<i64>) -> Result<()> {
        self.c().execute("INSERT INTO skill_uses(skill_id, run_id, ts) VALUES (?1,?2,?3)", params![skill, run, now()])?;
        Ok(())
    }

    /// Runs since `since` that read one of this chat's active skills and then went wrong: the run failed or was stopped,
    /// an action failed or was denied, or the person's next message corrected it. The evidence for a fix.
    #[allow(clippy::type_complexity)]
    pub fn skill_troubles(&self, scope: &str, chat: i64, since: i64) -> Result<Vec<SkillTrouble>> {
        let rows: Vec<(i64, i64, String, Option<String>, Option<String>, bool, Option<String>)> = {
            let c = self.c();
            let mut st = c.prepare(
                "SELECT DISTINCT u.skill_id, r.id, r.status, r.error, r.asked, r.tainted,
                   (SELECT m.content FROM messages m WHERE m.chat_id=r.chat_id AND m.bot_id=r.bot_id AND m.role='user'
                      AND m.id > (SELECT max(a.id) FROM messages a WHERE a.run_id=r.id) ORDER BY m.id LIMIT 1)
                 FROM skill_uses u JOIN runs r ON r.id = u.run_id JOIN skills s ON s.id = u.skill_id
                 WHERE s.scope=?1 AND s.status='active' AND r.chat_id=?2 AND u.ts > ?3
                 ORDER BY r.id DESC LIMIT 20",
            )?;
            st.query_map(params![scope, chat, since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?.collect::<Result<_, _>>()?
        };
        let mut out = vec![];
        for (skill_id, run_id, status, error, asked, tainted, next) in rows {
            let calls = self.tool_calls(run_id)?;
            let mut trouble = vec![];
            if status != "ok" && status != "running" {
                trouble.push(format!("the run didn't finish: {}", error.unwrap_or_else(|| status.clone())));
            }
            for c in calls.iter().filter(|c| !c.ok || c.decision == "deny") {
                trouble.push(format!("{} [{}]: {}", c.summary, if c.decision == "deny" { "denied" } else { "failed" }, crate::tools::truncate(&c.result, 150)));
            }
            if let Some(n) = next.filter(|n| crate::is_correction(n)) {
                trouble.push(format!("they corrected it: \"{}\"", crate::tools::truncate(&n, 200)));
            }
            if trouble.is_empty() {
                continue;
            }
            let Some(skill) = self.skills_where("id=?1", [skill_id])?.pop() else { continue };
            out.push(SkillTrouble { skill, tainted, run_id, asked: asked.unwrap_or_default(), steps: calls.iter().map(|c| c.summary.clone()).collect(), trouble: trouble.join("; ") });
        }
        Ok(out)
    }

    /// Propose a fix to an active skill: replace `old` (which must appear in its steps) with `new`. The result is a new
    /// version waiting for the person, with the evidence as its reason. Patches, never rewrites; at most 4,000 chars.
    pub fn skill_patch_propose(&self, scope: &str, skill: i64, old: &str, new: &str, reason: &str) -> Result<Option<i64>> {
        let Some(k) = self.skills_where("id=?1 AND scope=?2 AND status='active'", params![skill, scope])?.pop() else { bail!("no active skill #{skill}") };
        if old.trim().is_empty() || !k.body.contains(old) {
            bail!("the text to change isn't in that skill");
        }
        let body = k.body.replacen(old, new, 1);
        // A small fix: at most 500 characters longer (taught skills can be long already), never past 12,000.
        let limit = (k.body.chars().count().max(4000) + 500).min(12_000);
        if body.chars().count() > limit || body.trim() == k.body.trim() {
            bail!("the fixed skill would be too long, or unchanged");
        }
        self.skill_insert(scope, k.bot_id, &k.name, &k.description, &body, None, "patch", reason, &[])
    }

    /// What this conversation actually did lately: the successful actions of the bot's last few runs here (not memory,
    /// skill or search bookkeeping), oldest first. What a skill saved on request may be built from.
    pub fn recent_steps(&self, bot: i64, chat: i64, runs: usize) -> Result<Vec<String>> {
        let ids: Vec<i64> = {
            let c = self.c();
            let mut st = c.prepare("SELECT id FROM runs WHERE bot_id=?1 AND chat_id=?2 AND started_at > ?3 ORDER BY id DESC LIMIT ?4")?;
            st.query_map(params![bot, chat, now() - 6 * 3600, runs as i64], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        let mut steps = vec![];
        for id in ids.into_iter().rev() {
            for c in self.tool_calls(id)? {
                let bookkeeping = c.tool.starts_with("memory_") || c.tool.starts_with("skill_") || c.tool.ends_with("_search") || c.tool == "plan_propose" || c.tool == "ask_choice";
                if c.ok && c.decision != "deny" && !bookkeeping {
                    steps.push(c.summary);
                }
            }
        }
        Ok(steps)
    }

    /// Telegram chats where this bot has skills, with them: (chat id, [(name, description)]). For the "/" menu.
    pub fn skill_menus(&self, bot: i64) -> Result<Vec<SkillMenu>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT CAST(substr(s.scope, 6) AS INTEGER), s.name, s.description FROM skills s JOIN chats ch ON ch.chat_id = CAST(substr(s.scope, 6) AS INTEGER)
             WHERE s.bot_id IN (?1, 0) AND s.status='active' AND s.scope LIKE 'chat:%' AND ch.channel='telegram' AND ch.chat_id != 0 ORDER BY s.id",
        )?;
        let rows: Vec<(i64, String, String)> = st.query_map([bot], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        let mut out: Vec<SkillMenu> = vec![];
        for (chat, name, desc) in rows {
            match out.iter_mut().find(|(c, _)| *c == chat) {
                Some((_, v)) => v.push((name, desc)),
                None => out.push((chat, vec![(name, desc)])),
            }
        }
        Ok(out)
    }

    /// A skill the person asked a bot to save from a conversation: a proposal (Save / Not now), never active by itself.
    #[allow(clippy::too_many_arguments)]
    pub fn skill_saved_propose(&self, scope: &str, bot: i64, name: &str, description: &str, body: &str, reason: &str, run: Option<i64>) -> Result<Option<i64>> {
        if body.chars().count() > 4000 {
            bail!("the skill would be too long (keep it under 4,000 characters)");
        }
        self.skill_insert(scope, bot, name, description, body, run, "saved", reason, &[])
    }

    /// Add a skill from Mimir's library: you chose it, so it's accepted straight away (a new version of one by that name).
    /// None when that bot already has exactly this one.
    pub fn skill_from_library(&self, scope: &str, bot: i64, name: &str, description: &str, body: &str) -> Result<Option<i64>> {
        let Some(id) = self.skill_insert(scope, bot, name, description, body, None, "library", "From Mimir's skill library.", &[])? else { return Ok(None) };
        self.skill_decide(scope, id, true)?;
        Ok(Some(id))
    }

    /// Propose a skill taught from a source (a page, a document, pasted text): from outside, so it always waits for the
    /// person, with where it came from and any warnings as its reason. One with an accepted skill's name is a new version.
    pub fn skill_taught_propose(&self, scope: &str, bot: i64, name: &str, description: &str, body: &str, reason: &str) -> Result<Option<i64>> {
        if body.chars().count() > 12_000 {
            bail!("the skill would be too long");
        }
        self.skill_insert(scope, bot, name, description, body, None, "taught", reason, &[])
    }

    /// Propose merging overlapping skills of one bot into one (they're retired when it's accepted).
    pub fn skill_merge_propose(&self, scope: &str, ids: &[i64], name: &str, description: &str, body: &str, reason: &str) -> Result<Option<i64>> {
        let skills: Vec<Skill> = ids.iter().filter_map(|id| self.skills_where("id=?1 AND scope=?2 AND status='active'", params![id, scope]).ok()?.pop()).collect();
        if skills.len() < 2 || skills.len() != ids.len() || skills.iter().any(|k| k.bot_id != skills[0].bot_id) {
            bail!("a merge needs two or more active skills of the same bot");
        }
        if body.chars().count() > 4000 {
            bail!("the merged skill would be too long");
        }
        // Accepting replaces the bot's active skill of that name, so it must be one of the merged ones.
        if self.skills_active(scope, skills[0].bot_id)?.iter().any(|k| k.name.eq_ignore_ascii_case(name.trim()) && !ids.contains(&k.id)) {
            bail!("another skill already has that name");
        }
        self.skill_insert(scope, skills[0].bot_id, name, description, body, None, "merge", reason, ids)
    }

    /// Propose retiring a skill nobody has used for a while (it's kept, and can come back).
    pub fn skill_retire_propose(&self, scope: &str, skill: &Skill, reason: &str) -> Result<Option<i64>> {
        let recent_no = self
            .c()
            .prepare("SELECT 1 FROM skills WHERE scope=?1 AND kind='retire' AND replaces=?2 AND (status='proposed' OR (status='dismissed' AND created_at > ?3))")?
            .exists(params![scope, skill.id.to_string(), now() - 90 * 86400])?;
        if recent_no {
            return Ok(None);
        }
        let c = self.c();
        c.execute(
            "INSERT INTO skills(scope, bot_id, name, description, body, created_at, kind, reason, replaces) VALUES (?1,?2,?3,?4,?5,?6,'retire',?7,?8)",
            params![scope, skill.bot_id, skill.name, skill.description, skill.body, now(), reason, skill.id.to_string()],
        )?;
        Ok(Some(c.last_insert_rowid()))
    }

    /// Active skills not used (nor added) in the last `days` days.
    pub fn skills_unused(&self, scope: &str, days: i64) -> Result<Vec<Skill>> {
        let cutoff = now() - days * 86400;
        Ok(self.skills_where("scope=?1 AND status='active'", [scope])?.into_iter().filter(|k| k.last_used.unwrap_or(k.created_at) < cutoff && k.created_at < cutoff).collect())
    }

    /// Facts (not preferences or tasks) in standing memory that search hasn't brought up in `days` days.
    pub fn memories_unrecalled(&self, scope: &str, days: i64) -> Result<Vec<(i64, String)>> {
        // Only ones in standing memory: demoting the rest would change nothing.
        let standing: std::collections::HashSet<i64> = self.bootstrap(scope, 0)?.iter().map(|e| e.id).collect();
        let c = self.c();
        let mut st = c.prepare(
            "SELECT id, title FROM entries WHERE scope=?1 AND type='knowledge' AND superseded_by IS NULL AND invalid=0 AND pinned=0
               AND background=0 AND coalesce(last_recalled, created_at) < ?2 ORDER BY coalesce(last_recalled, created_at) LIMIT 10",
        )?;
        let v: Vec<(i64, String)> = st.query_map(params![scope, now() - days * 86400], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v.into_iter().filter(|(id, _)| standing.contains(id)).collect())
    }

    /// Bring back a retired or removed skill, unless the bot has another active one by that name now.
    pub fn skill_restore(&self, scope: &str, id: i64) -> Result<bool> {
        let Some(k) = self.skills_where("id=?1 AND scope=?2 AND status='removed' AND kind != 'retire'", params![id, scope])?.pop() else { return Ok(false) };
        // A shared skill clashes with any bot's skill of that name; a bot's own with its own or a shared one.
        let clash = self.skills_where("scope=?1 AND status='active' AND lower(name)=lower(?2)", params![scope, k.name])?;
        if clash.iter().any(|a| k.bot_id == 0 || a.bot_id == 0 || a.bot_id == k.bot_id) {
            bail!("the bot has another skill called \"{}\" now", k.name);
        }
        Ok(self.c().execute("UPDATE skills SET status='active' WHERE id=?1", [id])? > 0)
    }

    /// Skills taken out of use (retired, merged away or removed) in the last 180 days, to bring back.
    pub fn skills_removed(&self, scope: &str) -> Result<Vec<Skill>> {
        self.skills_where("scope=?1 AND status='removed' AND kind != 'retire' AND created_at > ?2", params![scope, now() - 180 * 86400])
    }

    pub fn skill_remove(&self, scope: &str, id: i64) -> Result<bool> {
        Ok(self.c().execute("UPDATE skills SET status='removed' WHERE id=?1 AND scope=?2 AND status='active'", params![id, scope])? > 0)
    }
}
