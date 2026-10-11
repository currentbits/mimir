# Privacy and your data

Mimir has no servers of its own and no account. Your data stays on your computer (or your own server), and talks only
to the services you connected.

## What's stored where

| What | Where |
|---|---|
| Memories, bots, chats (searchable by your bots with `chat_search`), routines, runs, approvals, meetings (transcripts, your notes, speaker names, summaries) | `~/.mimir/mimir.db` (one SQLite file; `MIMIR_HOME` moves it) |
| Bot tokens, API keys (including Brave Search's), saved logins, tool-server tokens and sign-ins, the calendar link, the email and calendar app passwords, the Outlook sign-in | The OS keychain (macOS Keychain, Windows Credential Manager, the Linux keyring); on a server, its database; the data folder is owner-only (mode 0700) |
| Your mailboxes' server names and addresses (not their passwords), the Outlook app's client ID | `~/.mimir/mimir.db` (settings) |
| The chats' file workspaces (notes, lists, drafts bots keep) | `~/.mimir/files/chat-<id>/`, with earlier versions under `.history/` |
| Photos you send (and a scanned PDF's pages) | `~/.mimir/media/`, deleted after 60 days |
| A file's text you send (PDF, Word, Excel, text file) | In the chat's history, with your message |
| Each bot's browser profile (its logins and cookies on sites) | `~/.mimir/browser/`; deleted with the bot |
| The AI CLIs' own copies of conversations | Claude Code: `~/.claude/projects/` (the folder for Mimir's workspace); Codex: `~/.codex/sessions/`; Grok: `~/.mimir/grok/.grok/sessions/`. Deleted when you start fresh, delete a chat's history or delete the bot |
| Speech, voice-activity and speaker models (and on Linux the speaker library) | `~/.mimir/whisper/` |
| Grok's sign-in (Mimir's own) | `~/.mimir/grok/` |
| Meeting audio | Not kept. The others' audio waits in `~/.mimir/meeting-audio/` only until the call ends (to tell speakers apart), then is deleted; any left by a crash is deleted when the app starts |

## What leaves your computer

- **Your messages and the context for a reply** (your instructions, your standing memories, memories that match the
  message, the conversation, tool results) go to the AI account the bot uses: Anthropic for Claude, OpenAI for
  ChatGPT/OpenAI keys (signed in with ChatGPT, each request asks OpenAI not to store it), xAI for Grok, OpenRouter
  and the model's provider, or nowhere for a local model.
- **The daily tidy-up** reads a chat's recent conversation with all your bots, on the AI account of the bot that does
  the tidy-up (Settings). So a conversation with a bot on a local model reaches that account too, if it's a cloud one.
- **Voice notes** from Telegram are transcribed with your OpenAI-compatible key.
- **Messaging apps** carry the messages you exchange there (Telegram, Slack, Discord).
- **Meetings**: transcription and speaker separation run on your computer. A transcript leaves only when a bot reads it
  (it then goes to that bot's AI account like any other message). Without the built-in engine, the OpenAI-compatible
  fallback sends audio to that service.
- **Downloads** (models, Chromium for the browser) come from Hugging Face, GitHub and the Playwright CDN, once. Every
  model (speech, voice activity, speakers) and the speaker library Linux loads are fixed versions checked against
  known SHA-256 checksums (a file that doesn't match is thrown away); the browser tool is a fixed version.
- **On this computer**, the data folder is readable only by your account. The AI CLIs get a bot's instructions (which
  hold your memories) and your message through private files that are deleted after each reply, never on their
  command line, which other accounts on the same computer could see. (On Windows, where other accounts can't read a
  command line, Grok still gets the instructions there; and a message with a photo goes to Grok on its command line,
  photo included, as it has no file form for that.)
- **Web pages** a bot reads see a plain `Mozilla/5.0` browser, nothing that says Mimir.
- **Tools you allowed**: web pages bots read, sites the browser opens, tool servers you added, your calendar link.
- **Email**, once connected: Mimir signs in to *your* mail server (over TLS) to search, read and save drafts, and
  sends through your outgoing server what you approve, to the people it's addressed to. An email a bot reads goes to
  its AI account like any other text it reads.
- **Web search**, once set up: the queries go to Brave Search or your own SearXNG.
- **Add-to-calendar links**: nothing is sent until you open one; then Google Calendar or Outlook gets the event in it.
- **Events you approve** go to the calendar server you connected in Settings → Calendar, and nowhere else.
- **Updates**: the desktop app asks GitHub for the newest release a minute after it starts and every 6 hours (GitHub
  sees your address and the request, nothing else), and downloads an update from there. Switch it off in
  Settings → Updates.

Nothing is sent anywhere else: the app has no analytics and no telemetry. (The website, heymimir.app, counts visits and
download clicks with Meterbird; that's the site, not the app.)

## Keeping and deleting

- **Erase** (Memory) deletes a memory with its earlier versions, the suggestions that added it and the tool log's lines
  about it; the search index forgets it at once. Conversations that mentioned it stay until you delete that history.
- **Delete this chat's history** (at the bottom of Activity) deletes the conversation, the run log with what each
  action did, corrections, digests, decided approvals and suggestions, and the AI CLIs' own copies. Memories and
  routines stay.
- **Deleting a bot** deletes its conversations, run log, routines, browser profile and the AI CLIs' copies; memories
  stay (they're yours, shared by all bots).
- **Kept for six months**: the details of each action in the run log, corrections and signals. Then they're deleted.
- **Export everything** (Settings → Your data) saves one JSON file with your memories, conversations, meetings,
  skills, routines, chats and bots (not tokens or keys).
- **Daily backups**: once a day Mimir checks the database (SQLite's `quick_check`) and keeps a copy in `backups/`
  in its data folder: the last 7 days and 4 weeks. Settings → Your data → **Restore a backup…** puts one back (a copy
  of now is kept first). If the check finds damage, no copy is taken and Needs you says so. The copies hold what the
  database holds (on a server, its secrets too) and stay in the owner-only data folder.
- **Delete everything**: quit Mimir, delete `~/.mimir` (or your `MIMIR_HOME`), remove the Mimir items from the
  keychain, and delete Mimir's workspace folders in `~/.claude/projects/` and Codex's sessions in `~/.codex/sessions/`.

## Incognito

An incognito chat keeps nothing: no history, run log or tool log; the bot can't see or save memories, routines or
meetings; and the AI CLIs are told not to save the session. See [Bots and chats](bots-and-chats.md#incognito).

## Recording calls

Transcribing a call records other people. Tell them, and check the rules where you are: many places require the
consent of everyone on the call.
