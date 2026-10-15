//! Bot replies are Markdown; each messaging app wants its own dialect.
//! Telegram → rich message blocks (HTML when those aren't taken), Slack → mrkdwn, Signal → plain text. Discord speaks
//! Markdown already. Covers what chat models write: paragraphs, lists, headings, quotes, code, tables, bold/italic, links.

use serde_json::{Value, json};

#[derive(Debug, PartialEq)]
enum Block {
    Para(String),
    Heading(usize, String),
    Quote(String),
    Code(String),
    List { ordered: bool, items: Vec<String> },
    /// The header row first; the |---| line under it is gone.
    Table(Vec<Vec<String>>),
}

/// "| a | b |" → ["a", "b"].
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_string()).collect()
}

/// The |---|:--:| line under a table's header.
fn table_rule(line: &str) -> bool {
    line.trim().starts_with('|') && line.contains('-') && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

#[derive(Debug, PartialEq)]
enum Inline {
    Text(String),
    Code(String),
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Link(Vec<Inline>, String),
}

fn list_item(line: &str) -> Option<(bool, &str)> {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("• ")) {
        return Some((false, rest));
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    let rest = &t[digits..];
    (digits > 0 && (rest.starts_with(". ") || rest.starts_with(") "))).then(|| (true, &rest[2..]))
}

fn blocks(md: &str) -> Vec<Block> {
    let lines: Vec<&str> = md.lines().collect();
    let mut out = vec![];
    let mut para: Vec<&str> = vec![];
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(para.join("\n")));
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim_start().starts_with("```") {
            flush(&mut para, &mut out);
            let mut code = vec![];
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            out.push(Block::Code(code.join("\n")));
        } else if line.trim_start().starts_with('|') && lines.get(i + 1).is_some_and(|l| table_rule(l)) {
            flush(&mut para, &mut out);
            let mut rows = vec![cells(line)];
            i += 2;
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                rows.push(cells(lines[i]));
                i += 1;
            }
            out.push(Block::Table(rows));
            continue;
        } else if let Some(h) = line.trim_start().strip_prefix('#').map(|r| r.trim_start_matches('#')).filter(|r| r.starts_with(' ')) {
            flush(&mut para, &mut out);
            out.push(Block::Heading(line.trim_start().len() - h.len(), h.trim().into()));
        } else if let Some((ordered, _)) = list_item(line) {
            flush(&mut para, &mut out);
            let mut items: Vec<String> = vec![];
            while i < lines.len() {
                match list_item(lines[i]) {
                    Some((o, item)) if o == ordered => items.push(item.into()),
                    None if lines[i].starts_with("  ") && !lines[i].trim().is_empty() && !items.is_empty() => {
                        let last = items.last_mut().unwrap();
                        last.push(' ');
                        last.push_str(lines[i].trim());
                    }
                    _ => break,
                }
                i += 1;
            }
            out.push(Block::List { ordered, items });
            continue;
        } else if let Some(q) = line.strip_prefix('>') {
            flush(&mut para, &mut out);
            out.push(Block::Quote(q.trim_start().into()));
        } else if line.trim().is_empty() {
            flush(&mut para, &mut out);
        } else {
            para.push(line);
        }
        i += 1;
    }
    flush(&mut para, &mut out);
    out
}

/// Inline spans. Anything that doesn't close stays literal text.
fn inline(s: &str) -> Vec<Inline> {
    let c: Vec<char> = s.chars().collect();
    let mut out = vec![];
    let mut text = String::new();
    // A pattern not found after position x isn't found after any later position either: remembering that keeps long
    // text full of unmatched '[' or '*' linear instead of quadratic.
    let missed: std::cell::RefCell<std::collections::HashMap<Vec<char>, usize>> = Default::default();
    let find = |from: usize, pat: &[char]| {
        if missed.borrow().get(pat).is_some_and(|&m| from >= m) {
            return None;
        }
        let hit = (from..c.len().saturating_sub(pat.len() - 1)).find(|&j| c[j..j + pat.len()] == *pat);
        if hit.is_none() {
            let mut m = missed.borrow_mut();
            let e = m.entry(pat.to_vec()).or_insert(usize::MAX);
            *e = (*e).min(from);
        }
        hit
    };
    let mut i = 0;
    while i < c.len() {
        let push = |text: &mut String, out: &mut Vec<Inline>| {
            if !text.is_empty() {
                out.push(Inline::Text(std::mem::take(text)));
            }
        };
        let word_before = i > 0 && c[i - 1].is_alphanumeric();
        if c[i] == '`' && let Some(j) = find(i + 1, &['`']) {
            push(&mut text, &mut out);
            out.push(Inline::Code(c[i + 1..j].iter().collect()));
            i = j + 1;
        } else if c[i..].starts_with(&['*', '*']) && let Some(j) = find(i + 2, &['*', '*']).filter(|&j| j > i + 2) {
            push(&mut text, &mut out);
            out.push(Inline::Bold(inline(&c[i + 2..j].iter().collect::<String>())));
            i = j + 2;
        } else if (c[i] == '*' || (c[i] == '_' && !word_before))
            && c.get(i + 1).is_some_and(|n| !n.is_whitespace() && *n != c[i])
            && let Some(j) = find(i + 1, &[c[i]]).filter(|&j| j > i + 1 && (c[i] == '*' || !c.get(j + 1).is_some_and(|n| n.is_alphanumeric())))
        {
            push(&mut text, &mut out);
            out.push(Inline::Italic(inline(&c[i + 1..j].iter().collect::<String>())));
            i = j + 1;
        } else if c[i] == '['
            && let Some(close) = find(i + 1, &[']', '('])
            && let Some(end) = find(close + 2, &[')'])
            && c[close + 2..end].iter().collect::<String>().starts_with("http")
        {
            push(&mut text, &mut out);
            out.push(Inline::Link(inline(&c[i + 1..close].iter().collect::<String>()), c[close + 2..end].iter().collect()));
            i = end + 1;
        } else {
            text.push(c[i]);
            i += 1;
        }
    }
    if !text.is_empty() {
        out.push(Inline::Text(text));
    }
    out
}

pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

// ---------------- Telegram (HTML) ----------------

fn tg_inline(v: &[Inline]) -> String {
    v.iter()
        .map(|x| match x {
            Inline::Text(t) => esc(t),
            Inline::Code(t) => format!("<code>{}</code>", esc(t)),
            Inline::Bold(v) => format!("<b>{}</b>", tg_inline(v)),
            Inline::Italic(v) => format!("<i>{}</i>", tg_inline(v)),
            Inline::Link(v, url) => format!("<a href=\"{}\">{}</a>", esc(url).replace('"', "&quot;"), tg_inline(v)),
        })
        .collect()
}

fn tg_block(b: &Block) -> String {
    match b {
        Block::Para(t) => tg_inline(&inline(t)),
        Block::Heading(_, t) => format!("<b>{}</b>", tg_inline(&inline(t))),
        Block::Quote(t) => format!("<blockquote>{}</blockquote>", tg_inline(&inline(t))),
        Block::Table(rows) => rows.iter().map(|r| r.iter().map(|c| tg_inline(&inline(c))).collect::<Vec<_>>().join(" | ")).collect::<Vec<_>>().join("\n"),
        Block::Code(t) => format!("<pre>{}</pre>", esc(t)),
        Block::List { ordered, items } => items
            .iter()
            .enumerate()
            .map(|(n, it)| format!("{} {}", if *ordered { format!("{}.", n + 1) } else { "•".into() }, tg_inline(&inline(it))))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Telegram HTML, packed into messages of at most `max` chars, split only between blocks.
pub fn telegram(md: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for b in blocks(md).iter().map(tg_block) {
        match out.last_mut() {
            Some(last) if last.chars().count() + b.chars().count() + 2 <= max => {
                last.push_str("\n\n");
                last.push_str(&b);
            }
            _ => out.push(b),
        }
    }
    out
}

// ---------------- Telegram (rich message blocks, Bot API 10.1) ----------------
// Explicit blocks, never Telegram's own Markdown/HTML parsing of the reply: that would take ![](https://…) or <img> as
// a media block Telegram fetches itself, so a link a misled bot wrote could carry data out unclicked. Here only text,
// formatting and links ever reach Telegram, and links open only when tapped (after Telegram's "Open this link?").

fn rich_inline(v: &[Inline]) -> Value {
    Value::Array(
        v.iter()
            .map(|x| match x {
                Inline::Text(t) => json!(t),
                Inline::Code(t) => json!({"type": "code", "text": t}),
                Inline::Bold(v) => json!({"type": "bold", "text": rich_inline(v)}),
                Inline::Italic(v) => json!({"type": "italic", "text": rich_inline(v)}),
                Inline::Link(v, url) => json!({"type": "url", "text": rich_inline(v), "url": url}),
            })
            .collect(),
    )
}

fn rich_para(t: &str) -> Value {
    json!({"type": "paragraph", "text": rich_inline(&inline(t))})
}

fn rich_block(b: &Block) -> Value {
    match b {
        Block::Para(t) => rich_para(t),
        Block::Heading(n, t) => json!({"type": "heading", "text": rich_inline(&inline(t)), "size": (*n).clamp(1, 6)}),
        Block::Quote(t) => json!({"type": "blockquote", "blocks": [rich_para(t)]}),
        Block::Code(t) => json!({"type": "pre", "text": t}),
        Block::List { ordered, items } => json!({"type": "list", "items": items.iter().enumerate().map(|(n, it)| {
            // Task lists: "- [ ] call Sam", "- [x] book flights".
            let task = it.strip_prefix("[ ] ").map(|t| (t, false)).or_else(|| it.strip_prefix("[x] ").or_else(|| it.strip_prefix("[X] ")).map(|t| (t, true)));
            let mut item = json!({"blocks": [rich_para(task.map_or(it.as_str(), |(t, _)| t))]});
            if let Some((_, done)) = task {
                item["has_checkbox"] = json!(true);
                if done {
                    item["is_checked"] = json!(true);
                }
            }
            if *ordered {
                item["value"] = json!(n + 1);
                item["type"] = json!("1");
            }
            item
        }).collect::<Vec<_>>()}),
        Block::Table(rows) => json!({"type": "table", "is_bordered": true, "cells": rows.iter().enumerate().map(|(r, row)| {
            row.iter().map(|c| json!({"text": rich_inline(&inline(c)), "is_header": r == 0, "align": "left", "valign": "top"})).collect::<Vec<_>>()
        }).collect::<Vec<_>>()}),
    }
}

/// Telegram rich messages (`rich_message` for sendRichMessage), split between blocks under Telegram's limits
/// (32768 characters, 500 blocks): counted on the JSON, which only overcounts.
pub fn telegram_rich(md: &str) -> Vec<Value> {
    let (mut out, mut size) = (vec![], (0, 0));
    for b in blocks(md).iter().map(rich_block) {
        let s = b.to_string();
        let cost = (s.chars().count(), s.matches('{').count());
        match out.last_mut() {
            Some(Value::Array(last)) if size.0 + cost.0 <= 30_000 && size.1 + cost.1 <= 450 => {
                last.push(b);
                size = (size.0 + cost.0, size.1 + cost.1);
            }
            _ => {
                out.push(json!([b]));
                size = cost;
            }
        }
    }
    out.into_iter().map(|blocks| json!({"blocks": blocks})).collect()
}

/// Our Telegram HTML back to readable plain text (fallback when Telegram rejects the markup).
pub fn html_to_plain(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&")
}

// ---------------- Slack (mrkdwn) ----------------

fn slack_inline(v: &[Inline]) -> String {
    v.iter()
        .map(|x| match x {
            Inline::Text(t) => esc(t),
            Inline::Code(t) => format!("`{}`", esc(t)),
            Inline::Bold(v) => format!("*{}*", slack_inline(v)),
            Inline::Italic(v) => format!("_{}_", slack_inline(v)),
            // A link's address can't close the <…|…> early (and smuggle in <!channel>).
            Inline::Link(v, url) => format!("<{}|{}>", url.replace('<', "%3C").replace('>', "%3E").replace('|', "%7C"), slack_inline(v).replace('|', "¦")),
        })
        .collect()
}

pub fn slack(md: &str) -> String {
    blocks(md)
        .iter()
        .map(|b| match b {
            Block::Para(t) => slack_inline(&inline(t)),
            Block::Heading(_, t) => format!("*{}*", slack_inline(&inline(t))),
            Block::Quote(t) => format!("> {}", slack_inline(&inline(t))),
            Block::Code(t) => format!("```{}```", esc(t)),
            Block::List { ordered, items } => items
                .iter()
                .enumerate()
                .map(|(n, it)| format!("{} {}", if *ordered { format!("{}.", n + 1) } else { "•".into() }, slack_inline(&inline(it))))
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Table(rows) => rows.iter().map(|r| r.iter().map(|c| slack_inline(&inline(c))).collect::<Vec<_>>().join(" | ")).collect::<Vec<_>>().join("\n"),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ---------------- Plain text (Signal) ----------------

fn plain_inline(v: &[Inline]) -> String {
    v.iter()
        .map(|x| match x {
            Inline::Text(t) | Inline::Code(t) => t.clone(),
            Inline::Bold(v) | Inline::Italic(v) => plain_inline(v),
            Inline::Link(v, url) => match plain_inline(v) {
                t if t == *url => t,
                t => format!("{t} ({url})"),
            },
        })
        .collect()
}

/// Readable text without markup, for apps that show none (Signal): links keep their address.
pub fn plain(md: &str) -> String {
    blocks(md)
        .iter()
        .map(|b| match b {
            Block::Para(t) | Block::Heading(_, t) => plain_inline(&inline(t)),
            Block::Quote(t) => format!("> {}", plain_inline(&inline(t))),
            Block::Code(t) => t.clone(),
            Block::List { ordered, items } => items
                .iter()
                .enumerate()
                .map(|(n, it)| format!("{} {}", if *ordered { format!("{}.", n + 1) } else { "•".into() }, plain_inline(&inline(it))))
                .collect::<Vec<_>>()
                .join("\n"),            Block::Table(rows) => rows.iter().map(|r| r.iter().map(|c| plain_inline(&inline(c))).collect::<Vec<_>>().join(" | ")).collect::<Vec<_>>().join("\n"),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text() {
        let s = plain(REPLY);
        assert!(s.starts_with("Today\n\nTwo things:\n\n1. Send Acme the invoice — due <today> & soon."), "{s}");
        assert!(s.contains("Dr. Lee (https://lee.example/book?a=1&b=2)"), "{s}");
        assert!(s.contains("• snake_case_names stay as is") && s.ends_with("let x = a < b;"), "{s}");
    }

    const REPLY: &str = "## Today\nTwo things:\n\n1. **Send Acme** the `invoice` — due <today> & soon.\n2. Book the dentist (see [Dr. Lee](https://lee.example/book?a=1&b=2)).\n\n- snake_case_names stay *as is*\n\n> remember the umbrella\n\n```\nlet x = a < b;\n```";

    #[test]
    fn long_unmatched_text_stays_fast() {
        let t = std::time::Instant::now();
        let _ = super::telegram(&"[".repeat(100_000), 4000);
        let _ = super::telegram(&"*a ".repeat(50_000), 4000);
        assert!(t.elapsed().as_millis() < 1500, "took {:?}", t.elapsed());
    }

    #[test]
    fn slack_links_cant_ping() {
        let out = super::slack("[x](http://a><!channel>)");
        assert!(!out.contains("<!channel>"), "{out}");
    }

    #[test]
    fn telegram_html() {
        let html = telegram(REPLY, 4000).join("\n---\n");
        assert_eq!(
            html,
            "<b>Today</b>\n\nTwo things:\n\n\
             1. <b>Send Acme</b> the <code>invoice</code> — due &lt;today&gt; &amp; soon.\n\
             2. Book the dentist (see <a href=\"https://lee.example/book?a=1&amp;b=2\">Dr. Lee</a>).\n\n\
             • snake_case_names stay <i>as is</i>\n\n\
             <blockquote>remember the umbrella</blockquote>\n\n\
             <pre>let x = a &lt; b;</pre>"
        );
    }

    #[test]
    fn telegram_splits_between_blocks() {
        let md = format!("{}\n\n{}", "a".repeat(30), "b".repeat(30));
        assert_eq!(telegram(&md, 40), vec!["a".repeat(30), "b".repeat(30)]);
        assert_eq!(telegram(&md, 100).len(), 1);
    }

    #[test]
    fn slack_mrkdwn() {
        let s = slack(REPLY);
        assert!(s.starts_with("*Today*\n\nTwo things:"), "{s}");
        assert!(s.contains("1. *Send Acme* the `invoice` — due &lt;today&gt; &amp; soon."), "{s}");
        assert!(s.contains("<https://lee.example/book?a=1&b=2|Dr. Lee>"), "{s}");
        assert!(s.contains("• snake_case_names stay _as is_"), "{s}");
        assert!(s.contains("```let x = a &lt; b;```"), "{s}");
    }

    #[test]
    fn telegram_rich_blocks() {
        let md = "# Plan\n\n| Day | **What** |\n|:--|--:|\n| Mon | [gym](https://g.example) |\n\n- [ ] call Sam\n- [x] book\n\n1. one\n\n![x](https://evil.example/?d=secret) <img src=https://evil.example>";
        let parts = telegram_rich(md);
        assert_eq!(parts.len(), 1);
        let b = &parts[0]["blocks"];
        assert_eq!(b[0], json!({"type": "heading", "text": ["Plan"], "size": 1}));
        assert_eq!(b[1]["type"], "table");
        assert_eq!(b[1]["cells"][0][1]["text"], json!([{"type": "bold", "text": ["What"]}]));
        assert_eq!(b[1]["cells"][0][1]["is_header"], true);
        assert_eq!(b[1]["cells"][1][1]["text"], json!([{"type": "url", "text": ["gym"], "url": "https://g.example"}]));
        assert_eq!(b[1]["cells"].as_array().unwrap().len(), 2, "the |---| rule isn't a row");
        assert_eq!(b[2]["items"][0], json!({"blocks": [{"type": "paragraph", "text": ["call Sam"]}], "has_checkbox": true}));
        assert_eq!(b[2]["items"][1]["is_checked"], true);
        assert_eq!(b[3]["items"][0]["value"], 1);
        // An image or HTML in the reply is text, never a media block Telegram would fetch.
        assert_eq!((b.as_array().unwrap().len(), &b[4]["type"]), (5, &json!("paragraph")));
        assert_eq!(b[4]["text"][1], json!({"type": "url", "text": ["x"], "url": "https://evil.example/?d=secret"}), "a link, opened only if tapped");
        assert_eq!(b[4]["text"][2], " <img src=https://evil.example>");
        let long = vec!["word ".repeat(200); 100].join("\n\n");
        let parts = telegram_rich(&long);
        assert!(parts.len() > 1 && parts.iter().all(|p| p.to_string().len() < 32_000), "{}", parts.len());
    }

    #[test]
    fn tables_elsewhere_stay_readable() {
        let md = "| a | b |\n|---|---|\n| 1 | **2** |";
        assert_eq!(telegram(md, 4000), vec!["a | b\n1 | <b>2</b>"]);
        assert_eq!(slack(md), "a | b\n1 | *2*");
        assert_eq!(plain(md), "a | b\n1 | 2");
        assert_eq!(telegram("| not | a table", 4000), vec!["| not | a table"], "no rule line: a paragraph");
    }

    #[test]
    fn plain_fallback() {
        let html = telegram("**a** < b & [x](https://e.x/?q=1&r=2)", 4000).remove(0);
        assert_eq!(html_to_plain(&html), "a < b & x");
    }

    #[test]
    fn unclosed_markers_stay_literal() {
        assert_eq!(telegram("2 * 3 = 6 and a ** b", 4000), vec!["2 * 3 = 6 and a ** b"]);
        assert_eq!(telegram("price is $5 [not a link]", 4000), vec!["price is $5 [not a link]"]);
    }
}
