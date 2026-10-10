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
| `computer_handoff` | Asks you to take over for a step only you should do (below) | allow |

Each action returns a fresh screenshot. With the browser switched on (Settings → Browser), the browser tools drive the
same Chromium, so a bot can mix precise page tools with looking and clicking.

## Watching and taking over

**Computer** in the sidebar shows the screen live. Click it to take over: your clicks, typing and scrolling go to the
computer, and while you're in control bots wait. **Give back** hands it over. One user at a time: a bot that needs the
computer while another bot has it waits up to 30 seconds, then says it's busy.

## When a bot needs you there

At a sign-in, a verification or 2FA code, a captcha or a payment, a bot doesn't type passwords or card numbers or try
to solve the captcha: it asks you to take over (`computer_handoff`). You get a card where you talk to it (your
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
- Opening a site asks (per-site "always" is available); saved logins ask every time. After outside content, jumping
  to the address bar (keys or a click on the browser's bar) asks too, so a page can't steer the bot to another site.
- With compose, the computer is a separate container: user `computer`, no `/data` mount, no published ports, all
  capabilities dropped. Mimir reaches it over the compose network only; the control channel needs
  `MIMIR_COMPUTER_TOKEN`, which both services get from `.env`.
- Its Chromium runs without any environment (not even that token) and with its own home, and the Docker image blocks
  `file://` and internal `chrome://` pages and downloads by policy.
- Never available in incognito chats (its browser keeps history and cookies).

## How it works

`mimir computer --listen 0.0.0.0:7300` (the compose service) starts a virtual screen (Xvfb, 1280×800) with a small
window manager (openbox) and a visible Chromium with its DevTools port on `127.0.0.1:9222`; the server asks it to
(re)start them when they're off. Mimir sends it requests on port 7300, one line of JSON each with the token
(`status`, `start`, `stop`, `screenshot`, `click`, `type`, `key`, `scroll`, `open`); both sides check coordinates, keys
and addresses. Chromium's DevTools are passed on at port 9223 for Playwright (`--cdp-endpoint`) and teach by showing's
page lookups; Mimir connects by IP, since Chromium only answers a Host that's an IP or localhost. Mimir uses the service
when `MIMIR_COMPUTER` (e.g. `computer:7300`) is set, and runs it in its own process otherwise.
On the computer's side, tools act through `xdotool` and take screenshots with `scrot`, as JPEG at screen size so
screenshot pixels are click coordinates. Images go to the model as
image content (MCP for the CLIs; image messages for API accounts, keeping only the latest two screenshots).
