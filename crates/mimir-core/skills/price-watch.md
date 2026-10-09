---
name: Price watch
description: Use when the person wants to know when a product's price drops or it's back in stock. Not for news about a topic (use Topic watch).
---
**When to use:** "Tell me when this drops below €200", "let me know when it's back in stock".

**You'll need:** The product page (web_fetch, or the browser if the page needs it), a quiet routine (routine_create with quiet=true), memory_store for the last price seen.

**Steps:**
1. Pin down the exact item: model, size or variant, and the shop. Read the page once now and note today's full price (with shipping, if shown) and stock.
2. Agree the condition: a target price, any drop, or back in stock.
3. Save the item, the condition and today's price in memory.
4. Create a quiet routine (daily is usually enough) whose prompt says: check this page, compare with the saved price, and only speak up if the condition is met.

**Check:** The first read worked (you saw a price) before scheduling anything; the routine runs at most daily unless they ask for more.

**Return:** Today's price and stock, the condition, and when it will check. When it fires: the new price, the link, and how it compares.

**Stop and ask if:** The page doesn't show a price you can read, several variants match, or the shop needs a login.
