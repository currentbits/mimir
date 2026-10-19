# AI accounts

Mimir has no model of its own. Each bot uses an AI account you choose, on the bot's page.

| Account | How Mimir uses it | Cost |
|---|---|---|
| **Claude** | Runs the official `claude` CLI (`claude -p`) with your signed-in subscription | Your plan |
| **ChatGPT** | Sign in with ChatGPT in Mimir; bots call OpenAI's API on your Plus or Pro plan. Nothing to install | Your plan |
| **Codex CLI** | Runs the official `codex` CLI (`codex exec`) with your ChatGPT login | Your plan |
| **Grok** | Runs Grok Build (`grok -p`) on your X Subscription plan (or an older SuperGrok or X Premium+ plan) | Your plan |
| **OpenRouter** | One key for hundreds of models (only ones that can use tools are offered) | What OpenRouter charges |
| **xAI API** | Grok models on xAI's API: sign in with your Grok plan (nothing to install), or a per-token key from console.x.ai | Your plan, or per token |
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
  `claude -p` draws on your Claude plan's usage limits. Anthropic's planned move of this use to a separate monthly
  credit (announced for June 15, 2026) was paused that day, and Anthropic says it will announce any new plan before it
  takes effect. If a reply ever fails because your plan counts it as paid "extra usage", the bot says so plainly; turn
  on extra usage in your Claude account, or use another account (ChatGPT's login is supported for tools like this).
- **Grok**: Settings → AI keys → *Sign in to Grok* (a device code). Mimir keeps that login in its own Grok home
  (`~/.mimir/grok`), so your personal Grok plugins, hooks and tool servers never load into a bot, and neither do
  your Claude or Cursor settings. Grok's own memory and `/goal` stay off, so bots can't read each other's notes and
  incognito chats leave nothing behind. A message that starts with `/` reaches the bot as written; Grok doesn't run
  it as one of its commands. Bots on Grok search the web, but not X: Grok Build can't yet turn on X search without
  also turning on its shell and file tools.
  Plans: since October 2026 Grok comes with [The X Subscription](https://x.ai/news/x-subscription) (Free, Plus,
  Premium, Super, Ultra). Any of them can sign in, but Free and Plus allow little use; Premium ($30 a month, with
  SuperGrok's and X Premium+'s benefits) or higher suits bots that run all day. SuperGrok and X Premium+ plans you
  already have keep working.
- **Grok plan on xAI's API**: Settings → AI keys → *Sign in with your Grok plan* (a device code: confirm it at x.ai,
  from any browser, so it works on a server too). Bots on the **xAI API** account then run on your plan through xAI's
  API, with nothing to install: they use Mimir's tools directly, photos go along at full size, and each bot's
  conversation is its own. A saved xAI key wins over the sign-in; remove the key to use the plan. xAI decides which
  plans may be used this way (the same sign-in xAI [set up for other apps](https://x.ai/news/grok-opencode)): if
  yours isn't one, the bot says so, and the **Grok** account above still works. *Sign out* in the same place makes
  Mimir forget the sign-in.
- **OpenRouter**: *Connect OpenRouter* signs you in through the browser and saves the key (OAuth PKCE with a one-off
  localhost callback). Or paste a key.
- **Keys** (OpenAI-compatible, xAI) go to the OS keychain (or the server's private database).

## A fallback for usage limits

A bot's page can name a second account for its routines while its own is at a usage limit, until the limit resets
([how it works](routines.md#when-a-run-doesnt-work)). It's another kind of account, not a second login of the same
kind: Mimir uses one Claude, ChatGPT or Grok sign-in for all bots.

On The X Subscription, Grok's allowance is one weekly pool shared by everything on that plan: the Grok apps, Grok
Bot and Cursor. Coding in Cursor or chatting in Grok uses up the same allowance your bots run on, so a fallback
account is worth setting for Grok bots.

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
that cache useful, the instructions carry no date or time (each message starts with them), Claude models on OpenRouter
get cache breakpoints, and API accounts are handed the last 10 to 19 messages, a window that moves ten messages at a
time rather than two every turn.
