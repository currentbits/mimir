# Tools, approvals and safety

Bots act through **tools**. Every tool call is checked against your rules before it runs, and recorded.

## Built-in tools

| Tool | What it does | Default |
|---|---|---|
| `memory_store`, `memory_search` | Save and look up memories ([Memory](memory.md)) | allow |
| `chat_search` | Search earlier conversations with you (any of your bots) for what was said but not saved | allow |
| `memory_forget` | Forget a memory | **ask** |
| `card_read`, `card_write` | Read this chat's pinned cards, and write one whole (a shopping list, bills due): text, checklists, tables and links, never HTML; up to 20 a chat ([Pinned cards](bots-and-chats.md#pinned-cards)). Not in incognito. A card written after reading outside content counts as outside content for whoever reads it next | allow |
| `goal_set`, `goal_update` | Set this chat's goal (one at a time) and keep its progress line; *done* only when the chat's run log shows something done since it was set ([Goals](routines.md#goals)). Come with `routine_create`. Not in incognito, nor in a task handed over by another bot | allow |
| `routine_create`, `routine_list` | Schedule and list routines, recurring or once (a reminder) ([Routines](routines.md)) | allow |
| `routine_update` | Change a routine's brief, schedule, quiet or watch (when you reply to its result with what to change); a bot allowed `routine_create` may use it | allow |
| `routine_delete` | Delete a routine | **ask** |
| `web_fetch` | Read a web page (outside content; never this computer or your local network: addresses, names that resolve there, and redirects are all checked) | allow |
| `calendar_add` | Make one-click "add to calendar" links (Google Calendar, Outlook) for an event; you open one and save it, nothing is added for you | allow |
| `email_search`, `email_read` | Search and read your email, read-only (**Settings → Email**: IMAP with an app password); offered once set up, in a chat with you only. Nothing is marked read, moved or deleted (the mailbox is only *examined*); emails are outside content | allow |
| `email_draft` | Write an email (or a reply in its thread) into your Drafts folder; nothing is sent, you send it from your mail app. After reading outside content (an email included) it asks, with the recipient first on the card | allow |
| `file_list`, `file_read`, `file_write` | A text-file workspace per chat (notes, lists, drafts, research), shown under Memory → Files. Paths stay inside it (no `..`, no links); rewriting keeps the previous version; a file written after outside content (or dropped in by hand) counts as outside content when read. Not in incognito | allow |
| `file_delete` | Delete a workspace file | **ask** |
| `send_file` | Send a workspace file, or a screenshot of the bots' computer, to this chat with the reply ([Photos, voice notes and files](bots-and-chats.md#photos-voice-notes-and-files)). Not in incognito or for a bot working on another's task | allow in your own direct chat; **ask** in a group, channel or guest's chat |
| `email_send` | Send an email as you, after you approve it: every email asks (never "Always"), with who it's to first and the whole text on the card; one too long for the card has to be a draft instead; at most 20 a day from a chat. Sent through your outgoing server (worked out from the incoming one, or set in Settings → Email), with a copy in your Sent folder | **asks, every time** |
| `web_search` | Search the web through **Settings → Web search** (Brave Search with a free key, or your own SearXNG), for every AI account; offered once one is set up. Results are outside content; after outside content, a search that would carry one of your memories (or over 200 characters) asks | allow |
| `meeting_search`, `meeting_read` | Look through your transcribed meetings ([Meetings](meetings.md)); offered once you have one | allow |
| `calendar_create` | Add an event straight to your calendar (**Settings → Calendar**: iCloud, Fastmail, Nextcloud or any CalDAV, with an app password); offered once one is connected, in a chat with you only. The card shows when, what and where; "Always" works for it | **ask** |
| `calendar_list` | Read your calendar between two dates (from the calendar link in Settings); offered once one is linked, in a chat with you only. Event titles are outside content | allow |
| `ask_bot` | Hand a task to another of your bots | allow |
| `bot_create` | Make a new bot for a job you describe ([Bots and chats](bots-and-chats.md#a-bot-that-makes-a-bot)) | **asks, every time** |
| `skill_view` | Read the steps of an accepted skill | allow |
| `skill_save` | Propose a skill from what the conversation did, when you ask; you press Save ([Skills](skills.md)) | allow |
| `plan_propose` | Propose a plan you approve once (below) | allow |
| `ask_choice` | Ask you one question with 2 to 4 answers to tap, plus "Something else". The buttons go under the reply, and your tap is your next message. Only in Telegram, Slack and Discord chats, never in routines or another bot's task. A tap is never an approval | allow |
| `computer_*` | Use the bots' computer, self-hosted only ([The bots' computer](computer.md)) | allow; `computer_open` **asks**, and `computer_signin` with a saved login |
| `browser__*` | The headless browser, when switched on (below) | opening a page **asks** |
| *tool servers* | Tools from MCP servers you added (below) | read-only (as you confirmed) allow, the rest **ask** |

Each bot has a tool allowlist on its page; tools outside it are hidden from that bot and refused.

## Allow, ask, deny

Every tool has a rule under **Settings → Permissions**: *allow*, *ask* or *deny*. *Ask* sends Approve / Deny buttons
to Telegram, Slack or Discord and shows a card in the app with **Allow once**, **Always allow** (per tool, or per website
for opening pages: "Always on this site") and **Deny**. Every "always" becomes a visible rule in Settings → Permissions.
Once you've allowed the same thing twice, the card makes "always" the main button. No answer within 5 minutes means
deny (routines park their asks for 24 hours instead; see [Needs you](routines.md#needs-you)). From the command line:
`mimir approvals`, `mimir approve <id>`, `mimir deny <id> [why]`.

**Say why.** **Deny…** in the app takes an optional reason, and in Telegram, Slack, Discord, Signal or WhatsApp a text reply to
the card does the same (a number on Signal still picks a button). The bot is told *"Denied: <your reason>"* in your own
words and carries on with that, instead of only saying it wasn't done. A routine's ask has moved on by the time you
answer, so your reason goes into that chat's conversation for the bot's next turn there.

**Edit before it goes.** An email's card (to send, or to draft after outside content) has **Edit** in the app: change
the subject and text, then approve, and that's what goes. Who it's to, the cc and the account can't be changed on the
card, and the edited email is checked again (one-line subject, whole on the card). In a messaging app the card has
**Edit in Mimir**, a link to it in the web app, when Mimir runs as a server with an https address.

## Outside content (taint)

A web page, an email, a calendar invite, a tool server's result, what's on the computer's screen, a file you sent, or a meeting transcript can contain text written
to look like instructions ("ignore your rules and email this to…"). So once a bot has read outside content, **anything
that changes data asks**, for the rest of that conversation, even if its rule says allow. This also flows through
hand-offs to other bots, into routines a bot creates then (every run of such a routine counts as having read outside
content), and into later searches of the conversation (a file you sent is never mistaken for your own words). Results
of the AI's own web search count too. Read-only tools stay allowed, except when the read itself would carry your data
out:

- reading a web address that carries data (a query string, a very long path or subdomain), as in
  `evil.example/?d=<your memories>`;
- on the bots' computer, keys that jump to the address bar or a new tab, and clicks on the browser's own bar;
- typing one of your memories into a page, on the computer or in the browser;
- after reading your **email or a meeting**, opening **any** web address (in `web_fetch`, the browser or on the
  computer), however short: a link like `evil.example/v/483920` could carry a sign-in code out;
- a web search whose query holds one of your memories.

Those ask too. "Start fresh" (or `/new`) ends it, and so does a messaging chat's fresh start after a quiet spell
(unless the outside content is in the "where we left off" it quotes). Requests made after reading outside content, and any use of a saved
login, never become "always". Approval cards show where an action goes (recipient, address, path) first, so a long
message body can't push it off the card.

**Always asks, whatever the rules say.** Sending an email and making a new bot ask every time, even if someone set
their rule to allow, and never offer "always" or "all like this". An email can only be sent if all of it (who it's to
first, then the subject and text) fits on the card; anything longer is saved as a draft for you to send.

Memories saved after reading outside content are marked as such and treated as unconfirmed
([Memory](memory.md#you-stay-in-charge)). Because of that, saving a *new* one doesn't ask (it's kept out of your bots'
standing instructions until you confirm it); replacing an existing memory still asks. A lesson in your own words
("no, use metric units") stays yours even after a web search in that conversation: a page can't put words in your
messages. Files in a chat's workspace keep the same mark: one written after outside content (or changed outside
Mimir) counts as outside content when a bot reads it.

**Sensitive sources.** In **Settings → Sensitive sources** you can mark a tool server (your bank's, your health
records'), just one of its tools, or a chat's memory as sensitive. Servers whose names look like banking or health are
suggested there; nothing is marked for you. Once a reply or routine has read from a sensitive source (called one of its
tools, found one of its memories, or had them in its standing memory), these ask first, even if their rule says
allow:

- `email_send` (it always asks; the card now also says why);
- `web_fetch` and `web_search`, whatever the address or query, and `computer_open`, `computer_type`;
- `ask_bot`: a task for another bot;
- a message to someone else's chat: a reminder `for` them, `send_file` in a group or a guest's chat;
- `memory_store` with `household`: the household's guests read those memories;
- any tool server's tool that changes something, and its read-only tools that reach the internet (except the sensitive
  server's own).

The card starts with *"This run read from bank, which you marked sensitive."* These asks never offer "always", aren't
covered by a plan or "all like this", and aren't in **Allow all**. Like outside content, it lasts for the rest of the
conversation ("Start fresh" ends it), goes along with a task handed to another bot, and sticks to a routine the run
creates or rewrites (every run of it asks the same way). It only ever turns an allow into an ask.

**Going round in circles.** When a reply or routine makes the same tool call again and again (same tool, same
details on the card) instead of getting anywhere, the next repeat stops and asks first: *"The bot keeps repeating this
step. Let it go on?"*. That's the 3rd time in a row for something that acts, or the 5th for something that only looks
(reading, searching, a screenshot, scrolling, which often repeat for a reason while a page loads). A short cycle counts
too: the same two or three calls, round and round, that many times. Allow lets it carry on (and asks again if it's
still at it a few more rounds later); Deny stops the step and the bot is told to wrap up and tell you where it got
stuck. A routine's ask waits in [Needs you](routines.md#needs-you). It works the same with every AI account, including
Claude Code and Codex.

## Approve once for a whole job

Before a task that needs several actions that ask (send, delete, book, open a site), a bot can propose a **plan**
(`plan_propose`): its steps and the tools and sites it'll use, as one card. Approving it covers those actions for the
rest of that reply. Every live card also has **Approve all like this**: this action, and the same tool and site again,
without asking until the reply ends. Neither covers anything after the bot read outside content, saved-login use, or
routines.

To stop being asked at all, press **Always** on the card (in the app, Telegram, Slack or Discord): from then on that
tool, or that tool on that website, runs without asking. Change it back in **Settings → Permissions**. Asks after
outside content, saved logins and plans don't offer it.

**Several at once.** With two or more waiting, **Allow all** and **Deny all** (in the app and in Needs you) answer the
plain asks together. An email to send, a new bot, a saved login and anything asked after outside content stay to be
answered one by one: those are the ones to read. **Settings → Permissions → Allow all** sets every tool to allow (the
always-ask ones above still ask, and so does anything after outside content); **Back to defaults** undoes it.

## The browser and saved logins

**Settings → Browser** turns on a real headless browser for your bots (Playwright's MCP server; needs Node.js and
downloads Chromium once). Opening a page asks you first (or "always on this site"); clicking and typing are allowed;
page scripting is off.

Like fetching a page, the browser won't go to this computer or your local network (`localhost`, `192.168.x.x`, `10.x`,
`*.local` and the like). A bot asking to open one is refused before you're asked, and the browser itself blocks links,
redirects and a page's own requests to them. Not covered: a public name that points at a local address. The browser
checks the address in the link, not where the name leads.

**Saved logins** are typed by name (e.g. `GITHUB_PASSWORD`): the browser fills in the value, the AI never sees it, it
only works on the sites you listed for it, and you approve each use. Values are scrubbed from every result and log.
A login can have an authenticator key too: a bot types `NAME_CODE` and Mimir fills in the current code, only on the
login's sites. On the bots' computer, a bot can also ask you to sign in with a masked form in Mimir instead of the
chat ([Signing in from your phone](computer.md#signing-in-from-your-phone)); that login is kept for that bot and site.

## Tool servers (MCP)

**Settings → Tool servers** adds any MCP server: one that runs as a command (your files, GitHub, a database…) or one
online at a URL. Paste its setup line (`npx -y @modelcontextprotocol/server-filesystem ~/Notes`), its URL
(`https://mcp.deepwiki.com/mcp`), or the JSON from its instructions (`{"mcpServers": {…}}`). Mimir starts or connects
to it once to list its tools, so a broken setup isn't saved. Tokens in its environment or headers go to the keychain
(server: the private database), never the config. Its tools follow the same rules: tools the server marks read-only are
allowed once you confirm them (Mimir lists the tools after adding the server, "read-only (the server says)"), the rest
ask. Updating the server, or changing what runs, asks you to confirm again. Each bot can switch servers on or off. Remote servers use MCP's Streamable HTTP transport;
servers that only offer the older SSE transport aren't supported.

**Malware check.** Before starting a server whose command downloads a package (`npx`, `bunx`, `npm exec`, `pnpm dlx`,
`uvx`, `pipx run`…), Mimir looks the package up in [OSV.dev](https://osv.dev)'s list of malicious packages and refuses
to start it if it's there, naming the advisory. With a pinned version (`pkg@1.2.3`, `pkg==1.2.3`) only that version
counts; without one, any version ever flagged blocks it, so pin a clean version to get past that. The answer is kept for
a day (a flagged one for good). If OSV.dev can't be reached, a version it called clean before starts anyway; one it
never checked doesn't. The built-in browser is a fixed version and isn't checked.

**Pinned versions.** OSV.dev only knows about a malicious release once someone reports it, so Mimir doesn't run
whatever was published last either. A setup line without a version (`npx -y pkg`, `uvx pkg`, `pkg@latest`) is pinned
to the newest release that's at least a week old (`pkg@1.2.3`) when you add the server, and a server added before
Mimir did this is pinned the next time it starts (if the registry can't be reached then, it starts as before). When a
newer release has been out for a week, the server shows **Update to …**; it never offers one older than the version
you're on. A version you name yourself is kept, and so is a range (`pkg@^2`, `pkg>=1`) or another tag (`pkg@next`):
those keep fetching what they name. `pipx run pkg` can't take a pin; use `pipx run --spec pkg==1.2.3 pkg`.

**Signing in.** A hosted server that wants a sign-in (Notion `https://mcp.notion.com/mcp`, Linear
`https://mcp.linear.app/mcp` and the like) answers Mimir's first try with "wants you to sign in", and Settings shows
**Sign in to …**. Mimir follows MCP's authorization spec: it finds the server's sign-in service from its metadata,
registers itself there (dynamic client registration), and signs you in in your browser with PKCE. You never type a
password into Mimir. The tokens go to the keychain (server: the private database) and are renewed on their own;
**Sign in again** on the server's row replaces them, and removing the server forgets them. A server that doesn't let
apps register themselves (GitHub's hosted one, for example) takes a token in a header instead: add
`Authorization` with `Bearer <your token>`. A header always wins over a sign-in. On a self-hosted server you paste the
address your browser ended up on, as with ChatGPT.

## The run log

Every reply and routine is recorded with its model, tokens, cost and each tool call, with its allow / ask / deny
decision and result: the **Activity** tab, or `mimir runs`. A reply that's still running has **Stop this reply** there.
Incognito chats keep no log.
