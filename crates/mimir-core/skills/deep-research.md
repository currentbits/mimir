---
name: Deep research
description: Use when a question needs a thorough report from many sources (a market, a technology, a big purchase or a due-diligence check), worth half an hour of reading. Not for a quick answer with sources (use Research brief).
---
**When to use:** "Do a deep dive on…", "full report on…", "everything about…", or a question with several parts that each need their own digging.

**You'll need:** web_search and web_fetch to read sources, file_write to keep notes in the workspace (research/<topic>/), progress_set to show the plan as you go, send_file for the report.

**Steps:**
1. Restate the question and the decision it serves in two lines. Split it into 3 to 6 parts that don't overlap (for a market: players, technology, prices, regulation, where it's heading). Show them with progress_set.
2. For each part on its own: search, read the best 4 to 8 sources (primary first: official sites, filings, papers, data; write-ups only for context), and write notes to research/<topic>/<part>.md: each finding with its source link and the date you read it. Tick the part off.
3. Label every finding: [verified] (a primary source you opened), [single source], [estimate], or [disputed] (sources disagree; keep both sides). A source you couldn't open is a gap, never guessed at.
4. Read all the notes together. Write down what the parts say together, where they contradict each other, and what's still unknown.
5. Review your own draft as a sceptic: which claims rest on one weak source, what's out of date, what a critic would ask. Go back for the two or three gaps that matter most.
6. Write research/<topic>/report.md: a summary of five to ten lines that answers the question, then a section per part, then open questions, then all sources numbered.

**Check:** Every claim in the summary traces to a labelled finding in the notes; nothing comes from memory without being marked; the sources are the pages you actually read.

**Return:** The summary in your reply, the report as a file (send_file), and one line on what you couldn't find out.

**Stop and ask if:** The question is too broad to split into a handful of parts (ask which matters most), a part needs paid or private data, or it's turning into far more reading than they'd want: say so with what you have, and ask before going on.
