# Contributing

Thanks for wanting to help. Issues with a clear "what I did, what I expected, what happened" are the most useful
thing you can send; say which AI account and which messaging app the bot was on.

**Pull requests.** Small and focused, with a test for the behaviour you changed (`cargo test --workspace`, and
`npx tsc -b` in `ui/`). Update the matching page in `docs/` when what people see changes, and add a line to
`CHANGELOG.md` in the words someone using Mimir would use.

## Develop

**Prerequisites**, on every OS: [Rust](https://rustup.rs) (stable), Node.js 22, Python 3 (for the test scripts), and
`cmake` (the built-in transcription builds whisper.cpp).

- **macOS**: Xcode's command line tools (`xcode-select --install`) and `brew install cmake`.
- **Windows**: Visual Studio Build Tools with "Desktop development with C++", [CMake](https://cmake.org) and
  [LLVM](https://llvm.org) (whisper.cpp's bindings need libclang). WebView2 comes with Windows 10 and 11.
- **Linux** (Debian / Ubuntu names, as in CI):
  `sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libdbus-1-dev
  libasound2-dev libclang-dev cmake`

**Run it.**

```sh
cd ui && npm install && npm run tauri dev       # the desktop app, reloading as you edit the UI
(cd ui && npm run build) && cargo run -p mimir -- server   # or the server, with the web UI on http://127.0.0.1:8080
cargo run -p mimir -- admin-token               # its sign-in token
```

Both use your real Mimir data (`~/.mimir`) unless you set `MIMIR_HOME` to another folder. More variables:
[docs/architecture.md](docs/architecture.md#environment-variables).

**Test it.**

```sh
cargo test --workspace                          # unit and integration tests
cd ui && npx tsc -b                             # the UI typecheck (`tsc -p .` checks nothing here)
cargo clippy --workspace --all-targets -- -D warnings; cd ui && npm run lint
```

End to end, without a real AI account, in a fresh data folder (CI runs these on Linux; the UI must be built):

```sh
python3 scripts/fake-ai.py 8899 &               # a scripted AI with an OpenAI-style API
export MIMIR_HOME=/tmp/flow-home MIMIR_ADMIN_TOKEN=testtoken-abcdefghijklmnop
MIMIR_CHATGPT_AUTH=http://127.0.0.1:8899 MIMIR_CHATGPT_API=http://127.0.0.1:8899/v1 \
  cargo run -p mimir -- server --listen 127.0.0.1:8789 &
python3 scripts/e2e-flows.py                    # bot flows, checked in the database
(cd ui && npx playwright install chromium) && node scripts/ui-flows.mjs   # the same, clicked through in a browser
```

- `scripts/mock-telegram.py <port> [log]`: a fake Telegram Bot API; start Mimir with
  `MIMIR_TELEGRAM_API=http://127.0.0.1:<port>` and any bot token.
- `scripts/mock-signal.py <port> [log]`: a fake signal-cli-rest-api; start Mimir with
  `MIMIR_SIGNAL_API=http://127.0.0.1:<port> SIGNAL_NUMBER=+15550000`. `scripts/e2e-signal.py <dir>` runs a bot on it.
- `scripts/e2e-whatsapp.py <dir>`: a bot on WhatsApp, against a fake Graph API it runs itself and `mimir server`.
- `scripts/fake-mail.py [dir]`: a fake IMAP (127.0.0.1:1143) and SMTP (127.0.0.1:1025) server for the email tools.
- `scripts/e2e-busy-chat.py <dir>`: Telegram + fake AI together, starting both itself.

Each script's header says what it checks and how to run it. The opt-in tests (real models, the meeting recorder, the
updater) are in [docs/architecture.md](docs/architecture.md#tests).

## Pull requests and licence

**Licence.** Mimir is AGPL-3.0-only. Before the first outside pull request is merged we'll ask contributors to sign a
contributor licence agreement (through CLA Assistant, one click), so the project can keep offering other licences
later. Until then, please open an issue first if you're planning something large.

**Security.** Please don't open public issues for security problems: use GitHub's private vulnerability reporting on
this repository instead. [docs/security.md](docs/security.md) describes what Mimir defends against.
