# Changelog

What changed for people using Mimir. The newest release is at the top; "Unreleased" is what the next one brings.

## Unreleased

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
