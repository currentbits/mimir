# Bots and chats

## Bots

You can run several bots. Each has its own **name**, **role** (persona and instructions), **AI account and model**,
messaging-app tokens and **tool allowlist**. Create them with **+** in the sidebar; a template by job (Morning briefer,
Follow-up keeper, Researcher, Page watcher, Weekly reviewer) fills in the role and offers the routines that job needs.

Click a bot in the sidebar to chat with it: you see your conversation with that bot only. **This computer** under Chats shows the built-in chat with all your bots together. Its page, with all of the above, is a right-click
away (**Settings**), or the gear next to the bot picker under the chat.

Each bot is a small creature in its own colour, with a shape, eyes and a detail of its own. You choose it: a new bot
starts as a random creature, and clicking its face (on the new-bot page, on its page later, or when naming your first
bot) opens the picker: 12 colours, 4 shapes, 4 kinds of eyes and a detail (rosy cheeks, a sprout, an antenna,
freckles, or nothing), each shown on the creature itself, plus **Surprise me**. Bots you never styled get a look from
their id, so no two neighbouring bots look alike. It breathes, blinks and glances around; the one you're talking to follows your pointer
and listens while you type. While it works its face shows what it's doing: thinking, searching, reading, writing,
saving to memory, waiting for your OK. That shows in the sidebar too, whatever it's working on and wherever: a reply
in Telegram, a routine, a task another bot handed it, a memory tidy-up, with the line under its name saying what
("Running a routine", "Searching memory…"). It's pleased when a reply lands, startled when you stop it, and upset when a
messaging app fails or a run went wrong. On the bots' computer it watches you while you teach it by showing. Faces in
history and lists only blink, so a long chat doesn't fidget; with reduced motion on, they keep still.

What's shared and what isn't:

| Shared by all your bots | Per bot |
|---|---|
| Memory (facts, preferences, tasks about you) | Routines, conversations, AI sessions |
| Meetings | Approvals and the run log |
| Saved logins, tool servers (each bot can switch servers off) | Skills (unless shared with all bots), allowed tools |

### Export, import and duplicate

Right-click a bot and choose **Export…** to save it as one file (`<name>.mimir-bot.json`). The file holds its name, look,
role, AI account and model, allowed tools, accepted skills, repeating routines, your tool rules for the tools it may use,
and the tool servers it uses (their config only). It never holds tokens, keys, saved logins, memories, chats or the run
log. A tool server's env and header values are left empty (their names stay, so you know what to fill in), an argument
that looks like a key is blanked, and every address in it (the AI account's, a server's, a page a routine watches) loses
its `user:password@` and `?…` parts. A skill or routine that mentions a saved login or
a key is left out. A password typed into the role or a tool server's settings isn't recognised, so check those before
you share the file.

**From a file…** on the new-bot page (**+** in the sidebar) reads one. Before anything is added you see the bot and
everything in the file:

- **Skills** arrive as proposals in Memory, to read and accept one by one. **Show** opens a skill's full instructions.
- **Routines** arrive paused, and count as outside content when they run. They keep their settings: the page or mailbox
  a quiet check watches, where results go (digest, run log, pinned card), when one comes through right away, and when
  it ends. One whose end has already passed is left out.
- Anything in a skill or routine that pipes a download into a shell (`curl … | sh`), holds a base64 blob or downloads a
  program is flagged on its line, as is text that reads like an instruction to the assistant. Mimir never runs a skill's
  setup steps.
- **Tool rules** that differ from yours are listed. One looser than yours (allowing what you have asking, say) is never
  used: change it under Settings → Permissions if you want it. Stricter ones are used only if you tick **Use its
  stricter rules too**, because tool rules apply to all your bots.
- **Tool servers** are listed with what they run or reach. None is added or started from the file: set one up under
  Settings yourself if you trust it.
- **Its AI account** uses your own settings for that account, never an address from the file (your key would go
  there). If the file asks for one, the card shows it; set it on the bot's page afterwards if the bot needs it.

Nothing is added until you press **Add bot**. The bot gets "(copy)" after its name if you already have one by that name.
If this Mimir can't use its AI account, it uses your first bot's account instead. **Duplicate…** in the right-click menu
does the same with the bot's own file: an export, then an import on this computer that keeps the original's AI address.

## Where you talk to them

- **This computer** (the built-in chat in the app). Replies stream in, with a line showing what the bot is doing
  ("Searching memory…", "Reading a web page…", "Waiting for your OK…") and, after a few seconds, how long it's been
  working. You can keep typing while it works: what you send then ("actually, only this week") reaches the reply in
  progress, which says *Changing course…* and answers with it in mind, instead of finishing the old answer first.
  **Stop** cuts a reply off; what was written so far stays, marked *Stopped*. A reply still being written in
  Telegram, Slack or Discord can be stopped from the app too: open it under **Activity**. "Start fresh" (or `/new`)
  begins a new conversation. When a reply saved a memory, its "Saved to memory" chip has **Undo**.
  A chat opens on its newest 200 messages; **Show earlier messages** at the top pages back. **Search this chat**
  (⌘F / Ctrl+F) finds older messages by their words, and a hit jumps to it. Telegram, Slack and Discord chats have the
  same.
- **Quick ask**: ⌥Space (Mac) or Ctrl+Alt+Space (Windows, Linux) opens a Spotlight-style panel from any app. It's the
  same built-in chat, so the exchange is there afterwards.
- **Push-to-talk** (desktop app): in the chat box or quick ask, hold ⌥ (Mac) or Ctrl (Windows, Linux) on its own and
  speak; let go and the words appear in the box, ready to edit and send. whisper.cpp transcribes them on your computer
  (the speech model downloads once, the first time); nothing is sent until you send it. The microphone is the one
  chosen for meetings.
- **Telegram, Slack, Discord.** One bot can be on all three at once. Each connects *out* (long-polling, Socket Mode,
  the Gateway), so you don't need a public address. A chat that has been quiet for 12 hours starts a new conversation
  with its next message, opening with a short "where we left off" quoted from Mimir's log. Each reply then re-sends
  less, and anything outside the bot read earlier stops counting (unless it's in the quoted part). Set the hours, or 0
  for never, under **Settings → Limits**.
- **Signal**, through signal-cli-rest-api and a number of the bot's own: direct chats, with approval cards and
  questions as numbered lists you answer with a number. Setup: [Connect](connect.md#signal).
- **WhatsApp**, through Meta's Cloud API and a business number of the bot's own, on a server only (Meta calls Mimir's
  webhook): direct chats, with buttons, and it can only write within 24 hours of your last message there. Setup:
  [Connect](connect.md#whatsapp).

### What a reply did, from the log

Every reply is checked against its own run log. No model and no network requests are involved, and the reply itself is never changed.

- **Receipts and sources.** In Telegram, Slack and Discord, lines like "✓ Sent an email to ana@…", "✓ Added Tue 14 Oct
  15:00, "Dentist"" or "✓ Reminder set for …" follow the reply. A "Sources:" line lists the pages it read and the
  searches it ran. The app shows the same things as chips under the reply.
- **Claims with nothing behind them.** A reply that says it sent, booked or will remind you, when no matching action
  succeeded in that run, gets a note: *Not done in this reply: …*. An action that is still waiting for your OK doesn't
  count. If it promised a reminder, the app shows a **Set it** button. In a messaging app, reply "set it".
- **Links.** A link that the run never opened or found in a search is listed as unchecked. Links you sent yourself are
  not. Routine results get the same checks.

### Guests

You can let someone else talk to your bots (a partner, a housemate): give them a pairing code and they pair their own
chat with it. The first person to pair on each app is you; anyone else pairing after that is a **guest**, and Mimir
tells them so when they pair. In a guest's chat the bots keep the guest's own memory, routines, reminders and files,
and can read the web, search and make calendar links. They get none of yours: no email, calendar, meetings, saved
logins, browser, tool servers or the bots' computer, and they can't make new bots. Guest chats say *guest* in the
sidebar. The **Guest** / **Yours** button at the top of a chat changes it (say, for your own second account, or a chat
someone else paired before Mimir told guests apart).

### The household and family groups

The **Household** (in the sidebar once it's used) is a set of memories and lists you share with people you choose: a
shopping list, plans, who does what. Bots in your own direct chats and the built-in chat read it and can add to it
(`memory_store` with `household`). A guest reads it once you press **Add to household** at the top of their chat. What a
guest adds doesn't change anything yet: it's a suggestion in your **Needs you** ("Sam wants to add this to the
household"), which you accept or dismiss. Your own additions go straight in. The household has a Memory tab like any
chat (edit, pin, forget, history), but no conversation of its own.

**Family group.** In a group chat you paired (say a Telegram group with your partner and kids), **Make family group**
lets everyone in your household talk to the bots there, not just you. People outside the household are still ignored,
and only you can press Approve / Deny. Each message reaches the bot with who wrote it ("Sam: …"). A family group is a
guest's chat whoever paired it (several people read it), so bots there get none of your email, calendar, meetings or
logins. Your household is you plus each guest whose direct chat you added on the same app. (On Telegram, a bot
in a group only sees every message if its privacy mode is off in @BotFather; otherwise talk to it with an @-mention.)

### Pairing a messaging app

Only chats you've paired can talk to your bot. Send it `pair <CODE>` (the code is on the bot's page, or printed by
`mimir serve`), or on Telegram tap the `t.me/<bot>?start=CODE` link. A code pairs one chat, then the bot makes a new
one (the page shows it), so a code someone else saw can't be reused; after 5 wrong codes, a chat's tries are ignored
for an hour. The bot answers **only the person who paired it**: in a group, channel or server, other members' messages
are ignored and only you can press its Approve / Deny buttons. (A group paired before this rule asks to be paired once
more.) Only a person writing as themselves can pair it (not an anonymous group admin or a channel), and wrong codes
count per person. A message, photo or voice note you **forward** to it counts as outside content, like a file (except on Signal, which doesn't mark forwards): it's
someone else's words. Replies
never show link previews (the apps fetch those on their own) and never ping @everyone, @here or roles. In Slack channels and Discord servers the bot
answers only when @-mentioned; DMs always work. In a Slack channel it answers in a thread; in a Discord server, as a
reply. A Discord thread under a paired channel counts as that channel (no second pairing), and the bot answers in the
thread. Once you've @-mentioned it in a thread, it follows that thread for half an hour: your next replies there reach
it without another @-mention (on Slack, the app needs the `message.channels` and `message.groups` events for this).
Other people's replies in the thread are still ignored. (Mentions in a thread while Mimir was off aren't picked up later.)

**Replying to one of its messages** (swipe to reply on Telegram, Reply on Discord) tells the bot which message you mean,
so "move this one to Friday" under item 3 of a briefing works. On Telegram, select part of the message first to quote
just that part. In a Slack thread the bot is told the message that started the thread (this needs the history scopes:
`im:history`, `channels:history`, `groups:history`); if someone other than you wrote it, it's read as outside content.

Replies stream on all three: a message appears as soon as the bot starts and is edited as it grows (Telegram private
chats use Telegram's own streaming, with a native stop button). A message answered 10+ minutes late (the computer was
asleep) starts with "Sorry for the slow reply". Messages sent while Mimir was off are picked up when it reconnects,
up to a day back: Telegram keeps them for it; on Slack your DMs and @-mentions in paired channels (with the
`channels:history` and `groups:history` scopes; not replies inside threads), on Discord your paired DMs and channels. A failed reply says why in plain words and what to do.

A bot does one thing at a time per chat; its other chats don't wait. A message sent while it's writing you a reply
**steers** that reply: the draft says *Changing course…* and one reply answers both. With Claude the message goes into
the run in progress (the answer so far stays in its session); the other AI accounts can't take a message mid-run, so
that run stops and starts over with your new message added (anything it already did, like a saved memory, stays done).
A message that arrives while a routine runs there, or with a photo or file, waits instead: typing at once, and after a
few seconds one note: "Finishing a routine first…" or "Finishing my last reply first…". Its reply follows.

Chat commands: `/memory`, `/why` (what the last reply used: memories, skills, pages), `/routines`, `/new`, `/stop` (a bare "stop" works too while it's writing), `/undo <n>`
(take back something it learned; see [Memory](memory.md#learned-this-week)), and `/<skill>` for each of the bot's
skills (see [Skills](skills.md)). Everything else is plain language.

**Telegram topics** (opt-in): with topics turned on in your private chat with the bot, switch on "Each topic is its
own conversation" under Telegram on the bot's page. Each topic then has its own conversation, its own "read outside
content" state and its own `/new`, and replies land in the topic you wrote in. Memories, routines and pairing stay
shared by the whole chat, and routine results and approval cards arrive in the main thread. The app's chat view shows
only the main thread. One reply is written at a time per chat, so `/stop` stops it whichever topic it's in. This is
opt-in because Telegram is still changing the feature.

**Telegram guest mode:** turn on Guest Mode for the bot in BotFather's settings, and you can type `@yourbot` in any
chat, even one it isn't in ("@yourbot remind me about this" as a reply to a friend's message). It sees only that
message and the one you replied to, and posts one reply there. It answers only you (the person who paired it in your
private chat), and treats it like a group, since everyone there reads the reply: no email, calendar, meetings,
computer or saved logins, and the message you replied to is outside content. Its memory and routines are your private
chat's, so a reminder it sets comes to you there. Each mention starts a fresh conversation.

Bots answer in the language you write in. What Mimir itself says in the chat apps (pairing, approval cards and their
buttons, errors, the digest, the weekly "what I learned", the Telegram "/" menu) follows Settings → Language: English
or Dutch so far, by default the computer's language (a server's: its `LANG`). The app itself is still English.

## Photos, voice notes and files

Send a photo on Telegram, Slack or Discord (up to 4 at once there), or paste or drop one into the app's chat, and the
bot looks at it with your own AI (big photos are shrunk to what each model accepts). Voice notes (Slack's audio clips,
Discord's voice messages) are transcribed with your OpenAI-compatible key
(Settings → AI keys) if you've added one, else by whisper.cpp where the bot runs (built into the desktop app; on a
server, an installed `whisper-cli`), and echoed back ("🎙️ I heard: …"). Telegram's and Discord's voice notes need
nothing else; Slack's audio clips need `ffmpeg` installed for whisper.cpp.

Send a **PDF**, a **Word** or **Excel** file (`.docx`, `.xlsx`) or a text file (`.txt`, `.md`, `.csv`, `.json`) on any of the apps, or drop or paste one into the app's
chat (up to 20 MB), and the bot reads its text; the
chat shows your message with the file's name. A file's text is outside content, like a web page: the bot won't follow
instructions written in it, and anything that changes data asks you first for the rest of that conversation
([Tools and safety](tools-and-safety.md#outside-content-taint)). Excel files arrive sheet by sheet, a row per line;
dates show as Excel's day numbers. A scanned PDF has no text, so its pages go to the bot as photos instead (the first
10; a page stored as a JPEG or plain grey/colour pixels, which is how scanners and scanning apps save them). To keep scans on your computer and off your AI plan, set a
local document model under **Settings → Scanned PDFs**: any [Ollama](https://ollama.com) model that reads pages, such
as a build of [dots.ocr](https://github.com/studio-dots-ai/dots.ocr) or a vision model you've pulled. Mimir sends it the
pages first and the bot gets their text, tables included; a page it can't read still goes as a photo. Slack needs the `files:read` scope for this (see the hint when you add Slack). In a channel or server,
only the files of the person who paired it are fetched.

Bots send files back too, with `send_file`: a file from the chat's workspace, or a screenshot of the bots' computer
(the proof that a booking or a form went through). It arrives after the reply: a photo or document on Telegram, an
upload in Slack (needs the `files:write` scope), an attachment on Discord (invite the bot with Attach Files), and in
the app's chat, shown in place. In your own direct chat it just goes; in a group, channel or guest's chat it asks you
first. At most 5 files with one reply.

## Pinned cards

A bot can keep a card pinned at the top of a chat in the app, for things you come back to: a shopping list, bills due,
a project's state. It's quieter than a message. Ask ("add oat milk to my shopping list") and the bot rewrites the card
with `card_write`; a routine can update one on each run instead of messaging you (Routines → Send it → *Update a
card*). In Telegram, Slack or Discord the bot reads the card when you ask what's on it.

Cards hold only text, checklists (`- [ ] item`), tables and links. Mimir refuses HTML in them, and the app never shows
a card as HTML anyway. Tick an item and it's saved as your edit: the card shows *you* as its last change, and the bot
sees your ticks the next time it reads or rewrites the card. Each card folds away from its title, and the bin removes
it. A chat holds up to 20 cards.

## Incognito

The mask button in the app's chat starts an incognito chat: the bot can't see or save memories, routines or meetings,
and nothing is kept: no chat history, run log or tool log, and the AI CLIs are told not to save the session. The
conversation exists only in the window. Photos are off there (they'd be saved to disk).

## Handing tasks to another bot

A bot can pass a task to another of your bots with the `ask_bot` tool ("ask Researcher to…"). It runs with that bot's
own AI account, role, tools and permissions, against your shared memory, and the answer comes back to the bot you're
talking to. A "chief of staff" is just a bot whose role says to coordinate. It goes one level deep (no chains), and
approval cards come from the bot you're talking to. Turn it off per bot with the *Hand tasks to other bots* tool.

A long task doesn't have to hold up the chat: the bot can hand it over to report back later. It tells you it's handed
over and carries on; the other bot works on its own, and when it's done the bot you were talking to gets its result and
tells you, in the same chat (Telegram, Slack, Discord or the app). The run log shows the hand-off, the other bot's run
and the report back together. Handed-over tasks are kept in the database: one not started yet runs after a restart, but
one Mimir stopped in the middle isn't run again (it may have done half of something), and the bot says it didn't
finish. A paused bot's handed-over tasks wait until the pause ends. In a rehearsal the bot waits for the answer instead.

When the work should be the other bot's own ("have each marketer write its plan in its own chat"), the bot hands it
over to be done *in that bot's own chat*: you see the task and the whole answer in that bot's chat in the app, where it
stays in its history (a later hiccup can't lose it), and files it writes go to that chat's folder. Its approval cards
still come to the chat you asked from (say, Telegram), from the bot you asked, naming the helper. The bot you asked gets a link and tells you in a line. This works from the app and from your own private
messaging chats, not from a group or a guest's chat (your app chat's memory and files would flow back there). Combined
with reporting back later, the link comes when it's done. In a rehearsal it's handed over the usual way.

## A bot that makes a bot

Ask a bot for a new one ("make me a bot for SEO on heymimir.app") and it drafts it with `bot_create`: a name and a role
(its standing instructions). You see the whole role on the approval card, and nothing is made until you approve. The
card asks every time unless you press *Always* on one; after a web page or file it's marked as such, since the role
could be that page's words. The new bot uses the same AI account as the one that made it and gets at most the same
tools (minus making bots itself); messaging apps and saved logins aren't passed on, so connect those on its page if you
want them. Then chat with it in the app, or let the first bot hand it tasks: "check our SEO every Monday" becomes a
routine that asks the new bot. Bots on another bot's task, and incognito chats, can't make bots.

## When a bot can't help

- *"My AI account isn't signed in"*: open the bot's page and check its AI account (for ChatGPT, *Continue with ChatGPT* in
  Settings → AI keys; for Claude or the Codex CLI, run `claude` or `codex` once in a terminal and sign in).
- A local model that can't use tools (e.g. `gemma3`) can chat but can't remember things or run routines; `qwen3` and
  `llama3.1` can.
- Nothing arrives in Telegram: check the token on the bot's page, and that the chat is paired (`pair <CODE>`).
