# AI accounts

Mimir has no model of its own. Each bot uses an AI account you choose, on the bot's page.

| Account | How Mimir uses it | Cost |
|---|---|---|
| **Claude** | Runs the official `claude` CLI (`claude -p`) with your signed-in subscription | Your plan |
| **ChatGPT** | Sign in with ChatGPT in Mimir; bots call OpenAI's API on your Plus or Pro plan. Nothing to install | Your plan |
| **Codex CLI** | Runs the official `codex` CLI (`codex exec`) with your ChatGPT login | Your plan |
| **Grok** | Runs Grok Build (`grok -p`) on your SuperGrok or X Premium+ plan | Your plan |
| **OpenRouter** | One key for hundreds of models (only ones that can use tools are offered) | What OpenRouter charges |
| **xAI API** | A per-token key from console.x.ai | Per token |
| **Local model** | Ollama (`localhost:11434`) or LM Studio (`localhost:1234`); nothing leaves your computer | Free |
| **OpenAI-compatible** | OpenAI or any compatible service, with a key and base URL | Per token |

**Web search.** Bots on Claude search the web on their own (Claude's built-in search). For bots on any other account,
set up Brave Search or your own SearXNG once in Settings → Web search, and every bot can search
([how](connect.md#web-search)).

## Signing in

- **ChatGPT**: Settings → AI keys → *Continue with ChatGPT* (or pick ChatGPT on the first screen). You sign in on
  chatgpt.com and allow Mimir to use your plan; nothing to install. Your plan's allowance for Mimir is separate from
  ChatGPT's own, and you choose it: ChatGPT → Settings sets how much Mimir may use each week. When that's used up,
  bots say so and wait. Needs ChatGPT Plus or Pro. On a [server](self-hosting.md), your browser ends on a page that
  doesn't load (an address starting with `http://127.0.0.1`): paste that address into Mimir to finish. *Sign out* in
  the same place ends the sign-in at OpenAI too. Bots use ChatGPT's own web search where your plan offers it to
  other apps, and otherwise the search set up in Settings → Web search.
- **Claude / Codex CLI**: install the CLI and sign in once in a terminal (`claude`, then `codex login`). The app shows
  whether it's installed and signed in. On a server, see [Self-hosting](self-hosting.md#your-ai-accounts-on-the-server).
  Since April 2026 Anthropic has at times counted Claude-plan use outside its own apps as paid "extra usage". If a reply
  fails because of that, the bot says so plainly; turn on extra usage in your Claude account, or use another account
  (ChatGPT's login is supported for tools like this).
- **Grok**: Settings → AI keys → *Sign in to Grok* (a device code). Mimir keeps that login in its own Grok home
  (`~/.mimir/grok`), so your personal Grok plugins, hooks and tool servers never load into a bot.
- **OpenRouter**: *Connect OpenRouter* signs you in through the browser and saves the key (OAuth PKCE with a one-off
  localhost callback). Or paste a key.
- **Keys** (OpenAI-compatible, xAI) go to the OS keychain (or the server's private database).

## How tools reach the model

The CLIs (Claude, Codex, Grok) start Mimir as an **MCP server** for each run, so the model sees Mimir's tools
(memory, routines, web pages, meetings…) the same way it sees any tool. API accounts get the same tools as
function-calling definitions. Either way, every call passes through Mimir's rules (see
[Tools, approvals and safety](tools-and-safety.md)).

## Local models

The app finds a running Ollama or LM Studio and lists its models. Models that can't use tools are flagged: they can
chat but can't remember things or run routines. Ollama's default 4k-token context drops older messages in a long
conversation (instructions and memory are kept); start it with `OLLAMA_CONTEXT_LENGTH=16384` for more.

## What a run costs

The run log shows each reply's model, tokens and cost. For Claude it's the API list-price equivalent that `claude -p`
reports (subscriptions aren't billed per token); OpenRouter runs show what OpenRouter charged; other key and local runs
show tokens only.

Hover the tokens to see how much of the input came from the AI's prompt cache and how much was written to it. To keep
that cache useful, the instructions carry no date or time (each message starts with them), Claude
models on OpenRouter get cache breakpoints, and API accounts are handed the last 10 to 19 messages, a window that
moves ten messages at a time rather than two every turn.
