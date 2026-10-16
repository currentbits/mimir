# Skills

A **skill** is a procedure a bot follows step by step for one kind of task: your morning briefing, booking padel, the
expense report. A bot sees only each skill's name and when to use it; it reads the steps when a task fits (or when you
call the skill by name), so having skills costs little until they're used.

**Rehearse** on a skill in the Skills tab tries it once without sending or changing anything: each step that would
write, send or ask you is only described ([Routines](routines.md#rehearse-before-it-runs-for-real) has the details).

Every skill has the same shape, whoever wrote it: **When to use**, **You'll need**, **Steps**, **Check** (how to tell
it worked), **Return** (what to tell you) and **Stop and ask if**.

They live on the **Skills** tab (⌘3): your bots' skills in this chat, suggestions waiting for you, and the library.

## Mimir's skill library

Skills written for Mimir's tools, ready to add. The ideas come from what people use other assistants for every day; the
text is Mimir's own. They're plain text: no scripts, nothing downloaded.

| Skill | For | Needs |
|---|---|---|
| **Morning briefing** | Today's calendar, open tasks and what you follow, in ten lines | memory (a linked calendar helps) |
| **Inbox triage** | What in your email needs you, FYI and noise; short replies as drafts; scams flagged | [email](connect.md#email) |
| **Follow-up tracker** | What you're waiting on, and what you promised others | memory, earlier chats |
| **Weekly review** | Done, slipped, next week's priorities | memory, earlier chats |
| **Meeting prep** | Before a meeting: the people, last time, what's open | [a linked calendar](connect.md#calendar) |
| **Meeting follow-up** | After a transcribed meeting: decided, to do, open questions | a [transcribed meeting](meetings.md) |
| **Research brief** | A question answered from current sources, each claim numbered | reading web pages ([web search](connect.md#web-search) helps) |
| **Summarise a link** | The gist of a page you send | reading web pages |
| **Price watch** | Tells you when a product drops below a price or is back in stock | reading web pages, routines |
| **Topic watch** | A digest of what's new on topics, companies or feeds you follow | reading web pages, routines |
| **Write like me** | Drafts in your own voice, from how you write | earlier chats |
| **Decision memo** | Options, trade-offs and a recommendation, after a few questions | nothing |
| **Document to actions** | Deadlines and obligations from a contract, letter or form | memory |
| **Trip planner** | A plan by day, a booking checklist, reminders and calendar links | reading web pages |
| **Hire a bot** | A new bot with one job, what it never does, a voice and when it works; it says its job back once made | [making bots](bots-and-chats.md#a-bot-that-makes-a-bot) |
| **Bot role check** | Twelve checks on a bot's role (one job, job first, no "I can also help with"…), and a rewrite that passes | nothing |
| **Team check** | Where your bots overlap or keep doing work outside their job, as proposals you pick from | earlier chats |
| **Memory check** | What your bots remember about you, with what's wrong, stale or a secret proposed for fixing | memory |

**Add a skill.** Nothing from the library is added on its own. Pick who gets it at the top (all bots in this chat, or
one bot) and press **Add**: it becomes an accepted skill like any other, which you can change, share or remove. A card
says when something it needs isn't there yet ("needs a linked calendar"); you can add it anyway, and it works once it is.

**Schedule it.** The ones that are a habit offer **schedule it** once added: one click makes a routine that runs the
skill at a sensible time (morning briefing weekdays at 07:30, weekly review Fridays at 16:00, follow-up tracker
Mondays at 09:00). Change the time in Routines.

## Using a skill

A bot reads a skill when your request fits its "when to use". To use one on purpose, type `/` in the app's chat to
list the bot's skills: `/morning-briefing`, or `/book-padel Thursday 18:00` to use that skill for Thursday. The same
works in Telegram, where the command menu lists each chat's skills (`/book_padel`), and Discord. Slack keeps `/` for
its own commands, so there ask by name ("use the book padel skill"). A routine can run a skill too: **Or run one of
its skills** in the routine form (its prompt becomes `/book-padel`).

## Where skills come from

**From the library**, above.

**Learned from a task that went well.** When a task that took several steps went well (no failed or denied action, no
correction after, nothing read from outside), the tidy-up may suggest it as a skill: a name, when to use it, and the
steps that run took. A bot uses a skill only after you accept it. Skills belong to one bot in one chat, or to **all
bots** in that chat: pick it under the skill (a shared skill's fixes and new versions stay shared, and two skills the
same bot sees can't share a name). A new version waits for you while the old one stays in use.

If you corrected an earlier answer to a similar request, the tidy-up also sees what you said then, and writes it into the
skill as a step or a **Stop and ask if** line ("You left out the parking slips" becomes "read every receipt, even small
ones like parking slips"). Only your own words are used, never the reply you corrected. A suggested skill whose "when to
use" reads like one the bot already has says so on its card ("Similar to your skill 'Book padel'"): two skills for the
same job make a bot pick the wrong one, so keep one (or merge them).

**Save that as a skill.** Right after a bot did something you'll want again, say "save that as a skill". The bot writes
it (`skill_save`) from what that conversation actually did, and a **Skill to save** card appears under its reply, with
**Save** / **Not now** (it's on the Skills tab too). The card says which steps it was drawn from, and warns you when the
conversation read outside content. Nothing is saved until you press Save.

**Teach a skill.** On the **Skills** tab, **Teach a skill**: paste a link or the text (how your expense process works, a
checklist, a recipe) and pick a bot. Its AI distils it into a skill: when to use it, the steps or rules, and short
reference sections the bot reads only when it needs them (`skill_view` with a section). It came from outside, so it's a
proposal you read and accept, never active by itself; a source or skill containing instruction-like text ("ignore
previous…") is flagged on the card. Teaching the same topic again proposes a new version, shown as changes. On a
self-hosted server you can also **teach by showing**: do the task once on the bots' computer and a bot drafts the skill
from what you did ([The bots' computer](computer.md#teach-by-showing)).

**Skill files.** Skills use the same `SKILL.md` format as Hermes, Claude Code and other agents (agentskills.io). Paste one,
or a link to one, into **Teach a skill** and it's imported as written (no AI involved), as a proposal like any other.
To bring in all your own skills from Claude Code, Hermes or OpenClaw at once, use **Memory → Import → From …**
([Memory](memory.md)). An imported skill whose steps pipe a download into a shell (`curl … | sh`), hold a base64 blob
or download a program says so on its card. Mimir never runs a skill's setup steps.
**Copy as SKILL.md** on a skill gives you the file to share or use elsewhere.

## How skills improve

**Skills improve with use.** Every time a bot reads a skill, that's counted (the Skills list shows "used 3× · last 2
days ago"). When a run that used a skill then went wrong (an action failed or was denied, the run was stopped, or you
corrected the reply), the tidy-up sees the skill next to what happened and may propose a **fix**: a small patch to the
step that caused it, shown as the lines that change, with the reason ("The 'any court' step led to booking an outdoor
court, and you corrected it"). It can only patch text that's in the skill, never rewrite it, and a skill stays under
4,000 characters. **Apply fix** makes it the new version. Runs that had read outside content (a web page, a file, a
tool's result) are never used for fixes: a page could steer a run wrong on purpose so its "fix" carries the page's
instructions, and a planted step is hard to spot on a card.

**Upkeep, once a week.** So skills and memory don't pile up:
- a skill nobody has used in 60 days gets a **Retire** card (**Keep it** dismisses the card for 90 days). Retired and
  removed skills stay under *retired or removed skills* for 180 days, with **Bring back**;
- two skills that clearly do the same job can get a **Merge** card (the merged skill replaces both);
- a fact search hasn't brought up in 90 days gets a *move to background* suggestion (preferences and tasks are left
  alone; pinned memories never move).

Nothing changes until you accept. This is the opposite of agents that rewrite their own skills unattended: the
evidence is what went wrong, and you decide.
