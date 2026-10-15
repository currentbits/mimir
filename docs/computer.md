# The bots' computer

On a [self-hosted server](self-hosting.md), **Settings → Computer** gives your bots a Linux desktop with Chromium. They
see it in screenshots and use the mouse and keyboard like a person, so they can work with any website, not only ones
with an API: book a court, fill in a form, check an order.

It runs on the server, never on your laptop. With `docker compose` it's a service of its own (`computer`), as its
own user and with none of Mimir's data, so a browser exploit from a page a bot visits doesn't reach your database,
secrets or AI logins. A plain `docker run` of the image keeps it inside Mimir's container instead. The Docker image has
everything (build with `--build-arg COMPUTER=` to leave it out).

## Tools

| Tool | Does | Default |
|---|---|---|
| `computer_screenshot` | Looks at the screen (read-only) | allow |
| `computer_click`, `computer_type`, `computer_key`, `computer_scroll` | Mouse and keyboard | allow |
| `computer_open` | Opens a website | **ask** (or "always on this site") |
| `computer_signin` | Signs in to the site on screen: with its saved login, or a sign-in card for you (below) | allow; a saved login asks each time |
| `computer_handoff` | Asks you to take over for a step only you should do (below) | allow |

Each action returns a fresh screenshot. With the browser switched on (Settings → Browser), the browser tools drive the
same Chromium, so a bot can mix precise page tools with looking and clicking.

## Each bot's own sign-ins

Each bot has its own browser profile on the computer: its cookies, sign-ins and history. A site you sign in to for one
bot isn't signed in for the others. When another bot starts using the computer, Chromium closes and opens again with
that bot's profile (its tabs from before are gone; its sign-ins stay). **Sign out everywhere** on a bot's page
(under Computer) clears its profile.

Before profiles were per bot, all bots shared one. After the update, that one becomes the profile of the first bot to
use the computer; the others start signed out.

## Watching and taking over

**Computer** in the sidebar shows the screen live. Click it to take over: your clicks, typing and scrolling go to the
computer, and while you're in control bots wait. **Give back** hands it over. One user at a time: a bot that needs the
computer while another bot has it waits up to 30 seconds, then says it's busy. The browser tools count as using it
too. When you take over, the browser shows the profile of the bot that used it last (the one asking you, at a
handoff).

## Signing in from your phone

At a sign-in page, a bot asks Mimir to sign in (`computer_signin`). It never types a password itself, and never sees one.

- **A login it saved for this site before:** Mimir types it, after you OK it on a card (like every saved login), and
  types the authenticator code too if the login has a key.
- **Otherwise, a sign-in card:** where you talk to the bot (your messaging app, the chat in Mimir, and Needs you) you
  get *I need you to sign in to github.com*, with a link to a small form in Mimir: user name or email, password, a code
  if the site asks for one, and optionally the site's authenticator setup key. Never type them into the chat. Mimir
  types what you enter into the page on the computer and presses Enter, step by step for a site that asks the email
  first, then the password, then a code. Once it's signed in, Mimir keeps the login (and the key) for **that bot on
  that site** only: under Settings → Browser → Saved logins, as e.g. `GITHUB_COM_BOT3`.

The site is the page's registrable domain: a sign-in on `accounts.google.com` is kept for `google.com` and works on its
subdomains, while one on `alice.github.io` stays on `alice.github.io` (anyone can have a `github.io` site). Mimir types
only while the page on screen is on that site, checked right before each field, and only into the page itself, not a
frame embedded from elsewhere. It types each field once: when the password field comes back after a wrong password,
it stops instead of trying again (which could lock the account). The bot hears only *signed in* or *failed*.

The link is only `#signin` on the address you last opened the web app at: you sign in to Mimir as usual, and the form
is for the newest card waiting. The card works once (the first form sent uses it up) and runs out after 10 minutes;
**Can't now** turns it down. A routine doesn't send sign-in cards (nobody's watching it): it uses a saved login, or asks
you to take over.

**Authenticator codes.** Paste a site's setup key ("JBSW Y3DP …") or its `otpauth://totp/…` link (what the QR code
holds) on the form, or on a saved login under Settings → Browser (the pencil). Mimir works out the 6-digit codes
itself (RFC 6238) and types them only on that login's sites. With the browser tools, a bot types `NAME_CODE`
(e.g. `GITHUB_COM_BOT3_CODE`) for the current code. The key is kept in the secret store, like the password.

## When a bot needs you there

At a sign-in that `computer_signin` can't do, a verification code sent to you, a captcha or a payment, a bot doesn't
type passwords or card numbers or try to solve the captcha: it asks you to take over (`computer_handoff`). You get a card where you talk to it (your
messaging app, the chat in Mimir, and Needs you) saying what's needed, with a link straight to the live screen:
*Open the computer*. Take over, do the step, and press **Give back**; the bot sees you're done and carries on from
there. **Done** on the card says the same, and **Can't now** stops it, so it tells you what's left. It waits up to
10 minutes. A routine doesn't wait (nobody's watching it): it asks, ends that run, and the card stays in Needs you for
12 hours; when you've done the step and press **Done**, the routine runs again from the start, and the computer keeps
what you did (a sign-in stays signed in). The link uses the address you last opened the web app at (or connected the desktop app to).

## Teach by showing

Instead of explaining a task, do it once. On **Computer**, press **Start showing**, then do the task on the screen
(up to 10 minutes). Mimir writes down each step: the page you were on, what you clicked ("the *Book now* button"), what
you typed and which keys you pressed, with a screenshot at each click. Typing is kept only when it goes into a plain
text field on the page: what you type into a password field, a form embedded in the page (card details), the address
bar or a browser dialog is never kept. With several tabs open, each step is read from the tab on screen.
Pick which bot and chat the skill is for and press **Done, draft a skill**. The bot writes it up in the usual
[skill shape](skills.md), with what you typed turned into inputs. It arrives on the Skills tab as a proposal
you read and accept, like a skill taught from a link. **Discard** throws the recording away. Bots wait while you show.

How it reads the page: just before each click, Mimir asks Chromium (through its DevTools port) what's under the
pointer, and which field has focus when you type. The page's address is kept without its query string, which can hold
tokens. Up to 6 of the click screenshots go to the bot with the steps, with a red ring where you clicked, and are deleted
afterwards.

## Safety

- What's on the screen counts as **outside content**: after a bot has looked, anything that changes data asks you
  first ([Tools and safety](tools-and-safety.md#outside-content-taint)).
- Opening a site asks (per-site "always" is available); saved logins ask every time. A sign-in card's login is typed
  by Mimir from its own form, only on the card's site, and never passes through a chat or the AI. After outside content, jumping
  to the address bar (keys or a click on the browser's bar) asks too, so a page can't steer the bot to another site.
- With compose, the computer is a separate container: user `computer`, no `/data` mount, no published ports, all
  capabilities dropped. Mimir reaches it over the compose network only; the control channel and the DevTools port
  need `MIMIR_COMPUTER_TOKEN`, which both services get from `.env`.
- Its Chromium runs without any environment (not even that token) and with its own home, and the Docker image blocks
  `file://` and internal `chrome://` pages and downloads by policy.
- Never available in incognito chats (its browser keeps history and cookies).

## How it works

`mimir computer --listen 0.0.0.0:7300` (the compose service) starts a virtual screen (Xvfb, 1280×800) with a small
window manager (openbox) and a visible Chromium with its DevTools port on `127.0.0.1:9222`; the server asks it to
(re)start them when they're off. Mimir sends it requests on port 7300, one line of JSON each with the token
(`status`, `start`, `stop`, `screenshot`, `click`, `type`, `key`, `scroll`, `open`, `forget`); both sides check
coordinates, keys and addresses. `start`, `open` and `forget` carry the bot's id: the profiles
(`computer/chromium-<bot>`) live on the computer's side. Which bot's Chromium is on screen is kept in
`computer/browser` (bot and process id, checked against the running Chromium's command line), since a bot on a CLI
account runs its tools in a process of its own; a Chromium on the DevTools port that no record accounts for is left
alone and not driven. Chromium's DevTools are passed on at port 9223 for Playwright (`--cdp-endpoint`) and teach by showing's
page lookups, only to a connection whose first request carries `Authorization: Bearer <token>` (Playwright gets it as
`PLAYWRIGHT_MCP_CDP_HEADERS`); Mimir connects by IP, since Chromium only answers a Host that's an IP or localhost. Mimir uses the service
when `MIMIR_COMPUTER` (e.g. `computer:7300`) is set, and runs it in its own process otherwise.
On the computer's side, tools act through `xdotool` and take screenshots with `scrot`, as JPEG at screen size so
screenshot pixels are click coordinates. Images go to the model as
image content (MCP for the CLIs; image messages for API accounts, keeping only the latest two screenshots).
