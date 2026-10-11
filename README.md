# Mimir

A free app that runs AI assistants for you, on **your own** AI subscription (Claude, ChatGPT, Grok, or any API key).
You message (or talk to) them in Telegram, Slack, Discord or the desktop app; they remember you, work on a schedule,
use your email, calendar and the web, and ask before they act. Mimir has no AI and no servers of its own. Website:
[heymimir.app](https://heymimir.app).

- **Bots that know you, and learn.** They remember facts, preferences and tasks (shared by all your bots or kept for
  one, plus a household everyone shares; all visible and correctable), search your earlier conversations, and learn
  skills: from tasks that went well, from your corrections, when you say "save that as a skill", from a link or
  SKILL.md, or by watching you do it once on their computer. Run one with `/book-padel`, from a routine, or share it
  with all your bots. Every change is a proposal; nothing changes until you accept.
- **Routines.** "Every weekday at 7:30, brief me on tech news." Or started by something happening: an hour before each
  meeting with someone from outside, when an invoice email arrives, or by a webhook. Quiet checks that only speak up
  when something changes, and that don't run the AI at all while a watched page or inbox stays the same. Daily caps, a
  rehearsal before a routine runs for real, and failures handled by cause (wait for the plan's limit, retry the
  network). A daily digest and quiet hours.
- **Tools, safely.** Web pages, a real browser with saved logins the AI never sees, any MCP tool server, and on your
  server a computer bots use like a person. Every action is allowed, asked or denied by your rules; after reading
  anything from outside, changes always ask.
- **Meetings and calls.** Transcribe Teams, Zoom, Meet and phone calls on your computer (no bot joins), tell the
  speakers apart, and have a bot write the notes. Ask "what did we decide in standup?" from Telegram.
- **Yours.** No Mimir servers or account: data in one SQLite file on your machine (or your own server), secrets in the
  OS keychain.

## Download

Installers for macOS (Apple silicon), Windows and Linux are on the [releases page](https://github.com/currentbits/mimir/releases/latest).
The Mac app is signed and notarized by Apple. The Windows installer isn't code-signed yet: the first time, Windows SmartScreen says
"Windows protected your PC"; click **More info**, then **Run anyway**. After that the app updates itself. Then: [Getting started](docs/getting-started.md).

## On your own server

For bots online 24/7: a ready-made Docker image (`ghcr.io/currentbits/mimir`, Intel/AMD and ARM). In an empty folder:

```sh
curl -fsSLO https://heymimir.app/docker-compose.yml
echo "MIMIR_COMPUTER_TOKEN=$(openssl rand -hex 24)" > .env
docker compose up -d
docker compose exec mimir mimir admin-token    # sign in to http://localhost:8080 (on the server) with this
```

How to open it safely, sign in to your AI accounts and update: [Self-hosting](docs/self-hosting.md).

## Build it yourself

```sh
cd ui && npm install && npm run tauri dev      # the desktop app (macOS, Windows, Linux)
docker compose up -d --build                   # the server image, from this source
```

```sh
cargo build --release
TELEGRAM_BOT_TOKEN=123:abc mimir serve         # or one bot from the command line (uses your `claude` login)
```

## Documentation

**[docs/](docs/README.md)** explains how everything works:

| | |
|---|---|
| [Getting started](docs/getting-started.md) | Install, connect an AI account, first bot |
| [What you can do with it](docs/examples.md) | Everyday jobs, step by step: briefings, inbox, calendar, watching pages, research |
| [Connect email, calendar and search](docs/connect.md) | App passwords, calendar links, search keys, provider by provider |
| [Bots and chats](docs/bots-and-chats.md) | Bots, Telegram / Slack / Discord, quick ask, incognito, hand-offs |
| [AI accounts](docs/ai-accounts.md) | Claude, ChatGPT, Grok, OpenRouter, local models, any API |
| [Memory](docs/memory.md) | What bots remember, the tidy-up, files |
| [Skills](docs/skills.md) | Step-by-step procedures, the skill library, learning and teaching skills |
| [Routines](docs/routines.md) | Schedules, quiet checks, digest, quiet hours, *Needs you* |
| [Tools, approvals and safety](docs/tools-and-safety.md) | Allow / ask / deny, outside content, plans, browser, MCP |
| [Meetings and calls](docs/meetings.md) | Transcription, speakers, notes, calendar, and how it works |
| [The bots' computer](docs/computer.md) | A Linux desktop on your server, watched and taken over live |
| [Self-hosting](docs/self-hosting.md) | `mimir server`, Docker, HTTPS, connecting the app |
| [Privacy and your data](docs/privacy.md) | What's stored where, what leaves your computer |
| [Security model](docs/security.md) | Who Mimir defends you against, the rules that do it, what's left |
| [Architecture](docs/architecture.md) | Crates, how a message becomes a reply, tests, environment variables |
| [Questions people ask](docs/faq.md) | Cost, accounts, data, safety, running it |

## License

Mimir is licensed under the [GNU Affero General Public License v3.0](LICENSE) (AGPL-3.0-only).
If you run a modified version as a service for others, you must share your changes under the same license.
