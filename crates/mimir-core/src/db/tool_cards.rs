use super::*;
use serde_json::Value;

/// A tool server's interactive card (MCP Apps), as a reply shows it: the page is fetched from /card/<key>.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCard {
    pub key: String,
    /// The full tool name (server__tool): the card may only call its own server's tools.
    pub tool: String,
    /// What the tool was called with, and what it answered (MCP's CallToolResult), handed to the page once it's up.
    pub input: Value,
    pub result: Value,
}

impl Db {
    #[allow(clippy::too_many_arguments)]
    pub fn tool_card_add(&self, run: i64, chat: i64, bot: i64, tool: &str, html: &str, csp: &str, input: &Value, result: &Value) -> Result<String> {
        let key = crate::random_hex(16)?;
        self.c().execute(
            "INSERT INTO tool_cards(key, run_id, chat_id, bot_id, tool, html, csp, input, result, ts) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![key, run, chat, bot, tool, html, csp, input.to_string(), result.to_string(), now()],
        )?;
        Ok(key)
    }

    /// A run's cards, in the order the tools ran.
    pub fn tool_cards(&self, run: i64) -> Result<Vec<ToolCard>> {
        let c = self.c();
        let mut st = c.prepare("SELECT key, tool, input, result FROM tool_cards WHERE run_id=?1 ORDER BY id")?;
        let v = st
            .query_map([run], |r| {
                let json = |i: usize| r.get::<_, String>(i).map(|s| serde_json::from_str(&s).unwrap_or(Value::Null));
                Ok(ToolCard { key: r.get(0)?, tool: r.get(1)?, input: json(2)?, result: json(3)? })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// The page at /card/<key>, and the CSP it's served with.
    pub fn tool_card_page(&self, key: &str) -> Result<Option<(String, String)>> {
        Ok(self.c().query_row("SELECT html, csp FROM tool_cards WHERE key=?1", [key], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
    }

    /// (chat, bot, tool) of a card, for the calls it makes.
    pub fn tool_card_owner(&self, key: &str) -> Result<Option<(i64, i64, String)>> {
        Ok(self.c().query_row("SELECT chat_id, bot_id, tool FROM tool_cards WHERE key=?1", [key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?)
    }
}
