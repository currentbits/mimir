use super::*;

impl Db {
    pub fn meeting_save(&self, title: &str, started: i64, ended: i64, lines: &[(i64, String, String)]) -> Result<i64> {
        let mut c = self.c();
        let tx = c.transaction()?;
        tx.execute("INSERT INTO meetings(title, started_at, ended_at) VALUES (?1,?2,?3)", params![title.trim(), started, ended])?;
        let id = tx.last_insert_rowid();
        for (ts, who, text) in lines {
            tx.execute("INSERT INTO meeting_lines(meeting_id, ts, who, text) VALUES (?1,?2,?3,?4)", params![id, ts, who, text])?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Meetings, newest first: (id, title, started, ended, words).
    pub fn meetings(&self) -> Result<Vec<serde_json::Value>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT m.id, m.title, m.started_at, m.ended_at, (SELECT coalesce(sum(length(text) - length(replace(text, ' ', '')) + 1), 0) FROM meeting_lines l WHERE l.meeting_id = m.id), m.notes, m.speakers, m.attendees, m.summary
             FROM meetings m ORDER BY m.id DESC LIMIT 200",
        )?;
        let v = st
            .query_map([], |r| Ok(serde_json::json!({"id": r.get::<_, i64>(0)?, "title": r.get::<_, String>(1)?, "started_at": r.get::<_, i64>(2)?, "ended_at": r.get::<_, i64>(3)?, "words": r.get::<_, i64>(4)?, "notes": r.get::<_, String>(5)?, "speakers": serde_json::from_str::<serde_json::Value>(&r.get::<_, String>(6)?).unwrap_or_default(), "attendees": serde_json::from_str::<serde_json::Value>(&r.get::<_, String>(7)?).unwrap_or_default(), "summary": r.get::<_, String>(8)?})))?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// One meeting's lines, in order.
    pub fn meeting_lines(&self, id: i64) -> Result<Vec<(i64, String, String)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT ts, who, text FROM meeting_lines WHERE meeting_id=?1 ORDER BY ts, id")?;
        let v = st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// A line transcribed during a meeting that's still going (saved as it comes, so nothing is lost if the app quits).
    pub fn meeting_line(&self, id: i64, ts: i64, who: &str, text: &str) -> Result<()> {
        let c = self.c();
        c.execute("INSERT INTO meeting_lines(meeting_id, ts, who, text) VALUES (?1,?2,?3,?4)", params![id, ts, who, text])?;
        c.execute("UPDATE meetings SET ended_at=max(ended_at, ?2) WHERE id=?1", params![id, now()])?;
        Ok(())
    }

    pub fn meeting_update(&self, id: i64, title: Option<&str>, ended: Option<i64>) -> Result<bool> {
        let c = self.c();
        Ok(c.execute("UPDATE meetings SET title=coalesce(?2, title), ended_at=coalesce(?3, ended_at) WHERE id=?1", params![id, title.map(str::trim), ended])? > 0)
    }

    /// The other side's lines, (line id, ts), for telling speakers apart.
    pub fn meeting_others(&self, id: i64) -> Result<Vec<(i64, i64)>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, ts FROM meeting_lines WHERE meeting_id=?1 AND who LIKE 'others%' ORDER BY ts, id")?;
        let v = st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// Who said each line, once speakers are told apart ("others:2").
    pub fn meeting_relabel(&self, labels: &[(i64, String)]) -> Result<()> {
        let mut c = self.c();
        let tx = c.transaction()?;
        for (line, who) in labels {
            tx.execute("UPDATE meeting_lines SET who=?2 WHERE id=?1", params![line, who])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn meeting_summary(&self, id: i64, summary: &str) -> Result<bool> {
        Ok(self.c().execute("UPDATE meetings SET summary=?2 WHERE id=?1", params![id, summary.trim()])? > 0)
    }

    pub fn meeting_attendees(&self, id: i64, names: &[String]) -> Result<bool> {
        Ok(self.c().execute("UPDATE meetings SET attendees=?2 WHERE id=?1", params![id, serde_json::to_string(names)?])? > 0)
    }

    /// Name a speaker ("others:2" → "Sam"); an empty name forgets it.
    pub fn meeting_speaker(&self, id: i64, key: &str, name: &str) -> Result<bool> {
        let c = self.c();
        let cur: String = c.query_row("SELECT speakers FROM meetings WHERE id=?1", [id], |r| r.get(0))?;
        let mut map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&cur).unwrap_or_default();
        if name.trim().is_empty() {
            map.remove(key);
        } else {
            map.insert(key.into(), name.trim().into());
        }
        Ok(c.execute("UPDATE meetings SET speakers=?2 WHERE id=?1", params![id, serde_json::Value::Object(map).to_string()])? > 0)
    }

    /// What the person jotted down during the meeting (their own words: what mattered to them).
    pub fn meeting_notes(&self, id: i64, notes: &str) -> Result<bool> {
        Ok(self.c().execute("UPDATE meetings SET notes=?2 WHERE id=?1", params![id, notes])? > 0)
    }

    pub fn has_meetings(&self) -> bool {
        self.c().query_row("SELECT EXISTS(SELECT 1 FROM meeting_lines)", [], |r| r.get(0)).unwrap_or(false)
    }

    /// Lines that mention `query` (any case), newest meetings first: (meeting id, title, started, ts, who, text).
    #[allow(clippy::type_complexity)]
    pub fn meeting_search(&self, query: &str, limit: usize) -> Result<Vec<(i64, String, i64, i64, String, String)>> {
        let c = self.c();
        let mut st = c.prepare(
            "SELECT m.id, m.title, m.started_at, l.ts, l.who, l.text FROM meeting_lines l JOIN meetings m ON m.id = l.meeting_id
             WHERE instr(lower(l.text), lower(?1)) > 0 ORDER BY m.started_at DESC, l.ts LIMIT ?2",
        )?;
        let v = st.query_map(params![query.trim(), limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn meeting_delete(&self, id: i64) -> Result<bool> {
        let c = self.c();
        c.execute("DELETE FROM meeting_lines WHERE meeting_id=?1", [id])?;
        Ok(c.execute("DELETE FROM meetings WHERE id=?1", [id])? > 0)
    }
}
