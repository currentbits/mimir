//! Mimir's skill library: a few curated skills that come with the app, written for its tools (ideas borrowed from other
//! assistants' skill collections, the text our own). Nothing is installed on its own: you add one to a bot, or to all
//! bots, in a chat, and it becomes an accepted skill like any other (you can change, share or remove it). Plain text
//! only: no scripts, no install steps, nothing fetched.

/// One skill from the library. `needs`: tools it can't do without (a bot without them gets a warning, not the skill).
/// `schedule`: a suggested routine (cron, in words) for skills that are a habit, offered after adding.
pub struct LibrarySkill {
    pub slug: &'static str,
    pub category: &'static str,
    pub needs: &'static [&'static str],
    pub schedule: Option<(&'static str, &'static str)>,
    pub name: String,
    pub description: String,
    pub body: String,
}

type Entry = (&'static str, &'static str, &'static [&'static str], Option<(&'static str, &'static str)>, &'static str);
const LIST: [Entry; 24] = [
    ("morning-briefing", "Every day", &["memory_search"], Some(("30 7 * * Mon-Fri", "Weekdays at 07:30")), include_str!("../skills/morning-briefing.md")),
    ("inbox-triage", "Every day", &["email_search"], None, include_str!("../skills/inbox-triage.md")),
    ("follow-up-tracker", "Every day", &["memory_search", "chat_search"], Some(("0 9 * * Mon", "Mondays at 09:00")), include_str!("../skills/follow-up-tracker.md")),
    ("weekly-review", "Every day", &["memory_search", "chat_search"], Some(("0 16 * * Fri", "Fridays at 16:00")), include_str!("../skills/weekly-review.md")),
    ("meeting-prep", "Meetings", &["calendar_list"], None, include_str!("../skills/meeting-prep.md")),
    ("meeting-follow-up", "Meetings", &["meeting_read"], None, include_str!("../skills/meeting-follow-up.md")),
    ("research-brief", "Research", &["web_fetch"], None, include_str!("../skills/research-brief.md")),
    ("summarize-link", "Research", &["web_fetch"], None, include_str!("../skills/summarize-link.md")),
    ("deep-research", "Research", &["web_fetch", "file_write"], None, include_str!("../skills/deep-research.md")),
    ("chart-check", "Research", &["web_fetch"], None, include_str!("../skills/chart-check.md")),
    ("price-watch", "Keeping watch", &["web_fetch", "routine_create"], None, include_str!("../skills/price-watch.md")),
    ("topic-watch", "Keeping watch", &["web_fetch", "routine_create"], None, include_str!("../skills/topic-watch.md")),
    ("write-like-me", "Writing and thinking", &["chat_search"], None, include_str!("../skills/write-like-me.md")),
    ("decision-memo", "Writing and thinking", &[], None, include_str!("../skills/decision-memo.md")),
    ("document-to-actions", "Writing and thinking", &["memory_store"], None, include_str!("../skills/document-to-actions.md")),
    ("make-it-human", "Writing and thinking", &[], None, include_str!("../skills/make-it-human.md")),
    ("seo-check", "Writing and thinking", &[], None, include_str!("../skills/seo-check.md")),
    ("prompt-polish", "Writing and thinking", &[], None, include_str!("../skills/prompt-polish.md")),
    ("trip-planner", "Planning", &["web_fetch"], None, include_str!("../skills/trip-planner.md")),
    ("landing-page", "Making things", &["web_fetch", "file_write"], None, include_str!("../skills/landing-page.md")),
    ("hire-a-bot", "Your bots", &["bot_create"], None, include_str!("../skills/hire-a-bot.md")),
    ("bot-role-check", "Your bots", &[], None, include_str!("../skills/bot-role-check.md")),
    ("team-check", "Your bots", &["chat_search"], None, include_str!("../skills/team-check.md")),
    ("memory-check", "Your bots", &["memory_search"], None, include_str!("../skills/memory-check.md")),
];

pub fn all() -> Vec<LibrarySkill> {
    LIST.iter()
        .filter_map(|(slug, category, needs, schedule, md)| {
            let (name, description, body) = crate::tidy::parse_skill_md(md)?;
            Some(LibrarySkill { slug, category, needs, schedule: *schedule, name, description, body })
        })
        .collect()
}

pub fn get(slug: &str) -> Option<LibrarySkill> {
    all().into_iter().find(|k| k.slug == slug)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_library_skill_parses_and_has_the_shape() {
        let all = super::all();
        assert_eq!(all.len(), super::LIST.len(), "a skill file didn't parse");
        let tools: Vec<String> = crate::tools::specs().into_iter().map(|t| t.0.to_string()).collect();
        for k in &all {
            for label in ["**When to use:**", "**You'll need:**", "**Steps:**", "**Check:**", "**Return:**", "**Stop and ask if:**"] {
                assert!(k.body.contains(label), "{} lacks {label}", k.slug);
            }
            assert!(k.description.contains("Not for") || k.description.contains("Not to"), "{}: say what it's not for", k.slug);
            assert!(k.needs.iter().all(|t| tools.contains(&t.to_string())), "{}: unknown tool", k.slug);
            assert!(k.body.chars().count() < 4000 && k.description.chars().count() < 300, "{} too long", k.slug);
            assert!(crate::tools::instruction_like(&k.body).is_none(), "{}: reads like an injection", k.slug);
            if let Some((cron, _)) = k.schedule {
                assert!(crate::db::min_gap_minutes(cron).unwrap() >= 60, "{}: a sane schedule", k.slug);
            }
        }
    }
}
