# Memory

Your bots remember things about you so you don't repeat yourself: facts ("my dentist is Dr. Lee"), preferences ("when
giving news, use 3 bullets with links") and open tasks ("send Sam the deck by Friday"). Memory belongs to **you** and
is shared by all your bots. You can see, correct and remove every bit of it in the **Memory** tab.

## How bots use it

- **Standing memory**: your preferences and the most recent facts and tasks go into every reply's instructions, within
  a size budget (about 2,000 tokens). **Pin** a memory to keep it there however much else there is. The Memory tab shows
  how full it is ("Standing memory 67% full · 12 memories in every reply").
- **Search**: everything else is found with `memory_search` when a question needs it ("who's my dentist?").
- **Recall**: up to six memories that match your message come along with it. A CLI conversation (Claude, Codex,
  Grok) keeps what it was sent, so a memory recalled earlier in it isn't sent again.
- **Saving**: a bot saves with `memory_store` when you tell it something worth keeping. To correct a memory it saves a
  new version; the old one stays under *earlier version*.
- **End dates**: something true only for a while ("I'm in Lisbon until Friday") is saved with an end date. Through that
  day it's in standing memory; after it, it drops out, and a search still finds it marked *ended*. A new version keeps
  the end date. Change or remove it on the memory (*until …* · *no end*); the tidy-up may suggest forgetting ended ones.

- **Earlier conversations**: what you said but nobody saved ("which hotel did I book in Lisbon?") is found with
  `chat_search`, over all your past conversations with your bots, optionally between two dates ("what did I say about
  the hotel last week?"). Your own words count as yours; a bot reply that was
  written after reading a web page is marked as such, and finding one counts as reading outside content again.
  Incognito chats are never stored, so never found.

Three kinds: **preference** ("convention": a standing instruction), **fact** ("knowledge") and **task** (an open
commitment).

## You stay in charge

- **Preferences need your own words.** A bot saves a "When …, …" preference directly only when *your* message in that
  reply states the rule ("from now on…", "always…", "I prefer…") and it matches what's saved. A habit a bot works out
  on its own becomes a *suggestion* you accept or dismiss.
- **No secrets.** Memory refuses passwords and API keys; use saved logins for those
  ([Tools and safety](tools-and-safety.md#the-browser-and-saved-logins)).
- **Where it came from.** Mimir records who wrote each memory (you; which bot, in which run; a tidy-up; an import)
  and never trusts the model's own claim. A memory a bot saved after reading web pages or tool results is marked
  *after reading outside content*, and your bots treat it as unconfirmed, so a planted instruction doesn't read like
  something you said: it stays out of their standing memory (a search can still find it, labelled, and then that
  reply counts as having read outside content). The mark sticks: a new version of an unconfirmed memory, a bot's
  proposal made after reading a page, or a tidy-up suggestion drawn from such a conversation is flagged on its card,
  stays unconfirmed when you accept it (also through "Accept all"). *Looks right* clears the mark. The exception
  is a suggestion that says what you wrote yourself ("No, use metric units only" becoming a metric-units preference):
  a page can't put words in your messages, so it stays yours even if a bot searched the web in that conversation. Memories
  waiting for *Looks right* are listed under **Needs you**, so they don't sit unseen (and unused).
- **All at once.** With two or more suggestions, **Accept all** and **Dismiss all** answer them together; with two or
  more memories waiting for *Looks right*, **All look right** confirms them after you've read them.

## Files

Next to memories, each chat has a small **workspace** of text files your bots keep there with `file_write`: a packing
list, running notes, a draft, research too long for a memory. They're under **Memory → Files** (open, or delete); the
folder is `files/chat-<id>/` in Mimir's data folder. Rewriting a file keeps the previous version, deleting one asks,
and a file written after reading a web page stays marked as outside content when a bot reads it again. Like memory,
it belongs to the chat, so a group chat can't read your private chat's files.

## Changing and removing

- **Edit** a memory in place; the previous wording is kept under *earlier version*.
- **Move to background** takes it out of standing memory without deleting it (search still finds it).
- **Forget** hides it but keeps it under *History*; **Erase** (in History) deletes it and its earlier versions for good.
- **Import** takes text pasted from another assistant ("List everything you remember about me") and turns it into
  suggestions, skipping what you already have and anything that looks like a key or password.

## The daily tidy-up

Once a day, after a quiet half hour, your first bot (setting `memory_bot`) reviews memory against recent conversations
and **suggests** changes: remember a new fact or habit, update an outdated one, forget a finished task or a duplicate,
move something rarely relevant to the background. Suggestions appear at the top of the Memory tab with a reason;
nothing changes until you accept. *Tidy up now* runs it on demand; a dismissed suggestion isn't proposed again for 90
days, and neither are lookalikes: one like a suggestion you dismissed says so on its card ("You dismissed a similar
one on 3 Oct"), and after two similar dismissals that kind isn't shown at all (imports excepted). It runs on your first bot's AI account; **Settings → General → Model for the memory tidy-up** can point it at a
cheaper model (e.g. `haiku`, `gpt-5-nano`), which matters on pay-per-token keys. The setting belongs to the kind of
account the memory bot uses (a Claude model name means nothing to a local model); a failing tidy-up retries after an hour.

**It learns from corrections.** When you push back on a reply ("no, I said one sentence"), stop a reply, or deny an
action, that's noted. So is asking nearly the same thing again within ten minutes (in any language), a hint the reply
missed, though weaker: it never counts against a memory, and the tidy-up uses it only next to a correction or your own
words. Correcting a routine's result ("skip the crypto stuff") is noted with the routine it's about, so the suggestion is
a preference for that routine ("When doing the tech news briefing, …"), not for everything. Two of those bring the tidy-up forward (after 10 quiet minutes, at most hourly). It proposes a
"When …, …" preference from them only when the same correction happened twice or you stated the rule yourself, and it
quotes the evidence. Each reply remembers which memories it looked up; when you correct it, those memories count a
*corrected after use*, and so do standing preferences your correction is about (they shape every reply, so they're
never "looked up"). A memory that keeps steering replies wrong shows up with a proposed fix. Memories reach your bots
with the date they were saved, so an old fact can be weighed against a newer one.

**A correction carries over to similar requests.** When you ask something like a request you corrected before (most
of the same words), the bot sees what you asked then and what you said ("Too long. Three short bullet points, no
intro."), so it doesn't make the same mistake again, even before any preference is saved.

**New facts update old ones.** The tidy-up checks each memory saved since its last run against the rest. When a new
one makes an older one outdated, directly ("moved to Utrecht" vs "lives in Amsterdam") or as a knock-on ("broke my leg"
vs "cycles to work"), it suggests updating the older one or moving it to the background, naming the new memory. It
never suggests forgetting one for this, so the history stays.

**Check before accepting.** On a suggestion, *Check* replays your past corrections that look related, each once with
the change and once without, and a judge that isn't told which is which says whether each reply still has the problem
you corrected. It says, for example, "Fixes 2 of 3 past corrections about this; 1 was already fine without it", so a
change your AI no longer needs shows as such. It uses your AI account for a few short replies, so it runs only when you
press it.

Accepted preferences have **Check** too (on the memory). It replays on your memory bot's own model, not the cheaper
tidy-up model, so after switching models you can see whether a preference is still needed: one that fixes nothing the
new model doesn't already get right offers *Move to background*.

## Learned this week

The Memory tab lists what was saved without you typing it (by a bot, an accepted tidy-up, an import) since you last
looked, each with **Undo** (which also brings back the version it replaced), plus a trend line: corrections per 10
messages against the weeks before, corrections that **repeated** an earlier one (the same kind of request corrected
again: what learning should bring down), **"I told you"** corrections (something a bot should have remembered), denied
and stopped actions, failed actions, suggestions accepted, and how many learned memories you **took back** with Undo
(how often learning gets it wrong). Once a week at
your digest time, each Telegram, Slack or Discord chat gets the same list (a new chat's first one comes a week
after it starts); reply `/undo 2` to take one back.

A memory a bot saved shows the message of yours it came from ("You said: 'I might start going to the gym on
Mondays'"), so a passing remark saved as a firm fact ("goes to the gym every Monday") is easy to spot.

## Skills

Procedures your bots follow step by step (the ones they learn from tasks that went well, the ones you teach, and the
ones from Mimir's library) have their own page: [Skills](skills.md).
