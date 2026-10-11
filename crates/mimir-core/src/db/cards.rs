use super::*;

/// A card pinned in a chat: Markdown text, checklists, tables and links (never HTML), replaced whole by its bot.
#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub id: i64,
    pub bot_id: i64,
    pub chat_id: i64,
    /// The routine whose results it shows (delivery "card"), if any.
    pub routine_id: Option<i64>,
    pub title: String,
    pub body: String,
    /// The last change was yours (a ticked item), not the bot's.
    pub by_you: bool,
    /// Last written by a run that had read outside content: reading it counts as reading that.
    pub tainted: bool,
    pub updated_at: i64,
}

/// A chat holds at most this many cards a bot made.
pub const CARDS_PER_CHAT: i64 = 20;

/// A card's title and body, trimmed, or why not: never HTML (the app shows text, checklists, tables and links).
fn card_check<'a>(title: &'a str, body: &'a str) -> Result<(&'a str, &'a str)> {
    let (title, body) = (title.trim(), body.trim());
    if title.is_empty() || title.chars().count() > 80 {
        bail!("a card needs a title of up to 80 characters");
    }
    if body.chars().count() > 6000 {
        bail!("a card holds up to 6,000 characters; keep it to what matters now");
    }
    // ponytail: a tag-shaped `<x` or `</` is refused outright; Markdown never needs one
    if body.as_bytes().windows(2).any(|w| w[0] == b'<' && (w[1].is_ascii_alphabetic() || w[1] == b'/' || w[1] == b'!')) {
        bail!("cards hold text, checklists (- [ ] item), tables and links, not HTML");
    }
    Ok((title, body))
}

/// "- [ ] milk" ⇄ "- [x] milk" (any bullet, any indent); None for a line that isn't a checklist item.
pub(super) fn toggle_box(line: &str) -> Option<String> {
    let rest = line.trim_start();
    let indent = &line[..line.len() - rest.len()];
    let (bullet, rest) = rest.split_once(' ')?;
    if !["-", "*", "•"].contains(&bullet) {
        return None;
    }
    let (mark, item) = (rest.get(..3)?, &rest[3..]);
    let flip = match mark {
        "[ ]" => "[x]",
        "[x]" | "[X]" => "[ ]",
        _ => return None,
    };
    Some(format!("{indent}{bullet} {flip}{item}"))
}

impl Db {
    fn cards_where(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Card>> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT id, bot_id, chat_id, routine_id, title, body, by_you, tainted, updated_at FROM cards WHERE {sql} ORDER BY id"))?;
        let v = st
            .query_map(p, |r| {
                Ok(Card { id: r.get(0)?, bot_id: r.get(1)?, chat_id: r.get(2)?, routine_id: r.get(3)?, title: r.get(4)?, body: r.get(5)?, by_you: r.get(6)?, tainted: r.get(7)?, updated_at: r.get(8)? })
            })?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    /// A chat's cards, oldest first.
    pub fn cards(&self, chat: i64) -> Result<Vec<Card>> {
        self.cards_where("chat_id=?1", [chat])
    }

    /// A bot writes a card whole: the one with this title in this chat, or a new one.
    pub fn card_write(&self, bot: i64, chat: i64, title: &str, body: &str, tainted: bool) -> Result<Card> {
        let (title, body) = card_check(title, body)?;
        let had = self.cards_where("chat_id=?1 AND bot_id=?2 AND lower(title)=lower(?3)", params![chat, bot, title])?.into_iter().next();
        let id = match had {
            Some(k) => k.id,
            None => {
                let n: i64 = self.c().query_row("SELECT COUNT(*) FROM cards WHERE chat_id=?1", [chat], |r| r.get(0))?;
                if n >= CARDS_PER_CHAT {
                    bail!("this chat already has {CARDS_PER_CHAT} cards; update or ask the person to remove one");
                }
                self.c().execute("INSERT INTO cards(bot_id, chat_id, title, body, updated_at) VALUES (?1,?2,?3,'',0)", params![bot, chat, title])?;
                self.c().last_insert_rowid()
            }
        };
        self.c().execute("UPDATE cards SET title=?2, body=?3, by_you=0, tainted=?4, updated_at=?5 WHERE id=?1", params![id, title, body, tainted, now()])?;
        self.cards_where("id=?1", [id])?.pop().context("card gone")
    }

    /// The card a routine delivers to.
    pub fn routine_card(&self, routine: i64) -> Result<Option<Card>> {
        Ok(self.cards_where("routine_id=?1", [routine])?.pop())
    }

    /// A routine's result replaces its card. A first line "# Title" names it; otherwise it keeps its name, or a new
    /// one is named after the routine's prompt.
    pub fn card_from_routine(&self, r: &Routine, reply: &str, tainted: bool) -> Result<Card> {
        let (title, body) = match reply.trim_start().split_once('\n') {
            Some((first, rest)) if first.starts_with("# ") => (first[2..].trim().to_string(), rest),
            _ => (String::new(), reply),
        };
        let had = self.routine_card(r.id)?;
        let title = if !title.is_empty() { title } else if let Some(k) = &had { k.title.clone() } else { crate::tools::truncate(r.prompt.lines().next().unwrap_or("Routine"), 60) };
        // A run's result isn't lost to a stray "<": it shows as a look-alike instead of being refused.
        let body = body.replace('<', "‹");
        let (title, body) = card_check(&title, &body)?;
        match had {
            Some(k) => {
                self.c().execute("UPDATE cards SET title=?2, body=?3, by_you=0, tainted=?4, updated_at=?5 WHERE id=?1", params![k.id, title, body, tainted, now()])?;
                self.cards_where("id=?1", [k.id])?.pop().context("card gone")
            }
            None => {
                self.c().execute(
                    "INSERT INTO cards(bot_id, chat_id, routine_id, title, body, tainted, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    params![r.bot_id, r.chat_id, r.id, title, body, tainted, now()],
                )?;
                let id = self.c().last_insert_rowid();
                self.cards_where("id=?1", [id])?.pop().context("card gone")
            }
        }
    }

    /// You tick (or untick) a checklist item: line `line` (0-based) of the body, which must still read `was`, so a
    /// tick never lands on a different item after the bot rewrote the card. Your edit, not the bot's.
    pub fn card_tick(&self, chat: i64, id: i64, line: usize, was: &str) -> Result<Card> {
        let k = self.cards_where("id=?1 AND chat_id=?2", [id, chat])?.pop().context("no such card")?;
        let mut lines: Vec<String> = k.body.lines().map(String::from).collect();
        let l = lines.get_mut(line).filter(|l| l.as_str() == was).context("the card changed; look again")?;
        *l = toggle_box(l).context("that line isn't a checklist item")?;
        self.c().execute("UPDATE cards SET body=?2, by_you=1, updated_at=?3 WHERE id=?1", params![id, lines.join("\n"), now()])?;
        self.cards_where("id=?1", [id])?.pop().context("card gone")
    }

    pub fn card_delete(&self, chat: i64, id: i64) -> Result<bool> {
        Ok(self.c().execute("DELETE FROM cards WHERE id=?1 AND chat_id=?2", [id, chat])? > 0)
    }
}
