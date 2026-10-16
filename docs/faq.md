# Questions people ask

## What it is

**What is Mimir, in one sentence?** An app that runs AI assistants for you, on the AI plan you already have: you
message them like a person, they remember you, work on a schedule, and ask before they act.

**Is it a chatbot?** You chat with it, but a chat is only one part. Bots also work when you're not talking to them
(routines, reminders, quiet checks), share one memory of you, and use tools: your email, calendar, the web, files. See
[What you can do with it](examples.md).

**How is it different from the Claude or ChatGPT app?** It uses them. Mimir adds what sits around the model: bots in
your messaging apps, routines that run while you're away, one memory for all your bots that you can read line by
line, several bots with their own jobs, and an approval for every action that matters.

**Does it have its own AI?** No. Each bot uses an AI account you choose ([AI accounts](ai-accounts.md)).

## Cost and accounts

**What does it cost?** Mimir is free and open source (AGPL-3.0). Bots on your Claude, ChatGPT or Grok plan cost
nothing extra. With an API key (OpenRouter, OpenAI, xAI) you pay that service for what you use, and Activity shows
what each reply cost. A model on your own computer is free.

**Do I need an API key?** No. ChatGPT and Grok sign in from Mimir's settings. For Claude you sign in once to its
official command-line app (`claude`) and Mimir uses that login.

**Can my Claude plan stop working with it?** Anthropic has at times counted Claude-plan use outside its own apps as
paid "extra usage". If that happens, the bot says so in plain words; turn on extra usage in your Claude account, or
switch that bot to another account ([AI accounts](ai-accounts.md#signing-in)).

**Can different bots use different AIs?** Yes. Each bot has its own account and model: a cheap local model for quiet
checks, Claude for research.

## Your data

**Where is my data?** In `~/.mimir` on your computer (or your server): one SQLite file with your memories, bots,
chats and routines. Passwords and keys are in the system keychain. There's no Mimir account and no Mimir server
([Privacy and your data](privacy.md)).

**What leaves my computer?** What a reply needs (your message, the conversation, the memories that matter) goes to the
AI account the bot uses; nothing goes anywhere with a local model. Messages pass through the messaging app you use
them in. Email, calendar and search go to the services you connected, and nowhere else.

**Can I see everything it knows about me?** Yes, in the **Memory** tab: every memory, who wrote it and when. Correct
one and the old version is kept; **Erase** removes both for good. `/memory` lists it in a chat too.

**Can I talk to it without it remembering?** Yes: an **incognito** chat (the mask button) can't see or save memories,
and keeps no history at all ([Bots and chats](bots-and-chats.md#incognito)).

## Safety

**Can it do things without asking me?** Only what you allow. Every tool is *allow*, *ask* or *off* in
Settings → Permissions. Reading, searching and remembering are allowed by default; forgetting, deleting and opening a
page in the browser ask. Sending an email and making a new bot ask every time, whatever the rules say.

**What if a web page or an email tells it to do something?** It's treated as someone else's words, not yours. After a
bot reads a page, an email or a file, anything that changes data asks you first for the rest of that conversation,
and what it saved then is marked unconfirmed until you say it looks right
([Tools, approvals and safety](tools-and-safety.md#outside-content-taint)).

**Is it safe to connect my email?** Reading is read-only: nothing is marked read, moved or deleted. Drafts go to your
Drafts folder. Sending shows who it's to and the whole text, and asks every time; at most 20 a day from one chat. It
signs in with an app password you can revoke without changing your real one.

**What if I don't answer an approval?** After 5 minutes it counts as no. A routine's approval waits 24 hours in
**Needs you** instead, since nobody was watching when it asked.

**Can someone else message my bot?** Only if you give them a pairing code. A bot answers only chats paired with a
one-time code, and in a group, only the person who paired it ([Pairing](bots-and-chats.md#pairing-a-messaging-app)).

**Can my partner or a housemate use it too?** Yes, as a **guest**: give them a pairing code and they pair their own
chat. Mimir sees it's someone other than you, so in their chat the bots have the guest's own memory and routines, the
web and files, and none of yours: no email, calendar, meetings, saved logins or tool servers
([Guests](bots-and-chats.md#guests)).

## Running it

**Does my computer have to stay on?** Bots work while the app runs. Closing the window keeps it running in the menu
bar or tray; **Settings → Keep this computer awake** stops idle sleep so routines fire on time. For bots that work
while your laptop is closed, run Mimir on a server you own ([Self-hosting](self-hosting.md)).

**Can I use it from my phone?** Yes: connect a bot to Telegram, Slack or Discord and message it there. A self-hosted
server also serves the app on the web. On a phone, a tab bar at the bottom moves between Chats (your bots, chats
and Settings), Needs you, Memory and Routines.

**Does it update itself?** The desktop app does: it downloads a new version in the background, checks it against
Mimir's signing key, and installs when you choose **Restart to update**.

**Where do I download it?** On [heymimir.app](https://heymimir.app/#get), or the
[releases page](https://github.com/currentbits/mimir/releases/latest) for macOS, Windows and Linux
([Getting started](getting-started.md#1-install)).

**Something isn't working.** The first item in the sidebar, **Needs you**, lists what failed in plain words with a fix
button. Every reply and routine is in **Activity** with each step it took. **Settings → Your data → Check everything**
(or `mimir doctor` on a server) checks AI sign-ins, each bot's Telegram, Slack, Discord and Signal connection, email, the bots'
computer, disk space, the database and its backups, and what leaves it open to others (plain HTTP over the internet,
"Allow all" permissions, tool servers without a fixed version, an old version), and says what to do about anything that's off.
