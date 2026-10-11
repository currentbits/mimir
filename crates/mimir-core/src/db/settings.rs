use super::*;

impl Db {
    // ---------- settings (non-secret; secrets live in the OS keychain) ----------

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.c().query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional()?)
    }

    /// Read a setting and delete it in one step: only one of two callers racing for it gets it.
    pub fn setting_take(&self, key: &str) -> Result<Option<String>> {
        Ok(self.c().query_row("DELETE FROM settings WHERE key=?1 RETURNING value", [key], |r| r.get(0)).optional()?)
    }

    /// Take or renew the computer for `who` ("run:<id>" or "you") for `secs`. Free, expired or already ours: it's
    /// ours. Otherwise returns who holds it ({who, name}). Atomic, so bots in separate processes can't both get it.
    pub fn computer_lease(&self, who: &str, name: &str, secs: i64) -> Result<Option<(String, String)>> {
        let c = self.c();
        let lease = serde_json::json!({"who": who, "name": name, "until": now() + secs}).to_string();
        let n = c.execute(
            "INSERT INTO settings(key, value) VALUES ('computer_lease', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value
             WHERE json_extract(settings.value, '$.until') < ?2 OR json_extract(settings.value, '$.who') = ?3",
            params![lease, now(), who],
        )?;
        if n == 1 {
            return Ok(None);
        }
        let held: Option<String> = c.query_row("SELECT value FROM settings WHERE key='computer_lease'", [], |r| r.get(0)).optional()?;
        let v: serde_json::Value = held.and_then(|h| serde_json::from_str(&h).ok()).unwrap_or_default();
        Ok(Some((v["who"].as_str().unwrap_or("").to_string(), v["name"].as_str().unwrap_or("someone").to_string())))
    }

    /// Who's using the computer right now, if anyone: (who, name).
    pub fn computer_holder(&self) -> Result<Option<(String, String)>> {
        let held: Option<String> = self.c().query_row("SELECT value FROM settings WHERE key='computer_lease'", [], |r| r.get(0)).optional()?;
        let v: serde_json::Value = held.and_then(|h| serde_json::from_str(&h).ok()).unwrap_or_default();
        Ok((v["until"].as_i64().unwrap_or(0) >= now()).then(|| (v["who"].as_str().unwrap_or("").to_string(), v["name"].as_str().unwrap_or("").to_string())))
    }

    /// Let go of the computer (only if `who` holds it).
    pub fn computer_release(&self, who: &str) -> Result<()> {
        self.c().execute("DELETE FROM settings WHERE key='computer_lease' AND json_extract(value, '$.who') = ?1", [who])?;
        Ok(())
    }

    /// Save a transcribed meeting; returns its id. Lines are (unix time, "you"|"others", text).
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.c().execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}
