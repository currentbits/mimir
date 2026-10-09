//! Email, read-only: the person's mailbox over IMAP with an app password (Gmail, iCloud, Fastmail, Outlook…), for the
//! bots' email_search and email_read. Mail is only looked at (EXAMINE, BODY.PEEK): nothing is marked read, moved or
//! deleted. What's in a message is someone else's words, so reading it counts as outside content (see tools.rs).
//! TLS always, except to this computer (a local bridge such as Proton Mail Bridge listens on 127.0.0.1 without it).

use anyhow::{Context, Result, bail};
use async_imap::Session;
use futures_util::TryStreamExt;
use mail_parser::MimeHeaders;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_util::either::Either;

/// The secret-store key for the app password; server, port and address are settings.
pub const SECRET: &str = "email_password";

type Stream = Either<tokio_rustls::client::TlsStream<TcpStream>, TcpStream>;

#[derive(Clone)]
pub struct Account {
    pub host: String,
    pub port: u16,
    pub user: String,
    pass: String,
    /// Outgoing mail (SMTP): set in Settings, else worked out from the incoming server (smtp_guess).
    pub smtp_host: String,
    pub smtp_port: u16,
}

/// The usual outgoing server for an incoming one: imap.gmail.com → smtp.gmail.com, and the odd ones by name.
pub fn smtp_guess(imap: &str) -> String {
    match imap.trim() {
        "outlook.office365.com" | "imap-mail.outlook.com" => "smtp.office365.com".into(),
        h if local(h) => h.into(),
        h => h.strip_prefix("imap.").map(|rest| format!("smtp.{rest}")).unwrap_or_else(|| h.into()),
    }
}

/// The account set up in Settings, if any.
pub fn account(db: &crate::db::Db) -> Option<Account> {
    let s = |k: &str| db.setting(k).ok().flatten().filter(|v| !v.is_empty());
    let host = s("email_host")?;
    Some(Account {
        smtp_host: s("email_smtp_host").unwrap_or_else(|| smtp_guess(&host)),
        smtp_port: s("email_smtp_port").and_then(|p| p.parse().ok()).unwrap_or(465),
        host,
        port: s("email_port").and_then(|p| p.parse().ok()).unwrap_or(993),
        user: s("email_user")?,
        pass: crate::host::secret_get(SECRET)?,
    })
}

impl Account {
    /// The app password, for the calendar on the same login (iCloud and Fastmail use one for both).
    pub(crate) fn pass(&self) -> &str {
        &self.pass
    }

    pub fn new(host: &str, port: u16, user: &str, pass: &str) -> Self {
        Account { smtp_host: smtp_guess(host), smtp_port: 465, host: host.trim().into(), port, user: user.trim().into(), pass: pass.into() }
    }
}

fn local(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

async fn session(a: &Account) -> Result<Session<Stream>> {
    let wait = Duration::from_secs(20);
    let tcp = tokio::time::timeout(wait, TcpStream::connect((a.host.as_str(), a.port))).await.context("the mail server didn't answer")??;
    let stream: Stream = if local(&a.host) && a.port != 993 {
        Either::Right(tcp)
    } else {
        let mut roots = tokio_rustls::rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let cfg = tokio_rustls::rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        let name = tokio_rustls::rustls::pki_types::ServerName::try_from(a.host.clone()).context("that isn't a server name")?;
        Either::Left(tokio::time::timeout(wait, tokio_rustls::TlsConnector::from(Arc::new(cfg)).connect(name, tcp)).await.context("no secure connection to the mail server")??)
    };
    let mut client = async_imap::Client::new(stream);
    client.read_response().await?.context("the mail server closed the connection")?;
    let session = tokio::time::timeout(wait, client.login(&a.user, &a.pass))
        .await
        .context("the mail server didn't answer the login")?
        .map_err(|(e, _)| anyhow::anyhow!("couldn't sign in to your email ({e}); check the address and the app password"))?;
    Ok(session)
}

/// A string for an IMAP search: quoted, without anything that could end the command.
fn quoted(s: &str) -> Result<String> {
    if s.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
        bail!("no line breaks in a search");
    }
    Ok(format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// The IMAP search for the asked filters (all mail when there are none).
pub fn criteria(text: Option<&str>, from: Option<&str>, since: Option<chrono::NaiveDate>, unread: bool) -> Result<String> {
    let mut c = vec![];
    if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
        c.push(format!("TEXT {}", quoted(t.trim())?));
    }
    if let Some(f) = from.filter(|f| !f.trim().is_empty()) {
        c.push(format!("FROM {}", quoted(f.trim())?));
    }
    if let Some(d) = since {
        c.push(format!("SINCE {}", d.format("%-d-%b-%Y")));
    }
    if unread {
        c.push("UNSEEN".into());
    }
    Ok(if c.is_empty() { "ALL".into() } else { c.join(" ") })
}

pub struct Summary {
    pub uid: u32,
    pub date: String,
    pub from: String,
    pub subject: String,
    pub unread: bool,
}

fn who(a: Option<&mail_parser::Address>) -> String {
    a.and_then(|a| a.first()).map(|a| match (a.name(), a.address()) {
        (Some(n), Some(m)) => format!("{n} <{m}>"),
        (Some(n), None) => n.to_string(),
        (None, Some(m)) => m.to_string(),
        _ => String::new(),
    })
    .unwrap_or_default()
}

fn date(m: &mail_parser::Message) -> String {
    m.date()
        .and_then(|d| chrono::DateTime::from_timestamp(d.to_timestamp(), 0))
        .map(|d| d.with_timezone(&chrono::Local).format("%a %-d %b %H:%M").to_string())
        .unwrap_or_default()
}

/// The newest messages in a folder matching the filters, newest first (headers only).
async fn search_now(a: &Account, folder: &str, query: &str, limit: usize) -> Result<Vec<Summary>> {
    let mut s = session(a).await?;
    s.examine(folder).await.with_context(|| format!("no folder called {folder}"))?;
    let mut uids: Vec<u32> = s.uid_search(query).await?.into_iter().collect();
    uids.sort_unstable_by(|x, y| y.cmp(x));
    uids.truncate(limit);
    let mut out = vec![];
    if !uids.is_empty() {
        let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let fetched: Vec<_> = s.uid_fetch(&set, "(UID FLAGS BODY.PEEK[HEADER])").await?.try_collect().await?;
        for f in fetched {
            let (Some(uid), Some(h)) = (f.uid, f.header()) else { continue };
            let Some(m) = mail_parser::MessageParser::default().parse(h) else { continue };
            let unread = !f.flags().any(|fl| matches!(fl, async_imap::types::Flag::Seen));
            out.push(Summary { uid, date: date(&m), from: who(m.from()), subject: m.subject().unwrap_or("(no subject)").to_string(), unread });
        }
    }
    let _ = s.logout().await;
    out.sort_unstable_by_key(|m| std::cmp::Reverse(m.uid));
    Ok(out)
}

/// One message as text: who, when, subject, the text (an HTML-only one as text), and its attachments by name.
async fn read_now(a: &Account, folder: &str, uid: u32) -> Result<String> {
    let mut s = session(a).await?;
    s.examine(folder).await.with_context(|| format!("no folder called {folder}"))?;
    // Its size first: a huge one (or one made huge to clog a reply) isn't pulled in to show a few pages of text.
    let size: Vec<_> = s.uid_fetch(uid.to_string(), "(UID RFC822.SIZE)").await?.try_collect().await?;
    if size.iter().filter_map(|f| f.size).any(|n| n > 10_000_000) {
        let _ = s.logout().await;
        bail!("that email is over 10 MB (big attachments); too big to read here");
    }
    let fetched: Vec<_> = s.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])").await?.try_collect().await?;
    let _ = s.logout().await;
    let raw = fetched.iter().find_map(|f| f.body()).with_context(|| format!("no message #{uid} in {folder}"))?;
    let m = mail_parser::MessageParser::default().parse(raw).context("couldn't read that message")?;
    let text = m.body_text(0).map(|t| t.to_string()).unwrap_or_default();
    let attachments: Vec<String> = m.attachments().filter_map(|p| p.attachment_name().map(String::from)).collect();
    let to = m.to().map(|t| t.iter().filter_map(|a| a.address()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    Ok(format!(
        "From: {}\nTo: {to}\nDate: {}\nSubject: {}\n\n{}{}",
        who(m.from()),
        date(&m),
        m.subject().unwrap_or("(no subject)"),
        crate::tools::truncate(text.trim(), 15_000),
        if attachments.is_empty() { String::new() } else { format!("\n\nAttachments: {}", attachments.join(", ")) }
    ))
}

/// A draft the bot writes for the person to read and send from their own mail app.
pub struct Draft<'a> {
    pub to: &'a str,
    pub cc: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
    /// The message (in the inbox) it answers, so the draft lands in that thread.
    pub reply_to: Option<u32>,
}

/// Addresses for a header: plain addresses, comma-separated, at most ten, nothing that could start another header.
pub(crate) fn addresses(list: &str) -> Result<String> {
    let v: Vec<&str> = list.split(',').map(str::trim).filter(|a| !a.is_empty()).collect();
    if v.len() > 10 {
        bail!("at most ten addresses");
    }
    for a in &v {
        let ok = a.len() < 254 && a.matches('@').count() == 1 && !a.starts_with('@') && !a.ends_with('@') && !a.chars().any(|c| c.is_whitespace() || c.is_control() || "<>\",;:()[]".contains(c));
        if !ok {
            bail!("{a} isn't an email address");
        }
    }
    Ok(v.join(", "))
}

/// A header value as UTF-8 that any mail app reads: plain ASCII as is, anything else base64-encoded (RFC 2047).
pub(crate) fn header_text(s: &str) -> Result<String> {
    if s.chars().any(|c| c.is_control()) {
        bail!("no line breaks in a subject");
    }
    Ok(if s.is_ascii() { s.to_string() } else { format!("=?utf-8?B?{}?=", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, s)) })
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() < 250 && !id.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>'))
}

/// Put a draft in the account's Drafts folder (the one the server marks as Drafts). Returns the folder's name.
/// The message as it will be saved or sent: headers checked, a reply threaded under the original (looked up in the
/// inbox), plain UTF-8 text. Returns (recipients for the envelope, the message, its Message-ID).
async fn compose(a: &Account, s: &mut Session<Stream>, d: &Draft<'_>) -> Result<(Vec<String>, String, String)> {
    let to = addresses(d.to)?;
    if to.is_empty() {
        bail!("who is it to?");
    }
    let cc = addresses(d.cc)?;
    let subject = header_text(d.subject.trim())?;
    // A reply: the original's Message-ID (and its thread), so mail apps put it in the conversation.
    let mut threading = String::new();
    if let Some(uid) = d.reply_to {
        s.examine("INBOX").await?;
        let fetched: Vec<_> = s.uid_fetch(uid.to_string(), "(UID BODY.PEEK[HEADER])").await?.try_collect().await?;
        if let Some(m) = fetched.iter().find_map(|f| f.header()).and_then(|h| mail_parser::MessageParser::default().parse(h))
            && let Some(id) = m.message_id()
            // An id is one token; anything else (a carriage return, a space, brackets) isn't copied into a header.
            && id_ok(id)
        {
            let refs: Vec<String> = m.references().as_text_list().map(|l| l.iter().filter(|r| id_ok(r)).map(|r| format!("<{r}>")).collect()).unwrap_or_default();
            threading = format!("In-Reply-To: <{id}>\r\nReferences: {}<{id}>\r\n", refs.iter().map(|r| format!("{r} ")).collect::<String>());
        }
    }
    let body = d.body.replace("\r\n", "\n").replace('\n', "\r\n");
    let id = format!("{}@mimir.local", crate::random_hex(12)?);
    let msg = format!(
        "From: {}\r\nTo: {to}\r\n{}Subject: {subject}\r\nDate: {}\r\nMessage-ID: <{id}>\r\n{threading}MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{body}\r\n",
        a.user,
        if cc.is_empty() { String::new() } else { format!("Cc: {cc}\r\n") },
        chrono::Local::now().to_rfc2822(),
    );
    let rcpt = to.split(", ").chain(cc.split(", ")).filter(|x| !x.is_empty()).map(String::from).collect();
    Ok((rcpt, msg, id))
}

async fn draft_now(a: &Account, d: &Draft<'_>) -> Result<String> {
    let mut s = session(a).await?;
    let (_, msg, _) = compose(a, &mut s, d).await?;
    let folder = special(&mut s, async_imap::types::NameAttribute::Drafts).await?.unwrap_or_else(|| "Drafts".into());
    s.append(&folder, Some("(\\Draft \\Seen)"), None, msg.as_bytes()).await?;
    let _ = s.logout().await;
    Ok(folder)
}

/// The folder the server marks with this role (\\Drafts, \\Sent), if any.
async fn special(s: &mut Session<Stream>, role: async_imap::types::NameAttribute<'static>) -> Result<Option<String>> {
    let names: Vec<_> = s.list(Some(""), Some("*")).await?.try_collect().await?;
    Ok(names.iter().find(|n| n.attributes().contains(&role)).map(|n| n.name().to_string()))
}

/// After sending: a copy in the Sent folder, unless the server filed one itself (Gmail does, iCloud doesn't). Waits a
/// moment for the server, then looks for the Message-ID; only if it isn't there is a copy saved. Some(folder) when
/// Mimir saved it.
async fn file_sent(s: &mut Session<Stream>, id: &str, msg: &str, wait: Duration) -> Result<Option<String>> {
    let Some(folder) = special(s, async_imap::types::NameAttribute::Sent).await? else { return Ok(None) };
    // ponytail: a server slower than `wait` to file its own copy gets a second one; Gmail files it at once.
    tokio::time::sleep(wait).await;
    s.examine(&folder).await?;
    if !s.uid_search(format!("HEADER Message-ID {}", quoted(id)?)).await?.is_empty() {
        return Ok(None);
    }
    s.append(&folder, Some("(\\Seen)"), None, msg.as_bytes()).await?;
    Ok(Some(folder))
}

/// Send an email as the person, over their outgoing server (TLS from the start on 465, STARTTLS on 587; plain only to
/// this computer). The caller has had it approved. Keeps the mail session open for filing the copy.
async fn deliver(a: &Account, d: &Draft<'_>) -> Result<(Session<Stream>, String, String)> {
    use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor, address::Envelope, transport::smtp::authentication::Credentials};
    let mut s = session(a).await?;
    let (rcpt, msg, id) = compose(a, &mut s, d).await?;
    let from: lettre::Address = a.user.parse().context("your email address doesn't look like one")?;
    let to: Vec<lettre::Address> = rcpt.iter().map(|r| r.parse()).collect::<Result<_, _>>().context("a recipient isn't an address")?;
    let envelope = Envelope::new(Some(from), to)?;
    let creds = Credentials::new(a.user.clone(), a.pass.clone());
    let mailer = if local(&a.smtp_host) {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&a.smtp_host).port(a.smtp_port).credentials(creds).build()
    } else if a.smtp_port == 587 {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&a.smtp_host)?.port(587).credentials(creds).build()
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&a.smtp_host)?.port(a.smtp_port).credentials(creds).build()
    };
    mailer.send_raw(&envelope, msg.as_bytes()).await.map_err(|e| anyhow::anyhow!("couldn't send through {}: {e}", a.smtp_host))?;
    Ok((s, id, msg))
}

/// Every mail operation, start to end (greeting, sign-in, the commands), within this: a stalled server can't hang a reply.
const LIMIT: Duration = Duration::from_secs(60);

async fn bounded<T>(f: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(LIMIT, f).await.context("the mail server took too long to answer")?
}

pub async fn search(a: &Account, folder: &str, query: &str, limit: usize) -> Result<Vec<Summary>> {
    bounded(search_now(a, folder, query, limit)).await
}

pub async fn read(a: &Account, folder: &str, uid: u32) -> Result<String> {
    bounded(read_now(a, folder, uid)).await
}

/// Sends, then files a copy in Sent (above), and says where (empty when the server keeps its own). Once it's sent,
/// nothing turns that into an error: the bot would say it failed, or send it again.
pub async fn send(a: &Account, d: &Draft<'_>) -> Result<String> {
    let (mut s, id, msg) = bounded(deliver(a, d)).await?;
    let note = match tokio::time::timeout(Duration::from_secs(20), file_sent(&mut s, &id, &msg, Duration::from_secs(3))).await {
        Ok(Ok(Some(folder))) => format!(" A copy is in {folder}."),
        Ok(Ok(None)) => String::new(),
        Ok(Err(e)) => format!(" (No copy was saved to Sent: {e}.)"),
        Err(_) => " (No copy was saved to Sent: the mail server took too long.)".into(),
    };
    let _ = tokio::time::timeout(Duration::from_secs(5), s.logout()).await;
    Ok(note)
}

pub async fn draft(a: &Account, d: &Draft<'_>) -> Result<String> {
    bounded(draft_now(a, d)).await
}

pub async fn check(a: &Account) -> Result<u32> {
    bounded(check_now(a)).await
}

/// Sign in and look at the inbox once, before saving an account: how many messages it has.
async fn check_now(a: &Account) -> Result<u32> {
    let mut s = session(a).await?;
    let inbox = s.examine("INBOX").await?;
    let _ = s.logout().await;
    Ok(inbox.exists)
}

#[cfg(test)]
mod tests {
    /// Against scripts/fake-mail.py: `python3 scripts/fake-mail.py` (like iCloud: Mimir saves the Sent copy) or with
    /// `--files-sent` (like Gmail: the server does, so no second copy), then
    /// `FAKE_MAIL=icloud|gmail cargo test -p mimir-core fake_mail -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn fake_mail_sends_and_files_one_copy() {
        let mode = std::env::var("FAKE_MAIL").unwrap_or_else(|_| "icloud".into());
        let mut a = super::Account::new("127.0.0.1", 1143, "me@example.com", "pw");
        (a.smtp_host, a.smtp_port) = ("127.0.0.1".into(), 1025);
        let d = super::Draft { to: "sam@example.com", cc: "", subject: "Invoice 4411", body: "Paid on Friday.", reply_to: Some(1) };
        let note = super::send(&a, &d).await.unwrap();
        if mode == "gmail" {
            assert_eq!(note, "", "the server filed it, so no copy");
        } else {
            assert_eq!(note, " A copy is in Sent Messages.");
            let copy = std::fs::read_to_string("/tmp/fake-mail/appended-Sent Messages.eml").unwrap();
            assert!(copy.contains("Paid on Friday.") && copy.contains("In-Reply-To: <inv4411@example.com>"), "{copy}");
        }
    }

    #[test]
    fn searches_are_quoted_and_cant_break_out() {
        let d = chrono::NaiveDate::from_ymd_opt(2026, 10, 5);
        assert_eq!(super::criteria(Some("invoice"), Some("sam@x.com"), d, true).unwrap(), r#"TEXT "invoice" FROM "sam@x.com" SINCE 5-Oct-2026 UNSEEN"#);
        assert_eq!(super::criteria(None, None, None, false).unwrap(), "ALL");
        assert_eq!(super::criteria(Some(r#"a"b\c"#), None, None, false).unwrap(), r#"TEXT "a\"b\\c""#);
        assert!(super::criteria(Some("x\r\nA1 DELETE INBOX"), None, None, false).is_err());
    }

    #[test]
    fn draft_headers_cant_be_smuggled() {
        assert_eq!(super::addresses("sam@x.com, priya@y.org").unwrap(), "sam@x.com, priya@y.org");
        for bad in ["sam@x.com\r\nBcc: evil@z.com", "not an address", "a@b@c", "<a@b.com>"] {
            assert!(super::addresses(bad).is_err(), "{bad}");
        }
        assert!(super::header_text("Re: invoice\r\nBcc: evil@z.com").is_err());
        assert_eq!(super::header_text("Factuur €1.250").unwrap(), "=?utf-8?B?RmFjdHV1ciDigqwxLjI1MA==?=");
        assert_eq!(super::smtp_guess("imap.gmail.com"), "smtp.gmail.com");
        assert_eq!(super::smtp_guess("imap.mail.me.com"), "smtp.mail.me.com");
        assert_eq!(super::smtp_guess("outlook.office365.com"), "smtp.office365.com");
        assert!(super::id_ok("inv4411@example.com") && !super::id_ok("a\rBcc: x@evil") && !super::id_ok("a b"));
    }
}
