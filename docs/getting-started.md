# Getting started

About ten minutes from nothing to a bot that knows your name. New to Mimir? [The overview](README.md) says what it
is in a page.

**Before you start**, have one of these ready:

- a **Claude** plan (Pro or Max) or a **ChatGPT** plan (Plus or Pro), so bots run on it at no extra cost;
- a **Grok** plan (The X Subscription Premium or higher, or an older SuperGrok or X Premium+ plan);
- an **API key** (OpenRouter, OpenAI, xAI or any OpenAI-compatible service);
- or **Ollama** or **LM Studio** on your computer, for a model that never leaves it.

## 1. Install

**Desktop app** (macOS 11+ on Apple silicon, Windows 10 or 11, Linux on x86-64). Download the installer from
[heymimir.app](https://heymimir.app/#get) or the [releases page](https://github.com/currentbits/mimir/releases/latest):
`.dmg` (Mac), `-setup.exe` or `.msi` (Windows), `.AppImage`, `.deb` or `.rpm` (Linux). The Mac app is signed and
notarized by Apple, so it opens like any other. The Windows installer isn't code-signed yet, so the first time Windows
SmartScreen says "Windows protected your PC": click **More info**, then **Run anyway**. (If your browser warns about the
download, choose **Keep**.) After that the app updates itself.

Or build it from source:

```sh
cd ui && npm install && npm run tauri build     # the app and installer end up under $CARGO_TARGET_DIR/release/bundle/
```

Or run it in development mode: `cd ui && npm install && npm run tauri dev`.

You need Rust, Node.js 22 and, for the built-in transcription, `cmake` (macOS: `brew install cmake`; Linux:
`libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libdbus-1-dev libasound2-dev
libclang-dev cmake`).

**Updates.** The app checks for a new version a minute after it starts and every 6 hours, and downloads it in the
background. Each download is checked against Mimir's signing key, so a tampered file is refused. It doesn't restart on
its own (that would stop your bots mid-reply): **Settings → Updates → Restart to update**, or **Restart to update** in
the menu-bar / tray menu. Switch off *Check for updates automatically* there to check only when you press **Check now**.
The `.dmg`, Windows installers and `.AppImage` update themselves; a `.deb` or `.rpm` install asks for your password to
install the new package.

**On a server** instead (bots online 24/7): see [Self-hosting](self-hosting.md).

## 2. Connect an AI account

The first screen asks which AI to use. Pick the one you already pay for:

- **ChatGPT**: *Continue with ChatGPT* and sign in; bots then run on your Plus or Pro plan. Nothing to install.
- **Claude** (or ChatGPT through the **Codex CLI**): install the official command-line app once and sign in; Mimir
  uses that login. No API key, no extra cost beyond your plan. In a terminal:

  ```sh
  npm install -g @anthropic-ai/claude-code && claude    # Claude: sign in when it asks, then type /exit
  npm install -g @openai/codex && codex login           # ChatGPT: signs in through the browser
  ```

  (Claude Code also has its own installer at [claude.com/claude-code](https://claude.com/claude-code).) The first
  screen shows each one as *Not installed*, *Not signed in* or signed in, with **Check again**.
- **OpenRouter**: *Connect OpenRouter* signs you in through the browser (hundreds of models, one key).
- **Local model**: run Ollama or LM Studio; Mimir finds it and lists its models. Nothing leaves your computer.
- **Grok**, **xAI API** or any **OpenAI-compatible** key: see [AI accounts](ai-accounts.md).

Pick a model if you like (the default is fine), then **Test connection**: it sends one short message so you know it
works. **Continue** unlocks once it does.

## 3. What you want off your plate

Next, tap up to three things you want help with (a morning briefing, your to-dos, email, meetings, looking things up,
watching pages and prices, a weekly look back), or skip it. At the end your first bot suggests a starter team for
them on one card: a new bot from a template for each job that has one (with its routines), and skills from the
library. Untick what you don't want; nothing on it is set up until you press **Set them up and start**, and you can
change or remove any of it later. A suggestion that needs something not connected yet (your email, say) says so.

## 4. Your first bot

Next, name your first bot and, if you like, say what it should focus on: its **role** ("You're my research
assistant; keep answers short"). Tap its face to change how it looks. Then the app opens its chat: tell it a few
things about yourself, and watch them appear in the **Memory** tab. A new bot can start from a template by job (Morning briefer, Follow-up keeper, Researcher, Page watcher,
Weekly reviewer), which fills in the role and offers the routines that job needs.

Click the bot in the sidebar to talk to it in the app's built-in chat, or press the quick-ask shortcut (⌥Space on a
Mac, Ctrl+Alt+Space elsewhere) from any app. Right-click a bot for its settings (its page).

## 5. Put it in your messaging app (optional)

On the bot's page (right-click the bot → **Settings**), add a Telegram bot token (from @BotFather), Slack tokens, or a Discord bot token. Then pair your
chat: send the bot `pair <CODE>` (the code is on the bot's page), or tap the `t.me/<bot>?start=CODE` link on Telegram.
No public URL or port forwarding is needed: Mimir connects out to each service. See
[Bots and chats](bots-and-chats.md).

## 6. Connect your email, calendar and search (optional)

Each takes a few minutes, and bots can do a lot more with them:

- **Email** (Settings → Email): an app password from Gmail, iCloud or Fastmail. Bots can search and read your inbox,
  write drafts, and send what you approve.
- **Calendar** (Meetings → Calendar): your calendar's private iCal link, so bots can read it. To let them add events
  too, connect an iCloud, Fastmail or Nextcloud calendar in Settings → Calendar; otherwise they give you one-click
  links to add events.
- **Web search** (Settings → Web search): a free Brave Search key, for bots on accounts other than Claude.

Once email or the calendar link is connected, **Settings → Get to know me** offers a one-time, read-only look at
both: who wrote to you this month and what's on your calendar (senders, subjects and events, never what an email
says). What it finds waits in **Memory** as suggestions; nothing is kept until you accept it, and what you accept stays
marked as found rather than told (see [Memory](memory.md#suggestions-from-your-email-and-calendar)).

Until something is connected, bots know it isn't: ask for your email and a bot says where to connect it
(*Settings → Email*), and the chat shows a button to that setting and a **Try again** for afterwards.

Step by step, provider by provider: [Connect your email, calendar and web search](connect.md).

## 7. Add a few skills

On the **Skills** tab (⌘3), **Mimir's skill library** has ready-made procedures: a morning briefing, inbox triage,
meeting prep, a weekly review, price and topic watches and more. **Add** the ones you want, and **schedule it** for
the daily ones. See [Skills](skills.md).

## 7. Things to try

- "Remember that my dentist is Dr. Lee in Stellenbosch." Then later: "Who's my dentist?"
- "Every weekday at 7:30, brief me on tech news in 3 bullets." (a [routine](routines.md))
- "Remind me in 2 hours to call mum." (a one-off)
- "Keep an eye on example.com/pricing and only tell me if it changes." (a quiet check)
- "Any emails about the invoice? Draft a reply saying I'll pay Friday." (with [email](connect.md#email) connected)
- "What's on tomorrow? Put the dentist on Tuesday at 3 in my calendar." (with a [calendar](connect.md#calendar) link)
- "Start a packing list for my Lisbon trip." (a file in the chat's [workspace](memory.md#files))
- "Make me a bot for SEO on heymimir.app." (a new bot, after you approve its role)
- When a bot gets something wrong, just say so ("no, too long, three bullets"): it learns, and suggests a preference
  for you to accept ([Memory](memory.md)).
- Start a Teams or Zoom call, then **Meetings → Start transcribing** ([Meetings](meetings.md)).

## Moving from OpenClaw or Hermes

Your bots keep running on your Claude plan, through Anthropic's own CLI (Claude Code): no API key, nothing extra to
pay. Updates are calm: the app downloads one in the background and installs it only when you choose **Restart to
update**, so an update never stops a bot mid-reply.

Bring your setup across with **Memory → Import → From OpenClaw** (or **From Hermes**), with Mimir on the same computer:

- **Memories**: OpenClaw's `USER.md`, `MEMORY.md` and `memory/` from its workspace (`OPENCLAW_WORKSPACE_DIR`, or
  `agents.defaults.workspace` in `openclaw.json`, or `~/.openclaw/workspace`); Hermes' `memories/`. They arrive as
  suggestions, marked *imported from OpenClaw*, and nothing is saved until you accept.
- **Profiles**: every `~/.openclaw-<name>` and `~/.hermes/profiles/<name>` comes in too, in the same import.
- **Persona**: `SOUL.md` and `IDENTITY.md` (Hermes: `SOUL.md`) aren't memories. The import offers **Use as
  <bot>'s role**; the role changes only if you click it.
- **Skills** become proposals, and **cron jobs** routines that start paused. A job set for another time zone
  says so ("Was set for 09:00 Europe/Amsterdam"): it runs on this computer's clock, so check the time before you
  resume it.

Keys, tokens and channel logins don't come over: connect Telegram, Slack or Discord again on the bot's page. More in
[Memory](memory.md#changing-and-removing).

## Finding a setting

The box at the top of **Settings** filters it as you type ("email", "token", "quiet hours"); ⌘F (Ctrl+F) jumps to it
there, and anywhere else searches your memory. Each bot's own settings (its name, role, AI account, tools, Telegram,
Slack or Discord) are on its page: right-click the bot in the sidebar.

## The menu-bar / tray icon

The app keeps running in the menu bar (macOS) or tray (Windows, Linux) when you close the window, so your bots keep
working. The icon shows approvals waiting for you, starts and stops meeting transcription, and quits Mimir.
**Settings → Keep this computer awake** stops idle sleep while Mimir runs so routines fire on time (the screen still
turns off; closing a laptop still sleeps it).

## The command line

```sh
cargo build --release
export TELEGRAM_BOT_TOKEN=123:abc      # from @BotFather; the AI defaults to your `claude` login
mimir serve                            # prints: /pair <CODE>; send that to your bot
mimir chat "remember my dentist is Dr. Lee"     # one turn, no messaging app needed
```

Other commands: `mimir server` (self-hosting), `mimir runs`, `mimir approvals`, `mimir approve <id>`,
`mimir transcribe <file>`, `mimir doctor` (is everything working?). Environment variables are listed in [Architecture](architecture.md#environment-variables).
