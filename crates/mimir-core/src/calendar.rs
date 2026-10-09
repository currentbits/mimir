//! Your calendar, from its private iCal link (Google Calendar's "secret address in iCal format", Outlook's published
//! calendar): which meeting is on now (its name and who's invited) and which are about to start. No account access,
//! no permissions; the link is kept in the keychain. Recurring meetings are expanded with their rules.

use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The secret-store key for the link.
pub const SECRET: &str = "calendar_url";

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub title: String,
    pub start: i64,
    pub end: i64,
    /// Invited people, by name (or address when there's no name), the organiser included.
    pub attendees: Vec<String>,
}

/// The link, cleaned up (webcal:// is https:// in disguise).
pub fn normalise(url: &str) -> Option<String> {
    let u = url.trim();
    let u = u.strip_prefix("webcal://").map(|r| format!("https://{r}")).unwrap_or_else(|| u.to_string());
    (u.starts_with("https://") && u.len() > 12).then_some(u)
}

/// Events overlapping [from, to) (unix seconds), recurring ones expanded; cancelled and all-day ones left out.
pub fn events_between(ics: &str, from: i64, to: i64) -> Vec<Event> {
    // Unfold: a line starting with a space continues the one before.
    let mut lines: Vec<String> = vec![];
    for l in ics.lines() {
        match l.strip_prefix(' ').or_else(|| l.strip_prefix('\t')) {
            Some(rest) if !lines.is_empty() => lines.last_mut().unwrap().push_str(rest),
            _ => lines.push(l.trim_end_matches('\r').to_string()),
        }
    }
    let mut events: Vec<Vec<(String, String, String)>> = vec![]; // each: (name, params, value)
    let mut ev: Option<Vec<(String, String, String)>> = None;
    for l in lines {
        if l == "BEGIN:VEVENT" {
            ev = Some(vec![]);
        } else if l == "END:VEVENT" {
            if let Some(props) = ev.take() {
                events.push(props);
            }
        } else if let Some(props) = ev.as_mut()
            && let Some((key, value)) = l.split_once(':')
        {
            let (name, params) = key.split_once(';').unwrap_or((key, ""));
            props.push((name.to_ascii_uppercase(), params.to_string(), value.to_string()));
        }
    }
    // A moved or cancelled single occurrence is its own VEVENT (same UID, with RECURRENCE-ID): the series skips it.
    let prop = |e: &[(String, String, String)], n: &str| e.iter().find(|p| p.0 == n).cloned();
    let replaced: std::collections::HashSet<(String, i64)> = events
        .iter()
        .filter_map(|e| Some((prop(e, "UID")?.2, prop(e, "RECURRENCE-ID").and_then(|r| parse_time(&r.1, &r.2))?)))
        .collect();
    let mut out: Vec<Event> = events.iter().flat_map(|e| expand(e, from, to, &replaced)).collect();
    out.sort_by_key(|e| e.start);
    out
}

fn expand(props: &[(String, String, String)], from: i64, to: i64, replaced: &std::collections::HashSet<(String, i64)>) -> Vec<Event> {
    let get = |n: &str| props.iter().find(|p| p.0 == n);
    // A changed or moved single occurrence is its own VEVENT (RECURRENCE-ID); it's read like any other one.
    if get("STATUS").is_some_and(|p| p.2.eq_ignore_ascii_case("CANCELLED")) {
        return vec![];
    }
    let Some((_, start_params, start_value)) = get("DTSTART") else { return vec![] };
    if start_params.contains("VALUE=DATE") && !start_params.contains("VALUE=DATE-TIME") {
        return vec![]; // all-day: a holiday or a reminder, not a call
    }
    let Some(start) = parse_time(start_params, start_value) else { return vec![] };
    let length = get("DTEND").and_then(|p| parse_time(&p.1, &p.2)).map(|end| end - start).or_else(|| get("DURATION").and_then(|p| duration(&p.2))).unwrap_or(3600).clamp(0, 30 * 86_400);
    let title = get("SUMMARY").map(|p| unescape(&p.2)).filter(|t| !t.is_empty()).unwrap_or_else(|| "Meeting".into());
    let attendees: Vec<String> = props
        .iter()
        .filter(|p| p.0 == "ATTENDEE" || p.0 == "ORGANIZER")
        .filter(|p| !p.1.to_ascii_uppercase().contains("CUTYPE=ROOM") && !p.1.to_ascii_uppercase().contains("CUTYPE=RESOURCE"))
        .filter_map(|p| {
            let cn = p.1.split(';').find_map(|kv| kv.strip_prefix("CN=")).map(|c| c.trim_matches('"').to_string());
            cn.or_else(|| p.2.strip_prefix("mailto:").or(p.2.strip_prefix("MAILTO:")).map(str::to_string))
        })
        .fold(vec![], |mut v, a| {
            if !v.contains(&a) {
                v.push(a);
            }
            v
        });
    let starts: Vec<i64> = match get("RRULE") {
        None => vec![start],
        Some(rule) => {
            // rrule takes the event's own DTSTART/RRULE/EXDATE lines.
            let mut spec = format!("DTSTART{}:{}\nRRULE:{}", if start_params.is_empty() { String::new() } else { format!(";{start_params}") }, start_value, rule.2);
            for ex in props.iter().filter(|p| p.0 == "EXDATE") {
                spec.push_str(&format!("\nEXDATE{}:{}", if ex.1.is_empty() { String::new() } else { format!(";{}", ex.1) }, ex.2));
            }
            // Meetings don't repeat every minute or second; working those out from an old start date could take minutes,
            // so such a rule counts as its first occurrence only.
            let sub_hourly = rule.2.to_ascii_uppercase().contains("FREQ=SECONDLY") || rule.2.to_ascii_uppercase().contains("FREQ=MINUTELY");
            match spec.parse::<rrule::RRuleSet>().ok().filter(|_| !sub_hourly).ok_or(()) {
                Ok(set) => {
                    let after = Utc.timestamp_opt(from - length, 0).single().unwrap_or_default().with_timezone(&rrule::Tz::UTC);
                    let before = Utc.timestamp_opt(to, 0).single().unwrap_or_default().with_timezone(&rrule::Tz::UTC);
                    set.after(after).before(before).all(50).dates.into_iter().map(|d| d.timestamp()).collect()
                }
                // A rule it can't read (or a Windows time zone name): at least the first occurrence.
                Err(_) => vec![start],
            }
        }
    };
    let uid = get("UID").map(|p| p.2.clone()).unwrap_or_default();
    let series = get("RRULE").is_some();
    starts
        .into_iter()
        .filter(|s| s + length > from && *s < to)
        .filter(|s| !(series && replaced.contains(&(uid.clone(), *s))))
        .map(|s| Event { title: title.clone(), start: s, end: s + length, attendees: attendees.clone() })
        .collect()
}

/// "20261007T090000Z", "TZID=Europe/Amsterdam" + "20261007T090000", or floating local time.
fn parse_time(params: &str, value: &str) -> Option<i64> {
    if let Some(v) = value.strip_suffix('Z') {
        return NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok().map(|t| t.and_utc().timestamp());
    }
    let t = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok().or_else(|| NaiveDate::parse_from_str(value, "%Y%m%d").ok()?.and_hms_opt(0, 0, 0))?;
    let tz = params.split(';').find_map(|kv| kv.strip_prefix("TZID=")).map(|z| z.trim_matches('"'));
    match tz.and_then(|z| z.parse::<chrono_tz::Tz>().ok()) {
        Some(tz) => tz.from_local_datetime(&t).earliest().map(|d| d.timestamp()),
        // ponytail: Outlook's Windows zone names ("W. Europe Standard Time") read as this computer's time zone
        None => Local.from_local_datetime(&t).earliest().map(|d| d.timestamp()),
    }
}

/// "PT1H30M", "PT45M", "P1D".
fn duration(v: &str) -> Option<i64> {
    let (mut secs, mut num) = (0i64, String::new());
    for c in v.trim_start_matches(['+', 'P']).chars() {
        match c {
            '0'..='9' => num.push(c),
            'T' => {}
            u => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                let unit = match u {
                    'W' => 604_800,
                    'D' => 86_400,
                    'H' => 3_600,
                    'M' => 60,
                    'S' => 1,
                    _ => return None,
                };
                secs = secs.checked_add(n.checked_mul(unit)?)?;
            }
        }
    }
    Some(secs)
}

fn unescape(s: &str) -> String {
    s.replace("\\n", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\").trim().to_string()
}

/// The calendar's text, fetched at most every 10 minutes.
static CACHE: Mutex<Option<(String, Instant, String)>> = Mutex::new(None);

async fn ics() -> Result<String> {
    let url = crate::host::secret_get(SECRET).context("no calendar linked")?;
    if let Some((u, at, body)) = CACHE.lock().unwrap().as_ref()
        && *u == url
        && at.elapsed() < Duration::from_secs(600)
    {
        return Ok(body.clone());
    }
    let body = fetch(&url).await?;
    if !body.contains("BEGIN:VCALENDAR") {
        anyhow::bail!("that link isn't a calendar (no iCal data)");
    }
    *CACHE.lock().unwrap() = Some((url, Instant::now(), body.clone()));
    Ok(body)
}

/// The calendar's text, up to 10 MB (a calendar is far smaller; a link that keeps sending isn't one).
async fn fetch(url: &str) -> Result<String> {
    let mut resp = reqwest::Client::builder().timeout(Duration::from_secs(20)).build()?.get(url).send().await?.error_for_status()?;
    let mut bytes = vec![];
    while let Some(chunk) = resp.chunk().await? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > 10_000_000 {
            anyhow::bail!("that calendar is over 10 MB, too big to be a calendar link");
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Check a link before saving it: how many meetings it has this week.
pub async fn check(url: &str) -> Result<usize> {
    let body = fetch(url).await?;
    if !body.contains("BEGIN:VCALENDAR") {
        anyhow::bail!("that link isn't a calendar (no iCal data). In Google Calendar it's Settings → your calendar → \"Secret address in iCal format\".");
    }
    let now = crate::db::now();
    Ok(events_between(&body, now - 86_400, now + 6 * 86_400).len())
}

/// The meeting on now (started up to 10 min ago, or starting within 5): the one that started last.
pub async fn now() -> Option<Event> {
    let t = crate::db::now();
    let events = events_between(&ics().await.ok()?, t - 4 * 3600, t + 300);
    events.into_iter().filter(|e| e.start <= t + 300 && e.end > t).max_by_key(|e| e.start)
}

/// Everything on the calendar overlapping [from, to), in order: for the bots' calendar_list.
pub async fn events(from: i64, to: i64) -> Result<Vec<Event>> {
    let mut v = events_between(&ics().await?, from, to);
    v.sort_by_key(|e| e.start);
    Ok(v)
}

/// Meetings starting in [from, to): for "about to start" offers.
pub async fn starting(from: i64, to: i64) -> Vec<Event> {
    match ics().await {
        Ok(body) => events_between(&body, from, to).into_iter().filter(|e| e.start >= from && e.start < to).collect(),
        Err(_) => vec![],
    }
}

/// How an event start reads in a notification ("09:30").
pub fn clock(t: i64) -> String {
    DateTime::from_timestamp(t, 0).map(|d| d.with_timezone(&Local).format("%H:%M").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:standup-1\r\nSUMMARY:Weekly standup\r\nDTSTART;TZID=Europe/Amsterdam:20261005T093000\r\nDTEND;TZID=Europe/Amsterdam:20261005T094500\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE\r\nEXDATE;TZID=Europe/Amsterdam:20261012T093000\r\nORGANIZER;CN=Sam Lee:mailto:sam@example.com\r\nATTENDEE;CN=\"Priya Shah\";ROLE=REQ-PARTICIPANT:mailto:priya@example.com\r\nATTENDEE;CUTYPE=ROOM;CN=Room 4:mailto:room4@example.com\r\nATTENDEE:mailto:jo@example.com\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:standup-1\r\nRECURRENCE-ID;TZID=Europe/Amsterdam:20261014T093000\r\nSUMMARY:Weekly standup (moved)\r\nDTSTART;TZID=Europe/Amsterdam:20261014T110000\r\nDTEND;TZID=Europe/Amsterdam:20261014T111500\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Dentist\\, Dr. Lee\r\nDTSTART:20261007T130000Z\r\nDURATION:PT30M\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261007\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Cancelled sync\r\nSTATUS:CANCELLED\r\nDTSTART:20261007T150000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Long \r\n line folded\r\nDTSTART:20261008T080000Z\r\nDTEND:20261008T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn at(s: &str) -> i64 {
        DateTime::parse_from_rfc3339(s).unwrap().timestamp()
    }

    #[test]
    fn hostile_repeats_cant_stall() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nSUMMARY:Spam\nDTSTART:19000101T000000Z\nDURATION:P99999999999999999W\nRRULE:FREQ=SECONDLY\nEND:VEVENT\nEND:VCALENDAR\n";
        let t = std::time::Instant::now();
        let _ = super::events_between(ics, 1_791_000_000, 1_791_600_000);
        assert!(t.elapsed().as_secs() < 2, "took {:?}", t.elapsed());
        assert_eq!(super::duration("P99999999999999999W"), None, "overflow is no duration, not a panic");
    }

    #[test]
    fn reads_a_calendar() {
        let week = events_between(ICS, at("2026-10-05T00:00:00Z"), at("2026-10-15T00:00:00Z"));
        let names: Vec<(&str, i64)> = week.iter().map(|e| (e.title.as_str(), e.start)).collect();
        assert_eq!(
            names,
            vec![
                ("Weekly standup", at("2026-10-05T07:30:00Z")), // 09:30 Amsterdam (summer time)
                ("Weekly standup", at("2026-10-07T07:30:00Z")),
                ("Dentist, Dr. Lee", at("2026-10-07T13:00:00Z")),
                ("Long line folded", at("2026-10-08T08:00:00Z")),
                ("Weekly standup (moved)", at("2026-10-14T09:00:00Z")), // the 12th is skipped (EXDATE); the 14th moved to 11:00
            ]
        );
        assert_eq!(week[0].end - week[0].start, 900);
        assert_eq!(week[0].attendees, vec!["Sam Lee", "Priya Shah", "jo@example.com"], "rooms aren't people");
        assert_eq!(week[2].end - week[2].start, 1800);
        // On now: the standup, from its start until it ends.
        let mid = at("2026-10-07T07:40:00Z");
        assert_eq!(events_between(ICS, mid - 3600, mid + 300).iter().filter(|e| e.start <= mid + 300 && e.end > mid).count(), 1);
    }

    /// MIMIR_TEST_ICS=<https link> cargo test -p mimir-core calendar_real -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn calendar_real() {
        let url = normalise(&std::env::var("MIMIR_TEST_ICS").unwrap()).unwrap();
        println!("this week: {:?}", check(&url).await);
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
        let t = crate::db::now();
        let year = events_between(&body, t - 180 * 86_400, t + 180 * 86_400);
        println!("{} timed events in a year; {} lines of iCal", year.len(), body.lines().count());
    }

    #[test]
    fn links() {
        assert_eq!(normalise(" webcal://calendar.google.com/x/basic.ics ").as_deref(), Some("https://calendar.google.com/x/basic.ics"));
        assert_eq!(normalise("http://insecure.example/cal.ics"), None);
        assert_eq!(duration("PT1H30M"), Some(5400));
    }
}
