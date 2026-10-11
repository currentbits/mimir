# Self-hosting

The desktop app runs your bots while it's open. To keep them online 24/7, run Mimir on a machine you own: a small VPS
(any cloud's cheapest Linux server is enough), a home server, or a Raspberry Pi 4 or 5. It runs all your bots, keeps
your memory, and serves the same app in your browser. You can also point the desktop app at it.

It comes as a ready-made Docker image (`ghcr.io/currentbits/mimir`, for Intel/AMD and ARM), started with Docker
Compose. You don't need the source code or to build anything.

## What you need

- A Linux machine that stays on, with about 2 GB of memory and 5 GB of disk. A Mac or Windows PC with Docker Desktop
  works too.
- **Docker** with Compose. On Linux the quickest way is Docker's install script
  (`curl -fsSL https://get.docker.com | sh`, see [docs.docker.com/engine/install](https://docs.docker.com/engine/install/));
  on a Mac or Windows, [Docker Desktop](https://docs.docker.com/desktop/). Check with `docker compose version`.
- A terminal on that machine (over SSH for a VPS).

## 1. Get Mimir's compose file

In an empty folder on the server:

```sh
mkdir mimir && cd mimir
curl -fsSLO https://heymimir.app/docker-compose.yml
echo "MIMIR_COMPUTER_TOKEN=$(openssl rand -hex 24)" > .env
```

[`docker-compose.yml`](https://github.com/currentbits/mimir/blob/master/docker-compose.yml) (the same file as in the repository) describes the two
containers Mimir runs as (below); `.env` holds a random key they use to talk to each other.

## 2. Start it

```sh
docker compose up -d                          # downloads the image (about 800 MB) and starts Mimir
docker compose exec mimir mimir admin-token   # prints your admin token: copy it
```

Mimir now runs at `http://localhost:8080` **on the server**, and starts again after a reboot. On purpose it only
answers on the server itself, so nobody else can reach it until you choose how (next step).

## 3. Open it in your browser

Pick one:

- **Through an SSH tunnel** (simplest, nothing exposed). On your own computer:
  `ssh -L 8080:localhost:8080 you@your-server`, then open `http://localhost:8080` and paste the admin token.
- **With HTTPS on your own domain** (to use it from anywhere, and to connect the desktop app over the internet).
  Point a DNS name (say `bots.example.com`) at the server, open ports 80 and 443, then in `docker-compose.yml`
  uncomment the `caddy` service and the `caddy-data` volume and put your name in. `docker compose up -d` again, and
  Caddy gets a certificate on its own: open `https://bots.example.com`.
- **At home or on Tailscale.** On a home network, change the port line to `"8080:8080"` and open
  `http://<the server's address>:8080` from your own devices. With [Tailscale](https://tailscale.com) on both, the
  server's `100.x` address or `.ts.net` name works the same way, from anywhere.

Then the app is the same as on the desktop: make a bot, connect Telegram, Slack or Discord, and set up email and web
search in Settings ([Getting started](getting-started.md)).

## 4. Sign in to your AI accounts

The image includes the official `claude` and `codex` apps, so bots can run on your Claude or ChatGPT plan. Sign in once,
in the server's terminal:

- Claude: `docker compose exec -it mimir claude`, sign in, then `/exit` (or set `CLAUDE_CODE_OAUTH_TOKEN` from
  `claude setup-token` on any machine).
- ChatGPT: nothing in the terminal. Settings → AI keys → *Continue with ChatGPT*; after you sign in, your browser
  lands on a page that doesn't load (`http://127.0.0.1:…`). Paste that address into Mimir. (Or the Codex CLI:
  `docker compose exec -it mimir codex login`.)
- Grok: not in the ready-made image (its installer is a script from x.ai without a fixed version). Build the image
  yourself with `--build-arg GROK_CLI=1` (below), then *Sign in to Grok* in Settings.

It's your server and your login, the same as on your laptop. Bots on an API key (OpenRouter, OpenAI, xAI) or a model
running elsewhere need nothing extra.

## Updating

```sh
docker compose pull && docker compose up -d
```

Your data is kept: it's in the `mimir-data` volume, not in the container. To stay on one version, put
`MIMIR_VERSION=0.1.0` in `.env`.

When a new version changes how the database is laid out, it first copies it aside as `mimir.db.before-<n>` in the
same folder (the last 2 are kept), so going back means stopping Mimir and putting that copy back as `mimir.db`. An
older version won't open a database a newer one has upgraded: it stops with "This data was written by Mimir X"
instead of changing it. The same holds for moving a database between the desktop app and a server.

The server doesn't update itself, but it tells you: twice a day it looks for a newer release, and when there is one it
says so **once per version** in your most recent private chat with your bots (Telegram, Slack or Discord; never a group
or a guest's chat), with the commands above, and shows it at the top of **Needs you** until you've updated. Switch it
off in Settings → This server → *Tell me when there's a new version*. [Releases](https://github.com/currentbits/mimir/releases)
say what changed.

## Day to day

- **Logs**: `docker compose logs -f mimir`. **Stop**: `docker compose down` (your data stays).
- **A new admin token**: `docker compose exec mimir mimir admin-token --new`, then `docker compose restart`. Signed-in
  browsers and connected apps need the new one.
- **Back up** the `mimir-data` volume: it holds the database (memories, bots, chats) and, on a server, the secrets.
  For example `docker run --rm -v mimir_mimir-data:/data -v "$PWD":/out busybox tar czf /out/mimir-backup.tgz /data`
  (the volume's name starts with the folder's name; `docker volume ls` shows it).
- **Two containers**: `mimir` (bots, web app, your data) and `computer` ([the bots' computer](computer.md): a screen
  and Chromium, as its own user, without your data). They talk on their own network only; the computer publishes no
  ports. Both drop all Linux capabilities and can't gain privileges.

## Building the image yourself

In a clone of the source (`git clone https://github.com/currentbits/mimir && cd mimir`), with the `.env` from step 1:
`docker compose up -d --build`. Build options: `--build-arg GROK_CLI=1` adds Grok, `--build-arg AI_CLIS=""` leaves
out the Claude and Codex apps, `--build-arg COMPUTER=` leaves out [the bots' computer](computer.md). The AI apps, the
browser tool and the base images are fixed versions, so a rebuild doesn't change what runs your bots. Update with
`git pull && docker compose up -d --build`.

## Without Docker

```sh
cargo build --release -p mimir && (cd ui && npm ci && npm run build)
MIMIR_UI_DIR=ui/dist mimir server --listen 127.0.0.1:8080
```

## Connecting the desktop app

**Settings → Your server**: enter the server's address and admin token. The app then shows the server's bots, memory
and routines, and the bots on this computer pause, so nothing runs twice. **Disconnect** switches back. The token is
kept in the keychain. Use an `https://` address for a server on the internet; plain `http://` is accepted only for
addresses at home or on Tailscale (localhost, 192.168.x, 10.x, 100.64–127.x, `.local`, `.ts.net`), since the token
would otherwise travel in the clear.

Some things stay on the computer even when connected, because they belong to it: [meeting](meetings.md) recording and
the meetings list, the quick-ask shortcut, keeping the computer awake.

## Chat from Shortcuts, Home Assistant and other apps

**Settings → Chat API** makes a token for one bot. Give an app that speaks OpenAI's chat API the base URL
`https://<your server>/v1`, the token as its API key, and any model name. It can then ask that bot
(`POST /v1/chat/completions`, `GET /v1/models`) and do nothing else: the token doesn't open `/api` or any other bot.

The bot answers in its built-in chat, so what the app asked shows up there. Mimir keeps the conversation itself: only
the last user message of each request counts, and the app's own history is ignored. `"stream": true` gets the whole
reply as one chunk once it's done. Anything the bot needs approved waits in Needs you as usual, so set up such a bot
with the tools it needs allowed. The token is shown once and only its hash is kept; delete it there to cut the app off,
and deleting a bot deletes its tokens.

## Moving from the desktop app

Copy `~/.mimir/mimir.db` into the server's data volume before its first start, then re-enter messaging-app tokens on
the server (the desktop keeps them in the OS keychain, not the database).

## How the server works

`mimir server` runs the same host as the desktop app: every bot's Telegram / Slack / Discord connection, the routine
scheduler and approval cards, plus an HTTP API (`POST /api/<command>` with the admin token, the same commands the
desktop UI uses) and the web UI. Secrets are stored in its database instead of a keychain; the data folder is
owner-only (mode 0700). The API checks the token before reading a request; the web UI can't be framed by other sites
and loads only its own scripts. `GET /healthz` answers `ok` for health checks. A `MIMIR_ADMIN_TOKEN` you set must be at
least 24 characters.

Email, web search, the file workspaces and the skill library work the same on a server: set them up in the web UI's
Settings ([Connect your email, calendar and web search](connect.md)). The calendar link is set under Meetings, which
is in the desktop app only, so a server's bots can't read a calendar yet (add-to-calendar links work everywhere). The
server doesn't update itself: see [Updating](#updating).
