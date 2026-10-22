---
name: Landing page
description: Use when the person wants a new, better-looking one-page site for a business, built from its current website. Not for editing a site that's already live.
---
**When to use:** "Make this site look modern", "build a landing page for my bakery from its website", "redo this page".

**You'll need:** The current site (web_fetch, the browser if it needs it), file_write to build the page in the workspace (site/<name>/index.html), send_file to hand it over.

**Steps:**
1. Read the site's home page and up to three more (services, about, contact). Note what the business does, who it's for, its colours and fonts, its tone, the main thing a visitor should do (call, book, order), and real proof: reviews, ratings, years, certifications, address, opening hours.
2. Use only what's on the site. Anything missing (a testimonial, a price) stays out or gets a clearly marked placeholder, never invented.
3. Plan the sections, each with a job: a hero that says what they do and for whom with the main button; a strip of proof; the problem they solve; services; reviews; how to reach them; a final call to action.
4. Write the copy from their own words, tightened. Keep their phone number, address and prices exactly.
5. Build one self-contained index.html: semantic HTML, CSS in the file, their colours, readable on a phone first, real links (tel:, mailto:, maps). Motion only small and calm (fade or slide as sections come into view), and none when the reader has reduced motion turned on.
6. Mark where images go. For each, write a short prompt for an image generator (subject, mood, their colours, aspect ratio, space left for text) and list them; use the site's own photos where they're good enough.

**Check:** Every fact on the page appears on their site; the page opens with no outside files except fonts; the main button works; text is readable on a narrow screen.

**Return:** The file (send_file), a five-line summary (sections, colours, the main action), the image prompts, and what's left for them: real photos, placeholders to fill, and where to host it.

**Stop and ask if:** The site isn't theirs or a client's (don't copy someone else's brand), the site can't be read, or they want it published: putting it online needs their hosting account, so explain the options and let them do it.
