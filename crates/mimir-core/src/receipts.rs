//! The log is the truth: a reply checked against what its run actually did. No model, no network; it only adds
//! lines under the reply (receipts, sources, and a note where the words and the log disagree), never changes it.

use crate::db::{Db, ToolCall};

/// The note the app shows a Set it button under (it asks the bot to set the reminder it promised).
pub const NO_REMINDER: &str = "I said I'd remind you, but no reminder was set.";

/// Did this call really happen? Not refused, not a rehearsal, not parked waiting for an OK.
fn done(c: &ToolCall) -> bool {
    c.ok && c.decision != "deny" && c.decision != "rehearsal" && !c.result.starts_with("Not done yet")
}

/// "Sent an email to ana@…", "Added Tue 14 Oct 15:00, "Dentist"", "Reminder set for …": the side effects, from their logged summaries.
fn receipt(c: &ToolCall) -> Option<String> {
    let s = &c.summary;
    match c.tool.as_str() {
        "email_send" => Some(format!("Sent an email to {}", s.strip_prefix("Send an email to ")?.split(": \"").next()?)),
        // "Add to Home: Tue 14 Oct 15:00, "Dentist" at …": when and what, up to the title's closing quote.
        "calendar_create" => {
            let rest = s.split_once(": ")?.1;
            let end = rest.match_indices('"').nth(1).map_or(rest.len(), |(i, _)| i + 1);
            Some(format!("Added {}", &rest[..end]))
        }
        "routine_create" => Some(match s.strip_prefix("Once at ") {
            Some(r) => format!("Reminder set for {}", r.split(": ").next().unwrap_or(r)),
            None if s.starts_with("Follow-up at ") => format!("Follow-up booked for {}", s["Follow-up at ".len()..].split(": ").next().unwrap_or_default()),
            // "Create routine [0 8 * * *] Brief me": what it does (its schedule is on its chip in the app).
            None => format!("Routine set: {}", s.split_once("] ").map_or(s.as_str(), |(_, p)| p)),
        }),
        _ => None,
    }
}

/// What the reply says it did, and the calls that would back it up. English only.
// ponytail: phrase lists, first person and past tense (or a promise); a claim in other words isn't checked
type Claim = (&'static [&'static str], fn(&str) -> bool, &'static str);
const CLAIMS: &[Claim] = &[
    (
        &["i've sent", "i have sent", "i sent it", "i sent the", "i sent an", "i sent a ", "email sent", "email has been sent", "message sent", "i've emailed", "i emailed", "i've forwarded"],
        |t| t == "email_send" || (t.contains("__") && ["send", "post", "message", "mail"].iter().any(|w| t.contains(w))),
        "I said I sent it, but nothing was sent.",
    ),
    (
        &["i've booked", "i have booked", "i booked", "booked it", "you're booked", "added it to your calendar", "been added to your calendar", "in your calendar now", "is in your calendar"],
        |t| t == "calendar_create" || t.contains("__") || t.starts_with("computer_"),
        "I said it's booked or in your calendar, but nothing was booked or added.",
    ),
    (
        &["i'll remind you", "i will remind you", "i'll ping you", "i'll nudge you", "i'll check back", "i will check back", "i'll follow up", "i will follow up", "reminder set", "reminder is set", "i've set a reminder", "i set a reminder", "i've set up a reminder", "i'll send you a reminder"],
        |t| t == "routine_create" || t == "routine_update",
        NO_REMINDER,
    ),
];

/// A link as it's compared: no scheme, "www.", fragment or trailing slash, lower case.
fn norm(url: &str) -> String {
    let u = url.to_lowercase();
    let u = u.split_once("://").map_or(u.as_str(), |(_, r)| r);
    let u = u.strip_prefix("www.").unwrap_or(u);
    u.split('#').next().unwrap_or(u).trim_end_matches('/').to_string()
}

/// The http(s) links in a text (a Markdown link's target too), without trailing punctuation.
fn links(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for (i, _) in text.match_indices("http") {
        let rest = &text[i..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            continue;
        }
        let end = rest.find(|c: char| c.is_whitespace() || "<>()[]\"'`|".contains(c)).unwrap_or(rest.len());
        let url = rest[..end].trim_end_matches(|c: char| ".,;:!?*_".contains(c));
        if url.len() > "https://".len() && !out.iter().any(|u| u == url) {
            out.push(url.to_string());
        }
    }
    out
}

/// Text as quotes are compared: lower-case words only, so spacing, punctuation, Markdown and entities don't count.
fn words(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        // "&rsquo;", "&#8217;": an entity strip_html left is a mark, not a word.
        let entity = rest.strip_prefix('&').and_then(|r| r.find(';').filter(|&n| n > 0 && n < 9 && r[..n].chars().all(|c| c == '#' || c.is_ascii_alphanumeric())));
        match entity {
            Some(n) => rest = &rest[n + 2..],
            None => {
                plain.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    plain.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The quoted spans in a reply: "…", “…”, ‘…’, '…' or «…». A quote opens after a non-letter and closes before one, so
/// "don't" and "users' rights" aren't quotes.
// ponytail: quote marks only; a Markdown blockquote ("> …") isn't checked
fn quotes(reply: &str) -> Vec<&str> {
    const PAIRS: &[(char, &[char])] = &[('"', &['"', '”']), ('“', &['”', '"']), ('‘', &['’', '\'']), ('\'', &['\'', '’']), ('«', &['»'])];
    let mut out = vec![];
    let mut prev = ' ';
    let mut i = 0;
    while let Some(c) = reply[i..].chars().next() {
        i += c.len_utf8();
        let Some((_, closers)) = PAIRS.iter().find(|(o, _)| *o == c).filter(|_| !prev.is_alphanumeric()) else {
            prev = c;
            continue;
        };
        let close = reply[i..].char_indices().find(|&(j, d)| closers.contains(&d) && !reply[i + j + d.len_utf8()..].starts_with(char::is_alphanumeric));
        match close {
            Some((j, d)) => {
                out.push(&reply[i..i + j]);
                i += j + d.len_utf8();
                prev = d;
            }
            None => prev = c,
        }
    }
    out
}

/// Quotes of 8 words or more that aren't in the pages this run read (or anything else it was given or got back).
/// Each part between "…" is looked for on its own: an ellipsis is a cut, not a claim.
fn misquoted<'a>(reply: &'a str, read: &str) -> Vec<&'a str> {
    // Padded, so a quote's first and last words match whole words.
    let read = format!(" {} ", words(read));
    quotes(reply)
        .into_iter()
        .filter(|q| q.split('…').flat_map(|p| p.split("...")).map(words).any(|p| p.split(' ').count() >= 8 && !read.contains(&format!(" {p} "))))
        .collect()
}

/// The lines added under a reply: receipts and sources (for a messaging app; the app shows them as chips), then a
/// note for each claim with no successful call behind it, the links it gives that it never opened or found, and the
/// quotes it gives from pages it read that aren't on them.
/// `asked` is what the run was given: links from there are the person's own and aren't flagged.
pub fn footer(calls: &[ToolCall], asked: &str, reply: &str, receipts: bool) -> String {
    let done: Vec<&ToolCall> = calls.iter().filter(|c| done(c)).collect();
    let mut out = vec![];
    if receipts {
        out.extend(done.iter().filter_map(|c| receipt(c)).map(|r| format!("✓ {r}")));
        let mut sources: Vec<String> = vec![];
        for c in &done {
            let s = match c.tool.as_str() {
                "web_fetch" => c.summary.strip_prefix("Read ").map(|u| norm(u).split('/').next().unwrap_or_default().to_string()),
                "web_search" => c.summary.strip_prefix("Search the web: ").map(|q| format!("a search for \"{q}\"")),
                "x_search" => c.summary.strip_prefix("Search X: ").map(|q| format!("an X search for \"{q}\"")),
                _ => None,
            };
            if let Some(s) = s.filter(|s| !s.is_empty() && !sources.contains(s)) {
                sources.push(s);
            }
        }
        if !sources.is_empty() {
            out.push(format!("Sources: {}", sources.join(", ")));
        }
    }
    let mut notes = vec![];
    let said = reply.to_lowercase().replace('’', "'");
    for (phrases, backs, note) in CLAIMS {
        if phrases.iter().any(|p| said.contains(p)) && !done.iter().any(|c| backs(&c.tool)) {
            let fix = if *note == NO_REMINDER && receipts { " Reply \"set it\" and I will." } else { "" };
            notes.push(format!("_Not done in this reply: {note}{fix}_"));
        }
    }
    let seen = done.iter().map(|c| format!("{}\n{}", c.summary, c.result).to_lowercase()).collect::<Vec<_>>().join("\n") + "\n" + &asked.to_lowercase();
    let unread: Vec<String> = links(reply).into_iter().filter(|u| !seen.contains(&norm(u))).collect();
    if !unread.is_empty() {
        notes.push(format!("_I didn't open {} in this reply, so I haven't checked {}: {}_", if unread.len() == 1 { "this link" } else { "these links" }, if unread.len() == 1 { "it" } else { "them" }, unread.join(", ")));
    }
    // Only for a reply that read pages: a quote from memory or an email isn't the web's to check.
    if done.iter().any(|c| c.tool == "web_fetch") {
        let read = done.iter().map(|c| c.result.as_str()).chain([asked]).collect::<Vec<_>>().join("\n");
        for q in misquoted(reply, &read) {
            let q = q.trim();
            let short = q.char_indices().nth(120).map_or(q.to_string(), |(i, _)| format!("{}…", &q[..i]));
            notes.push(format!("_Not found on the pages read: \"{short}\"_"));
        }
    }
    [out.join("\n"), notes.join("\n")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n")
}

/// What the run log shows was done (a sent email, a calendar entry), for a goal to be marked done on: booking a
/// follow-up or a routine is more work, not a result.
pub fn backing(calls: &[ToolCall]) -> Vec<String> {
    calls.iter().filter(|c| done(c) && c.tool != "routine_create").filter_map(receipt).collect()
}

/// Every call a run made, and those of the runs it handed tasks to (what another bot did counts for the reply).
pub fn calls_of(db: &Db, run: i64) -> anyhow::Result<Vec<ToolCall>> {
    let mut calls = db.tool_calls(run)?;
    for child in db.child_runs(run)? {
        calls.extend(db.tool_calls(child.id)?);
    }
    Ok(calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(tool: &str, summary: &str, decision: &str, ok: bool, result: &str) -> ToolCall {
        ToolCall { id: 0, tool: tool.into(), summary: summary.into(), decision: decision.into(), ok, result: result.into(), ts: 0 }
    }

    #[test]
    fn receipts_and_notes() {
        let sent = call("email_send", "Send an email to ana@example.com: \"Invoice\" — paid", "ask", true, "Sent");
        let cal = call("calendar_create", "Add to Home: Tue 14 Oct 15:00, \"Dentist\" at Main St", "ask", true, "added");
        let read = call("web_fetch", "Read https://www.example.com/news/", "allow", true, "…");
        let f = footer(&[sent.clone(), cal, read], "", "I've sent it, booked the dentist. See https://example.com/news.", true);
        assert_eq!(f, "✓ Sent an email to ana@example.com\n✓ Added Tue 14 Oct 15:00, \"Dentist\"\nSources: example.com", "{f}");
        assert_eq!(footer(&[sent], "", "Done, I've sent it.", false), "", "the app shows chips instead; nothing to flag");

        // Claims with nothing behind them: refused, parked for an OK, or never called.
        let parked = call("email_send", "Send an email to ana@example.com: \"x\" — y", "ask", true, "Not done yet: this needs the user's OK…");
        let f = footer(&[parked], "", "I’ve sent the email and I'll remind you on Friday.", true);
        assert!(f.contains("nothing was sent") && f.contains(NO_REMINDER) && f.contains("\"set it\""), "{f}");
        assert!(!f.contains('✓'), "a parked call is no receipt: {f}");
        let reminded = call("routine_create", "Once at 2026-10-17 09:00: call mum", "allow", true, "routine #4 created");
        assert_eq!(footer(std::slice::from_ref(&reminded), "", "I'll remind you on Friday.", false), "");
        assert_eq!(footer(&[reminded], "", "I'll remind you on Friday.", true), "✓ Reminder set for 2026-10-17 09:00");
        let routine = call("routine_create", "Create routine [0 8 * * *] Brief me on tech news", "allow", true, "routine #5 created");
        assert_eq!(footer(&[routine], "", "Done.", true), "✓ Routine set: Brief me on tech news");
        assert!(!footer(&[], "", "I'll remind you.", false).contains("\"set it\""), "the app has a button");
        assert_eq!(footer(&[], "", "I haven't sent anything yet. Shall I?", true), "");

        // Links: what it fetched or found counts, the person's own links count, anything else is listed.
        let search = call("web_search", "Search the web: rust", "allow", true, "https://blog.rust-lang.org/1.99");
        let f = footer(&[search], "see https://example.org/x", "[Rust 1.99](https://blog.rust-lang.org/1.99/) and https://example.org/x, also https://made.up/page.", true);
        assert!(f.ends_with("I didn't open this link in this reply, so I haven't checked it: https://made.up/page_"), "{f}");
        assert!(f.starts_with("Sources: a search for \"rust\""), "{f}");
    }

    #[test]
    fn quotes_checked_against_pages_read() {
        let page = call("web_fetch", "Read https://shop.example/terms", "allow", true, "Refunds. Customers may return &rsquo;unused&rsquo; items within   thirty days of delivery for a full refund. Sale items can't be returned.");
        let f = |reply: &str| footer(std::slice::from_ref(&page), "", reply, false);
        // On the page, give or take case, spacing, punctuation, entities and a cut.
        assert_eq!(f("The terms say “customers may return unused items within thirty days of delivery.”"), "");
        assert_eq!(f("They say: 'Customers may return unused items within thirty days … for a full refund'"), "");
        // Not on it: the note quotes it; the reply stays as it was.
        let made_up = "They say 'customers may return any item within ninety days for a full refund' and don't charge.";
        assert_eq!(f(made_up), "_Not found on the pages read: \"customers may return any item within ninety days for a full refund\"_");
        // Short quotes, apostrophes, and replies that read no pages aren't checked.
        assert_eq!(f("It says \"sale items\" can't be returned, and the users' rights don't change."), "");
        assert_eq!(footer(&[], "", made_up, false), "");
        // A whole word only: "ustomers" isn't "customers".
        assert!(!f("\"ustomers may return unused items within thirty days of\"").is_empty());
        // What the person sent counts as read.
        assert_eq!(footer(std::slice::from_ref(&page), "my note: we never refund opened boxes after the first week", "You wrote \"we never refund opened boxes after the first week\".", false), "");
    }
}
