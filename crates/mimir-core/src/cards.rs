//! Paying with a saved card (issue #415). The rules, in the order they bite:
//! - A card's details live only in the secret store (the Mac's Keychain). The AI sees a label and the last 4 digits.
//! - A purchase starts with `purchase_request` on the checkout page, as the browser itself reports it (its tab list,
//!   never page text). Limits (per purchase, per day, currency, the card's sites) are checked then, and again, with
//!   the day's total, in one transaction when approving.
//! - Only Touch ID in the Mac app approves it (`approve`, called by the desktop app after the person authenticated):
//!   the shared approval path refuses to (db::approval_decide), so no chat button, web page, rule or routine can.
//! - Approved, a grant opens for 10 minutes. Only then does that bot's browser get the card's details at all (it
//!   restarts to take them in, and again to drop them), so outside a grant there's nothing a page could probe. During
//!   the grant: each detail is typed once, only with browser_type or browser_fill_form, only into a field of the main
//!   page (not a frame), only on that site, from the chat it was asked in, with the person there; the browser can't
//!   go to another site, and leaving ends the grant. Results are scrubbed of the details (also as formatted in a form),
//!   and for half an hour after the last typing no screenshots are taken.
//! - Every request, approval, denial and use is in the purchases table, shown under Settings → Browser.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const GRANT_SECS: i64 = 600;
const LOG_KEEP: usize = 500;

/// A saved card, without its secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    /// Its placeholder stem, CARD_<last 4>[_n] (set when saved).
    #[serde(default)]
    pub name: String,
    /// What the person calls it ("Visa (personal)").
    pub label: String,
    #[serde(default)]
    pub last4: String,
    /// ISO code; a purchase in another currency is refused (no conversion guesses).
    pub currency: String,
    /// Most for one purchase, and for one day, in cents.
    pub max_cents: i64,
    pub day_cents: i64,
    /// Sites it may be used on; empty = any (each purchase is still approved with Touch ID).
    #[serde(default)]
    pub sites: Vec<String>,
}

/// What the person types in when adding a card (never stored together, never sent back).
#[derive(Debug, Clone, Deserialize)]
pub struct Secret {
    pub number: String,
    /// MM/YY or MM/YYYY.
    pub exp: String,
    pub cvc: String,
    pub holder: String,
    #[serde(default)]
    pub zip: String,
}

pub fn cards(db: &crate::db::Db) -> Vec<Card> {
    db.setting("cards").ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

fn luhn(digits: &str) -> bool {
    let mut sum = 0;
    for (i, c) in digits.chars().rev().enumerate() {
        let Some(mut d) = c.to_digit(10) else { return false };
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    sum % 10 == 0
}

/// "12.50" → 1250. Two decimals at most, more than zero.
pub fn cents(amount: &str) -> Result<i64> {
    let a = amount.trim().replace(',', ".");
    let (whole, frac) = a.split_once('.').unwrap_or((&a, ""));
    if whole.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) || frac.len() > 2 || !frac.chars().all(|c| c.is_ascii_digit()) || whole.len() > 9 {
        bail!("give the amount as a number, like 24.99");
    }
    let c = whole.parse::<i64>()? * 100 + format!("{frac:0<2}").parse::<i64>()?;
    if c <= 0 {
        bail!("the amount must be more than zero");
    }
    Ok(c)
}

pub fn money(cents: i64, currency: &str) -> String {
    format!("{} {}.{:02}", currency, cents / 100, cents % 100)
}

/// Add a card (the Mac app only: host.rs). Checked: a real card number, a date not past, a 3–4 digit code.
pub fn save(db: &crate::db::Db, mut card: Card, s: &Secret) -> Result<Card> {
    let number: String = s.number.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    if !(12..=19).contains(&number.len()) || !luhn(&number) {
        bail!("that isn't a valid card number");
    }
    let (m, y) = s.exp.trim().split_once('/').context("expiry as MM/YY")?;
    let (m, y): (u32, u32) = (m.trim().parse()?, y.trim().parse()?);
    let y = if y < 100 { 2000 + y } else { y };
    let today = chrono::Local::now().date_naive();
    if !(1..=12).contains(&m) || (y as i32, m) < (chrono::Datelike::year(&today), chrono::Datelike::month(&today)) {
        bail!("that card has expired, or the date isn't MM/YY");
    }
    let cvc = s.cvc.trim();
    if !(3..=4).contains(&cvc.len()) || !cvc.chars().all(|c| c.is_ascii_digit()) {
        bail!("the security code is 3 or 4 digits");
    }
    card.currency = card.currency.trim().to_uppercase();
    if card.currency.len() != 3 || !card.currency.chars().all(|c| c.is_ascii_uppercase()) {
        bail!("currency as its 3-letter code, like EUR");
    }
    if card.max_cents <= 0 || card.day_cents < card.max_cents {
        bail!("set a most-per-purchase above zero, and a most-per-day at least that");
    }
    card.label = card.label.trim().chars().take(40).collect();
    card.sites = card.sites.iter().filter_map(|s| crate::ext::site_of(&format!("https://{}/", s.trim().trim_start_matches("https://").trim_end_matches('/')))).collect();
    card.last4 = number[number.len() - 4..].to_string();
    let mut list = cards(db);
    let base = format!("CARD_{}", card.last4);
    card.name = (1..).map(|k| if k == 1 { base.clone() } else { format!("{base}_{k}") }).find(|n| !list.iter().any(|c| &c.name == n)).unwrap_or(base);
    let put = |f: &str, v: &str| crate::host::secret_set(&format!("card:{}:{f}", card.name), v);
    put("NUMBER", &number)?;
    put("EXP", &format!("{m:02}/{:02}", y % 100))?;
    put("EXP_MONTH", &format!("{m:02}"))?;
    put("EXP_YEAR", &y.to_string())?;
    put("CVC", cvc)?;
    put("NAME", s.holder.trim())?;
    put("ZIP", s.zip.trim())?;
    list.push(card.clone());
    db.set_setting("cards", &serde_json::to_string(&list)?)?;
    Ok(card)
}

pub fn delete(db: &crate::db::Db, name: &str) -> Result<()> {
    for f in FIELDS {
        crate::host::secret_set(&format!("card:{name}:{f}"), "")?;
    }
    let mut list = cards(db);
    list.retain(|c| c.name != name);
    db.set_setting("cards", &serde_json::to_string(&list)?)
}

/// One purchase: its request, and what happened to it.
#[derive(Debug, Clone, Serialize)]
pub struct Purchase {
    pub approval: i64,
    pub bot: i64,
    pub chat: i64,
    pub card: String,
    pub site: String,
    pub page: String,
    pub cents: i64,
    pub currency: String,
    pub what: String,
    pub asked_at: i64,
    pub approved_at: Option<i64>,
    /// requested | approved | used | denied | ended
    pub status: String,
    /// The details typed so far (NUMBER, CVC …), each once.
    pub typed: String,
    pub last_typed: Option<i64>,
    #[serde(skip)]
    pub typed_refs: String,
}

impl Purchase {
    /// (page, ref) for each field a detail was typed into.
    fn typed_refs_list(&self) -> Vec<(String, String)> {
        self.typed_refs.lines().filter_map(|l| l.rsplit_once('|')).map(|(p, r)| (p.to_string(), r.to_string())).collect()
    }
}

const COLS: &str = "approval, bot, chat, card, site, page, cents, currency, what, asked_at, approved_at, status, typed, last_typed, typed_refs";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Purchase> {
    Ok(Purchase {
        approval: r.get(0)?,
        bot: r.get(1)?,
        chat: r.get(2)?,
        card: r.get(3)?,
        site: r.get(4)?,
        page: r.get(5)?,
        cents: r.get(6)?,
        currency: r.get(7)?,
        what: r.get(8)?,
        asked_at: r.get(9)?,
        approved_at: r.get(10)?,
        status: r.get(11)?,
        typed: r.get(12)?,
        last_typed: r.get(13)?,
        typed_refs: r.get(14)?,
    })
}

fn purchases_where(db: &crate::db::Db, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Purchase>> {
    let c = db.c();
    let mut st = c.prepare(&format!("SELECT {COLS} FROM purchases WHERE {sql} ORDER BY approval"))?;
    let v = st.query_map(p, row)?.collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

/// The latest purchases, newest first.
pub fn log(db: &crate::db::Db) -> Vec<Purchase> {
    let mut v = purchases_where(db, "1", []).unwrap_or_default();
    v.reverse();
    v.truncate(LOG_KEEP);
    v
}

/// The start of today, on this machine's clock (a day's most is per local day).
fn today_start() -> i64 {
    let now = chrono::Local::now();
    chrono::TimeZone::from_local_datetime(&chrono::Local, &now.date_naive().and_hms_opt(0, 0, 0).expect("midnight")).earliest().map_or(crate::db::now() - 86400, |t| t.timestamp())
}

/// A purchase within this card's rules, or why not. `spent`: already approved on it today.
fn within(c: &Card, site: &str, cents: i64, currency: &str, spent: i64) -> Result<()> {
    if currency.trim().to_uppercase() != c.currency {
        bail!("{} is in {}; this purchase is in {currency}", c.label, c.currency);
    }
    if cents > c.max_cents {
        bail!("{} is over this card's most per purchase ({})", money(cents, &c.currency), money(c.max_cents, &c.currency));
    }
    if spent + cents > c.day_cents {
        bail!("that would take today's total on {} over its daily most ({} of {} used)", c.label, money(spent, &c.currency), money(c.day_cents, &c.currency));
    }
    if !c.sites.is_empty() && !c.sites.iter().any(|s| s == site) {
        bail!("{} may only be used on {}", c.label, c.sites.join(", "));
    }
    Ok(())
}

fn card(db: &crate::db::Db, name: &str) -> Result<Card> {
    cards(db).into_iter().find(|c| c.name == name).with_context(|| format!("no saved card {name}"))
}

fn spent_today(c: &rusqlite::Connection, card: &str) -> Result<i64> {
    Ok(c.query_row("SELECT coalesce(sum(cents), 0) FROM purchases WHERE card=?1 AND status IN ('approved','used','ended') AND approved_at >= ?2", rusqlite::params![card, today_start()], |r| r.get(0))?)
}

/// The tool: ask for one purchase on the page the browser is on, wait for Touch ID, and say how to go on.
pub(crate) async fn request(ctx: &crate::tools::ToolCtx, args: &serde_json::Value) -> Result<String> {
    if ctx.unattended || ctx.depth > 0 || ctx.db.chat_guest(ctx.chat_id) || ctx.incognito() || !ctx.db.chat_private(ctx.chat_id) {
        bail!("purchases need the person there, in a chat with them alone, to approve with Touch ID: not in a routine, another bot's task, a guest's chat or a group");
    }
    let name = args["card"].as_str().unwrap_or("").trim().to_string();
    let cents = cents(args["amount"].as_str().map(String::from).unwrap_or_else(|| args["amount"].to_string()).trim_matches('"'))?;
    let currency = args["currency"].as_str().unwrap_or("").trim().to_uppercase();
    let what: String = args["what"].as_str().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect();
    if what.is_empty() {
        bail!("say what it's for (what, from whom), in a line");
    }
    // Where the browser really is: from its own tab list, not from anything a page printed.
    #[cfg(test)]
    let page = TEST_PAGE.lock().unwrap().clone().context("no page")?;
    #[cfg(not(test))]
    let page = crate::ext::browser_current_url(ctx).await?;
    if !page.starts_with("https://") {
        bail!("this checkout page isn't secure (no https): don't pay here; tell the person");
    }
    let site = crate::ext::site_of(&page).context("the checkout page has no website address")?;
    let c = card(&ctx.db, &name)?;
    within(&c, &site, cents, &currency, spent_today(&ctx.db.c(), &name)?)?;
    // A site that looks like one of the person's known ones, or uses letters from another alphabet: said first.
    let w = crate::ext::signin_warnings(&ctx.db, &page, &site, false);
    let warn = match (w["foreign"].as_str(), w["lookalike"].as_str()) {
        (Some(f), _) => format!("⚠ This address uses letters from another alphabet (it may imitate {f}). "),
        (_, Some(l)) => format!("⚠ This address looks like {l}, but isn't it. "),
        _ if w["foreign"].as_bool() == Some(true) => "⚠ This address uses letters from another alphabet. ".to_string(),
        _ => String::new(),
    };
    let summary = format!(
        "{warn}Pay {} with {} (•••• {}) at {site}\nFor: {what}\nPage: {}\nApprove only in the Mimir app on this Mac, with Touch ID.",
        money(cents, &c.currency),
        c.label,
        c.last4,
        crate::tools::truncate(&page, 300)
    );
    let (db, bot, chat, card_name, site2, page2, cur, what2) = (ctx.db.clone(), ctx.bot_id, ctx.chat_id, name.clone(), site.clone(), page.clone(), c.currency.clone(), what.clone());
    let id = std::sync::Arc::new(std::sync::Mutex::new(0i64));
    let slot = id.clone();
    // The request is kept the moment its card exists (before anyone can answer it): Touch ID checks exactly that.
    let answer = crate::tools::wait_for_approval_hooked(ctx, "purchase_request", args, &summary, "purchase", Some(&site), move |a| {
        *slot.lock().unwrap() = a;
        db.c().execute(
            &format!("INSERT INTO purchases({COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL,'requested','',NULL,'')"),
            rusqlite::params![a, bot, chat, card_name, site2, page2, cents, cur, what2, crate::db::now()],
        )?;
        Ok(())
    })
    .await;
    let a = *id.lock().unwrap();
    if let Err(e) = answer {
        ctx.db.c().execute("UPDATE purchases SET status='denied' WHERE approval=?1 AND status='requested'", [a])?;
        return Err(e);
    }
    Ok(format!(
        "Approved with Touch ID. The browser restarts to take the card in: open the checkout page on {site} again (your cart \
         is kept). Then, within 10 minutes, type each detail once with browser_type or browser_fill_form, into fields of the \
         page itself: {name}_NUMBER, {name}_EXP (MM/YY; or {name}_EXP_MONTH and {name}_EXP_YEAR), {name}_CVC, {name}_NAME, \
         {name}_ZIP. Don't leave {site} (that ends it). If the card fields are inside a frame (a payment provider's box), \
         stop and hand the payment to the person. Place the order once and tell them what it came to."
    ))
}

/// After Touch ID (the Mac app only): check the request again, with the day's total, and approve it, all in one
/// transaction (two approvals at once can't both slip under the limit). The only way a purchase is approved.
pub fn approve(db: &crate::db::Db, approval: i64) -> Result<Purchase> {
    // Read before taking the database's lock (reading settings takes it too).
    let all = cards(db);
    let mut c = db.c();
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let p = tx.query_row(&format!("SELECT {COLS} FROM purchases WHERE approval=?1 AND status='requested'"), [approval], row).ok().context("this purchase isn't waiting any more")?;
    let cd = all.into_iter().find(|x| x.name == p.card).context("that card was removed")?;
    within(&cd, &p.site, p.cents, &p.currency, spent_today(&tx, &p.card)?)?;
    let now = crate::db::now();
    if tx.execute("UPDATE approvals SET status='approved' WHERE id=?1 AND status='pending' AND kind='purchase' AND expires_at > ?2", rusqlite::params![approval, now])? != 1 {
        bail!("this purchase isn't waiting any more");
    }
    tx.execute("UPDATE purchases SET status='approved', approved_at=?2 WHERE approval=?1", rusqlite::params![approval, now])?;
    tx.commit()?;
    Ok(Purchase { status: "approved".into(), approved_at: Some(now), ..p })
}

/// What the Mac app shows next to Touch ID: the request as kept, not as any page or bot words it.
pub fn reason(db: &crate::db::Db, approval: i64) -> Option<String> {
    let p = purchases_where(db, "approval=?1 AND status='requested'", [approval]).ok()?.pop()?;
    Some(format!("pay {} at {}", money(p.cents, &p.currency), p.site))
}

/// This bot's open grant: approved in the last 10 minutes, not ended.
pub(crate) fn open_grant(db: &crate::db::Db, bot: i64) -> Option<Purchase> {
    purchases_where(db, "bot=?1 AND status IN ('approved','used') AND approved_at > ?2", rusqlite::params![bot, crate::db::now() - GRANT_SECS]).ok()?.pop()
}

/// The cards whose details this bot's browser may hold right now, in this chat (its open grant's, asked here), to
/// know when to restart it.
pub(crate) fn granted(db: &crate::db::Db, bot: i64, chat: i64) -> Vec<String> {
    open_grant(db, bot).filter(|p| p.chat == chat).map(|p| vec![p.card]).unwrap_or_default()
}

/// The browser that held a card closed the shop's tabs: the card can't be on its screen any more.
pub(crate) fn cleared(db: &crate::db::Db, bot: i64) -> Result<()> {
    db.c().execute(
        "UPDATE purchases SET cleared=1 WHERE bot=?1 AND approved_at IS NOT NULL AND NOT (status IN ('approved','used') AND approved_at > ?2)",
        rusqlite::params![bot, crate::db::now() - GRANT_SECS],
    )?;
    Ok(())
}

/// The sites this bot was approved to pay on, whose tabs are closed when its browser drops the card.
pub(crate) fn paid_sites(db: &crate::db::Db, bot: i64) -> Vec<String> {
    let open: Vec<String> = purchases_where(db, "bot=?1 AND status IN ('approved','used') AND approved_at > ?2", rusqlite::params![bot, crate::db::now() - GRANT_SECS]).unwrap_or_default().into_iter().map(|p| p.site).collect();
    purchases_where(db, "bot=?1 AND approved_at IS NOT NULL AND cleared=0", [bot]).unwrap_or_default().into_iter().map(|p| p.site).filter(|s| !open.contains(s)).collect()
}

/// End this bot's grant (it left the site, or it's done): the browser restarts without the card.
pub(crate) fn end_grant(db: &crate::db::Db, bot: i64) -> Result<()> {
    db.c().execute("UPDATE purchases SET status='ended' WHERE bot=?1 AND status IN ('approved','used')", [bot])?;
    Ok(())
}

const FIELDS: [&str; 7] = ["NUMBER", "EXP", "EXP_MONTH", "EXP_YEAR", "CVC", "NAME", "ZIP"];

fn secret_of(card: &str, field: &str) -> Option<String> {
    crate::host::secret_get(&format!("card:{card}:{field}")).filter(|v| !v.is_empty())
}

/// What the browser types for the placeholders of this bot's granted card, in the chat it was asked in (and nothing
/// outside a grant).
pub(crate) fn values(db: &crate::db::Db, bot: i64, chat: i64) -> Vec<(String, String)> {
    granted(db, bot, chat).iter().flat_map(|c| FIELDS.iter().filter_map(move |f| secret_of(c, f).map(|v| (format!("{c}_{f}"), v)))).collect()
}

/// Every card placeholder in a text, exactly: (card, detail), the longest card name and detail first, so CARD_4444_2
/// isn't CARD_4444 and _EXP_MONTH isn't _EXP. A card's name without a known detail still counts (as "").
fn placeholders(text: &str, names: &[String]) -> Vec<(String, &'static str)> {
    let mut names: Vec<&String> = names.iter().collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    let mut fields = FIELDS.to_vec();
    fields.sort_by_key(|f| std::cmp::Reverse(f.len()));
    let mut out = vec![];
    let mut i = 0;
    while let Some(at) = text[i..].find("CARD_").map(|a| a + i) {
        let rest = &text[at..];
        let Some(name) = names.iter().find(|n| rest.starts_with(n.as_str()) && !rest[n.len()..].starts_with(|c: char| c.is_ascii_digit())) else {
            i = at + 5;
            continue;
        };
        let after = &rest[name.len()..];
        let field = fields.iter().find(|f| after.strip_prefix('_').is_some_and(|a| a.starts_with(**f) && !a[f.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))).copied().unwrap_or("");
        out.push((name.to_string(), field));
        i = at + name.len();
    }
    out
}

/// The cards whose placeholders (or names) appear anywhere in these arguments.
pub(crate) fn mentioned(db: &crate::db::Db, args: &serde_json::Value) -> Vec<String> {
    let mut all = vec![];
    crate::ext::strings(args, &mut all);
    let names: Vec<String> = cards(db).into_iter().map(|c| c.name).collect();
    let mut out: Vec<String> = all.iter().flat_map(|s| placeholders(s, &names)).map(|(c, _)| c).collect();
    out.dedup();
    out.sort();
    out.dedup();
    out
}

/// Card details may be typed by this call, or why not; recorded as typed (each detail once). `site`: where the
/// browser is, from its own tab list.
pub(crate) fn may_type(ctx: &crate::tools::ToolCtx, tool: &str, args: &serde_json::Value, card: &str, page: &str) -> Result<()> {
    if !page.starts_with("https://") {
        bail!("card details only go into a secure page (https)");
    }
    let site = crate::ext::site_of(page);
    let site = site.as_deref();
    if !matches!(tool, "browser_type" | "browser_fill_form") {
        bail!("card details can only be typed into a form field (browser_type or browser_fill_form), never anywhere else");
    }
    let p = open_grant(&ctx.db, ctx.bot_id).filter(|p| p.card == card).with_context(|| format!("no open approval for {card}: call purchase_request first; the person approves it with Touch ID (each approval lasts 10 minutes)"))?;
    if ctx.chat_id != p.chat || ctx.unattended || ctx.depth > 0 {
        bail!("{card} was approved for the chat it was asked in, with the person there, only");
    }
    if site != Some(p.site.as_str()) {
        bail!("{card} was approved for {} only, not {}", p.site, site.unwrap_or("this page"));
    }
    // Only into the main page's own fields: a frame (an ad, a widget, someone else's box) isn't the shop's.
    let mut targets = vec![];
    if tool == "browser_type" {
        targets.push(args["target"].as_str().or(args["ref"].as_str()).unwrap_or(""));
    } else {
        for f in args["fields"].as_array().into_iter().flatten().filter(|f| f["value"].as_str().is_some_and(|v| v.contains(card))) {
            targets.push(f["target"].as_str().or(f["ref"].as_str()).unwrap_or(""));
        }
    }
    let main_page = |t: &str| t.len() > 1 && t.starts_with('e') && t[1..].chars().all(|c| c.is_ascii_digit());
    if targets.is_empty() || !targets.iter().all(|t| main_page(t)) {
        bail!("card details can only go into a field of the page itself (a ref like e12), not into a frame: hand this payment to the person");
    }
    let mut all = vec![];
    crate::ext::strings(args, &mut all);
    let names: Vec<String> = cards(&ctx.db).into_iter().map(|c| c.name).collect();
    let fields: Vec<&str> = all.iter().flat_map(|s| placeholders(s, &names)).filter(|(c, f)| c == card && !f.is_empty()).map(|(_, f)| f).collect();
    if fields.is_empty() {
        bail!("type a card detail by its placeholder, like {card}_NUMBER");
    }
    let mut seen = fields.clone();
    seen.sort_unstable();
    seen.dedup();
    if seen.len() != fields.len() {
        bail!("each card detail goes into one field only");
    }
    let typed: Vec<&str> = p.typed.split(',').filter(|t| !t.is_empty()).collect();
    if let Some(again) = fields.iter().find(|f| typed.contains(f)) {
        bail!("{card}_{again} was typed already for this purchase; each detail goes in once (a new purchase needs a new approval)");
    }
    let now: Vec<&str> = typed.into_iter().chain(fields).collect();
    // The fields it went into: the only places the browser may show a card detail's marker afterwards.
    ctx.db.c().execute(
        "UPDATE purchases SET status='used', typed=?2, last_typed=?3, typed_refs = typed_refs || ?4 WHERE approval=?1",
        // With the page it's on: refs are numbered per page, so another page's e12 isn't this field.
        rusqlite::params![p.approval, now.join(","), crate::db::now(), targets.iter().map(|t| format!("{}|{t}\n", same_page(page))).collect::<String>()],
    )?;
    Ok(())
}

/// A card may be on this bot's screen: approved, or typed, in the last half hour. Text is scrubbed of its details;
/// an image isn't, so no screenshots then.
/// Also until the browser that held it closed the shop's tabs (the bots' computer keeps its pages).
pub(crate) fn on_screen(db: &crate::db::Db, bot: i64) -> bool {
    let since = crate::db::now() - 1800;
    db.c()
        .query_row(
            "SELECT count(*) FROM purchases WHERE bot=?1 AND (approved_at >= ?2 OR last_typed >= ?2 OR (typed != '' AND cleared = 0 AND approved_at >= ?2 - 84600))",
            rusqlite::params![bot, since],
            |r| r.get::<_, i64>(0),
        )
        .is_ok_and(|n| n > 0)
}

/// A page as the same document: without its #fragment (single-page checkouts change it from step to step).
fn same_page(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

/// A line where the browser may show a card detail's marker: a field this bot typed it into ("[ref=e12]"), holding
/// exactly one marker and nothing else. Anywhere else, a marker means a page showing (or probing for) a detail.
pub(crate) fn marker_allowed(db: &crate::db::Db, bot: i64, line: &str, page: &str) -> bool {
    let refs: Vec<String> = purchases_where(db, "bot=?1 AND typed_refs != '' AND cleared = 0", [bot])
        .unwrap_or_default()
        .iter()
        .flat_map(|p| p.typed_refs_list())
        .filter(|(p, _)| p == same_page(page))
        .map(|(_, r)| r)
        .collect();
    let Some((head, value)) = line.rsplit_once(": ") else { return false };
    let v = value.trim();
    field_line(line)
        && refs.iter().any(|r| head.contains(&format!("[ref={r}]")))
        && v.starts_with("<secret>CARD_")
        && v.ends_with("</secret>")
        && v.matches("<secret>").count() == 1
}

/// Every card marker in a browser result is where it may be: on a field this bot typed into (on this page), or as the
/// value shown nested under such a field ("- text: <secret>…</secret>"). Anything else is a page showing (or probing
/// for) a detail.
pub(crate) fn markers_ok(db: &crate::db::Db, bot: i64, text: &str, page: &str) -> bool {
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut field: Option<(usize, &str)> = None;
    for l in text.lines() {
        if field_line(l) {
            field = Some((indent(l), l));
        } else if field.is_some_and(|(f, _)| indent(l) <= f) {
            field = None;
        }
        if !l.contains("<secret>CARD_") {
            continue;
        }
        let ok = if field_line(l) {
            marker_allowed(db, bot, l, page)
        } else {
            // A value under its field: judged as that field's value.
            match (field, l.trim_start().strip_prefix("- text: ")) {
                (Some((_, parent)), Some(value)) => {
                    let head = parent.trim_end().trim_end_matches(':').rsplit_once(": ").map_or(parent.trim_end().trim_end_matches(':'), |(h, _)| h);
                    marker_allowed(db, bot, &format!("{head}: {value}"), page)
                }
                _ => false,
            }
        };
        if !ok {
            return false;
        }
    }
    true
}

/// A line of the browser's page snapshot that is a form field itself.
pub(crate) fn field_line(l: &str) -> bool {
    let t = l.trim_start();
    ["- textbox", "- spinbutton", "- combobox"].iter().any(|k| t.starts_with(k))
}

/// For each line: whether it's a form field, or a value shown nested under one ("- text: 737" indented below a
/// textbox with a placeholder). Page text elsewhere ("- text: Order 123") isn't.
pub(crate) fn in_fields(text: &str) -> Vec<bool> {
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut field_at: Option<usize> = None;
    text.lines()
        .map(|l| {
            if field_line(l) {
                field_at = Some(indent(l));
                return true;
            }
            match field_at {
                Some(f) if indent(l) > f => true,
                _ => {
                    field_at = None;
                    false
                }
            }
        })
        .collect()
}

/// Scrub card details from what the browser returned, also as a form shows them: the number with spaces or dashes,
/// and form fields whose value is a card detail. Only for a bot that may have one on screen (else there's nothing
/// there, and comparing would only tell a page whether it guessed).
pub(crate) fn scrub(db: &crate::db::Db, bot: i64, text: &str) -> String {
    if !on_screen(db, bot) {
        return text.to_string();
    }
    let cards: Vec<String> = purchases_where(db, "bot=?1 AND (approved_at >= ?2 OR last_typed >= ?2 OR (typed != '' AND cleared = 0 AND approved_at >= ?2 - 84600))", rusqlite::params![bot, crate::db::now() - 1800])
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.card)
        .collect();
    let digits = |s: &str| s.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
    let mut out = text.to_string();
    for c in cards {
        if let Some(n) = secret_of(&c, "NUMBER") {
            out = scrub_number(&out, &n);
        }
        let short: Vec<String> = ["EXP", "EXP_MONTH", "EXP_YEAR", "CVC", "NAME", "ZIP"].iter().filter_map(|f| secret_of(&c, f)).collect();
        let fields = in_fields(&out);
        // Digits compared only for a value that's all digits and separators (12/29, 05 / 29): "Order 123" isn't "123".
        let numeric = |v: &str| !v.trim().is_empty() && v.trim().chars().all(|c| c.is_ascii_digit() || " /-.".contains(c));
        out = out
            .lines()
            .zip(fields)
            .map(|(l, field)| {
                // A form field's value: "- textbox "Security code" [ref=e14]: 123", or on its own line under the field
                // ("- text: 123") when the field has a placeholder.
                match l.rsplit_once(": ").filter(|_| field) {
                    Some((head, value)) if short.iter().any(|v| value.trim() == v || (numeric(value) && !digits(v).is_empty() && digits(value) == digits(v))) => format!("{head}: [card detail]"),
                    _ => l.to_string(),
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    out
}

/// The card number anywhere in the text, however it's grouped (spaces, dashes), as [card number].
fn scrub_number(text: &str, number: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            // The longest run of digits, spaces and dashes from here.
            let mut j = i;
            let mut d = String::new();
            let mut last_digit = i;
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == ' ' || chars[j] == '-') {
                if chars[j].is_ascii_digit() {
                    d.push(chars[j]);
                    last_digit = j;
                }
                j += 1;
            }
            if d.contains(number) {
                let at = d.find(number).unwrap_or(0);
                // Keep what came before the number in this run, then the mark.
                let mut seen = 0;
                let mut k = i;
                while seen < at {
                    if chars[k].is_ascii_digit() {
                        seen += 1;
                    }
                    out.push(chars[k]);
                    k += 1;
                }
                out.push_str("[card number]");
                let mut left = number.len();
                while left > 0 && k <= last_digit {
                    if chars[k].is_ascii_digit() {
                        left -= 1;
                    }
                    k += 1;
                }
                i = k;
                continue;
            }
            out.extend(&chars[i..=last_digit]);
            i = last_digit + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Tests have no browser: the page it would report.
#[cfg(test)]
pub(crate) static TEST_PAGE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[cfg(test)]
mod tests {
    use crate::db::Db;
    use crate::tools::ToolCtx;
    use std::time::Duration;

    struct Mem(std::sync::Mutex<std::collections::HashMap<String, String>>);
    impl crate::host::SecretStore for Mem {
        fn get(&self, n: &str) -> Option<String> {
            self.0.lock().unwrap().get(n).cloned().filter(|s| !s.is_empty())
        }
        fn set(&self, n: &str, v: &str) -> anyhow::Result<()> {
            self.0.lock().unwrap().insert(n.into(), v.into());
            Ok(())
        }
    }

    const PAY: &str = "https://www.shop.example/checkout";

    fn card(max: i64, day: i64) -> super::Card {
        super::Card { name: String::new(), label: "Visa".into(), last4: String::new(), currency: "eur".into(), max_cents: max, day_cents: day, sites: vec![] }
    }

    fn secret(number: &str) -> super::Secret {
        super::Secret { number: number.into(), exp: "12/39".into(), cvc: "123".into(), holder: "Dana Doe".into(), zip: "1011AB".into() }
    }

    /// Ask for a purchase (live) and wait for its card; returns the waiting call and the approval.
    async fn ask(ctx: &ToolCtx, amount: &str) -> (tokio::task::JoinHandle<anyhow::Result<String>>, crate::db::Approval) {
        let before: Vec<i64> = ctx.db.approvals_pending().unwrap().iter().map(|a| a.id).collect();
        let c = ctx.clone();
        let amount = amount.to_string();
        let task = tokio::spawn(async move { super::request(&c, &serde_json::json!({"card": "CARD_4444", "amount": amount, "currency": "EUR", "what": "Padel balls from Shop"})).await });
        loop {
            if let Some(a) = ctx.db.approvals_pending().unwrap().into_iter().find(|a| a.kind == "purchase" && !before.contains(&a.id)) {
                return (task, a);
            }
            if task.is_finished() {
                panic!("no card: {:?}", task.await.unwrap());
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }

    #[tokio::test]
    async fn a_purchase_is_approved_only_with_touch_id_within_its_limits_and_typed_only_there() {
        crate::host::install_secrets(std::sync::Arc::new(Mem(Default::default())));
        let p = std::env::temp_dir().join(format!("mimir-cards-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).unwrap();
        // Only a real card, not expired, with a real code; limits that make sense.
        assert!(super::save(&db, card(5000, 6000), &secret("5555555555554443")).is_err(), "not a card number");
        assert!(super::save(&db, card(5000, 6000), &super::Secret { exp: "01/20".into(), ..secret("5555555555554444") }).is_err(), "expired");
        assert!(super::save(&db, card(5000, 6000), &super::Secret { cvc: "12".into(), ..secret("5555555555554444") }).is_err());
        assert!(super::save(&db, card(5000, 4000), &secret("5555555555554444")).is_err(), "a day's most below one purchase's");
        let c = super::save(&db, card(5000, 6000), &secret("5555 5555 5555 4444")).unwrap();
        assert_eq!((c.name.as_str(), c.last4.as_str(), c.currency.as_str()), ("CARD_4444", "4444", "EUR"));
        // The details are in the secret store only, and no browser gets them outside a grant (nothing to probe).
        assert!(!db.setting("cards").unwrap().unwrap().contains("5555555555554444"));
        assert!(super::values(&db, 1, crate::host::LOCAL_CHAT).is_empty() && !crate::ext::vault_values(&db).iter().any(|(_, v)| v.contains("5555")));

        let ctx = ToolCtx::new(db.clone(), 1, crate::host::LOCAL_CHAT);
        *super::TEST_PAGE.lock().unwrap() = Some("https://www.shop.example/checkout".into());
        db.set_hands_off(1, true).unwrap(); // a bot set to never ask: a purchase still asks

        // The card says what's paid where, without the number; no chat button, the web UI or the API approves it.
        let (task, a) = ask(&ctx, "24.99").await;
        assert!(a.summary.starts_with("Pay EUR 24.99 with Visa (•••• 4444) at shop.example") && !a.summary.contains("5555555555554444"), "{}", a.summary);
        assert!(db.approval_decide(a.id, true, None).unwrap().is_none() && db.approval(a.id).unwrap().unwrap().status == "pending");
        assert_eq!(crate::runtime::card_buttons(&a), vec![vec![("Deny".to_string(), format!("ap:{}:n", a.id))]]);
        let type_ = |field: &str| serde_json::json!({"target": "e12", "text": format!("CARD_4444_{field}")});
        assert!(super::may_type(&ctx, "browser_type", &type_("NUMBER"), "CARD_4444", PAY).is_err(), "nothing before Touch ID");
        // Touch ID's side: approved, the grant opens; only now does this bot's browser get the details.
        assert_eq!(super::reason(&db, a.id).as_deref(), Some("pay EUR 24.99 at shop.example"));
        super::approve(&db, a.id).unwrap();
        assert!(super::approve(&db, a.id).is_err(), "not twice");
        assert!(task.await.unwrap().unwrap().contains("CARD_4444_NUMBER"));
        assert!(super::values(&db, 1, crate::host::LOCAL_CHAT).contains(&("CARD_4444_NUMBER".to_string(), "5555555555554444".to_string())));
        assert!(super::values(&db, 2, crate::host::LOCAL_CHAT).is_empty() && super::values(&db, 1, 42).is_empty(), "not another bot, nor this bot in another chat");
        // Typed into a field of the page itself, on that site, from that chat, each detail once.
        super::may_type(&ctx, "browser_type", &type_("NUMBER"), "CARD_4444", PAY).unwrap();
        assert!(super::may_type(&ctx, "browser_type", &type_("NUMBER"), "CARD_4444", PAY).unwrap_err().to_string().contains("typed already"));
        let form = serde_json::json!({"fields": [{"target": "e14", "value": "CARD_4444_CVC"}, {"target": "e15", "value": "CARD_4444_EXP_MONTH"}, {"target": "e16", "value": "CARD_4444_EXP_YEAR"}]});
        super::may_type(&ctx, "browser_fill_form", &form, "CARD_4444", PAY).unwrap();
        // The browser may show a detail's marker only in a field it was typed into, as the whole value.
        assert!(super::marker_allowed(&db, 1, "- textbox \"Card number\" [ref=e12]: <secret>CARD_4444_NUMBER</secret>", PAY));
        assert!(super::marker_allowed(&db, 1, "  - textbox \"CVC\" [ref=e14]: <secret>CARD_4444_CVC</secret>", PAY));
        assert!(super::marker_allowed(&db, 1, "  - textbox \"CVC\" [ref=e14]: <secret>CARD_4444_CVC</secret>", &format!("{PAY}#step-2")), "the same page at another step");
        // A value shown under its field counts as that field's; page text with a marker doesn't.
        assert!(super::markers_ok(&db, 1, "- textbox \"CVC\" [ref=e14]:\n  - /placeholder: CVC\n  - text: <secret>CARD_4444_CVC</secret>", PAY));
        assert!(!super::markers_ok(&db, 1, "- paragraph:\n  - text: <secret>CARD_4444_CVC</secret>", PAY));
        assert!(!super::markers_ok(&db, 1, "- textbox \"kq\" [ref=e5]:\n  - text: <secret>CARD_4444_CVC</secret>", PAY));
        // Not over plain http, and each detail into one field only.
        assert!(super::may_type(&ctx, "browser_type", &type_("ZIP"), "CARD_4444", "http://www.shop.example/checkout").is_err());
        let twice = serde_json::json!({"fields": [{"target": "e30", "value": "CARD_4444_NAME"}, {"target": "e31", "value": "CARD_4444_NAME"}]});
        assert!(super::may_type(&ctx, "browser_fill_form", &twice, "CARD_4444", PAY).unwrap_err().to_string().contains("one field only"));
        // The same ref on another page of the shop (refs restart per page) isn't that field.
        assert!(!super::marker_allowed(&db, 1, "  - textbox \"CVC\" [ref=e14]: <secret>CARD_4444_CVC</secret>", "https://www.shop.example/listing/9"));
        for bad in [
            "- textbox \"kq\" [ref=e5]: <secret>CARD_4444_CVC</secret>",
            "- textbox \"x\" [ref=e12]: 1 <secret>CARD_4444_CVC</secret>",
            "- paragraph [ref=e12]: <secret>CARD_4444_CVC</secret>",
            "- text: <secret>CARD_4444_CVC</secret>",
        ] {
            assert!(!super::marker_allowed(&db, 1, bad, PAY), "{bad}");
        }
        assert!(!super::marker_allowed(&db, 2, "- textbox \"Card number\" [ref=e12]: <secret>CARD_4444_NUMBER</secret>", PAY), "another bot");
        for (tool, args, site) in [
            ("browser_navigate", type_("NAME"), "shop.example"),
            ("browser_evaluate", type_("NAME"), "shop.example"),
            ("browser_type", type_("NAME"), "evil.example"),
            ("browser_type", serde_json::json!({"target": "f1e3", "text": "CARD_4444_NAME"}), "shop.example"),
            ("browser_type", serde_json::json!({"target": "#card", "text": "CARD_4444_NAME"}), "shop.example"),
        ] {
            assert!(super::may_type(&ctx, tool, &args, "CARD_4444", &format!("https://{site}/pay")).is_err(), "{tool} {args} {site}");
        }
        assert!(super::may_type(&ToolCtx::new(db.clone(), 1, 42), "browser_type", &type_("NAME"), "CARD_4444", PAY).is_err(), "another chat");
        assert!(super::may_type(&ToolCtx::new(db.clone(), 2, crate::host::LOCAL_CHAT), "browser_type", &type_("NAME"), "CARD_4444", PAY).is_err(), "another bot");
        // What the browser returns: the number however it's grouped, and form fields holding a detail, scrubbed.
        let page = "- textbox \"Card number\" [ref=e12]: 5555 5555 5555 4444\n- textbox \"CVC\" [ref=e14]: 123\n- text: Order 123 total\nRef 5555-5555-5555-4444 done\n- textbox \"Security code\" [ref=e20]:\n  - /placeholder: CVC\n  - text: 123";
        let s = super::scrub(&db, 1, page);
        assert!(!s.contains("5555") && s.contains("[card number]") && s.contains("CVC\" [ref=e14]: [card detail]") && s.contains("Order 123 total"), "{s}");
        assert!(s.ends_with("  - text: [card detail]"), "a value under its field, too: {s}");
        assert_eq!(super::scrub(&db, 2, page), page, "another bot (no card on its screen): untouched, no probing");

        // Limits, checked before asking: over one purchase's most, or the day's (24.99 already), or another currency.
        let req = |amount: &str, cur: &str| serde_json::json!({"card": "CARD_4444", "amount": amount, "currency": cur, "what": "x"});
        assert!(super::request(&ctx, &req("50.01", "EUR")).await.is_err());
        assert!(super::request(&ctx, &req("40", "EUR")).await.unwrap_err().to_string().contains("daily most"));
        assert!(super::request(&ctx, &req("5", "USD")).await.is_err());
        // And again when approving: the limit lowered while the card waited.
        super::end_grant(&db, 1).unwrap();
        let (task, a) = ask(&ctx, "20").await;
        let mut list = super::cards(&db);
        list[0].day_cents = 3000;
        db.set_setting("cards", &serde_json::to_string(&list).unwrap()).unwrap();
        assert!(super::approve(&db, a.id).unwrap_err().to_string().contains("daily most"));
        db.approval_decide(a.id, false, None).unwrap();
        assert!(task.await.unwrap().is_err());
        assert!(super::log(&db).iter().any(|r| r.status == "denied" && r.cents == 2000));
        // A grant lasts 10 minutes; a card typed recently means no screenshots for half an hour.
        db.c().execute("UPDATE purchases SET approved_at = approved_at - 700, last_typed = last_typed - 700", []).unwrap();
        assert!(super::open_grant(&db, 1).is_none() && super::granted(&db, 1, crate::host::LOCAL_CHAT).is_empty());
        assert!(super::on_screen(&db, 1) && !super::on_screen(&db, 2));
        db.c().execute("UPDATE purchases SET approved_at = approved_at - 1200, last_typed = last_typed - 1200", []).unwrap();
        // Half an hour on, but the shop's tab was never closed (the bots' computer keeps pages): still no screenshots,
        // until the browser closed it.
        assert!(super::on_screen(&db, 1) && super::paid_sites(&db, 1).contains(&"shop.example".to_string()));
        super::cleared(&db, 1).unwrap();
        assert!(!super::on_screen(&db, 1) && super::paid_sites(&db, 1).is_empty());
        // Never in a routine, another bot's task, or a guest's chat.
        let mut later = ctx.clone();
        later.unattended = true;
        assert!(super::request(&later, &req("1", "EUR")).await.is_err());
        db.pair(200, "Sam").unwrap();
        db.chat_set_guest(200, true).unwrap();
        assert!(super::request(&ToolCtx::new(db.clone(), 1, 200), &req("1", "EUR")).await.is_err());
        // Removed: its details are gone from the secret store.
        super::delete(&db, "CARD_4444").unwrap();
        assert!(crate::host::secret_get("card:CARD_4444:NUMBER").is_none() && super::cards(&db).is_empty());
    }

    #[test]
    fn placeholders_are_matched_exactly() {
        let names = vec!["CARD_4444".to_string(), "CARD_4444_2".to_string()];
        assert_eq!(super::placeholders("CARD_4444_EXP_MONTH", &names), [("CARD_4444".to_string(), "EXP_MONTH")]);
        assert_eq!(super::placeholders("CARD_4444_2_CVC and CARD_4444_EXP", &names), [("CARD_4444_2".to_string(), "CVC"), ("CARD_4444".to_string(), "EXP")]);
        assert_eq!(super::placeholders("CARD_44445_NUMBER", &names), []);
        assert_eq!(super::placeholders("CARD_4444", &names), [("CARD_4444".to_string(), "")], "a name alone still counts");
        let snap = "- textbox \"CVC\" [ref=e1]:\n  - /placeholder: CVC\n  - text: 737\n- text: Order 737\n- paragraph: 737";
        assert_eq!(super::in_fields(snap), [true, true, true, false, false], "a value under its field, not page text");
    }

    #[test]
    fn amounts_and_numbers() {
        assert_eq!(super::cents("24.99").unwrap(), 2499);
        assert_eq!(super::cents("7").unwrap(), 700);
        assert_eq!(super::cents("3,5").unwrap(), 350);
        for bad in ["", "0", "-1", "1.234", "abc", "1e5", "9999999999"] {
            assert!(super::cents(bad).is_err(), "{bad}");
        }
        assert!(super::luhn("4242424242424242") && !super::luhn("4242424242424241"));
        assert_eq!(super::money(2499, "EUR"), "EUR 24.99");
    }
}
