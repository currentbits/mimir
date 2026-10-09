# Self-hosting

The desktop app runs your bots while it's open. To keep them online 24/7, run `mimir server` on a machine you own (a
VPS, a home server, a Raspberry Pi). It runs all your bots, keeps your memory, and serves the same UI on the web.

## With Docker

```sh
echo "MIMIR_COMPUTER_TOKEN=$(openssl rand -hex 24)" >> .env   # once: the key between Mimir and its computer
docker compose up -d                                   # builds the image; data in the mimir-data volume
docker compose exec mimir mimir admin-token            # sign in to http://localhost:8080 with this token
```

- **A new admin token**: `docker compose exec mimir mimir admin-token --new`, then restart (`docker compose restart`).
  Signed-in browsers and connected apps need the new one.
- **HTTPS**: uncomment the `caddy` service in `docker-compose.yml` and set your domain (automatic certificates). The
  server listens on localhost by default; only expose it through TLS.
- **Back up** the `/data` volume: it holds the database (memories, bots, chats) and, on a server, the secrets.
- **Two services**: `mimir` (bots, web UI, your data) and `computer` ([the bots' computer](computer.md): screen and
  Chromium, as its own user, no `/data`). They talk on the compose network only; the computer publishes no ports.

### Your AI accounts on the server

The image includes the official `claude`, `codex` and `grok` CLIs. Sign in once:

- Claude: `docker compose exec -it mimir claude` (or set `CLAUDE_CODE_OAUTH_TOKEN` from `claude setup-token`).
- ChatGPT: `docker compose exec -it mimir codex login`.
- Grok: build the image with `--build-arg GROK_CLI=1` (its installer is a script from x.ai without a fixed version, so
  it's left out by default), then *Sign in to Grok* in the web UI (a device code).

It's your server and your login, the same as on your laptop. API-key bots need nothing extra.
The Claude and Codex CLIs, the browser tool and the base images are fixed versions, so a rebuild doesn't change what
runs your bots. `--build-arg AI_CLIS=""` builds a slim image without the CLIs; `--build-arg COMPUTER=` leaves out
[the bots' computer](computer.md). The compose file drops all Linux capabilities and forbids gaining privileges.

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
server doesn't update itself: get the new code and rebuild (`git pull && docker compose up -d --build`).
