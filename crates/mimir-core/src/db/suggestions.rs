use super::*;

/// A proposed memory change from the nightly tidy-up. Nothing is applied until the user accepts it.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
pub struct Suggestion {
    #[serde(default)]
    pub id: i64,
    /// add | update | forget | demote (move to background)
    pub action: String,
    /// The entry to update/forget.
    #[serde(default)]
    pub entry_id: Option<i64>,
    #[serde(default = "default_kind", rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub reason: String,
    /// What replaying past corrections with this change showed, once you asked for it ("avoids 2 of 3 …").
    #[serde(default)]
    pub check: Option<String>,
    /// Drawn from outside content (a bot proposing it after reading a page, a tidy-up that saw such text, or an
    /// update of an unconfirmed memory): accepting it keeps it marked unconfirmed, and "Accept all" leaves it out.
    #[serde(default)]
    pub tainted: bool,
}

fn default_kind() -> String {
    "knowledge".into()
}

/// Reason prefix of a habit a bot wanted to remember but the person never stated.
pub const BOT_PROPOSED: &str = "A bot wants to remember this";

/// The other memory of a pair a suggestion settles ("Duplicate of #N …", "Contradicts #N …"): the one it keeps.
pub fn paired(reason: &str) -> Option<i64> {
    let r = reason.strip_prefix("Duplicate of #").or_else(|| reason.strip_prefix("Contradicts #"))?;
    r.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// How long a suggestion seen in one conversation waits for a second one.
const WAIT_DAYS: i64 = 30;

fn day(at: i64) -> String {
    chrono::TimeZone::timestamp_opt(&chrono::Local, at, 0).single().map(|d| d.format("%-d %b").to_string()).unwrap_or_default()
}

impl Db {
    // ---------- memory suggestions ----------

    /// Validates and queues a suggestion; skips one that's already pending. Returns its id (None if skipped).
    pub fn suggestion_add(&self, scope: &str, s: &Suggestion) -> Result<Option<i64>> {
        match s.action.as_str() {
            "add" | "update" if !TYPES.contains(&s.kind.as_str()) || s.title.trim().is_empty() || s.body.trim().is_empty() => {
                bail!("add/update needs a valid type, title and body")
            }
            "update" | "forget" | "demote" if s.entry_id.is_none() => bail!("{} needs entry_id", s.action),
            "add" | "update" | "forget" | "demote" => {}
            other => bail!("unknown action {other}"),
        }
        let mut tainted = s.tainted;
        if let Some(id) = s.entry_id {
            let Some(e) = self.entries(scope)?.into_iter().find(|e| e.id == id && e.status == "active") else {
                bail!("entry #{id} isn't an active memory here");
            };
            // A new version of an unconfirmed memory stays unconfirmed.
            tainted |= e.tainted;
            // A pin is the person's call; the tidy-up doesn't argue with it.
            if s.action == "demote" && (e.pinned || e.background) {
                bail!("entry #{id} is pinned or already in the background");
            }
        }
        let c = self.c();
        // One pending suggestion per memory, and no second pending "add" with the same title:
        // conflicting proposals (e.g. forget A and forget B for a duplicate pair) could lose the fact.
        // A suggestion you dismissed doesn't come back for 90 days.
        // A change you made in the memory folder is asked about every time, whatever you dismissed before.
        let yours = crate::vault::from_folder(&s.reason);
        let dup: bool = c
            .prepare(
                "SELECT 1 FROM suggestions WHERE scope=?1
                   AND (status='pending' OR (status='dismissed' AND NOT ?6 AND action=?4 AND created_at > ?5))
                   AND ((?2 IS NOT NULL AND entry_id = ?2) OR (?2 IS NULL AND entry_id IS NULL AND lower(title) = lower(?3)))",
            )?
            .exists(params![scope, s.entry_id, s.title.trim(), s.action, now() - 90 * 86400, yours])?;
        if dup {
            return Ok(None);
        }
        let text = format!("{} {}", s.title, s.body);
        // The same change in other words as one already waiting (two proposals in one tidy-up, or a bot's and the
        // tidy-up's): one card is enough.
        if matches!(s.action.as_str(), "add" | "update") {
            let mut st = c.prepare("SELECT title || ' ' || body FROM suggestions WHERE scope=?1 AND status='pending' AND action IN ('add','update') AND type=?2")?;
            let pending: Vec<String> = st.query_map(params![scope, s.kind], |r| r.get(0))?.collect::<Result<_, _>>()?;
            if pending.iter().any(|p| alike(&text, p)) {
                return Ok(None);
            }
        }
        // Like ones you dismissed lately, in other words: twice is a pattern (it isn't shown), once is said on the card.
        // Not for imports, which you pasted yourself. (Rules drawn from dismissals, or examples in the prompt, make a
        // proposer worse; filtering lookalikes is what works: Greptile, 19% → 55% of comments acted on.)
        // ponytail: the 90 days let a real need come back
        let mut reason = s.reason.trim().to_string();
        if matches!(s.action.as_str(), "add" | "update") && !reason.starts_with("Imported") && !yours {
            let mut st = c.prepare(
                "SELECT title, body, created_at FROM suggestions WHERE scope=?1 AND status='dismissed' AND action IN ('add','update') AND type=?2 AND created_at > ?3
                 ORDER BY id DESC LIMIT 200",
            )?;
            let dismissed: Vec<(String, String, i64)> = st.query_map(params![scope, s.kind, now() - 90 * 86400], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            let similar: Vec<(String, i64)> = dismissed.into_iter().filter(|(t, b, _)| alike(&text, &format!("{t} {b}"))).map(|(t, _, at)| (t, at)).collect();
            match similar.as_slice() {
                [] => {}
                [(t, at)] => reason.push_str(&format!(" You dismissed a similar one on {} (“{t}”).", day(*at))),
                _ => return Ok(None),
            }
        }
        c.execute(
            "INSERT INTO suggestions(scope, action, entry_id, type, title, body, reason, created_at, tainted) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![scope, s.action, s.entry_id, s.kind, s.title.trim(), s.body.trim(), reason, now(), tainted],
        )?;
        Ok(Some(c.last_insert_rowid()))
    }

    /// A suggestion the tidy-up found in one conversation only waits, unseen, until a later tidy-up (another
    /// conversation) proposes it again; then it's shown, saying when it first came up. `started`: when this tidy-up
    /// began, so the same thing twice in one reply isn't a second conversation. Returns the id when it's shown now.
    // ponytail: waiting rows past WAIT_DAYS are just ignored, never pruned
    pub fn suggestion_recur(&self, scope: &str, s: &Suggestion, started: i64) -> Result<Option<i64>> {
        let text = format!("{} {}", s.title, s.body);
        let earlier: Vec<(i64, i64)> = {
            let c = self.c();
            let mut st = c.prepare("SELECT id, title || ' ' || body, created_at FROM suggestions WHERE scope=?1 AND status='waiting' AND type=?2 AND created_at > ?3 AND created_at < ?4")?;
            let rows: Vec<(i64, String, i64)> = st.query_map(params![scope, s.kind, now() - WAIT_DAYS * 86400, started], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            rows.into_iter().filter(|(_, t, _)| alike(&text, t)).map(|(id, _, at)| (id, at)).collect()
        };
        let Some(first) = earlier.iter().map(|e| e.1).min() else {
            if let Some(id) = self.suggestion_add(scope, s)? {
                self.c().execute("UPDATE suggestions SET status='waiting' WHERE id=?1", [id])?;
            }
            return Ok(None);
        };
        let id = self.suggestion_add(scope, &Suggestion { reason: format!("{} It came up in an earlier conversation too ({}).", s.reason.trim(), day(first)), ..s.clone() })?;
        for (w, _) in earlier {
            self.c().execute("UPDATE suggestions SET status='recurred' WHERE id=?1", [w])?;
        }
        Ok(id)
    }

    pub fn suggestions(&self, scope: &str) -> Result<Vec<Suggestion>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, action, entry_id, type, title, body, reason, check_note, tainted FROM suggestions WHERE scope=?1 AND status='pending' ORDER BY id")?;
        let v = st
            .query_map([scope], |r| {
                Ok(Suggestion {
                    id: r.get(0)?,
                    action: r.get(1)?,
                    entry_id: r.get(2)?,
                    kind: r.get(3)?,
                    title: r.get(4)?,
                    body: r.get(5)?,
                    reason: r.get(6)?,
                    check: r.get(7)?,
                    tainted: r.get(8)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// How many times a tool ran successfully in a chat in the last 24 hours (a daily cap on sending).
    pub fn sent_today(&self, chat: i64, tool: &str) -> Result<i64> {
        Ok(self.c().query_row("SELECT count(*) FROM tool_calls WHERE chat_id=?1 AND tool=?2 AND ok=1 AND ts > ?3", params![chat, tool, now() - 86400], |r| r.get(0))?)
    }

    pub fn suggestion_set_tainted(&self, scope: &str, id: i64, tainted: bool) -> Result<()> {
        self.c().execute("UPDATE suggestions SET tainted=?3 WHERE id=?1 AND scope=?2", params![id, scope, tainted])?;
        Ok(())
    }

    pub fn suggestion_set_check(&self, scope: &str, id: i64, note: &str) -> Result<()> {
        self.c().execute("UPDATE suggestions SET check_note=?3 WHERE id=?1 AND scope=?2", params![id, scope, note])?;
        Ok(())
    }

    pub fn case_add(&self, bot: i64, chat: i64, asked: &str, bad: &str, correction: &str) -> Result<()> {
        let t = |s: &str| crate::tools::truncate(s, 1500);
        self.c().execute(
            "INSERT INTO cases(bot_id, chat_id, asked, bad, correction, ts) VALUES (?1,?2,?3,?4,?5,?6)",
            params![bot, chat, t(asked), t(bad), t(correction), now()],
        )?;
        Ok(())
    }

    /// Up to `limit` past corrections in a chat that share the most words with `text` (newest wins ties);
    /// ones sharing none are left out. (asked, bad reply, correction)
    pub fn cases_like(&self, chat: i64, text: &str, limit: usize) -> Result<Vec<(String, String, String)>> {
        let words = |s: &str| -> std::collections::HashSet<String> { s.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() > 3).map(str::to_lowercase).collect() };
        let want = words(text);
        let c = self.c();
        let mut st = c.prepare("SELECT asked, bad, correction FROM cases WHERE chat_id=?1 ORDER BY id DESC LIMIT 200")?;
        let all: Vec<(String, String, String)> = st.query_map([chat], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        let mut scored: Vec<(usize, (String, String, String))> =
            all.into_iter().map(|k| (words(&format!("{} {} {}", k.0, k.1, k.2)).intersection(&want).count(), k)).filter(|(n, _)| *n > 0).collect();
        scored.sort_by_key(|k| std::cmp::Reverse(k.0));
        Ok(scored.into_iter().take(limit).map(|(_, k)| k).collect())
    }

    /// Accept (apply to memory) or dismiss. A suggestion whose target changed since is dismissed, not applied.
    pub fn suggestion_decide(&self, scope: &str, id: i64, accept: bool) -> Result<String> {
        let Some(s) = self.suggestions(scope)?.into_iter().find(|s| s.id == id) else { bail!("no pending suggestion #{id}") };
        let applied = if !accept {
            Ok("dismissed".to_string())
        } else {
            // ponytail: import proposals are told apart by their reason; a column if more origins appear
            // "Imported from OpenClaw: …" keeps where from, so what one agent brought can be told apart.
            let imported = s.reason.split_once(':').map(|(o, _)| o).filter(|o| o.starts_with("Imported from ")).map(|o| format!("i{}, approved by you", &o[1..]));
            let (src, mut prov) = if s.reason.starts_with("Imported") {
                (imported.as_deref().unwrap_or("imported, approved by you"), Prov::by("import"))
            } else if s.reason.starts_with("Found in your") {
                ("found in your email or calendar, approved by you", Prov::by("know_me"))
            } else if crate::vault::from_folder(&s.reason) {
                ("edited in your memory folder, approved by you", Prov::by("you"))
            } else if s.reason.starts_with(BOT_PROPOSED) {
                ("proposed by a bot, approved by you", Prov::by("bot"))
            } else {
                ("tidy-up, approved by you", Prov::by("tidy"))
            };
            // Accepting text drawn from outside content doesn't make it the person's own: it stays unconfirmed
            // until they confirm it ("Looks right" in Memory).
            prov.tainted = s.tainted;
            match s.action.as_str() {
                "add" => self.store_by(scope, &s.kind, &s.title, &s.body, src, None, &prov).map(|(n, _)| format!("added #{n}")),
                "update" => self.store_by(scope, &s.kind, &s.title, &s.body, src, s.entry_id, &prov).map(|(n, _)| format!("#{} replaced by #{n}", s.entry_id.unwrap_or(0))),
                // The target may have changed since (pinned, corrected, forgotten): then nothing happens and the
                // suggestion is dismissed. A pin is the person's call, so a demote never undoes one.
                "demote" => {
                    let n = self.c().execute(
                        "UPDATE entries SET background=1 WHERE id=?1 AND scope=?2 AND pinned=0 AND background=0 AND superseded_by IS NULL AND invalid=0",
                        params![s.entry_id.unwrap_or(0), scope],
                    )?;
                    if n == 0 { Err(anyhow::anyhow!("#{} changed since: it's pinned, replaced or gone", s.entry_id.unwrap_or(0))) } else { Ok(format!("#{} moved to background", s.entry_id.unwrap_or(0))) }
                }
                _ => {
                    let n = self.c().execute("UPDATE entries SET invalid=1 WHERE id=?1 AND scope=?2 AND superseded_by IS NULL AND invalid=0", params![s.entry_id.unwrap_or(0), scope])?;
                    if n == 0 { Err(anyhow::anyhow!("#{} changed since: it's replaced or already gone", s.entry_id.unwrap_or(0))) } else { Ok(format!("forgot #{}", s.entry_id.unwrap_or(0))) }
                }
            }
        };
        let status = if accept && applied.is_ok() { "accepted" } else { "dismissed" };
        self.c().execute("UPDATE suggestions SET status=?2 WHERE id=?1", params![id, status])?;
        applied
    }

    /// Settle a suggestion by forgetting another memory than it proposes: the other one of a pair (you keep the one
    /// it would forget), or its own instead of moving it to background.
    pub fn suggestion_forget(&self, scope: &str, id: i64, entry: i64) -> Result<String> {
        let Some(s) = self.suggestions(scope)?.into_iter().find(|s| s.id == id) else { bail!("no pending suggestion #{id}") };
        if !(s.entry_id == Some(entry) || paired(&s.reason) == Some(entry)) {
            bail!("#{entry} isn't one this suggestion is about");
        }
        let n = self.c().execute("UPDATE entries SET invalid=1 WHERE id=?1 AND scope=?2 AND superseded_by IS NULL AND invalid=0", params![entry, scope])?;
        self.c().execute("UPDATE suggestions SET status=?2 WHERE id=?1", params![id, if n > 0 { "accepted" } else { "dismissed" }])?;
        if n == 0 { bail!("#{entry} changed since: it's replaced or already gone") } else { Ok(format!("forgot #{entry}")) }
    }
}
