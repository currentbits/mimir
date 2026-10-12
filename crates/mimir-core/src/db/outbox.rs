use super::*;

/// A message to a chat app, kept until it's sent: a reply, a routine's result, a note for the decisions chat.
#[derive(Debug, Clone)]
pub struct Outgoing {
    pub id: i64,
    pub chat_id: i64,
    /// The channel ("telegram", "slack", "discord") and the address in it (a Telegram topic is "chat:topic").
    pub kind: String,
    pub ext: String,
    /// The streamed draft this replaces, if one was posted.
    pub draft: Option<String>,
    pub text: String,
    /// Buttons under it, as JSON rows of (label, data).
    pub rows: String,
    /// Files to attach after the text, as a JSON list (`[[file:…]]` names).
    pub files: String,
    /// The routine run it's the result of: its delivery status follows this message.
    pub run_id: Option<i64>,
}

/// Tries before a message is given up on (one every `OUTBOX_STALE`).
pub const OUTBOX_TRIES: i64 = 5;
/// A message claimed this long ago and not sent: its sender gave up or died, so it's taken again.
pub const OUTBOX_STALE: i64 = 300;
/// Older than this, a message isn't sent any more (and is cleared out).
const OUTBOX_KEEP: i64 = 3 * 86_400;

/// This process, on the messages it's sending: one another process left half-sent (a crash, an update) is taken
/// at once; one of its own (a reply still being sent while the runtime restarted) only once it's stale.
static SENDER: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| crate::random_hex(8).unwrap_or_else(|_| std::process::id().to_string()));

impl Db {
    /// Kept before it's sent, claimed by this process (which sends it right away).
    #[allow(clippy::too_many_arguments)]
    pub fn outbox_add(&self, bot: i64, chat: i64, kind: &str, ext: &str, draft: Option<&str>, text: &str, rows: &str, files: &str, run: Option<i64>) -> Result<i64> {
        let c = self.c();
        c.execute(
            "INSERT INTO outbox(bot_id, chat_id, kind, ext, draft, text, rows, files, run_id, created_at, claimed_by, claimed_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?10)",
            params![bot, chat, kind, ext, draft, text, rows, files, run, now(), *SENDER],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// Sent: gone from the outbox.
    pub fn outbox_sent(&self, id: i64) -> Result<()> {
        self.c().execute("DELETE FROM outbox WHERE id=?1", [id])?;
        Ok(())
    }

    /// Didn't go: tried again once it's stale, until it's out of tries. True when that was its last try.
    pub fn outbox_failed(&self, id: i64, error: &str) -> Result<bool> {
        let c = self.c();
        c.execute("UPDATE outbox SET tries = tries + 1, error=?2, claimed_by=?3, claimed_at=?4 WHERE id=?1", params![id, error, *SENDER, now()])?;
        Ok(c.query_row("SELECT tries FROM outbox WHERE id=?1", [id], |r| r.get::<_, i64>(0)).optional()?.is_none_or(|t| t >= OUTBOX_TRIES))
    }

    /// A bot's messages still to send, oldest first, each claimed for this process so no other sender takes it too.
    pub fn outbox_take(&self, bot: i64, now: i64) -> Result<Vec<Outgoing>> {
        let c = self.c();
        c.execute("DELETE FROM outbox WHERE created_at < ?1", [now - OUTBOX_KEEP])?;
        let mut st = c.prepare(
            "SELECT id, chat_id, kind, ext, draft, text, rows, files, run_id, claimed_by, claimed_at FROM outbox
             WHERE bot_id=?1 AND tries < ?2 AND (claimed_by IS NOT ?3 OR claimed_at < ?4) ORDER BY id",
        )?;
        let due: Vec<(Outgoing, Option<String>, Option<i64>)> = st
            .query_map(params![bot, OUTBOX_TRIES, *SENDER, now - OUTBOX_STALE], |r| {
                let o = Outgoing { id: r.get(0)?, chat_id: r.get(1)?, kind: r.get(2)?, ext: r.get(3)?, draft: r.get(4)?, text: r.get(5)?, rows: r.get(6)?, files: r.get(7)?, run_id: r.get(8)? };
                Ok((o, r.get(9)?, r.get(10)?))
            })?
            .collect::<Result<_, _>>()?;
        // Claimed only if no one claimed it since it was read (another process's pass, at the same moment).
        let mut claim = c.prepare("UPDATE outbox SET claimed_by=?2, claimed_at=?3 WHERE id=?1 AND claimed_by IS ?4 AND claimed_at IS ?5")?;
        Ok(due.into_iter().filter(|(o, by, at)| claim.execute(params![o.id, *SENDER, now, by, at]).is_ok_and(|n| n == 1)).map(|(o, ..)| o).collect())
    }

    /// A routine result's delivery, once its message went out later (or didn't in the end); a note after " · " (the
    /// fallback account) stays.
    pub fn run_redelivered(&self, run: i64, how: &str) -> Result<()> {
        self.c().execute(
            "UPDATE runs SET delivered = ?2 || CASE WHEN instr(coalesce(delivered, ''), ' · ') > 0 THEN substr(delivered, instr(delivered, ' · ')) ELSE '' END WHERE id=?1",
            params![run, how],
        )?;
        Ok(())
    }
}
