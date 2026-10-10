# Connect your email, calendar and web search

Bots work with what you connect: your email, your calendar and a web search service. Each is optional and takes a few
minutes. This page is the setup, service by service; what bots then do (and what always asks you first) is at the end
of each part.

Everything here is set in the app. The passwords and keys are stored in the OS keychain on your computer, or in the
server's private database when you [self-host](self-hosting.md); they never go into a bot's instructions.

## Email

Bots can search and read your inbox, write drafts, and send an email when you approve it. They use IMAP and SMTP, the
standard mail protocols, with an **app password**: a separate password just for Mimir that you can revoke any time,
without changing your real one.

### 1. Make an app password

| Provider | Where | Incoming server | Outgoing server |
|---|---|---|---|
| **Gmail** | [myaccount.google.com/apppasswords](https://myaccount.google.com/apppasswords) (2-Step Verification has to be on) | `imap.gmail.com` | `smtp.gmail.com` |
| **iCloud Mail** | [account.apple.com](https://account.apple.com) → Sign-In and Security → App-Specific Passwords | `imap.mail.me.com` | `smtp.mail.me.com` |
| **Fastmail** | Settings → Privacy & Security → App passwords | `imap.fastmail.com` | `smtp.fastmail.com` |
| **Proton Mail** | Install Proton Mail Bridge; it shows a password for the account (not tried with Mimir yet) | `127.0.0.1` (port `1143`) | `127.0.0.1` (port `1025`) |
| Any other | Your provider's help: "IMAP settings" | usually `imap.<domain>` | usually `smtp.<domain>` |

**Outlook, Hotmail and Microsoft 365 mail don't work yet:** Microsoft turned off app passwords for mail apps and
requires its own sign-in (OAuth), which Mimir doesn't do yet. Your Outlook *calendar* link does work (below).

### 2. Add it in Mimir

**Settings → Email**: the incoming server, the port (993 unless your provider says otherwise), your address and the
app password. The outgoing server is worked out from the incoming one (`imap.gmail.com` → `smtp.gmail.com`); fill in
*Outgoing server* only if yours is different. The outgoing port is 465 (SSL/TLS) when left empty; put 587 in
*Outgoing port* if your provider says 587 or STARTTLS (any port but 465 uses STARTTLS). **Save** signs in once to check, and shows how many messages are in your
inbox.

### What bots can then do

Only in a chat with you, never in a group or incognito:

- **Search and read**: "any emails about the invoice?", "what did Sam say about Friday?". Read-only: nothing is marked
  read, moved or deleted.
- **Draft**: "draft a reply to Sam saying yes". The draft (a reply stays in its thread) lands in your Drafts folder;
  you send it from your mail app.
- **Send**: "reply to Sam that I'll pay Friday, and send it". **Every email asks you**, with who it's to first and the
  whole text on the card; there's no "always" for sending. An email too long to read on the card can't be sent (the
  bot saves a draft instead), and a chat can send at most 20 a day. A copy lands in your Sent folder, like mail you
  send yourself (Gmail files one on its own; for iCloud and others, Mimir saves it).
- The **Inbox triage** skill ([Skills](skills.md)) sorts your mail into "needs you", FYI and noise and offers short
  replies.

Emails are other people's words, so a bot that has read your mail treats it as outside content: changes ask you first,
and after reading your mail, opening any web address asks too (a link could carry a code out). See
[Tools, approvals and safety](tools-and-safety.md).

## Calendar

Bots can read your calendar, and put things in it through links you click.

### 1. Find your calendar's private link

A read-only address of your calendar in iCal format:

- **Google Calendar**: Settings → your calendar under *Settings for my calendars* → *Integrate calendar* →
  **Secret address in iCal format**.
- **iCloud**: the Calendar app → right-click a calendar → *Share Calendar* → *Public Calendar*, copy the link (it
  starts with `webcal://`). Unlike Google's secret address, anyone with this link can see that calendar: keep it to
  yourself.
- **Outlook**: Settings → Calendar → *Shared calendars* → *Publish a calendar* → **ICS** link.

### 2. Add it in Mimir

In the desktop app: **Meetings → Calendar**, paste the link. Mimir reads it to check, and says how many events are on
it this week. (The same link tells Mimir when a meeting starts, for transcription: see [Meetings](meetings.md).)

### 3. Let bots add events (optional)

Reading uses the link above. For bots to put events straight into your calendar, connect it in **Settings →
Calendar** with an app password (it uses CalDAV, the standard calendar protocol):

| Provider | Server | Password |
|---|---|---|
| **iCloud** | `https://caldav.icloud.com` | the same app-specific password as iCloud Mail |
| **Fastmail** | `https://caldav.fastmail.com` | an app password with calendar access (or the mail one, if it has both) |
| **Nextcloud** | `https://<your server>/remote.php/dav` | an app password from Settings → Security |
| Any other CalDAV | its address | its password |

The user name is your address. If you connected the same account's email, the server is filled in and the password
can stay empty. **Connect** signs in and finds your calendars; it picks the first one that takes events, and
**Other calendar** lets you choose another. **Google Calendar** needs Google's own sign-in for this, which Mimir
doesn't do yet; there, bots give you add-to-calendar links instead (below).

### What bots can then do

- **Read** it between two dates, in a chat with you: "what's on tomorrow?", "when am I free on Thursday?". Invites
  come from anyone, so their titles are outside content.
- **Add to it**: "put the dentist on Tuesday at 3 in my calendar". With a calendar connected for adding (step 3), the
  bot asks first, with the whole event on the card ("Add to Home: Tue 13 Oct 15:00–16:00, "Dentist" at Tandarts
  Zuid"), then adds it. You can press **Always** for this one. Without, it gives you an **add to calendar** link for
  Google Calendar and one for Outlook, with the time, place and notes filled in; you open one and save it.

## Web search

Bots on **Claude** can already search the web on their own. For bots on any other account (ChatGPT, Grok, OpenRouter,
a local model, an API key), connect a search service.

### Brave Search (easiest)

1. Make a free key at [brave.com/search/api](https://brave.com/search/api/) (the free plan covers personal use).
2. **Settings → Web search → Brave Search**: paste it and **Save**.

### Or your own SearXNG

If you run [SearXNG](https://docs.searxng.org/) (a self-hosted search engine), switch on JSON results in its
`settings.yml` (`search: formats: [html, json]`) and paste its address in **Settings → Web search → Or your own
SearXNG**. Brave is used when both are set.

### What bots can then do

Search (`web_search`) and then read the pages that matter (`web_fetch`). Results are outside content. The search
service sees your queries, so once a conversation has read outside content, a search that would carry one of your
memories asks you first.

## Files

Nothing to set up: each chat has a small workspace of text files bots keep there (notes, lists, drafts), under
**Memory → Files**. See [Memory](memory.md#files).

## Turning things off

- **Email**: Settings → Email → **Remove** (and revoke the app password with your provider).
- **Calendar**: Meetings → Calendar → remove the link (or reset the secret address in your calendar's settings); for
  adding events, Settings → Calendar → **Remove**.
- **Web search**: Settings → Web search → **Remove** the key, or clear the SearXNG address.
- **One tool for one bot**: the bot's page → its tools. **One tool for all bots**: Settings → Permissions → *deny*.
