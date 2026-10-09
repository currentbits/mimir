//! Bot replies are Markdown; each messaging app wants its own dialect.
//! Telegram → HTML (parse_mode=HTML), Slack → mrkdwn. Discord speaks Markdown already.
//! Covers what chat models write: paragraphs, lists, headings, quotes, code, bold/italic, links.

#[derive(Debug, PartialEq)]
enum Block {
    Para(String),
    Heading(String),
    Quote(String),
    Code(String),
    List { ordered: bool, items: Vec<String> },
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
        } else if let Some(h) = line.trim_start().strip_prefix('#').map(|r| r.trim_start_matches('#')).filter(|r| r.starts_with(' ')) {
            flush(&mut para, &mut out);
            out.push(Block::Heading(h.trim().into()));
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
        Block::Heading(t) => format!("<b>{}</b>", tg_inline(&inline(t))),
        Block::Quote(t) => format!("<blockquote>{}</blockquote>", tg_inline(&inline(t))),
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
            Block::Heading(t) => format!("*{}*", slack_inline(&inline(t))),
            Block::Quote(t) => format!("> {}", slack_inline(&inline(t))),
            Block::Code(t) => format!("```{}```", esc(t)),
            Block::List { ordered, items } => items
                .iter()
                .enumerate()
                .map(|(n, it)| format!("{} {}", if *ordered { format!("{}.", n + 1) } else { "•".into() }, slack_inline(&inline(it))))
                .collect::<Vec<_>>()
                .join("\n"),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

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
