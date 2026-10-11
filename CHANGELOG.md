# Changelog

What changed for people using Mimir. The newest release is at the top; "Unreleased" is what the next one brings.

## Unreleased

- **A starter team in the first minutes:** the first run asks what you want off your plate (tap up to three), and your
  first bot suggests bots, routines and library skills for them on one card. Untick what you don't want, and the rest
  is set up when you start.
- **Get to know me** (Settings, once email or the calendar link is connected): one read-only look at who wrote to you
  this month and what's on your calendar. What it finds waits in Memory as suggestions, and nothing is kept until you
  accept it.
- **Bots know what isn't connected yet:** ask for your email before it's set up and the bot says where to connect it.
  The chat shows a button to that setting and a **Try again** for afterwards.

- **Email providers that send on port 587 work:** Settings → Email has an *Outgoing port* next to the outgoing server.
  Empty is 465 as before; 587 (or any other port) uses STARTTLS. Saving used to set it back to 465 every time.

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
  - The quick-ask panel sees bots added, renamed or deleted since it opened, and its bot can't be changed mid-answer.
  - Guest/Yours on a chat and "Open in Mimir" say so when they fail; a second "are you sure?" no longer leaves the first
    one hanging.

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
