# Routines

A routine is a task a bot runs on a schedule: "every weekday at 7:30, brief me on tech news", "every Sunday evening,
review my week". Ask in plain words and the bot creates it (`routine_create`), or add one in the **Routines** tab with a
schedule picker (every day, weekdays, weekly, every few hours, or custom cron). You can edit or pause a routine without
losing it. Schedules are in your local time. A routine a bot creates runs at most every 15 minutes, its prompt is up to
1,000 characters, and a chat holds at most 25 of them (routines you add yourself aren't limited).

**Reminders and one-offs.** "Remind me in 2 hours to call mum" or "on Friday at 9, check if the invoice was paid"
becomes a routine that runs once, at that time, and is then gone (the Routines tab shows it as *Once · Fri 16 Oct
09:00*). It can be paused or deleted like any routine.

**Timed off your calendar.** "An hour before each meeting with someone outside, brief me on who they are" runs off the
calendar you linked in Meetings → Calendar instead of a clock. Where a cron goes, the schedule is
`calendar:<minutes before>`, optionally `:meetings` (only events with someone else invited) or `:outside` (only ones
with someone from another email domain): `calendar:60:outside`. The bot can make one when you ask, or type it in the
Routines tab under *Custom*. Each run is told the event (its title, time and who's invited), so pair it with the
Meeting prep skill. A meeting that's added, moved or cancelled moves the run with it (the calendar is read every 10
minutes); one added with less notice than the lead time gets no run. Such a routine only lives in a private chat, since
your calendar is private. For "the evening before tomorrow's first meeting", use a plain cron (`0 20 * * *`) and have
the routine look at tomorrow's calendar.

The scheduler checks every 30 seconds, so a routine runs within half a minute of its time, as long as Mimir is running.
On a laptop, **Settings → Keep this computer awake** helps; for 24/7, [self-host](self-hosting.md).

## Rehearse before it runs for real

**Rehearse** on a routine (or on a skill, in the Skills tab) runs it once for real, except that every step that would
change or send something (a message, an email, a memory, a new routine, a click on a site) or would ask you first is
only described. You see what it found, the list of what it would have done, and what it would have said. Nothing lands
in the chat; Activity keeps the run as a rehearsal, and it doesn't count as the routine's run. Reading (web pages, your
email, memory) does happen, so a rehearsal uses tokens like a run.

## Is it working, and what does it cost?

Each routine in the Routines tab shows its last runs as dots (green ran, red failed, hollow missed; hover one for
when and how many tokens) and its last 30 days: how many runs, about how many tokens each, the API-equivalent cost
where the AI reports it, and how many failed or were missed. A routine that quietly uses a lot, or keeps failing,
shows on the routine itself.

A routine **missed** its time when it ran more than 15 minutes late: Mimir wasn't running then (the app was closed),
the computer was asleep, or a long run before it held it up. It still runs once as soon as it can (not once for every
slot it missed), and the run log gets a line saying which times it missed and why ("Missed 3 runs, 15:00 to 17:00:
Mimir wasn't running then. It ran at 17:17 instead."). Missed and failed runs show in **Needs you**. The late run is
told when it was due, so a 7:30 briefing that runs at 13:10 leaves out what has already passed instead of saying "this
morning".

## Caps

**Settings → Limits** sets daily caps, in tokens: for each routine, for each bot, and for all bots together (none
by default). A routine whose run would go over one pauses until midnight instead of running (one routine for its own
cap; all of a bot's routines for the bot's or the day's), says so where it speaks, and **Needs you** asks: *Let it go
on today* lifts that cap for the rest of the day. Otherwise the routine runs once after midnight, and the run log says
which times it skipped and why. Replies to your own messages are never stopped by a cap, though they count towards
the bot's and the day's.

Tokens read back from the AI's cache (Claude Code and Codex send the bot's instructions that way) count a tenth, as
they're billed, so a cap isn't eaten by the same instructions read again; each routine shows what it uses on average
on that scale, which is a good guide for its cap. An AI that doesn't report what it used (some local models) gets an
estimate from the text, about four characters a token, shown with ≈ in Activity, so caps still apply to it. Where the AI reports them (Claude Code does), Limits also shows
your plan's own windows: how much of the 5-hour and weekly windows is used, and when each resets.

**Model for routines** (also in Limits) runs every routine on a lighter model of the same account, e.g. `haiku` on
Claude or `gpt-5-mini` on ChatGPT, while replies to you keep the bot's own model. Routines are usually the bulk of
what a plan's windows go to, and most don't need the biggest model. A routine's **Test run** uses it too, so you can
see whether it's good enough first. With bots on different kinds of account, there's one field for each kind.

## When a run doesn't work

What happens next depends on why:

- **The AI plan's usage limit.** The routines of every bot on that AI login (your Claude, ChatGPT or Grok sign-in is
  one for all your bots; bots on an API key only themselves) wait until the limit resets (the time Claude Code reports; an
  hour when the AI doesn't say), then each runs once, and the one that hit the limit runs again. You get one message
  where that routine speaks, and **Needs you** shows "routines wait until 15:00" with *Run them now* (if you raised the
  limit or switched account).
- **Couldn't reach the AI** (the network, a local model that's starting): it tries again after 5, 15 and 30 minutes,
  without messaging you, and only then says it failed.
- **The AI account isn't set up** (not installed, signed out): Mimir checks this before each run, so nothing is spent;
  it retries like a network error, so signing in again is enough.
- **Anything else**: it says so where the routine speaks, as before.

A routine that keeps failing the same way is **one** item in Needs you ("4 times"), and once it works again its
failures leave Needs you by themselves. A one-off (a reminder) that fails this way is kept and tried again, not lost.

Activity shows where each result went: *sent to telegram*, *in the digest*, *only in the log*, *in the app*. When the
messaging app didn't take it (tried twice), that's an item in Needs you, and the result is still in Activity.

Every run starts a fresh conversation with the AI, so a routine that runs every 15 minutes doesn't grow its context
(or its cost) run after run.

## Quiet checks

A routine can be **quiet**: it runs with read-only tools and messages you only when something needs your attention;
otherwise it stays silent (the run log still shows it). "Keep an eye on X and only tell me if it changes" makes one.
The Routines tab also offers ready-made ones: a morning task check, an evening look-ahead, watching a page.

A quiet check about one page, feed or your inbox can **watch** it (the page's address, or `mail` for new unread email;
bots set it when you ask them to keep an eye on something). Mimir then looks itself before each run: when nothing
changed, the AI doesn't run at all (the run log shows "nothing changed, no AI run", and the routine counts these as
free). When something did change, the AI runs with what changed: the lines that are new or gone (for mail, the new
unread messages), up to about 3,000 characters. So a check every 15 minutes costs a fetch, not 96 AI runs a day.
The first look runs the AI as usual, since there's nothing to compare with yet. If Mimir can't reach the page, the
check runs as it would without a watch.

Pages say things that change on their own. Times of day, timestamps and "5 minutes ago" are never counted. When a
page did change, Mimir looks again a few seconds later: lines that differ between those two looks (a visitor counter,
a session id, a rotating ad) are the page's own churn, and lines like them are left out from then on. The routine
shows "ignoring 2 lines that change on every load" (hover for which), with **Reset** to notice them again. A line
that only moved isn't a change.

## When routines speak

Each routine is sent:

- **right away**: a message and a notification;
- **in your daily digest**: one message a day at the time you set (Settings → Daily digest), with a line like "4 quiet
  checks ran; nothing needed you";
- **just logged**: nothing is sent; Activity shows what it found.

Every run's reply is kept in **Activity**.

You can reply to what a routine sent ("skip the crypto stuff"), and the same goes for the digest and the weekly review.
A new routine's first three results end by asking "Anything here you didn't need, or something missing?": reply, and
the bot rewrites the routine's brief to match (`routine_update`), keeping what you didn't ask to change. You can ask
for changes any time after that too ("move the news to 9", "make it shorter").
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
