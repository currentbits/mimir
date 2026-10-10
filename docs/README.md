# Mimir documentation

Mimir is a free, open-source app that runs AI assistants for you. We call them **bots**. You message a bot the way
you'd message a person, in the app or in Telegram, Slack or Discord, and it does things for you: it remembers what you
tell it, reminds you, runs tasks on a schedule, reads and drafts your email, checks your calendar, searches the web,
keeps notes, and transcribes your calls. Anything that changes something asks you first.

## What Mimir is, and isn't

**It isn't an AI model.** Mimir has no AI of its own. Each bot uses an AI account you already have: your Claude,
ChatGPT or Grok plan (signed in, or through their official apps, so there's no API key and nothing extra to pay), an API key, or a
model running on your own computer. Mimir is everything around the model: where you talk to it, what it remembers,
what it may do, and when.

**It isn't a cloud service.** There's no Mimir account and no Mimir server. The app runs on your computer (or a server
you own), and what your bots know about you is one file there that you can read, correct and delete. The only things
that leave your computer are what a reply needs, sent to the AI you chose, and the messages in the messaging apps you
connect.

**It's more than a chat window.** A chat app answers when you ask. Mimir's bots also work when you don't: a briefing
every morning, a check on a web page every hour that only speaks up when it changes, a reminder on Friday at 5. They
share one memory of you, so telling one bot something tells them all, and when you correct one, it offers to remember
the lesson.

## What you need

| | |
|---|---|
| **A computer** | macOS (Apple silicon), Windows or Linux. Or a server you own (a VPS, a home server, a Raspberry Pi) for bots that are online while your laptop sleeps. |
| **An AI account** | One of: a Claude plan with Claude Code, a ChatGPT Plus or Pro plan (signed in from Mimir), a SuperGrok or X Premium+ plan, an OpenRouter or other API key, or Ollama / LM Studio on your computer. |
| **Optional** | A Telegram, Slack or Discord bot token, to talk to your bots from your phone. An email app password, a calendar link and a Brave Search key, to give them more to work with. |

Setting it all up takes about ten minutes: [Getting started](getting-started.md).

## Words these docs use

| Word | What it means |
|---|---|
| **Bot** | One assistant: a name, a role, an AI account and the tools it may use. You can have several. |
| **Role** | A bot's standing instructions, in your words ("You're my research assistant; keep answers short"). |
| **Chat** | Where you talk to a bot: the app's built-in chat, or a Telegram, Slack or Discord chat you've paired with it. |
| **Memory** | What your bots know about you: facts, preferences and open tasks. Shared by all your bots, all visible in the Memory tab. |
| **Suggestion** | Something a bot thinks it should remember, forget or change. Nothing changes until you accept it. |
| **Routine** | A task a bot runs on a schedule ("every weekday at 7:30…"), or once (a reminder). |
| **Quiet check** | A routine that only messages you when something needs you. |
| **Skill** | Step-by-step instructions for one kind of task, which a bot follows when a request fits. |
| **Tool** | One thing a bot can do: search memory, read a web page, draft an email. Every tool is allowed, asks first, or is off. |
| **Approval** | A card asking you to allow or deny one action. |
| **Outside content** | Words that didn't come from you: a web page, an email, a file. After reading it, a bot asks before changing anything. |
| **Run** | One reply or one routine, recorded in Activity with what it did. |

## Start here

- [Getting started](getting-started.md): install the app, connect an AI account, make your first bot, talk to it.
- [What you can do with it](examples.md): everyday jobs, from a morning briefing to your inbox, step by step.
- [Connect your email, calendar and web search](connect.md): app passwords, calendar links and search keys, step by
  step, and what bots then do.

## Using Mimir

- [Bots and chats](bots-and-chats.md): bots, roles and templates, Telegram / Slack / Discord, the built-in chat, quick
  ask, photos, voice notes and PDFs, incognito, handing tasks to another bot.
- [AI accounts](ai-accounts.md): Claude, ChatGPT, Codex, Grok, OpenRouter, xAI, local models, any OpenAI-compatible
  service.
- [Memory](memory.md): what bots remember, how they learn from your corrections, how you stay in charge, the daily
  tidy-up, files.
- [Skills](skills.md): step-by-step procedures, Mimir's skill library, learning and teaching skills, `/commands`.
- [Routines](routines.md): scheduled tasks, reminders, quiet checks, the daily digest, quiet hours, *Needs you*.
- [Tools, approvals and safety](tools-and-safety.md): what bots can do, allow / ask / deny, outside content, plans,
  the browser and saved logins, tool servers (MCP), the run log.
- [Meetings and calls](meetings.md): transcribing Teams, Zoom, Meet and phone calls on your computer, speakers, notes,
  the calendar.
- [The bots' computer](computer.md): a Linux desktop on your server that bots use like a person.
- [Self-hosting](self-hosting.md): `mimir server` on a machine you own, so bots stay online 24/7.

## Questions

- [Questions people ask](faq.md): cost, accounts, privacy, safety, what happens when your computer sleeps.

## Under the hood

- [Privacy and your data](privacy.md): what's stored where, what leaves your computer, and when.
- [Security model](security.md): who Mimir defends you against, the rules that do it, and what's left.
- [Architecture](architecture.md): how the pieces fit, for contributors (crates, data, providers, MCP, tests).

## How it fits together, in one page

```
 You ──▶ Telegram / Slack / Discord / desktop app / web UI
             │
             ▼
        a bot (name, role, AI account, allowed tools)
             │  each message or routine becomes one "run"
             ▼
        your AI account (claude, codex, grok CLI, or an API)  ◀── memory, conversation, skills
             │  the model calls tools
             ▼
        tools: memory, routines, email, calendar, web search, files, browser, computer, meetings…
             │  each call checked against your rules: allow / ask / deny
             ▼
        the reply streams back to where you asked
```

- **One data folder.** Everything lives in `~/.mimir` (one SQLite file, `mimir.db`, plus downloaded models and photos).
  Secrets (bot tokens, API keys, saved logins, the calendar link, the email app password) go to the OS keychain on the
  desktop, or to the server's private database when self-hosting.
- **Your AI, your subscription.** Mimir doesn't have its own model. It starts the official CLI you're signed in to
  (`claude -p`, `codex exec`, `grok -p`) or calls an API with your key, and gives the model Mimir's tools through
  MCP (the standard tool protocol).
- **Memory is yours, shared by all your bots.** Routines, chats and approvals belong to each bot.
- **Nothing risky happens silently.** Every tool has a rule. After a bot reads anything from outside (a web page, an
  email, a tool result, a meeting transcript), anything that changes data asks you first for the rest of that
  conversation. Sending an email or making a bot asks every time.
- **Learning goes through you.** Bots save what you tell them; anything they work out themselves (a preference, a
  skill, a fix) is a suggestion you accept or dismiss.
- **Desktop or server.** The desktop app runs your bots while it's open (and updates itself); `mimir server` runs the same thing on a
  machine you own and serves the same UI on the web. The desktop app can connect to your server instead of running
  bots itself.
