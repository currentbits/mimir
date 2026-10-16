# Changelog

What changed for people using Mimir. The newest release is at the top; "Unreleased" is what the next one brings.

## Unreleased

### Safety
- **Tool server pinning, tightened:** servers added before 0.4.0 are pinned the next time they start. A version range
  (`pkg@^2`, `pkg@1`) is left as you wrote it instead of jumping to the newest release, and **Update** never offers an
  older release than the one you're on. A package OSV.dev has never checked doesn't start while OSV.dev can't be reached.

## 0.4.0

0.3.0 was never published on its own, so updating from 0.2.0 also brings
[everything in 0.3.0](https://github.com/currentbits/mimir/blob/master/CHANGELOG.md#030) (Signal, replies that survive
a restart and more).

### Talking to your bots
- **Telegram rich messages:** replies show real tables, headings and task lists, and a long one fits in one message
  (up to 32,768 characters). An older Bot API server gets the formatting it had before.
- **Slack streams natively:** in a thread you mentioned the bot in, a reply streams in with Slack's own streaming
  (no edit rate limits) under an "is thinking…" status.
- **Skills for your bots:** four more in the skill library. *Hire a bot* makes a bot with one job, what it never does,
  a voice and when it works, and has it say its job back; *Bot role check* checks a role and rewrites it; *Team check*
  finds where your bots overlap or stray and proposes fixes you pick; *Memory check* proposes fixes for wrong, stale or
  secret memories.

### Routines
- **Goals:** ask a bot to see something through ("find a plumber who can come this week") and it keeps at it until
  it's done, with a progress line pinned in the chat and in its header. The follow-ups it books and the tasks it hands to
  other bots belong to the goal: pause it and they all stop at once, even mid-task; remove it and
  they're gone. A bot can only call a goal done when the run log shows it, and a goal can have a token budget of its own.
  When the budget runs out, Needs you asks whether to go on. [More](docs/routines.md#goals)
- **Routines remember their last results:** each run sees what the routine said its last three times, so a daily
  briefing says what's new instead of repeating yesterday's news.

### Safety
- **Sensitive sources:** mark a tool server (your bank's, your health records'), a single tool or a chat's memory as
  sensitive in **Settings → Sensitive sources**. Once a reply or routine has read from one, everything that goes out
  asks first, and the card says why ("This run read from bank"): an email, a web page or search, a task for another
  bot, a message to someone else's chat, and any tool server's change. Servers whose names look like banking or
  health are suggested, never marked for you.
- **Security:** the TLS library (rustls 0.23.46) is updated for four advisories published on October 14
  (RUSTSEC-2026-0346 to 0349).
- **Edit an email before it goes, and say why when you deny:** on an email's approval card in the app, **Edit** lets
  you change the subject and text before you send it. What you approve is what goes; who it's to stays as shown. In
  Telegram, Slack, Discord and Signal, **Edit in Mimir** opens the card in the web app (on a server with an https
  address). **Deny…** takes an optional "Why?", and in a messaging app a reply to the card does the same: the bot
  reads your reason and changes course instead of just stopping.
- **Export, import and duplicate a bot:** right-click a bot to save it as one file (its role, look, skills, routines,
  tool rules and the tool servers it uses), without tokens, keys, saved logins, memories or chats. **From a file…** on
  the new-bot page shows everything in it first, and flags skill steps that pipe a download into a shell, hold a base64
  blob or download a program. Skills come in as proposals and routines come in paused; only its tool rules that are at
  least as strict as yours can be used, and its AI account uses your own settings.
  **Duplicate…** does both in one go. See [Bots and chats](docs/bots-and-chats.md#export-import-and-duplicate).
- **Grok bots are more private:** Grok's own memory and `/goal` are always off for bots, and your Claude and Cursor
  settings no longer load into them, so bots can't read each other's notes and incognito chats leave nothing behind
  in Grok. A message that starts with `/` now reaches the bot instead of running as a Grok command. If a Grok update
  ever offers a bot more tools than web search and Mimir's own, the message stops with an error instead of running.
  The docs cover The X Subscription's plans and its shared weekly allowance.
- **Security fixes from the fourth review:**
  - A routine can no longer change its own times or end date while it runs, or book itself again a minute later;
    from a routine, a one-off is at least 15 minutes away.
  - A follow-up stays in the chat it was booked in, and a reminder for someone else counts toward their chat's limit.
  - With docker compose, Codex bots now use the separate computer service; before, they started a browser inside
    Mimir's own container.
  - After reading a sensitive source, saving a memory for the household asks too.
  - A bot file can't pass an option to the AI's command line as its model.
  - `mimir doctor` no longer creates a database, or upgrades an older one, while it checks.
- **Anonymous usage stats:** once a day Mimir sends a count (a random id for this install, the version, the OS and each
  bot's AI provider and model; never names, messages or keys) so we know how many people use it and on what.
  Settings → Your data shows exactly what's sent and switches it off (`MIMIR_USAGE_STATS=off` on a server).
- **Security checks in `mimir doctor`:** it warns when the web app is reached over plain HTTP from the internet,
  Permissions are at "Allow all", a tool server's package has no fixed version, a newer Mimir is out, or the bots'
  computer runs in Mimir's own process on a Linux host without Docker (its DevTools have no token there).
- **Tool servers stay on one version:** adding one that downloads its package (`npx -y pkg`, `uvx pkg`) pins it to
  the newest release that's at least a week old, so a bad release published tomorrow doesn't reach you on the next
  start. When a newer one has been out a week, the server shows **Update to x.y.z**.
- **Hidden text doesn't reach your bots:** invisible characters that spell out words only a model reads (Unicode tag
  characters, stray variation selectors) are taken out of pages, mail, files and tool results. A Word file's text
  formatted to be invisible (white, 1 pt, hidden) is marked as hidden for the bot.
- **A tool server's "read-only" needs your OK:** a server saying a tool only reads is no longer enough to skip asking;
  after you add a server, Settings → Tool servers asks you to confirm which of its tools really only read.
- **More security fixes:**
  - Signing in to a tool server checks the answer comes from the server you started with (RFC 9207).
  - Codex's own web search answers from OpenAI's cache only, and is off after a bot has read outside content, so it
    can't open an address that carries your data out.
  - Email approval cards flag addresses with look-alike letters from another alphabet and show their real domain.
  - Only the chat's owner can answer a bot's buttons, and screenshots are kept in Mimir's own folder instead of `/tmp`.

### The bots' computer
- **Sign in from your phone:** at a sign-in page, a bot sends you a card with a link to a small masked form in Mimir,
  instead of asking you to take over the screen. Type your login there, never in the chat; Mimir types it into the
  page and keeps it for that bot on that site only. The bot never sees it, only whether it worked. If the bot got to
  that site after reading a web page or a message, the site looks like one you've signed in to before, or its
  address uses letters from another alphabet, the form warns you and shows the full address. Taking over stays for
  captchas and payments. [Details](docs/computer.md#signing-in-from-your-phone)
- **Authenticator codes on saved logins:** paste a site's setup key or `otpauth://` link once, and Mimir types the
  6-digit codes itself, only on that login's site.

## 0.3.0

### Talking to your bots
- **Signal:** a bot can be on Signal too, with a number of its own, through signal-cli-rest-api. Text, photos, voice
  notes and files work both ways; approval cards and a bot's questions come as a numbered list you answer with the
  number. Direct chats only. Setup is under [Connect](docs/connect.md#signal).
- **Replies survive a crash, an update or a restart:** a reply, a routine's result or a note for your decisions chat
  is kept until Telegram, Slack or Discord has it. If Mimir stops before then, it sends it when it starts again. One
  that didn't go through is tried again for a while. A routine's run shows *waiting to send* until it's sent.
- **Hand over a long task and keep talking:** a bot can pass a task to another of your bots to report back later. You
  get an answer straight away, and when the other bot is done its result comes back in the same chat.
- **Loop breaker:** a bot that keeps making the same call (the 3rd time in a row, the 5th for just looking) stops and
  asks you whether to go on, instead of burning time and money. Routines ask in Needs you.
- **Tool servers stay on one version:** adding one that downloads its package (`npx -y pkg`, `uvx pkg`) pins it to
  the newest release that's at least a week old, so a bad release published tomorrow doesn't reach you on the next
  start. When a newer one has been out a week, the server shows **Update to x.y.z**.
- **Hidden text doesn't reach your bots:** invisible characters that spell out words only a model reads (Unicode tag
  characters, stray variation selectors) are taken out of pages, mail, files and tool results. A Word file's text
  formatted to be invisible (white, 1 pt, hidden) is marked as hidden for the bot.

### Routines
- **Follow-ups the bot books itself:** "I'll check back Friday whether they replied" becomes a one-off follow-up, said
  in the reply and shown in the Routines tab, where you delete it to cancel. It counts like any routine.
- **Routines that end:** "every morning until the 30th" or "for two weeks": an optional end, shown on the routine, after
  which it's gone.

### Safety and upkeep
- **Security fixes** from a review of the whole app:
  - A bot can't open a page in a new browser tab past the "open a site" ask and its checks.
  - After reading outside content, no approval card offers **Always**, not even for actions that already ask. A
    guest's OK is always for that once, so it can't change your rules.
  - After reading your email or a meeting, every web address asks for the rest of the conversation, and in tasks
    handed to other bots (before: that turn only).
  - An audio file sent to a bot (not a voice note you recorded) counts as outside content, like a document.
  - A reply that quotes the bot's message no longer counts the quote as your words when the tidy-up learns from it.
  - The bots' computer is offered only in chats with you alone, never a group.
  - Slack approval cards can't ping `@channel`.
  - The computer service's DevTools port needs its token, like its control channel.
  - A signed webhook call can't be replayed.
  - Tool-server sign-in checks addresses properly (`http://localhost.evil.com` isn't this computer) and follows
    redirects only to public addresses.
  - The server no longer prints the admin token in its log; `mimir admin-token` shows it.
  - Voice notes are written to owner-only temp files.
- **Malware check for tool servers:** a tool server that downloads its package (`npx`, `uvx`…) is looked up on
  OSV.dev's list of malicious packages first, and Mimir refuses to start it if it's there, saying which advisory.
- **Each bot has its own sign-ins on the computer:** a site you sign in to for one bot (when it asks you to take over)
  is signed in for that bot only. **Sign out everywhere** on a bot's page clears its sign-ins. Sign-ins from before
  go to the first bot that uses the computer after the update.
- **Check everything:** `mimir doctor` (and Settings → Your data → *Check everything*) checks each bot's AI sign-in,
  its Telegram, Slack and Discord connection, your mailboxes, the bots' computer, free disk space, the database and its
  backups, one line each with what to do. Nothing is spent or sent.
- **Security checks in `mimir doctor`:** it warns when the web app is reached over plain HTTP from the internet,
  Permissions are at "Allow all", a tool server's package has no fixed version, a newer Mimir is out, or the bots'
  computer runs in Mimir's own process on a Linux host without Docker (its DevTools have no token there).

## 0.2.0

### Talking to your bots
- **Change course mid-reply:** send "actually, only this week" while a bot is still answering and the reply in progress
  takes it in, instead of waiting until it's done. The chat shows *Changing course…* until the new answer starts.
- **Tap instead of type:** in Telegram, Slack and Discord a bot can ask with 2 to 4 buttons (plus **Something else**),
  and routine and reminder results have one-tap feedback buttons.
- **Bots send files and screenshots back:** a file from the chat's workspace, or a screenshot of its computer's screen
  after a booking or a filled-in form, comes with the reply.
- **Receipts under replies:** in a messaging chat, each thing that really happened gets a line ("Sent an email to …",
  "Reminder set for …") and the pages and searches it used. A reply that says it sent or booked something it didn't
  gets a note: *Not done in this reply*.
- **What a reply used:** under each reply in the app, "2 memories · 1 skill · read 3 pages". Open it to edit or forget
  a memory that steered it wrong. In a messaging app, `/why` lists the same.
- **A busy bot doesn't leave other chats in silence:** it shows it's typing everywhere at once and works on each chat
  in turn.
- **A fresh start after a quiet spell:** a Telegram, Slack or Discord chat that's been quiet for 12 hours starts a new
  session, opening with where you left off.
- **Telegram topics:** turn on *Each topic is its own conversation* on a bot's page, and every topic in your private
  chat with it gets its own conversation.
- **Slack threads say which message:** a reply in a thread reaches the bot with the message that started it.
- **Mimir in Dutch:** Settings → Language. Pairing, approval cards, errors, the digest and the Telegram menu follow it.
  Bots already answered in your language.
- **Chats show earlier messages, and you can search them.**
- **On a phone:** a tab bar at the bottom (Chats, Needs you, Memory, Routines) with the sidebar as the Chats screen.
  Before, the web UI on a phone had no way to get to your bots.
- **Push-to-talk:** in the chat or quick ask, hold ⌥ (Ctrl on Windows and Linux) and speak; let go and the words are
  in the box, not sent. Transcribed on your computer by the same whisper.cpp as meetings.
- **Voice notes without an API key:** with no OpenAI key, Telegram and Discord voice notes are transcribed by
  whisper.cpp on your computer.
- **Word and Excel files and scanned PDFs:** send a `.docx` or `.xlsx` and the bot reads it (Excel sheet by sheet).
  A scanned PDF's pages go to the bot as photos, so a scanned contract or letter no longer needs photographing.

### Memory
- **Memories for one bot only:** "just for you: …", or switch a memory between *all bots* and *only <bot>* on its card.
- **The Household:** shared memories and lists (shopping, plans, who does what) your bots read and add to, and family
  group chats that use them.
- **Pinned cards:** a bot keeps a card at the top of a chat up to date, like a shopping list or bills due. "Add oat milk
  to my shopping list" rewrites the card instead of sending a message.

### Routines
- **Started by email or a webhook:** a routine's schedule can be `mail:from:acme.com subject:invoice`, so it runs when
  matching mail arrives (mail that doesn't match costs no AI), or `webhook` on your server (GitHub-compatible
  signatures). "Tell me when the invoice from Acme comes in" sets one up.
- **Timed off your calendar:** `calendar:60` runs an hour before each event; `calendar:60:meetings` only when someone
  else is invited, `calendar:60:outside` only when someone from outside is. "Brief me on who they are before each
  outside meeting" now works.
- **Urgent results come through right away, on your rule:** a routine that waits for the digest or quiet hours can carry
  one line, "Come through right away if my flight is delayed". At most 3 a day.
- **Approvals on your phone:** Settings → Decisions sends every routine approval to one private chat as well, so one
  from the app chat doesn't wait until you open the app.
- **A fallback account at the usage limit:** on a bot's page, under *When it hits a usage limit*, routines move to
  another account while the bot's own is at its limit, and come back when it resets.
- **Pause:** all bots (next to **Bots** in the sidebar) or one (its right-click menu), until you resume, for an hour,
  until tomorrow or until Monday. Nothing runs on its own meanwhile; you can still ask them directly.

### Email, tools and your server
- **Tasks by email:** forward or CC mail to `you+mimir@…`, or file it in a **Mimir** folder, and your note on top is
  the task.
- **More than one mailbox, and Outlook** (with your own Microsoft sign-in).
- **Tool servers that need a sign-in:** hosted MCP servers such as Notion and Linear sign in from Settings.
- **A chat API:** `mimir server` answers OpenAI-style `/v1/chat/completions` for Shortcuts, Home Assistant and the like,
  with a token per client that reaches one bot (Settings → Chat API).
- **Backups you can restore:** Mimir checks its database every day and keeps 7 daily and 4 weekly copies. Settings →
  Your data → **Restore a backup…**; damage shows in Needs you.

### First run
- **A starter team in the first minutes:** in the first minutes:** the first run asks what you want off your plate (tap up to three), and your
  first bot suggests bots, routines and library skills for them on one card. Untick what you don't want, and the rest
  is set up when you start.
- **Get to know me** (Settings, once email or the calendar link is connected): one read-only look at who wrote to you
  this month and what's on your calendar. What it finds waits in Memory as suggestions, and nothing is kept until you
  accept it.
- **Bots know what isn't connected yet:** ask for your email before it's set up and the bot says where to connect it.
  The chat shows a button to that setting and a **Try again** for afterwards.
- **Email providers that send on port 587 work:** Settings → Email has an *Outgoing port* next to the outgoing server.
  Empty is 465 as before; 587 (or any other port) uses STARTTLS. Saving used to set it back to 465 every time.

### Faster and steadier
- **Faster replies:** the tools process starts in about 25 ms instead of 1.3 s, and the app launches faster.
- **The app asks less:** it polls only the view you're looking at, one request at a time, and not at all while hidden.
  The web UI is compressed and cached.
- **A crash in one routine fails that run, not the whole bot,** and a bot that crashes starts again by itself.
- **A chat past 200 messages no longer goes blank.**
- A database from a newer Mimir is refused with a clear message instead of being half-upgraded.
- The quick-ask panel sees bots added, renamed or deleted since it opened, and its bot can't be changed mid-answer.
- Guest/Yours on a chat and "Open in Mimir" say so when they fail; a second "are you sure?" no longer leaves the first
  one hanging.

## 0.1.5

- **Sign in with ChatGPT:** bots run on your ChatGPT Plus or Pro plan without installing anything. *Continue with
  ChatGPT* on the first screen or in Settings → AI keys, sign in, done. You choose how much of your plan Mimir may use
  each week in ChatGPT's settings; when it's used up, bots say so. On a server, you paste the address your browser ends
  up on. The Codex CLI route stays, as **Codex CLI**.
- **A bot no longer forgets the conversation without saying so:** when its AI tool can't reopen the earlier session
  (after an update, moving to a server, or switching the bot to another account), it rereads the last part of your chat
  from Mimir's own log and tells you it did.
- **Replying to a message says which one:** swipe to reply on Telegram (or select part of it first), or Reply on
  Discord, and "move this one to Friday" reaches the bot with the message it's about.
- **Threads without @-mentioning again:** after you mention a bot in a Slack or Discord thread, your next replies there
  reach it for half an hour (Slack needs the `message.channels` and `message.groups` events).
- **Model for routines** (Settings → Limits): every routine on a lighter model of the same account, e.g. haiku, so
  your plan's windows last longer. Replies to you keep the bot's own model.
- **A late routine knows it's late:** a morning briefing that runs at lunchtime is told when it was due.
- **One question, not two:** bots let the approval card ask, instead of asking "shall I?" first.
- **The memory tidy-up doesn't learn that something is broken:** no memories or skill steps from setup problems or
  errors that went away.
- Settings no longer says that urgent things in the digest come right away; they don't (yet).
- **Buttons that did nothing, fixed:**
  - Photo thumbnails in the message box and the bots' computer screen show in the desktop app again.
  - "@Another bot …" from a bot's own chat keeps your message on screen and opens that bot's chat when the answer is in.
  - Leave a chat while a bot is replying and come back: the reply still shows as it's written.
  - Switching bots mid-reply no longer streams (or stops) the wrong one.
  - Long chats keep following new messages after 200.
  - After you connect or disconnect Telegram, Slack or Discord, the bot's page shows it straight away.
  - A slow Save no longer pulls you back to the bot you left.
  - Deleting the bot you were chatting with no longer leaves its chat behind.
  - Delete, Dismiss, Run now and a few switches now say so when they fail.

- **Quiet checks that cost almost nothing:** a quiet check can watch a page, a feed or your inbox. Mimir looks first and
  runs the AI only when something changed, with just what changed. Unchanged checks show as free. Times, counters
  and anything else that changes on every load don't count as a change.
- **Daily caps:** Settings → Limits caps what each routine, each bot and all bots may use in a day. One that would go
  over pauses until midnight, and Needs you asks whether to let it go on today. Tokens from the AI's cache count a
  tenth, as they're billed; an AI that reports nothing gets an estimate. Limits also shows your Claude plan's 5-hour and
  weekly windows.
- **Routines that heal:** when the AI plan hits its usage limit, routines of every bot on that login wait until it
  resets and then run; network
  trouble is retried after 5, 15 and 30 minutes; a signed-out AI account is noticed before anything is spent. A
  routine that keeps failing is one item in Needs you, and it leaves once the routine works again. Activity says
  where each result went, and a result a messaging app wouldn't take shows in Needs you.
- **Tune a new routine by replying:** its first three results ask what you didn't need; reply and the bot rewrites it.
- **Rehearse** a routine or a skill: it runs once, but anything it would send or change is only described.
- **"I need you on the computer":** at a sign-in, a 2FA code, a captcha or a payment, a bot on your server's computer
  asks you to take over, with a link to the live screen, and carries on once you give it back. A routine asks without
  waiting, and runs again once you press Done.

## 0.1.4

- **All at once:** **Accept all** / **Dismiss all** for memory suggestions (from two up, the outside-content ones
  included: they stay unconfirmed), **All look right** for memories waiting to be confirmed, **Allow all** / **Deny all**
  for waiting approvals (an email, a new bot, a saved login and asks after outside content stay one by one), and
  **Allow all** / **Back to defaults** in Settings → Permissions.
- **Your server tells you when to update it:** once per new version, in your own chat with your bots (Telegram, Slack
  or Discord) and at the top of Needs you, with the exact commands for how it's installed. Settings → This server
  switches it off.

## 0.1.3

- **Export everything works in the desktop app:** the file goes straight to your Downloads folder (the app's window
  couldn't download anything, though it said it had).
- **Copying works in the desktop app** (a reply, a transcript, a skill): the app copies itself.
- **Switching chats or bots never shows the one before:** a slow answer for the chat you just left can't land in the
  new one, and nothing of the last bot (its "read outside content" note) stays on screen.
- **From Needs you, Open the chat shows the whole chat,** and after **Summarise with…** on a meeting you land in the chat
  with the bot that's writing the notes.
- **A button that fails says so:** Undo on a learned memory, Copy, Stop and opening a link.
- Windows: files in a chat's workspace are listed as `notes/todo.md`, not `notes\todo.md`.

## 0.1.2

- **Deleting works in the desktop app again:** routines, bots, files, meetings, tool servers, chat history and erased
  memories. Their "are you sure?" now asks inside the app (the desktop app couldn't show the browser's own question).

## 0.1.1

- **The Mac app is notarized by Apple:** it opens with a double-click, no right-click → Open.
- **Each bot has its own conversation:** clicking a bot in the sidebar shows your chat with that bot, not every
  bot's messages. This computer still shows them all together.

- **Search in Settings.** Type in the box at the top of Settings (or press ⌘F / Ctrl+F there) and only the matching
  settings stay: "email", "token", "brave", "quiet hours". A bot's own settings (its role, AI account, Telegram, Slack
  or Discord) are on its page, and the search says so.

## 0.1.0 (the first public release)

The highlights of what went into it, besides what the [docs](docs/README.md) describe.

### New things your bots can do
- **Email.** Search and read your inbox, write drafts (new mail, or a reply in its thread) into your Drafts folder,
  and send an email when you approve it: every one asks, showing who it's to and the whole text, at most 20 a day.
  Nothing is marked read, moved or deleted. What a bot sends is in your Sent folder, as if you'd sent it. Set up in Settings → Email with an app password; only in a chat with you,
  never a group.
- **Your calendar.** Read it between two dates (from the calendar link in Settings), and add events to it: straight
  into an iCloud, Fastmail or Nextcloud calendar once you approve (Settings → Calendar), or with one-click "add to
  calendar" links for Google Calendar and Outlook.
- **Web search for every AI account,** through Brave Search (a free key) or your own SearXNG, in Settings → Web search.
- **Reminders.** "Remind me in 2 hours to call mum" runs once at that time, then it's gone.
- **A file workspace per chat** for notes, lists, drafts and research too long for a memory, under Memory → Files.
  Rewriting a file keeps the previous version; deleting asks.
- **Make another bot** for a job ("make me a bot for SEO on heymimir.app"). You see its whole role and approve it;
  it gets at most the tools of the bot that made it.

### Guests
- **Let a partner or housemate use your bots.** Someone you give a pairing code to gets their own guest chat: their
  own memory, routines and reminders, and none of your email, calendar, meetings, saved logins or tool servers. Mimir
  tells guests apart on its own; a button on the chat changes it.

### Skills
- A **Skills** tab (⌘3), with **Mimir's skill library**: 14 skills written for Mimir's tools (morning briefing,
  inbox triage, meeting prep and follow-up, weekly review, research brief, price watch, topic watch, write like me,
  trip planner and more). Nothing is added on its own: Add gives one to all bots or one bot; habits like the morning
  briefing can be scheduled in one click.
- A suggested skill that does the same job as one you have says so on its card.

### Learning
- A request like one you corrected before comes with what you said then, so the same mistake isn't made twice.
- New facts update the older memories they make outdated, knock-on effects included ("broke my leg" vs "cycles to
  work"); memories can have an end date ("in Lisbon until Friday").
- Skills learned from a task take your earlier corrections on similar requests as "stop and ask if" lines.
- Suggestions like ones you dismissed say so, and after two, that kind isn't shown again.
- What a bot learned shows the message of yours it came from; **Check** works on accepted preferences too (useful
  after switching AI model); the weekly trend counts repeated corrections, "I told you"s and what you took back.
- Learning keeps working in a chat where a bot searched the web (lessons in your own words stay yours).
- **Needs you** lists memories waiting for you to confirm them.

### The app
- **Routines show whether they're working:** their last runs (ran, failed, missed) and what they used in 30 days,
  tokens and cost. A routine that didn't run on time (the app was closed or the computer asleep) says so, in the run log
  and in Needs you.
- **Updates itself:** a new version downloads in the background (checked against Mimir's signing key) and installs
  when you choose Restart to update, in Settings or the tray.
- A bot at work shows it in the sidebar: its face animates and the line under its name says what it's doing.

### Safety
- After a bot reads your email or a meeting, any web address it would open asks you first (a short link can carry a
  code out).
- Only your own words come back as past corrections or skill pitfalls; outside content can't end or rewrite a memory.
- Making a bot always asks, with no "Always".

### Fixes
- Windows: the desktop app no longer stops on a debug-heap error at launch in development builds.
