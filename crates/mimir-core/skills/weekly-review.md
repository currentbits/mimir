---
name: Weekly review
description: Use once a week to look back (done, slipped) and plan the next week. Not for a single day's overview (use Morning briefing).
---
**When to use:** "Let's do my weekly review", or a Friday or Sunday routine runs it.

**You'll need:** Tasks in memory (memory_search), this week's conversations (chat_search), meetings (meeting_search), next week's calendar (calendar_list, if linked).

**Steps:**
1. Gather what got done this week (tasks closed, things reported done in chats, meetings held).
2. Gather what slipped: open tasks past their date, promises not kept.
3. Look at next week's calendar for busy days and deadlines.
4. Suggest up to three priorities for next week, and anything to drop or postpone.

**Check:** You only suggest; nothing is changed, scheduled or forgotten without the person saying so.

**Return:** Done, Slipped, Next week (calendar highlights), Priorities (three at most). Under 20 lines. If they want it as a PDF (to keep or share), write the same as markdown with file_write (reviews/<date>.md) and send it with send_file, format pdf, instead of pasting it.

**Stop and ask if:** They want tasks closed, moved or forgotten: confirm each one.
