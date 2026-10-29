//! The person's own Mac, from the desktop app: send an email from Mail, a message from Messages, open an app. Each asks
//! first by default (gate.rs) and shows exactly what goes where. Only where the desktop app runs Mimir on a Mac: the
//! tools process it spawns reads that from the setting `host_mode` (Host::new writes it).

use anyhow::{Context, Result, bail};

/// Whether this Mimir is the desktop app on a Mac.
pub fn here(db: &crate::db::Db) -> bool {
    cfg!(target_os = "macos") && db.setting("host_mode").ok().flatten().as_deref() == Some("desktop")
}

/// Mail's script: everything it's given comes in as arguments, so nothing in a subject or body is read as AppleScript.
const MAIL: &str = r#"on run argv
  set {theSubject, theBody, toList, ccList} to {item 1 of argv, item 2 of argv, item 3 of argv, item 4 of argv}
  tell application "Mail"
    set m to make new outgoing message with properties {subject:theSubject, content:theBody, visible:false}
    tell m
      repeat with a in paragraphs of toList
        if contents of a is not "" then make new to recipient at end of to recipients with properties {address:contents of a}
      end repeat
      repeat with a in paragraphs of ccList
        if contents of a is not "" then make new cc recipient at end of cc recipients with properties {address:contents of a}
      end repeat
    end tell
    send m
  end tell
end run"#;

const MESSAGES: &str = r#"on run argv
  tell application "Messages" to send (item 1 of argv) to participant (item 2 of argv) of (1st account whose service type = iMessage)
end run"#;

/// osascript's arguments for an email: the script, then subject, body, and the addresses one per line.
pub(crate) fn mail_args(to: &str, cc: &str, subject: &str, body: &str) -> Result<Vec<String>> {
    let to = crate::email::addresses(to)?;
    if to.is_empty() {
        bail!("who is it to?");
    }
    let cc = crate::email::addresses(cc)?;
    let lines = |s: &str| s.split(", ").filter(|a| !a.is_empty()).collect::<Vec<_>>().join("\n");
    Ok(vec!["-e".into(), MAIL.into(), subject.into(), body.into(), lines(&to), lines(&cc)])
}

/// Who a message can go to: a phone number (+31…) or an Apple ID email.
pub(crate) fn handle(to: &str) -> Result<String> {
    let t: String = to.trim().chars().filter(|c| !c.is_whitespace() && !"()-".contains(*c)).collect();
    let phone = t.starts_with('+') && t.len() > 6 && t[1..].chars().all(|c| c.is_ascii_digit());
    if !phone && crate::email::addresses(&t).map(|a| a != t).unwrap_or(true) {
        bail!("send it to a phone number with its country code (+31…) or an Apple ID email");
    }
    Ok(t)
}

/// An app's name as Finder shows it ("Numbers", "Visual Studio Code"): letters, digits, spaces and . - _ only.
pub(crate) fn app_name(app: &str) -> Result<&str> {
    let a = app.trim().trim_end_matches(".app");
    if a.is_empty() || a.len() > 60 || !a.chars().all(|c| c.is_alphanumeric() || " .-_".contains(c)) {
        bail!("give the app's name as it shows in Finder, like Numbers");
    }
    Ok(a)
}

async fn osascript(args: &[String], app: &str) -> Result<()> {
    let out = tokio::process::Command::new("osascript").args(args).output().await.context("can't run osascript")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("-1743") {
            bail!("Mimir may not control {app}: turn it on in System Settings → Privacy & Security → Automation → Mimir");
        }
        bail!("{app} didn't do it: {}", crate::tools::truncate(err.trim(), 200));
    }
    Ok(())
}

pub async fn mail_send(to: &str, cc: &str, subject: &str, body: &str) -> Result<String> {
    osascript(&mail_args(to, cc, subject, body)?, "Mail").await?;
    Ok(format!("Sent from Mail on this Mac to {}", crate::email::addresses(to)?))
}

pub async fn messages_send(to: &str, text: &str) -> Result<String> {
    let to = handle(to)?;
    osascript(&["-e".into(), MESSAGES.into(), text.into(), to.clone()], "Messages").await?;
    Ok(format!("Sent from Messages on this Mac to {to}"))
}

/// Open an app, or a workspace file in it.
pub async fn app_open(app: &str, file: Option<std::path::PathBuf>) -> Result<String> {
    let app = app_name(app)?;
    let mut cmd = tokio::process::Command::new("open");
    cmd.args(["-a", app]);
    if let Some(f) = &file {
        cmd.arg(f);
    }
    let out = cmd.output().await.context("can't run open")?;
    if !out.status.success() {
        bail!("couldn't open {app}: {}", crate::tools::truncate(String::from_utf8_lossy(&out.stderr).trim(), 200));
    }
    Ok(format!("Opened {app}{}", file.map(|f| format!(" with {}", f.display())).unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn what_goes_to_mail_and_messages_is_checked_and_passed_as_data() {
        let a = super::mail_args("dana@acme.com, sam@acme.com", "", "Q4\" end tell", "Hi").unwrap();
        assert_eq!(a[2..], ["Q4\" end tell", "Hi", "dana@acme.com\nsam@acme.com", ""].map(String::from), "the subject stays data");
        assert!(super::mail_args("", "", "x", "y").is_err() && super::mail_args("not an address", "", "x", "y").is_err());
        assert_eq!(super::handle("+31 6 1234-5678").unwrap(), "+31612345678");
        assert!(super::handle("dana@icloud.com").is_ok() && super::handle("0612").is_err() && super::handle("x\"y").is_err());
        assert_eq!(super::app_name("Numbers.app").unwrap(), "Numbers");
        assert!(super::app_name("Terminal; rm -rf ~").is_err() && super::app_name("").is_err());
    }
}
