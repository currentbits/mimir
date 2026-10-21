# Connect your email, calendar, web search, Signal, WhatsApp and iMessage

Bots work with what you connect: your email, your calendar and a web search service, and you can reach them on Signal, WhatsApp and iMessage.
Each is optional and takes a few minutes. This page is the setup, service by service; what bots then do (and what always asks you first) is at the end
of each part.

Everything here is set in the app. The passwords and keys are stored in the OS keychain on your computer, or in the
server's private database when you [self-host](self-hosting.md); they never go into a bot's instructions.

## Email

Bots can search and read your inbox, write drafts, and send an email when you approve it. They use IMAP and SMTP, the
standard mail protocols, with an **app password**: a separate password just for Mimir that you can revoke any time,
without changing your real one. Outlook and Microsoft 365 sign in with Microsoft instead ([below](#outlook)).

You can add **more than one mailbox** (work and personal, say). Bots search all of them unless you name one, show which
mailbox each email is in, and send or draft from the one you say (the first one otherwise). The approval card says
which mailbox an email goes out from.

### 1. Make an app password

| Provider | Where | Incoming server | Outgoing server |
|---|---|---|---|
| **Gmail** | [myaccount.google.com/apppasswords](https://myaccount.google.com/apppasswords) (2-Step Verification has to be on) | `imap.gmail.com` | `smtp.gmail.com` |
| **iCloud Mail** | [account.apple.com](https://account.apple.com) → Sign-In and Security → App-Specific Passwords | `imap.mail.me.com` | `smtp.mail.me.com` |
| **Fastmail** | Settings → Privacy & Security → App passwords | `imap.fastmail.com` | `smtp.fastmail.com` |
| **Proton Mail** | Install Proton Mail Bridge; it shows a password for the account (not tried with Mimir yet) | `127.0.0.1` (port `1143`) | `127.0.0.1` (port `1025`) |
| Any other | Your provider's help: "IMAP settings" | usually `imap.<domain>` | usually `smtp.<domain>` |

Outlook, Hotmail and Microsoft 365 have no app passwords for mail any more: see [Outlook](#outlook).

### 2. Add it in Mimir

**Settings → Email**: the incoming server, the port (993 unless your provider says otherwise), your address and the
app password. The outgoing server is worked out from the incoming one (`imap.gmail.com` → `smtp.gmail.com`); fill in
*Outgoing server* only if yours is different. The outgoing port is 465 (SSL/TLS) when left empty; put 587 in
*Outgoing port* if your provider says 587 or STARTTLS (any port but 465 uses STARTTLS). **Save** signs in once to check, and shows how many messages are in your
inbox. **Add another mailbox** adds the next one; the same address again replaces it.

### Outlook

Microsoft turned off app passwords for mail, so Mimir signs in with Microsoft (OAuth with PKCE) and uses that for
IMAP and SMTP (XOAUTH2). Mimir doesn't ship its own Microsoft app yet, so you register one, once. It's free and takes
about five minutes:

1. Open the [Microsoft Entra admin center](https://entra.microsoft.com) (or the Azure portal) with any Microsoft
   account → **App registrations** → **New registration**.
2. Name it "Mimir". *Supported account types*: "Accounts in any organizational directory and personal Microsoft
   accounts" (or only your organisation, for a work account).
3. *Redirect URI*: platform **Public client/native (mobile & desktop)**, address `http://localhost`. Register.
4. Copy the **Application (client) ID** from its overview page.
5. In Mimir, **Settings → Email → Outlook / Microsoft 365**: paste the client ID and click **Sign in with Microsoft**.
   If you chose "only your organisation", paste its **Directory (tenant) ID** too.

Microsoft asks you to allow reading and sending your mail (`IMAP.AccessAsUser.All`, `SMTP.Send`). At work, your admin
may have to allow the app, and SMTP sign-in has to be on for your mailbox (it often is; admins can turn it off). The
sign-in is kept in the keychain and renewed on its own; if it ends (you changed your password, or revoked it), the bot
says so and **Sign in with Microsoft** again replaces it. On a [self-hosted server](self-hosting.md), the browser comes
back to an address that doesn't load: copy it from the address bar and paste it in Mimir, as with ChatGPT.

Gmail isn't done this way: Google requires a security review for apps that read mail, so Gmail keeps the app
password, which works well.

### What bots can then do

Only in a chat with you, never in a group or incognito:

- **Search and read**: "any emails about the invoice?", "what did Sam say about Friday?". Read-only: nothing is marked
  read, moved or deleted.
- **Draft**: "draft a reply to Sam saying yes". The draft (a reply stays in its thread) lands in your Drafts folder;
  you send it from your mail app.
- **Send**: "reply to Sam that I'll pay Friday, and send it". **Every email asks you**, with who it's to first and the
  whole text on the card, until you press "Always" on one. An email too long to read on the card can't be sent (the
  bot saves a draft instead), and a chat can send at most 20 a day. A copy lands in your Sent folder, like mail you
  send yourself (Gmail files one on its own; for iCloud and others, Mimir saves it).
- The **Inbox triage** skill ([Skills](skills.md)) sorts your mail into "needs you", FYI and noise and offers short
  replies.

Emails are other people's words, so a bot that has read your mail treats it as outside content: changes ask you first,
and after reading your mail, opening any web address asks too (a link could carry a code out). See
[Tools, approvals and safety](tools-and-safety.md).

### Tasks by email

Turn on **Settings → Email → Tasks by email** to give your bot a task from any phone, without an app: forward or CC
an email to your own address with `+mimir` added (`you+mimir@gmail.com`), or file it in a folder called **Mimir**.
Write what you want on top ("add this to my calendar", "pay this by Friday"). If you forward nothing, the subject is
enough ("call the dentist at 4").

- The bot never watches your inbox. Every two minutes Mimir checks only mail sent to `+mimir` and the Mimir folder,
  and only mail that came in after you turned this on. Your first bot (the one that also tidies up memory) handles it and
  answers in your most recent private chat with it, or in the app.
- Only mail **from your own address** counts, and only when **DKIM passes** for its domain. Mimir trusts the check
  your mail provider stamped on the email when it arrived, not one written into the email itself. Mail from your
  address that fails the check isn't run, and you're told about it: someone may be pretending to be you.
- Your note on top is the instruction. Everything you forwarded or quoted below it, and any email attached to it, is
  **outside content**: the bot reads it but doesn't follow instructions in it, and the conversation counts as having
  read outside content.
- Gmail, iCloud and Fastmail deliver `+` addresses to your inbox. If yours doesn't, send the mail to yourself and
  move it into the Mimir folder.

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

## X search

Lets every bot search recent posts on X (Twitter), e.g. a routine that pulls this morning's posts on a topic. It
uses X's official API, so you need your own X developer account and token. There's no login or browser involved.

### 1. Get a bearer token

1. Sign in at [console.x.com](https://console.x.com) with your X account and accept the developer terms.
2. **Buy credits**: X's API is pay-per-use. Reading posts costs a fraction of a cent each (plus a bit per author), so
   a daily search for 10 posts is a few cents a day. Current rates are on the console's pricing page. A search without
   credits fails with an error from X, which the bot passes on.
3. **Create a Project**, then an **App** inside it.
4. In the app's **Keys and tokens**, generate a **Bearer Token** and copy it. It's shown only once. You can make a
   new one there at any time, which also cancels the old one.

You don't need the API key/secret, the access token or OAuth settings. Searching only reads public posts.

### 2. Add it in Mimir

**Settings → Web search → X (Twitter)**: paste the token and **Save**. On a server you can instead set the
`X_BEARER_TOKEN` environment variable.

### What bots can then do

Search posts from the **last 7 days**, newest first (`x_search`), with X's search operators:
`from:rustlang`, `#rustlang`, `"exact phrase"`, `-is:retweet`, `lang:en`. Each result has the author, the time,
the text and a link. For a morning digest, make a routine like "every day at 7: search X for `(rust OR tokio)
-is:retweet lang:en` and give me the five most interesting posts with links".

Posts are outside content. X sees your queries, so once a conversation has read outside content, a search that would
carry one of your memories asks you first.

## Signal

A bot can be on Signal next to Telegram, Slack and Discord (those are set up on the bot's page, see
[Bots and chats](bots-and-chats.md)). Signal has no bot accounts, so the bot needs a Signal number of its own (a spare
SIM or a second number; not yours, or its replies would come from you), and Mimir reaches Signal through
[signal-cli-rest-api](https://github.com/bbernhard/signal-cli-rest-api), a small service you run next to it.

1. **Run signal-cli-rest-api** in json-rpc mode. On a [server](self-hosting.md), uncomment the `signal` service and
   `MIMIR_SIGNAL_API` in `docker-compose.yml`. On your own computer, with Docker:
   ```sh
   docker run -d --name signal --restart unless-stopped -p 127.0.0.1:8081:8080 -e MODE=json-rpc \
     -v "$HOME/.local/share/signal-api:/home/.local/share/signal-cli" bbernhard/signal-cli-rest-api:0.101
   ```
   signal-cli-rest-api has no password: anything that can reach it can send messages as the bot and link another
   device to its number. `127.0.0.1` keeps it off your network, but every program and user account on this computer
   can still reach it, so run it only on a computer that's yours alone. (In the server's `docker-compose.yml` it's on
   a network of its own with Mimir, out of reach of the bots' browser.)
2. **Link the bot's number** as a linked device. Set up Signal on a phone with that number, open
   `http://127.0.0.1:8081/v1/qrcodelink?device_name=mimir` in a browser and scan the code with that phone: Signal →
   Settings → Linked devices → **+**. (On a server, `ssh -L 8081:127.0.0.1:8081 you@server` first, then open
   `http://127.0.0.1:8081/v1/qrcodelink?device_name=mimir`.)
3. **Add it in Mimir**: the bot's page → Messaging apps → **Signal** → **Set up**: the bot's number with its country
   code (`+31…`), and signal-cli-rest-api's address if it isn't `http://127.0.0.1:8081`. Save; it shows
   *Connected · +31…*.
4. **Pair**: from your own Signal, message the bot's number `pair` and the code on the bot's page.

Text, photos, voice notes (transcribed as on Telegram; Signal's are AAC, so whisper.cpp needs `ffmpeg` installed),
PDFs and other files work both ways, with a typing indicator while the bot writes. Reply under one of its messages and
the bot knows which one you mean. Signal has no buttons, so approval cards and a bot's questions come as a numbered list:
answer with just the number (`1` approves). The numbers belong to the bot's latest numbered message, and a card's
keep working until it's decided. Replies arrive whole, as plain text (no streaming). Direct chats only: in a Signal
group the bot stays silent. Messages sent while Mimir isn't connected to signal-cli-rest-api aren't fetched later, as
they are on Telegram. For `mimir serve`, set `SIGNAL_NUMBER` (and `MIMIR_SIGNAL_API`).

## WhatsApp

A bot can be on WhatsApp through Meta's official Cloud API (unofficial WhatsApp Web bridges get numbers banned). Meta
sends the bot's messages to Mimir, so this needs Mimir running as a [server](self-hosting.md) at an `https://` address
Meta can reach; the desktop app on its own can't take them. You need a Meta business account, and the bot needs a
phone number of its own that isn't already on WhatsApp.

1. **Make a Meta app** at [developers.facebook.com](https://developers.facebook.com/apps): type *Business*, add the
   **WhatsApp** product, and add the bot's number under WhatsApp → API Setup. Note its **phone number id** (digits,
   not the number itself).
2. **Get an access token that doesn't expire**: in Business Settings → Users → System users, add one, give it the
   app with `whatsapp_business_messaging` and `whatsapp_business_management`, and generate a token. (The temporary
   token on API Setup works for a day.)
3. **Note the app secret**: App settings → Basic → App secret. Every call Meta makes to Mimir is signed with it.
4. **Add it in Mimir**: the bot's page → Messaging apps → **WhatsApp** → **Set up**: the phone number id, the token
   and the app secret. Save; it shows *Connected · +1 555…*.
5. **Point the webhook at Mimir**: WhatsApp → Configuration → Webhook: callback URL `https://<your server>/whatsapp`,
   verify token the phone number id, then **Verify and save** and subscribe to **messages**.
6. **Pair**: from your own WhatsApp, message the bot's number `pair` and the code on the bot's page.

Text, photos, voice notes (transcribed as on Telegram; WhatsApp's are Ogg Opus), PDFs and other files work both ways,
with a typing indicator while the bot writes. Approval cards and a bot's questions come as buttons when they have up to
three short choices, else as a numbered list you answer with a number, as on Signal. A forwarded message counts as
someone else's words. Replies arrive whole, as plain text (no streaming). Direct chats only.

WhatsApp lets a business write to you only within **24 hours of your last message**. A routine's result or a card
after that isn't delivered: it shows as not sent in the run (Runs), where you can still read it.
Messages sent while Mimir was down are delivered when Meta retries, if they're less than a day old. Not for
`mimir serve`, which has no webhook.

## iMessage

On a Mac, the desktop app can answer in **one Messages conversation you choose**, and never reads any other. Pick
either:

- **A note to yourself**: the conversation with your own number or email. Everything in it is yours; the bot's
  messages end with an invisible character so Mimir can tell them from yours.
- **A chat with a second Apple ID** signed in to Messages on this Mac: you write to it from your iPhone, and it
  answers as that Apple ID.

1. **Have the conversation in Messages on this Mac**: send one message in it if it's new.
2. **Give Mimir Full Disk Access**: System Settings → Privacy & Security → Full Disk Access → add Mimir (from
   Applications), then quit and reopen it. Mimir reads `~/Library/Messages/chat.db` every two seconds, only the
   conversation you chose. macOS has no prompt for this; until it's on, the bot's page says it can't read Messages.
3. **Add it in Mimir**: the bot's page → Messaging apps → **iMessage** → **Set up**: the conversation's number with
   its country code (`+31…`) or its email, as Messages shows it. Save; it shows *Connected · …*.
4. **Allow Mimir to control Messages** when macOS asks, the first time the bot answers (System Settings → Privacy &
   Security → Automation → Mimir → Messages, if you said no).
5. **Pair**: send `pair` and the code on the bot's page in that conversation.

Text, photos and voice messages (transcribed) come in; the bot's replies are plain text, whole (no streaming, no
typing indicator). It can't send files there yet: it says so instead. Approval cards and a bot's questions come as
numbered lists you answer with a number, as on Signal; to answer an older card, reply to it with the number.

It works only while the Mac is awake, Mimir is open and Messages is signed in. Messages that arrive while it's closed
aren't answered later. Not on a server, and not in the app connected to one.

## Files

Nothing to set up: each chat has a small workspace of text files bots keep there (notes, lists, drafts), under
**Memory → Files**. See [Memory](memory.md#files).

## Turning things off

- **Email**: Settings → Email → **Remove** (and revoke the app password with your provider).
- **Calendar**: Meetings → Calendar → remove the link (or reset the secret address in your calendar's settings); for
  adding events, Settings → Calendar → **Remove**.
- **Web search**: Settings → Web search → **Remove** the key, or clear the SearXNG address.
- **Signal**: the bot's page → Signal → **Disconnect**, and remove *mimir* under Linked devices on the bot's phone.
- **WhatsApp**: the bot's page → WhatsApp → **Disconnect**, and remove the webhook (or the app) in Meta's dashboard.
- **iMessage**: the bot's page → iMessage → **Disconnect**, and take Mimir off Full Disk Access if nothing else needs
  it.
- **One tool for one bot**: the bot's page → its tools. **One tool for all bots**: Settings → Permissions → *deny*.
