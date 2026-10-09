# Routines

A routine is a task a bot runs on a schedule: "every weekday at 7:30, brief me on tech news", "every Sunday evening,
review my week". Ask in plain words and the bot creates it (`routine_create`), or add one in the **Routines** tab with a
schedule picker (every day, weekdays, weekly, every few hours, or custom cron). You can edit or pause a routine without
losing it. Schedules are in your local time. A routine a bot creates runs at most every 15 minutes, its prompt is up to
1,000 characters, and a chat holds at most 25 of them (routines you add yourself aren't limited).

**Reminders and one-offs.** "Remind me in 2 hours to call mum" or "on Friday at 9, check if the invoice was paid"
becomes a routine that runs once, at that time, and is then gone (the Routines tab shows it as *Once · Fri 16 Oct
09:00*). It can be paused or deleted like any routine.

The scheduler checks every 30 seconds, so a routine runs within half a minute of its time, as long as Mimir is running.
On a laptop, **Settings → Keep this computer awake** helps; for 24/7, [self-host](self-hosting.md).

## Is it working, and what does it cost?

Each routine in the Routines tab shows its last runs as dots (green ran, red failed, hollow missed; hover one for
when and how many tokens) and its last 30 days: how many runs, about how many tokens each, the API-equivalent cost
where the AI reports it, and how many failed or were missed. A routine that quietly uses a lot, or keeps failing,
shows on the routine itself.

A routine **missed** its time when it ran more than 15 minutes late: Mimir wasn't running then (the app was closed),
the computer was asleep, or a long run before it held it up. It still runs once as soon as it can (not once for every
slot it missed), and the run log gets a line saying which times it missed and why ("Missed 3 runs, 15:00 to 17:00:
Mimir wasn't running then. It ran at 17:17 instead."). Missed and failed runs show in **Needs you**.

Every run starts a fresh conversation with the AI, so a routine that runs every 15 minutes doesn't grow its context
(or its cost) run after run.

## Quiet checks

A routine can be **quiet**: it runs with read-only tools and messages you only when something needs your attention;
otherwise it stays silent (the run log still shows it). "Keep an eye on X and only tell me if it changes" makes one.
The Routines tab also offers ready-made ones: a morning task check, an evening look-ahead, watching a page.

## When routines speak

Each routine is sent:

- **right away**: a message and a notification;
- **in your daily digest**: one message a day at the time you set (Settings → Daily digest), with a line like "4 quiet
  checks ran; nothing needed you";
- **just logged**: nothing is sent; Activity shows what it found.

Every run's reply is kept in **Activity**.

You can reply to what a routine sent ("skip the crypto stuff"), and the same goes for the digest and the weekly review.
A routine runs in its own session, so on Claude, Codex and Grok the bot's next turn gets that last message with your
reply. If the routine had read a web page, that turn counts as having read it too; so does a reply to the digest or the
weekly review, which are put together from routine results and learned memories.

## Quiet hours

Set quiet hours under Settings → Daily digest (e.g. 22:00–07:00). They hold everything proactive: routines that would
message right away go into the digest, the digest and the weekly review wait until quiet hours end, and approval cards
from routines wait until morning. Replies to your own messages still come.

## Needs you

The first item in the sidebar collects what's waiting on you: approvals, runs that didn't work (in plain words, with a
fix button: "Couldn't reach the AI. Is it running?" → *Check the AI account*), bots that can't connect (a rejected
token, an AI account that's signed out; the bot's chat says so too, with a way to its settings), approvals nobody
answered in time, and memory suggestions.

Routines run with nobody watching, so an approval a routine needs is **parked** there for 24 hours instead of being
denied after 5 minutes; when you approve, that exact action runs and the chat says so. The menu-bar / tray icon shows
the count, and its menu lists what's waiting with Allow and Deny, so you can answer without opening the window.
