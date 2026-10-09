# What you can do with it

Everyday jobs people give their bots, each with what to say, what it needs, and what happens. Say it in your own words;
these are only examples. Anything below that needs setting up links to how.

## A morning briefing

**Say:** "Every weekday at 7:30, give me today's calendar, what's due, and the weather in Cape Town, in ten lines."

**Needs:** nothing, though a [linked calendar](connect.md#calendar) makes it much better. Or add **Morning briefing**
from the skill library ([Skills](skills.md)) and press **schedule it**.

**What happens:** the bot makes a [routine](routines.md). Each weekday at 7:30 it reads your calendar and open tasks
and sends you the briefing, in the app or in Telegram, Slack or Discord. Reply to it ("skip the weather") and the bot
knows which message you mean; it can change the routine for you.

## Reminders and follow-ups

**Say:** "Remind me in two hours to call mum." "On Friday at 9, check whether the Acme invoice was paid." "I promised
Sam the deck by Thursday."

**Needs:** nothing.

**What happens:** a reminder is a routine that runs once and is then gone. The bot saves a promise as an open **task** in
memory, so your morning briefing and the **Follow-up tracker** skill bring it up until you say it's done.

## Your inbox

**Say:** "Anything from Sam about the invoice?" "What needs me in my email today?" "Draft a reply saying I'll pay on
Friday." "Send it."

**Needs:** [email connected](connect.md#email) with an app password (Gmail, iCloud, Fastmail and others).

**What happens:** the bot searches and reads your mail without marking anything read. A draft lands in your Drafts
folder, in the right thread, for you to send. If you ask it to send, a card shows who it's to and the whole text, and
nothing goes until you press Allow, every time. The **Inbox triage** skill sorts a day's mail into needs you, FYI and
noise.

## Your calendar

**Say:** "What's on tomorrow?" "When am I free on Thursday afternoon?" "Put the dentist on Tuesday at 3 in my
calendar."

**Needs:** your [calendar's private link](connect.md#calendar) (Google, iCloud or Outlook) for reading it. For adding,
an iCloud, Fastmail or Nextcloud calendar connected in Settings → Calendar; without one, it gives you links.

**What happens:** the bot reads your calendar between the dates you ask about. To add something, it shows you the
event on a card and adds it when you approve. Without a connected calendar (Google's needs a sign-in Mimir doesn't do
yet), it gives you a link for Google Calendar and one for Outlook with everything filled in; you open one and save.

## Watching a page or a price

**Say:** "Keep an eye on example.com/pricing and only tell me if it changes." "Tell me when these headphones drop
below €200."

**Needs:** nothing.

**What happens:** the bot makes a **quiet check**: a routine that looks at the page on a schedule and stays silent
unless something changed. Every run is in Activity. The **Price watch** skill remembers today's price so it knows what counts as a drop.

## Research

**Say:** "What are the main options for home batteries in South Africa? Compare three, with sources." "Summarise this
link." "Keep a file of what we find."

**Needs:** nothing on Claude (it searches the web on its own). On other AI accounts, a free
[Brave Search key](connect.md#web-search).

**What happens:** the bot searches, reads the pages that matter, and answers with its sources (the **Research brief** skill numbers
each claim). Longer work goes
in the chat's file workspace (Memory → Files), so it's there next time. Pages are outside content: the bot won't
follow instructions written in them, and asks before changing anything after reading one.

## Meetings and calls

**Say (after a call):** "What did we decide in standup?" "What did Priya say about the budget?"

**Needs:** the desktop app. Start **Meetings → Start transcribing** when the call begins (or from the menu-bar / tray
icon). No bot joins the call; it's transcribed on your computer.

**What happens:** the transcript fills in during the call, split into you and the other speakers. When it ends, a bot
can write the notes (decisions, action items, what you promised). Later, any bot can answer questions about it. See
[Meetings and calls](meetings.md).

## Several bots for several jobs

**Say:** "Make me a bot for SEO on heymimir.app that checks our rankings every Monday." "Ask Researcher what the Acme
contract says about late payment."

**Needs:** nothing.

**What happens:** a new bot is drafted with a name and a role; you read the whole role on a card and approve it. Bots
hand tasks to each other: the one you're talking to asks the other, which works with its own AI account and tools,
and the answer comes back to you. All of them share your memory. See [Bots and chats](bots-and-chats.md).

## Teaching it how you like things

**Say:** "From now on, keep answers under five lines." Or just correct it: "No, too long. Three bullets."

**Needs:** nothing.

**What happens:** a rule in your own words ("from now on…", "always…") is saved as a preference right away, and every
bot follows it. A correction becomes a **suggestion** in the Memory tab: accept it and it sticks, dismiss it and it's
gone. The next time you ask for something like what you corrected, the bot sees what you said then, so the same
mistake isn't made twice. See [Memory](memory.md).

## Doing a task the same way every time

**Say:** after a task went well, "save that as a skill". Or paste a checklist into **Skills → Teach a skill**.

**What happens:** a card shows the steps it would save; nothing is saved until you press **Save**. From then on the
bot follows those steps when a request fits, or when you type `/` and the skill's name. See [Skills](skills.md).
