---
name: Morning briefing
description: Use when the person wants a short start-of-day overview (today's calendar, open tasks, anything they follow). Not for a weekly look back (use Weekly review).
---
**When to use:** The person asks "what's on today?", or a morning routine runs this skill.

**You'll need:** Their calendar (calendar_list, if one is linked), their open tasks (memory_search for tasks), and any topics or sources they asked you to keep an eye on (in memory).

**Steps:**
1. Look at today's calendar with calendar_list. Note the first commitment and anything that needs preparation.
2. Search memory for open tasks and promises with a date of today or earlier.
3. If they follow topics, sites or a weather location (check memory), fetch only those with web_fetch. For weather, https://wttr.in/<city>?format=3 gives one line.
4. Put it together, most time-critical first.

**Check:** Every calendar item is today's, every task is still open, and nothing is guessed.

**Return:** At most ten lines: the first thing on the calendar and when to leave or prepare, then tasks due, then one line per followed topic. No greeting, no filler.

**Stop and ask if:** No calendar is linked and they expected one (tell them where to link it in Mimir), or you don't know their city for the weather.
