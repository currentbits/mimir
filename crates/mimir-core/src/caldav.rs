//! Adding events to the person's calendar over CalDAV (iCloud, Fastmail, Nextcloud, any CalDAV server), with an app
//! password. Reading stays with the calendar link (calendar.rs); this only adds, and every event is approved first.
//! Google Calendar needs Google's own sign-in for this, so there it's still the add-to-calendar links.
use anyhow::{Context, Result, bail};
use reqwest::{Method, StatusCode, Url};
use std::time::Duration;

pub const SECRET: &str = "caldav_password";

/// The calendar events go to: its collection URL and name, and the login.
pub struct Cal {
    pub url: String,
    pub name: String,
    pub user: String,
    pass: String,
}

/// The CalDAV server for an email provider whose app password works for both.
pub fn guess(email_host: &str) -> &'static str {
    match email_host.trim().to_lowercase().as_str() {
        "imap.mail.me.com" => "https://caldav.icloud.com",
        "imap.fastmail.com" => "https://caldav.fastmail.com",
        _ => "",
    }
}

/// The password: its own, or the email app password when it's the same login (iCloud and Fastmail share them).
pub(crate) fn password(db: &crate::db::Db, user: &str) -> Option<String> {
    crate::host::secret_get(SECRET).filter(|p| !p.is_empty()).or_else(|| crate::email::accounts(db).into_iter().find(|a| a.user.eq_ignore_ascii_case(user) && a.oauth.is_none()).map(|a| a.pass().to_string()))
}

pub fn account(db: &crate::db::Db) -> Option<Cal> {
    let s = |k: &str| db.setting(k).ok().flatten().filter(|v| !v.trim().is_empty());
    let (url, user) = (s("caldav_url")?, s("caldav_user")?);
    Some(Cal { name: s("caldav_name").unwrap_or_else(|| "your calendar".into()), pass: password(db, &user)?, url, user })
}

/// https, or plain http only to this computer (a CalDAV server of your own on it).
fn checked(u: &Url) -> Result<()> {
    let local = matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if u.scheme() == "https" || (u.scheme() == "http" && local) { Ok(()) } else { bail!("a calendar server address starts with https://") }
}

fn client() -> Result<reqwest::Client> {
    // Redirects by hand: PROPFIND has to stay PROPFIND, and every hop is checked like the first.
    Ok(reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(30)).build()?)
}

async fn propfind(c: &reqwest::Client, url: &Url, depth: &str, body: &str, user: &str, pass: &str) -> Result<(Url, String)> {
    let mut u = url.clone();
    for _ in 0..4 {
        checked(&u)?;
        let r = c
            .request(Method::from_bytes(b"PROPFIND")?, u.clone())
            .basic_auth(user, Some(pass))
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body.to_string())
            .send()
            .await
            .with_context(|| format!("couldn't reach {}", u.host_str().unwrap_or("the calendar server")))?;
        if r.status().is_redirection() {
            let to = r.headers().get("location").and_then(|l| l.to_str().ok()).context("a redirect without a Location")?;
            u = u.join(to)?;
            continue;
        }
        if r.status() == StatusCode::UNAUTHORIZED {
            bail!("the calendar server didn't accept that user name and app password");
        }
        if !r.status().is_success() {
            bail!("the calendar server said {}", r.status());
        }
        return Ok((u, r.text().await?));
    }
    bail!("the calendar server redirected too many times")
}

/// One <response> of a multistatus answer: what discovery needs.
#[derive(Default, Debug)]
struct Resp {
    href: String,
    principal: String,
    home: String,
    calendar: bool,
    name: String,
    comps: Vec<String>,
}

/// Namespace prefixes vary by server (d:, D:, none), so elements are matched by local name.
fn parse(xml: &str) -> Result<Vec<Resp>> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let (mut out, mut path, mut text): (Vec<Resp>, Vec<String>, String) = (vec![], vec![], String::new());
    let name = |n: &str| n.to_lowercase();
    loop {
        match r.read_event()? {
            Event::Start(e) => {
                let n = name(e.local_name().as_ref());
                if n == "response" {
                    out.push(Resp::default());
                }
                comp_or_kind(out.last_mut(), &path, &n, &e)?;
                path.push(n);
                text.clear();
            }
            Event::Empty(e) => comp_or_kind(out.last_mut(), &path, &name(e.local_name().as_ref()), &e)?,
            Event::Text(t) => text.push_str(&t.xml10_content()),
            Event::GeneralRef(g) => match g.resolve_char_ref()? {
                Some(c) => text.push(c),
                None => text.push_str(match &*g.xml10_content() { "amp" => "&", "lt" => "<", "gt" => ">", "quot" => "\"", "apos" => "'", _ => "" }),
            },
            Event::End(_) => {
                let parent = path.len().checked_sub(2).map(|i| path[i].as_str());
                if let (Some(cur), Some(last)) = (out.last_mut(), path.last()) {
                    let t = text.trim().to_string();
                    match (parent, last.as_str()) {
                        (Some("response"), "href") => cur.href = t,
                        (Some("current-user-principal"), "href") => cur.principal = t,
                        (Some("calendar-home-set"), "href") => cur.home = t,
                        (_, "displayname") => cur.name = t,
                        _ => {}
                    }
                }
                path.pop();
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn comp_or_kind(cur: Option<&mut Resp>, path: &[String], n: &str, e: &quick_xml::events::BytesStart) -> Result<()> {
    let Some(cur) = cur else { return Ok(()) };
    if n == "calendar" && path.last().is_some_and(|p| p == "resourcetype") {
        cur.calendar = true;
    }
    if n == "comp"
        && let Some(a) = e.try_get_attribute("name")?
    {
        cur.comps.push(a.normalized_value(quick_xml::XmlVersion::Implicit1_0)?.to_uppercase());
    }
    Ok(())
}

const PRINCIPAL: &str = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
const HOME: &str = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#;
const LIST: &str = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:resourcetype/><d:displayname/><c:supported-calendar-component-set/></d:prop></d:propfind>"#;

/// Sign in and list the calendars that take events: (collection URL, name), in the server's order.
pub async fn discover(server: &str, user: &str, pass: &str) -> Result<Vec<(String, String)>> {
    let server = server.trim();
    let base = Url::parse(&if server.contains("://") { server.to_string() } else { format!("https://{server}") }).context("that isn't a server address")?;
    let c = client()?;
    // Where the account's calendars are: asked at the address given, then at the standard /.well-known/caldav.
    let mut principal = None;
    let mut last_err = None;
    for at in [base.clone(), base.join("/.well-known/caldav")?] {
        match propfind(&c, &at, "0", PRINCIPAL, user, pass).await {
            Ok((u, xml)) => {
                if let Some(p) = parse(&xml)?.into_iter().map(|r| r.principal).find(|p| !p.is_empty()) {
                    principal = Some(u.join(&p)?);
                    break;
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    let principal = match (principal, last_err) {
        (Some(p), _) => p,
        (None, Some(e)) => return Err(e),
        (None, None) => bail!("no calendars found at {server}: is it a CalDAV address?"),
    };
    let (u, xml) = propfind(&c, &principal, "0", HOME, user, pass).await?;
    let home = parse(&xml)?.into_iter().map(|r| r.home).find(|h| !h.is_empty()).context("the server didn't say where your calendars are")?;
    let (u, xml) = propfind(&c, &u.join(&home)?, "1", LIST, user, pass).await?;
    let mut cals = vec![];
    for r in parse(&xml)?.into_iter().filter(|r| r.calendar && (r.comps.is_empty() || r.comps.iter().any(|c| c == "VEVENT"))) {
        let url = u.join(&r.href)?;
        let name = if r.name.is_empty() { url.path_segments().and_then(|mut s| s.rfind(|x| !x.is_empty())).unwrap_or("Calendar").to_string() } else { r.name };
        cals.push((url.to_string(), name));
    }
    if cals.is_empty() {
        bail!("signed in, but found no calendar that takes events");
    }
    Ok(cals)
}

/// When an event is: a whole day, or from..to (unix seconds).
pub enum When {
    Day(chrono::NaiveDate),
    At(i64, i64),
}

pub struct NewEvent<'a> {
    pub title: &'a str,
    pub when: When,
    pub location: &'a str,
    pub notes: &'a str,
}

/// iCalendar text: backslash, semicolon, comma and line breaks escaped; other control characters dropped, so nothing
/// in a title can start a property of its own.
fn esc(s: &str) -> String {
    s.replace("\r\n", "\n")
        .chars()
        .filter_map(|c| match c {
            '\\' => Some("\\\\".to_string()),
            ';' => Some("\\;".to_string()),
            ',' => Some("\\,".to_string()),
            '\n' => Some("\\n".to_string()),
            c if c.is_control() => None,
            c => Some(c.to_string()),
        })
        .collect()
}

/// Lines longer than 75 bytes are folded (a CRLF and a space), never inside a character.
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut len = 0;
    for c in line.chars() {
        if len + c.len_utf8() > 75 {
            out.push_str("\r\n ");
            len = 1;
        }
        out.push(c);
        len += c.len_utf8();
    }
    out + "\r\n"
}

fn ics(ev: &NewEvent, uid: &str, now: i64) -> String {
    let utc = |t: i64| chrono::DateTime::from_timestamp(t, 0).map(|d| d.format("%Y%m%dT%H%M%SZ").to_string()).unwrap_or_default();
    let mut lines = vec!["BEGIN:VCALENDAR".to_string(), "VERSION:2.0".into(), "PRODID:-//Mimir//EN".into(), "BEGIN:VEVENT".into(), format!("UID:{uid}"), format!("DTSTAMP:{}", utc(now))];
    match ev.when {
        When::Day(d) => {
            lines.push(format!("DTSTART;VALUE=DATE:{}", d.format("%Y%m%d")));
            lines.push(format!("DTEND;VALUE=DATE:{}", (d + chrono::Duration::days(1)).format("%Y%m%d")));
        }
        When::At(from, to) => {
            lines.push(format!("DTSTART:{}", utc(from)));
            lines.push(format!("DTEND:{}", utc(to)));
        }
    }
    lines.push(format!("SUMMARY:{}", esc(ev.title.trim())));
    if !ev.location.trim().is_empty() {
        lines.push(format!("LOCATION:{}", esc(ev.location.trim())));
    }
    if !ev.notes.trim().is_empty() {
        lines.push(format!("DESCRIPTION:{}", esc(ev.notes.trim())));
    }
    lines.extend(["END:VEVENT".into(), "END:VCALENDAR".into()]);
    lines.iter().map(|l| fold(l)).collect()
}

/// Put one new event in the calendar (it never replaces one: If-None-Match).
pub async fn add(cal: &Cal, ev: &NewEvent<'_>) -> Result<()> {
    let id = crate::random_hex(16)?;
    let mut base = Url::parse(&cal.url)?;
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    let url = base.join(&format!("{id}.ics"))?;
    checked(&url)?;
    let r = client()?
        .put(url)
        .basic_auth(&cal.user, Some(&cal.pass))
        .header("Content-Type", "text/calendar; charset=utf-8")
        .header("If-None-Match", "*")
        .body(ics(ev, &format!("{id}@mimir"), crate::db::now()))
        .send()
        .await
        .context("couldn't reach the calendar server")?;
    match r.status() {
        s if s.is_success() => Ok(()),
        StatusCode::UNAUTHORIZED => bail!("the calendar server didn't accept the app password any more (Settings → Calendar)"),
        StatusCode::FORBIDDEN => bail!("{} doesn't take new events from this login", cal.name),
        s => bail!("the calendar server said {s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multistatus_with_any_prefix() {
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/</D:href><D:propstat><D:prop>
<D:current-user-principal><D:href>/123/principal/</D:href></D:current-user-principal></D:prop></D:propstat></D:response></D:multistatus>"#;
        assert_eq!(parse(xml).unwrap()[0].principal, "/123/principal/");
        let list = r#"<multistatus xmlns="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav">
<response><href>/cal/me/</href><propstat><prop><resourcetype><collection/></resourcetype></prop></propstat></response>
<response><href>/cal/me/home/</href><propstat><prop><resourcetype><collection/><cal:calendar/></resourcetype><displayname>Home &amp; family</displayname>
<cal:supported-calendar-component-set><cal:comp name="VEVENT"/></cal:supported-calendar-component-set></prop></propstat></response>
<response><href>/cal/me/tasks/</href><propstat><prop><resourcetype><collection/><cal:calendar/></resourcetype><displayname>Tasks</displayname>
<cal:supported-calendar-component-set><cal:comp name="VTODO"></cal:comp></cal:supported-calendar-component-set></prop></propstat></response></multistatus>"#;
        let v = parse(list).unwrap();
        assert_eq!(v.len(), 3);
        assert!(!v[0].calendar && v[1].calendar && v[2].calendar);
        assert_eq!((v[1].name.as_str(), v[1].comps.as_slice()), ("Home & family", &["VEVENT".to_string()][..]));
        assert_eq!(v[2].comps, vec!["VTODO"]);
    }

    #[test]
    fn event_text_cant_add_properties() {
        let ev = NewEvent { title: "Dentist; check-up\r\nATTENDEE:mailto:evil@x.com", when: When::At(1_791_810_000, 1_791_811_800), location: "Room 4, 2nd floor", notes: "" };
        let out = ics(&ev, "abc@mimir", 1_791_800_000);
        assert!(out.contains("SUMMARY:Dentist\\; check-up\\nATTENDEE:mailto:evil@x.com\r\n"), "{out}");
        assert!(!out.contains("\r\nATTENDEE"), "{out}");
        assert!(out.contains("LOCATION:Room 4\\, 2nd floor\r\n") && out.contains("DTSTART:20261012T130000Z"), "{out}");
        assert!(!out.contains("DESCRIPTION"));
        let day = ics(&NewEvent { title: "Holiday", when: When::Day(chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap()), location: "", notes: "" }, "x", 0);
        assert!(day.contains("DTSTART;VALUE=DATE:20261012\r\nDTEND;VALUE=DATE:20261013\r\n"), "{day}");
        let long = fold(&format!("DESCRIPTION:{}", "é".repeat(60)));
        assert!(long.split("\r\n").all(|l| l.len() <= 75) && long.contains("\r\n "), "{long}");
    }

    /// Against a real CalDAV server on this computer: Radicale with a user me:pw and a calendar at /me/home/
    /// (`pip install radicale`, MKCALENDAR it with curl), then `cargo test -p mimir-core radicale -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn radicale_finds_the_calendar_and_adds_an_event() {
        let cals = discover("http://127.0.0.1:5232", "me", "pw").await.unwrap();
        assert_eq!(cals.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec!["http://127.0.0.1:5232/me/home/"], "{cals:?}");
        assert!(discover("http://127.0.0.1:5232", "me", "wrong").await.unwrap_err().to_string().contains("didn't accept"));
        let cal = Cal { url: cals[0].0.clone(), name: cals[0].1.clone(), user: "me".into(), pass: "pw".into() };
        add(&cal, &NewEvent { title: "Dentist, check-up", when: When::At(crate::db::now() + 86400, crate::db::now() + 88200), location: "Room 4", notes: "Bring the card" }).await.unwrap();
    }

    #[test]
    fn only_https_except_this_computer() {
        assert!(checked(&Url::parse("https://caldav.icloud.com/").unwrap()).is_ok());
        assert!(checked(&Url::parse("http://127.0.0.1:5232/").unwrap()).is_ok());
        assert!(checked(&Url::parse("http://caldav.example.com/").unwrap()).is_err());
    }
}
