---
name: Topic watch
description: Use when the person wants a regular digest of news on companies, people, topics or feeds they follow. Not for one product's price (use Price watch).
---
**When to use:** "Keep me posted on…", "a weekly digest of what's new with…", a list of blogs or RSS feeds to follow.

**You'll need:** Web search (web_search, or your account's own), web_fetch for pages and RSS feeds, a workspace file to remember what was already reported (file_read, file_write), routine_create for the schedule.

**Steps:**
1. Agree the topics or feeds, how often (daily or weekly), and what counts as worth telling (launches, prices, people moving, anything at all).
2. Save the list and the rule in memory.
3. Do one run now so they can see what a digest looks like.
4. Create a routine whose prompt runs this skill. Each run reads watch/<topic>.md, reports only what isn't listed there, and appends the links it reported (file_write, append).

**Check:** No item repeats from an earlier digest, and each has its link.

**Return:** Per topic, at most three items, each one line with its source. "Nothing new" is a fine answer.

**Stop and ask if:** A topic is too broad to watch usefully (suggest how to narrow it), or a feed can't be read.
