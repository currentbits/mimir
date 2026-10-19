use super::*;

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
// Sent by the UI: a misspelled field is an error, not silently dropped.
#[serde(deny_unknown_fields)]
pub struct Bot {
    pub id: i64,
    pub name: String,
    /// Extra instructions appended to the base persona.
    pub persona: String,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    /// Comma-separated tool allowlist; empty = all tools.
    pub tools: String,
    /// How its creature looks, chosen by you: {"shape","eyes","extra","color"} as JSON. Empty = the look its id gives it.
    #[serde(default)]
    pub look: String,
}

/// A bot's look: empty, or a small JSON object of short names and a #RRGGBB colour (the UI ignores names it doesn't know).
pub(super) fn valid_look(look: &str) -> bool {
    if look.is_empty() {
        return true;
    }
    let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(look) else { return false };
    look.len() <= 200
        && m.iter().all(|(k, v)| {
            let v = v.as_str().unwrap_or("\u{0}");
            match k.as_str() {
                "color" => v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit()),
                "shape" | "eyes" | "extra" => v.len() <= 16 && v.chars().all(|c| c.is_ascii_lowercase()),
                _ => false,
            }
        })
}

impl Bot {
    /// Allowlist check; "browser__*" style entries match a whole tool server.
    pub fn allows(&self, tool: &str) -> bool {
        // A bot that may set routines may also change them.
        // So may it keep a goal (its follow-ups are routines).
        if tool == "routine_update" || tool.starts_with("goal_") {
            return self.allows("routine_create");
        }
        self.tools.trim().is_empty()
            || self.tools.split(',').map(str::trim).any(|t| t == tool || t.strip_suffix('*').is_some_and(|p| !p.is_empty() && tool.starts_with(p)))
    }
}

impl Db {
    // ---------- bots ----------

    pub fn bots(&self) -> Result<Vec<Bot>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, name, persona, provider, model, base_url, tools, look FROM bots ORDER BY id")?;
        let v = st
            .query_map([], |r| {
                Ok(Bot {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    persona: r.get(2)?,
                    provider: r.get(3)?,
                    model: r.get(4)?,
                    base_url: r.get(5)?,
                    tools: r.get(6)?,
                    look: r.get(7)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn bot(&self, id: i64) -> Result<Bot> {
        self.bots()?.into_iter().find(|b| b.id == id).ok_or_else(|| anyhow::anyhow!("no bot #{id}"))
    }

    /// Insert (id <= 0) or update. Returns the bot id.
    pub fn bot_save(&self, b: &Bot) -> Result<i64> {
        if b.name.trim().is_empty() {
            bail!("a bot needs a name");
        }
        if !valid_look(&b.look) {
            bail!("that look isn't valid");
        }
        let c = self.c();
        if b.id <= 0 {
            c.execute(
                "INSERT INTO bots(name, persona, provider, model, base_url, tools, look, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![b.name.trim(), b.persona, b.provider, b.model.trim(), b.base_url.trim(), b.tools, b.look, now()],
            )?;
            return Ok(c.last_insert_rowid());
        }
        let n = c.execute(
            "UPDATE bots SET name=?2, persona=?3, provider=?4, model=?5, base_url=?6, tools=?7, look=?8 WHERE id=?1",
            params![b.id, b.name.trim(), b.persona, b.provider, b.model.trim(), b.base_url.trim(), b.tools, b.look],
        )?;
        if n == 0 {
            bail!("no bot #{}", b.id);
        }
        Ok(b.id)
    }

    /// Removes the bot and its routines/sessions. Memory is the person's, so it stays.
    pub fn bot_delete(&self, id: i64) -> Result<()> {
        if self.bots()?.len() <= 1 {
            bail!("keep at least one bot");
        }
        // Its conversations and what it did go with it (memories are the person's, shared by all bots: they stay).
        self.c().execute_batch(&format!(
            "DELETE FROM tool_calls WHERE run_id IN (SELECT id FROM runs WHERE bot_id={id}); DELETE FROM tool_cards WHERE bot_id={id};
             DELETE FROM runs WHERE bot_id={id}; DELETE FROM messages WHERE bot_id={id};
             DELETE FROM signals WHERE bot_id={id}; DELETE FROM cases WHERE bot_id={id}; DELETE FROM digest_items WHERE bot_id={id}; DELETE FROM cards WHERE bot_id={id};
             DELETE FROM routines WHERE bot_id={id}; DELETE FROM sessions WHERE bot_id={id}; DELETE FROM conv_taint WHERE bot_id={id};
             DELETE FROM settings WHERE key LIKE 'seen:{id}:%' OR key LIKE 'private:{id}:%' OR key IN ('pause:{id}', 'fallback:{id}', 'fallback_spent:{id}');
             DELETE FROM bots WHERE id={id};"
        ))?;
        self.wipe_free_space();
        Ok(())
    }

    /// The account a bot's routines run on while its own is at a usage limit: (account kind, model). None: off.
    pub fn fallback(&self, bot: i64) -> Result<Option<(String, String)>> {
        Ok(self.setting(&format!("fallback:{bot}"))?.and_then(|v| serde_json::from_str(&v).ok()))
    }

    /// Set (or, with an empty kind, turn off) a bot's fallback account.
    pub fn set_fallback(&self, bot: i64, kind: &str, model: &str) -> Result<()> {
        let key = format!("fallback:{bot}");
        if kind.is_empty() {
            self.c().execute("DELETE FROM settings WHERE key=?1", [key])?;
            return Ok(());
        }
        if kind == self.bot(bot)?.provider && ["claude", "codex", "grok", "chatgpt"].contains(&kind) {
            bail!("that's the same login as the bot's own, so it's at the same limit");
        }
        self.set_setting(&key, &serde_json::to_string(&(kind, model.trim()))?)
    }

    /// The fallback a bot's routines run on right now: its own account is held for a usage limit (not a daily cap,
    /// which applies whatever the account), and the fallback isn't at its own limit. With when the hold ends: back then.
    pub fn on_fallback(&self, bot: i64, now: i64) -> Result<Option<(String, String, i64)>> {
        let Some(h) = self.hold(&format!("hold:{bot}"))?.filter(|h| h.until > now && h.waive.is_none()) else { return Ok(None) };
        let spent = self.setting(&format!("fallback_spent:{bot}"))?.and_then(|v| v.parse::<i64>().ok()).is_some_and(|t| t > now);
        Ok(self.fallback(bot)?.filter(|_| !spent).map(|(k, m)| (k, m, h.until)))
    }

    /// The fallback hit its own usage limit: routines wait for the bot's own account again, until `until`.
    pub fn fallback_spent(&self, bot: i64, until: i64) -> Result<()> {
        self.set_setting(&format!("fallback_spent:{bot}"), &until.to_string())
    }

    /// Delete a chat's history: its conversation, the run log and tool calls, corrections, digests, decided
    /// approvals and decided suggestions. Memories, routines and the pairing stay. Returns how many messages went.
    pub fn chat_history_clear(&self, chat: i64) -> Result<usize> {
        let mut c = self.c();
        let tx = c.transaction()?;
        let n = tx.execute("DELETE FROM messages WHERE chat_id=?1", [chat])?;
        for sql in [
            "DELETE FROM tool_calls WHERE chat_id=?1",
            "DELETE FROM tool_cards WHERE chat_id=?1",
            "DELETE FROM runs WHERE chat_id=?1 AND status != 'running'",
            "DELETE FROM signals WHERE chat_id=?1",
            "DELETE FROM cases WHERE chat_id=?1",
            "DELETE FROM digest_items WHERE chat_id=?1",
            "DELETE FROM approvals WHERE chat_id=?1 AND status != 'pending'",
            "DELETE FROM sessions WHERE chat_id=?1",
            "DELETE FROM conv_taint WHERE chat_id=?1",
            "DELETE FROM settings WHERE key LIKE 'private:%:' || ?1",
        ] {
            tx.execute(sql, [chat])?;
        }
        tx.execute("DELETE FROM suggestions WHERE scope=?1 AND status != 'pending'", [format!("chat:{chat}")])?;
        tx.commit()?;
        drop(c);
        self.wipe_free_space();
        Ok(n)
    }

    /// Everything kept about the person, for them to take away: memories (with earlier versions), conversations,
    /// meetings and their transcripts, skills, routines, chats and bots (tokens live in the keychain, not here).
    pub fn export_all(&self) -> Result<serde_json::Value> {
        let c = self.c();
        let mut out = serde_json::Map::new();
        out.insert("exported_at".into(), serde_json::json!(now()));
        for table in ["entries", "messages", "meetings", "meeting_lines", "skills", "routines", "chats", "bots", "suggestions"] {
            let mut st = c.prepare(&format!("SELECT * FROM {table}"))?;
            let cols: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
            let rows: Vec<serde_json::Value> = st
                .query_map([], |r| {
                    let mut m = serde_json::Map::new();
                    for (i, col) in cols.iter().enumerate() {
                        let v = match r.get_ref(i)? {
                            rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                            rusqlite::types::ValueRef::Integer(n) => serde_json::json!(n),
                            rusqlite::types::ValueRef::Real(f) => serde_json::json!(f),
                            rusqlite::types::ValueRef::Text(t) => serde_json::json!(String::from_utf8_lossy(t)),
                            rusqlite::types::ValueRef::Blob(_) => serde_json::Value::Null,
                        };
                        m.insert(col.clone(), v);
                    }
                    Ok(serde_json::Value::Object(m))
                })?
                .collect::<Result<_, _>>()?;
            out.insert(table.into(), serde_json::Value::Array(rows));
        }
        Ok(serde_json::Value::Object(out))
    }

    /// After deleting: fold the write-ahead log back into the file, so deleted text isn't left there either.
    pub(super) fn wipe_free_space(&self) {
        let _ = self.c().execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }

    /// At startup: the run log's details, corrections and signals older than six months go.
    pub fn prune_old(&self) -> Result<()> {
        let before = now() - 180 * 86400;
        let c = self.c();
        c.execute("DELETE FROM tool_calls WHERE ts < ?1", [before])?;
        c.execute("DELETE FROM tool_cards WHERE ts < ?1", [before])?;
        c.execute("DELETE FROM cases WHERE ts < ?1", [before])?;
        c.execute("DELETE FROM signals WHERE ts < ?1", [before])?;
        c.execute("DELETE FROM digest_items WHERE sent_at IS NOT NULL AND sent_at < ?1", [before])?;
        // Buttons nobody tapped in a month (runtime::Tap): they'd only say "already done" by then anyway.
        c.execute("DELETE FROM settings WHERE key LIKE 'tap:%' AND CAST(json_extract(value, '$.at') AS INTEGER) < ?1", [now() - 30 * 86400])?;
        Ok(())
    }
}
