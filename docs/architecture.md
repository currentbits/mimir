# Architecture

For contributors: how the pieces fit together.

## Crates

| Crate | What it is |
|---|---|
| `crates/mimir-core` | Everything that isn't a UI shell: the database, bots, providers, tools, channels, routines, meetings |
| `crates/mimir` | The CLI and the self-hosted server (`server.rs`: HTTP API + web UI) |
| `crates/mimir-desktop` | The Tauri desktop app: window, tray, keychain, quick ask, notifications, meeting recorder |
| `ui/` | React + Vite UI, shared by the desktop app and the server's web UI. `src/App.tsx` is the window (sidebar, tabs); each screen is in `src/views/` (chat, memory, skills, routines, activity, inbox, bot, accounts, onboarding, settings, meetings, computer); `types.ts` holds the shapes more than one screen uses and `shared.tsx` the helpers they share |

### mimir-core modules

| Module | Does |
|---|---|
| `db` | SQLite (one file): memories with FTS5 search and versions, bots, chats, messages (FTS5 too, for `chat_search`), routines, runs, tool calls, approvals, rules, suggestions, skills (versions, uses, proposals by kind), meetings |
| `host` | `Host::call(cmd, args)`: every command the UI sends; starts and stops each bot's runtime; the secret store |
| `runtime` | Per-bot channel listeners (Telegram, Slack, Discord), the 30 s routine scheduler, approval cards, the daily digest |
| `telegram`, `slack`, `discord` | The messaging apps (outbound connections, streaming replies) |
| `provider` | AI accounts: starts `claude -p` / `codex exec` / `grok -p`, or calls an OpenAI-compatible API; streams replies |
| `chatgpt` | Sign in with ChatGPT: OAuth (PKCE, dynamic registration), the plan's tokens, and replies on the Responses API |
| `lib.rs` (`turn`) | One run: builds the instructions (role, standing memory, skills), calls the provider, records the run; `/skill` commands (`skill_command`) |
| `tools` | Tool definitions, the allow / ask / deny gate, taint, plans, and the built-in tools' code |
| `mcp` | The MCP server the CLIs start for each run (`<app> mcp --bot B --chat C --run R`, plus `--read-only` for quiet checks, `--no-tools`, `--depth` for hand-offs), exposing Mimir's tools |
| `ext` | Tool servers (MCP clients over stdio and Streamable HTTP), the browser, saved logins |
| `tidy` | The daily memory tidy-up (on `tidy_model` if set): corrections, skills from good runs, fixes for skills that went wrong, weekly upkeep (retire / merge / move to background), the weekly review; teaching a skill from a source or a SKILL.md |
| `computer` | The bots' computer (Xvfb, xdotool, scrot, Chromium), self-hosted Linux only; recording a showing for teach by showing (page details through Chromium's DevTools port) |
| `transcribe` | Speech to text (whisper.cpp built in, or `whisper-cli`, or an API), voice activity, speaker separation |
| `calendar` | iCal links: parsing, recurrence (`rrule`), the meeting on now, events between dates (`calendar_list`) |
| `email` | IMAP (async-imap, mail-parser): search, read, drafts by APPEND, all EXAMINE/PEEK; SMTP sending (lettre); every operation within 60 s |
| `files` | The per-chat file workspace: path checks, versions under `.history/`, outside-content marks keyed by canonical name and modification time |
| `library` | Mimir's skill library: the SKILL.md files in `crates/mimir-core/skills/`, compiled in, with what each needs and a suggested schedule |
| `fmt`, `live`, `openrouter` | Message formatting per app, "what it's doing now", OpenRouter sign-in |

## How a message becomes a reply

1. A channel listener (or the UI's `send_local`) receives a message for a bot and chat.
2. `turn` builds the instructions: the bot's role, standing memory, accepted skills, the conversation.
3. `provider` runs the bot's AI account. For the CLIs, it passes MCP arguments so the CLI starts `<app> mcp …` and sees
   Mimir's tools; for APIs, tools are function definitions handled in-process.
4. Each tool call goes through `tools::call`: the allow / ask / deny rule, taint (outside content read in this
   conversation), the bot's allowlist, then the tool itself. "Ask" creates an approval and waits for your answer from
   any app.
5. The reply streams back to where you asked; the run and every tool call are recorded.

## Desktop and server

Both run the same `Host`. The desktop calls it in-process through one Tauri command (`api`); the server exposes it as
`POST /api/<cmd>` with a bearer token. The UI's `invoke()` goes to whichever is active, so the same React app is the
desktop UI and the web UI. A few commands are handled by the desktop itself, before the host, because they belong to
this computer (`api` in `mimir-desktop/src/main.rs`):
- `meeting`: recording, the meetings list, the microphone and calendar settings;
- `keep_awake`, `open_at_login`, `quick_shortcut`, `show_main`;
- `copy_text`, `save_download`, `open_url`: what the webview can't do itself (clipboard after a wait, downloads, links
  in the browser);
- `remote_token`: the admin token of the server the app is connected to, kept in the keychain;
- `local_bots`: runs this computer's bots, or stops them while the app shows a server;
- `update_status`, `update_check`, `update_install`, `update_auto`;
- `selftest`, `selftest_report`: the launch test (`MIMIR_SELFTEST=1`).

## Meetings

`mimir-desktop/src/meeting.rs` records (cpal for the microphone; a Core Audio process tap on macOS 14.4+, else
ScreenCaptureKit / WASAPI loopback / `parec` for the computer's sound), chunks at pauses, and calls `transcribe` (whisper.cpp through `whisper-rs`, with Silero VAD run
separately because whisper-rs's per-state call skips whisper.cpp's own). After Stop, speaker separation runs through
`sherpa-rs` (feature `diarize`, linked in, macOS and Windows) or, on Linux, sherpa-onnx's shared libraries downloaded on
first use and loaded at runtime (feature `diarize-dl`: `transcribe::sherpa_dl`, structs mirroring its C API). Call detection (`mic_in_use`) is per OS. See
[Meetings](meetings.md#how-it-works).

Build notes:
- whisper.cpp builds with cmake (and libclang on Linux).
- `.cargo/config.toml` sets `GGML_NATIVE=OFF` (+ AVX2 on x86) so builds run on any CPU of the platform.
- The desktop `build.rs` adds the `/usr/lib/swift` rpath that ScreenCaptureKit's Swift bridge needs on macOS.
- Speech models must be freed before exit (`transcribe::unload`), or Metal aborts.

## Learning

Bots can't be fine-tuned (they run on your own subscription), so they improve only through memory and skills, and
everything learned is a **proposal you accept**, with its provenance recorded. Signals are objective (corrections,
stops, denials, failed actions), never the model judging its own success. Nothing learned can change permissions,
tool grants, saved logins or schedules. See [Memory](memory.md) for the user-facing side, and `reports/` (local, not
published) for the research behind it.

## Security rules in the code

The [Security model](security.md) in terms of where it lives. Keep these when changing the code.

- **Outside content.**
  - `ToolCtx::taint` marks the run (`runs.tainted`) and the conversation (`conv_taint`).
  - `is_tainted` also re-reads the run, so the tools server sees what the parent process marked.
  - What carries the mark: `routines.tainted`, `approvals.tainted` (restored by `run_parked`), `suggestions.tainted`,
    `digest_items.tainted`, and `Db::recent_outside` for replayed history (only after the `fresh:{bot}:{chat}` marker).
  - A file's or forward's text is wrapped by `with_document` (its position math uses ASCII lowercasing).
- **The gate.**
  - `tools::call`: `precheck` (limits, before any card), then `gate`, then `leaks_out` (data-carrying reads), then the
    approval.
  - `card_text` builds every card.
  - `site_of` / `ext::host_allowed` parse hosts with `url::Url` and refuse user info.
- **Learning.**
  - `Db::bootstrap` and automatic recall skip unconfirmed memories, and `Entry::line` flattens text.
  - The tidy-up sees unconfirmed memories by title only, and labels outside lines.
  - `suggestion_decide` carries the mark into accepted entries.
- **Channels.**
  - `Core::sender_ok`: only the person who paired a chat (`chats.owner`; `chats.dm` for direct chats).
  - Every Telegram, Slack and Discord entry point passes a `Sender`.
  - The listener loop recovers from panics.
- **Processes.**
  - The AI CLIs get instructions through private files (`Private`), never the command line: Claude
    `--append-system-prompt-file`, Codex `model_instructions_file`, Grok a rules file in a trusted per-bot-and-chat
    folder (`grok_trust`, `grok_rules`; Windows keeps `--rules`).
  - PDFs are read by `<binary> read-pdf` (`pdf_helper_main`); `read-pdf pages` gives a scanned PDF's page pictures.
  - The AI CLIs' transcripts are removed by `Provider::forget_session`.
  - `Host::new` does startup cleanup (`incognito_leftovers`, `prune_old`, `close_interrupted`, `tmp/`, old media). Never put that in
    `Db::open`: the tools server opens the database on every turn.
- **Downloads.** `transcribe::download(url, path, min, sha256)` with pinned hashes. Models come from a fixed Hugging
  Face revision.

## Data

One SQLite file in the data folder (`~/.mimir`, or `MIMIR_HOME`). Its schema version is `PRAGMA user_version`
(`VERSION` in `db.rs`). A schema change bumps `VERSION` and adds a `migrate_v<n>` step to `migrate`. Each step runs
once, in one transaction, after `VACUUM INTO mimir.db.before-<n>` (the last 2 are kept). An up-to-date database only
reads its version. A database with a higher version than the build is refused, and the desktop app shows the error
and the data folder in a dialog. Installs from before the rename to Mimir (`~/.botpoc`,
`botpoc.db`) are moved over once on startup.

## Environment variables

| Variable | |
|---|---|
| `MIMIR_HOME` | Data folder (default `~/.mimir`) |
| `MIMIR_PROVIDER`, `MIMIR_MODEL` | CLI bot: `claude` (default), `codex`, `grok`, `openrouter`, `xai`, `local`, `openai`; model override |
| `OPENROUTER_API_KEY`, `XAI_API_KEY`, `OPENAI_API_KEY`, `OPENAI_BASE_URL` | Keys for the CLI bot; `OPENAI_BASE_URL` also points `local` elsewhere (LM Studio: `http://localhost:1234/v1`) |
| `TELEGRAM_BOT_TOKEN`, `SLACK_BOT_TOKEN` + `SLACK_APP_TOKEN`, `DISCORD_BOT_TOKEN` | Messaging apps for `mimir serve` |
| `MIMIR_BOT` | Which bot the CLI acts as (default 1) |
| `MIMIR_ADMIN_TOKEN` | Server: a fixed admin token, at least 24 characters (else one is generated on first run) |
| `MIMIR_UI_DIR` | Server: the web UI folder (default `ui/dist` or `/app/ui`) |
| `MIMIR_COMPUTER`, `MIMIR_COMPUTER_TOKEN` | Server: the bots' computer as a separate service (`computer:7300`, `mimir computer`) and the control channel's secret (24+ characters, on both sides) |
| `MIMIR_TELEGRAM_API` | Telegram Bot API base URL (a self-hosted Bot API server, or the test mock) |
| `MIMIR_CHATGPT_AUTH`, `MIMIR_CHATGPT_API` | Sign in with ChatGPT: the sign-in and API base URLs (for the test fake, `scripts/fake-ai.py`) |
| `MIMIR_CLAUDE_BIN`, `MIMIR_CODEX_BIN`, `MIMIR_GROK_BIN`, `MIMIR_NPX` | Paths to the CLIs, if not on PATH |

## Releases

`.github/workflows/release.yml` builds the installers when a version tag is pushed:

1. Set `version` under `[workspace.package]` in the root `Cargo.toml`. It's the only place: every crate inherits it
   and the desktop app takes it from its crate (`tauri.conf.json` has no version). Rename the "Unreleased" section of
   `CHANGELOG.md` to the version (`## 0.1.6`) and commit.
2. `git tag v0.1.6 && git push origin v0.1.6`.
3. The workflow checks the tag against `cargo metadata` and that `CHANGELOG.md` has the section, then drafts a GitHub
   release with that section as its notes and attaches the `.dmg`, `.exe`/`.msi`, `.deb`/`.rpm` and `.AppImage`
   (macOS on Apple silicon; Linux built on Ubuntu 22.04 so it runs on older systems). Installed apps show the section
   with the update.
4. Read the draft over and publish.

**Signing.** With the Apple secrets set (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_P8`), CI signs the Mac app with Developer ID, notarizes the app and
the `.dmg`, and only then adds "signed and notarized by Apple" to the notes. Without them the app is ad-hoc signed
(`signingIdentity: "-"`, which Apple silicon needs to open a downloaded app at all) and the notes don't mention
notarization: sign and notarize on a Mac with the Developer ID certificate instead, and replace the Mac assets (and
their signatures in `latest.json`) before publishing. The released Mac apps so far were done that way. Windows builds
aren't signed.

**Updates** (`crates/mimir-desktop/src/update.rs`, Tauri's updater plugin). Apps read `latest.json` from the newest
*published* release (`plugins.updater.endpoints` in `tauri.conf.json`; drafts aren't served), so publishing the draft is
what ships an update. With the `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets set, each
build signs its update files (`.app.tar.gz`, the installers) and a last job writes `latest.json` from those signatures;
without them, releases still build but installed apps don't see them. Apps refuse anything not signed by the private
key whose public half is in `tauri.conf.json`: losing that key means users must reinstall by hand once, so keep a copy
somewhere safe. End-to-end test on a Mac: build 0.1.1 signed and 0.1.0 pointed at a local server
(`--config '{"plugins":{"updater":{"endpoints":["http://127.0.0.1:8899/latest.json"],"dangerousInsecureTransportProtocol":true}}}'`),
serve the first, and start the second with `MIMIR_UPDATE_TEST=1`: it installs and restarts as 0.1.1.

## Tests

```sh
cargo test --workspace          # unit and integration tests (needs cmake for the desktop crate)
cd ui && npx tsc -b             # the UI typecheck (`tsc -p .` checks nothing here)
cargo clippy --workspace --all-targets -- -D warnings; cd ui && npm run lint
```

CI (`.github/workflows/build.yml`) runs all of these, plus the end-to-end flows on Linux: `scripts/fake-ai.py` and
`mimir server` with `MIMIR_CHATGPT_*` pointed at it, then `scripts/e2e-flows.py` and `scripts/ui-flows.mjs` (see each
script's header; Playwright comes with `npm ci` in `ui/`). If another checkout's test server already has ports 8789/8899,
start yours on others and set `E2E_SERVER` / `E2E_AI`. A pull request that touches the `Dockerfile` or a lockfile also
builds the server image (`docker.yml`, not pushed). Unit tests use a data folder of their own and no real channel tokens.

Test hooks and opt-in tests:
- `MIMIR_SELFTEST=1 <desktop binary>`: starts the real app, reports what rendered, exits 0 only without UI errors
  (CI runs this on all three OSes).
- `MIMIR_MEETING_TEST=<seconds>` (+ `MIMIR_MEETING_TEST_QUIT=1`): records for that long, then stops (or quits) and
  prints what was saved. With PulseAudio null sinks in a container, this tests the whole meeting pipeline on Linux.
- `MIMIR_WHISPER_DEBUG=1`: prints voice-activity stretches and chunk timings.
- `scripts/mock-telegram.py`: a fake Telegram Bot API; set `MIMIR_TELEGRAM_API` to it.
- `scripts/fake-mail.py`: a fake IMAP (port 1143) and SMTP (port 1025) server, any login accepted, with a planted
  instruction in one message for outside-content tests. Point a bot's email at `127.0.0.1`.
- `scripts/e2e-busy-chat.py <dir>`: a Telegram message to a busy bot gets an answer, not silence. Starts its own
  `mock-telegram.py` and `fake-ai.py`; wants a `mimir` binary in `<dir>`.
- `scripts/learning-eval.py <mimir binary> [model] [--control] [--web]`: does learning work end to end? A simulated
  person with three hidden preferences talks to a real bot for three "days", correcting it; the tidy-up runs between
  days. Prints how many replies broke a preference each day (with learning it should drop to about zero by day 2).
  `--control` learns nothing, for comparison; `--web` forces a web search on day 1 (learning must survive it).
- `MIMIR_UPDATE_TEST=1`: checks for an update after 3 s and installs what it finds (the update test, see Releases).
- Ignored tests that need real models or files (run with `-- --ignored`): `builtin_real`, `diarize_real`,
  `speakers_real`, `calendar_real`, `mic_probe`. Each says which variables to set.
