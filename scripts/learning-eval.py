"""Does learning work end to end? A simulated person with three hidden preferences talks to a real bot for three
"days"; they correct every reply that breaks one, and between days the tidy-up runs and they accept the suggestions
that are about their preferences (dismissing the rest). Fewer corrections on later days = the loop works.

  python3 scripts/learning-eval.py <path to the mimir binary> [model, default haiku] [--control]

Each day starts a fresh conversation (like /new), so only what Mimir learned carries over. --control learns nothing
(no tidy-up, past corrections wiped each day): the same questions' baseline, to compare with.

Uses a throwaway MIMIR_HOME and your signed-in `claude` CLI (about 25 short replies and 3 tidy-ups).
"""
import json, os, re, sqlite3, subprocess, sys, tempfile, time, urllib.request

CONTROL = "--control" in sys.argv
# --web: day 1 starts with a question that needs a web search, so the conversation has read outside content (what
# learning must survive: lessons in the person's own words stay theirs).
WEB = "--web" in sys.argv
argv = [a for a in sys.argv if a not in ("--control", "--web")]
BIN, MODEL = argv[1], (argv[2] if len(argv) > 2 else "haiku")
HOME = tempfile.mkdtemp(prefix="mimir-eval-")
ENV = {**os.environ, "MIMIR_HOME": HOME, "MIMIR_MODEL": MODEL, "MIMIR_ADMIN_TOKEN": "eval-token-0123456789abcdef"}
PORT = 8791

# (name, broken-by check on a reply, what they say, words that mark a suggestion as being about it)
PREFS = [
    ("metric", lambda r: re.search(r"\b(feet|foot|ft|miles?|mph|fahrenheit|°f|pounds|lbs?|inch(es)?)\b", r, re.I),
     "No, use metric units only: no feet, miles or Fahrenheit.", r"metric|celsius|kilomet|unit"),
    ("short", lambda r: len(r.split()) > 70, "No, too long. Keep answers under 50 words.", r"short|brief|concise|words|length|long"),
    ("prose", lambda r: re.search(r"^\s*([-*•]|\d+[.)])\s", r, re.M), "No, don't use bullet points or lists, just plain sentences.", r"bullet|list|prose|sentence"),
]
DAYS = [
    ["How tall is Mount Everest and how cold is it at the top?", "What are the main benefits of cycling to work?", "How far is the Moon from Earth?", "Any tips for packing a carry-on bag?"],
    ["How deep is the Mariana Trench and what's the pressure down there?", "What should I know before adopting a dog?", "How long is the Great Wall of China?", "How can I sleep better?"],
    ["How hot does the surface of Venus get?", "What are good habits for working from home?", "How big is Texas compared to France?", "How do I start running as a beginner?"],
]


def chat(text):
    out = subprocess.run([BIN, "chat", text], env=ENV, capture_output=True, text=True, timeout=300)
    return out.stdout.strip()


def api(cmd, **args):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}/api/{cmd}", data=json.dumps(args).encode(), method="POST",
                                 headers={"Authorization": f"Bearer {ENV['MIMIR_ADMIN_TOKEN']}", "content-type": "application/json"})
    return json.load(urllib.request.urlopen(req, timeout=600))


def main():
    chat("Hi!")  # creates the database and the bot
    db = sqlite3.connect(os.path.join(HOME, "mimir.db"))
    db.execute("UPDATE bots SET model=? WHERE id=1", (MODEL,))
    db.commit()
    server = subprocess.Popen([BIN, "server", "--listen", f"127.0.0.1:{PORT}"], env=ENV, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(3)
    try:
        results = []
        for day, asks in enumerate(DAYS, 1):
            # A new day, a new conversation: only memory (and stored corrections) carry over.
            db.execute("DELETE FROM sessions WHERE bot_id=1 AND chat_id=0")
            if CONTROL:
                db.execute("DELETE FROM cases")
            db.commit()
            broken = {p[0]: 0 for p in PREFS}
            if WEB and day == 1:
                chat("Search the web: what is today's top news headline? One line.")
                print("web search first; runs marked as outside content:", db.execute("SELECT count(*) FROM runs WHERE tainted=1").fetchone()[0])
            for ask in asks:
                reply = chat(ask)
                bad = [p for p in PREFS if p[1](reply)]
                for p in bad:
                    broken[p[0]] += 1
                print(f"day {day} | {ask[:48]:48} | {len(reply.split()):3} words | broke: {', '.join(p[0] for p in bad) or '-'}")
                if bad:
                    chat(bad[0][2])  # one correction per reply, like a person would
            results.append(broken)
            if day < len(DAYS) and not CONTROL:
                api("tidy", chat_id=0)
                for s in api("suggestions", chat_id=0):
                    text = f"{s['title']} {s['body']}"
                    about = any(re.search(p[3], text, re.I) for p in PREFS) and s["action"] in ("add", "update")
                    api("decide_suggestion", chat_id=0, id=s["id"], accept=about)
                    print(f"  {'accepted ' if about else 'dismissed'} [{s['action']}] {s['title']}: {s['body'][:80]}")
        print(f"\n{'CONTROL (nothing learned)' if CONTROL else 'LEARNING'}, {MODEL}: replies breaking each preference, per day (4 replies a day):")
        for day, broken in enumerate(results, 1):
            print(f"  day {day}: " + ", ".join(f"{k} {v}" for k, v in broken.items()) + f"  (total {sum(broken.values())})")
        standing = db.execute("SELECT title, body FROM entries WHERE type='convention' AND invalid=0 AND superseded_by IS NULL").fetchall()
        print("preferences in memory:", standing)
        print("unconfirmed (out of the bots' instructions):", db.execute("SELECT count(*) FROM entries WHERE tainted=1 AND invalid=0").fetchone()[0])
    finally:
        server.terminate()


main()
