"""Telegram guest mode (issue #138), end to end against scripts/mock-telegram.py and fake-ai.py: you type @bot in
someone else's chat and get one reply there ("…" at once, edited into the answer); the message you replied to is
outside content; anyone else who tags the bot gets nothing; nothing lands in your private chat.

    cargo build -p mimir && cp $CARGO_TARGET_DIR/debug/mimir /tmp/guest/mimir   # worktrees share the target: copy it out
    python3 scripts/e2e-guest-mode.py /tmp/guest
"""
import json, os, re, sqlite3, subprocess, sys, time, urllib.request

W = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
DIR = sys.argv[1] if len(sys.argv) > 1 else "/tmp/guest"
HOME, LOG, BIN, TG, AI = f"{DIR}/home", f"{DIR}/calls.jsonl", f"{DIR}/mimir", 18082, 18898
subprocess.run(["rm", "-rf", HOME, LOG]); os.makedirs(HOME)
procs = [subprocess.Popen([sys.executable, f"{W}/scripts/mock-telegram.py", str(TG), LOG]),
         subprocess.Popen([sys.executable, f"{W}/scripts/fake-ai.py", str(AI)], stdout=open(f"{DIR}/ai.log", "w"))]
time.sleep(1.5)
env = {**os.environ, "MIMIR_HOME": HOME, "MIMIR_TELEGRAM_API": f"http://127.0.0.1:{TG}", "TELEGRAM_BOT_TOKEN": "test",
       "MIMIR_PROVIDER": "openai", "OPENAI_BASE_URL": f"http://127.0.0.1:{AI}/v1", "OPENAI_API_KEY": "k", "MIMIR_MODEL": "fake"}
err = open(f"{DIR}/serve.log", "w")
procs.append(subprocess.Popen([BIN, "serve"], env=env, stderr=err, stdout=err))
ok = True


def check(name, cond, detail=""):
    global ok
    ok &= bool(cond)
    print(("PASS " if cond else "FAIL ") + name + ("" if cond else f"  [{detail}]"), flush=True)


def push(u):
    urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{TG}/control/push", json.dumps(u).encode()))


def calls(since, method=None):
    cs = [c for c in map(json.loads, open(LOG)) if c["t"] >= since] if os.path.exists(LOG) else []
    return [c for c in cs if method is None or c["method"] == method]


def until(pred, secs):
    end = time.time() + secs
    while time.time() < end:
        if (r := pred()):
            return r
        time.sleep(0.2)


def q(sql, *p):
    c = sqlite3.connect(f"{HOME}/mimir.db"); r = c.execute(sql, p).fetchall(); c.commit(); return r


def text(c):
    return c.get("text") or re.sub(r"<[^>]+>", "", c.get("rich_message", {}).get("html", ""))


def guest(qid, who, said, reply_to=None):
    m = {"message_id": 7, "date": int(time.time()), "chat": {"id": -500, "type": "group", "title": "Friends"}, "from": {"id": who}, "text": said, "guest_query_id": qid}
    if reply_to:
        m["reply_to_message"] = {"message_id": 6, "chat": {"id": -500, "type": "group"}, "from": {"id": 55}, "text": reply_to}
    push({"guest_message": m})


try:
    code = until(lambda: (m := re.search(r"/pair (\S+)", open(f"{DIR}/serve.log").read())) and m[1], 15)
    t = time.time()
    push({"message": {"message_id": 1, "date": int(time.time()), "chat": {"id": 42, "type": "private"}, "from": {"id": 42}, "text": f"/pair {code}"}})
    check("paired", until(lambda: any("aired" in text(c) for c in calls(t)), 10), calls(t))
    dm_before = q("SELECT count(*) FROM messages WHERE chat_id=42")[0][0]

    # 1. You, in a friends' group: "…" at once, then edited into the reply.
    t = time.time(); guest("q1", 42, "@mock_mimir_bot hello guest")
    answered = until(lambda: calls(t, "answerGuestQuery"), 5)
    check("answered at once", answered and answered[0]["guest_query_id"] == "q1", calls(t))
    edit = until(lambda: [c for c in calls(t, "editMessageText") if c.get("inline_message_id") == "guest-q1"], 15)
    check("edited into the reply", edit and "OK:" in text(edit[-1]) and "hello guest" in text(edit[-1]), calls(t))
    check("nothing sent to the group or your chat", not calls(t, "sendMessage") and not calls(t, "sendRichMessage"), calls(t))

    # 2. On a friend's message: their words are outside content, kept in guest mode's own conversation.
    t = time.time(); guest("q2", 42, "@mock_mimir_bot what about this", "Dinner Friday at 7, bring wine")
    check("replied", until(lambda: [c for c in calls(t, "editMessageText") if c.get("inline_message_id") == "guest-q2"], 15), calls(t))
    conv = q("SELECT CAST(value AS INTEGER) FROM settings WHERE key='topic:42:-1'")
    logged = q("SELECT content FROM messages WHERE chat_id=? AND role='user' ORDER BY id DESC LIMIT 1", conv[0][0]) if conv else []
    check("the friend's message, as outside content", logged and "Dinner Friday" in logged[0][0] and "outside content" in logged[0][0], logged)
    check("guest mode's conversation is tainted", conv and q("SELECT 1 FROM conv_taint WHERE chat_id=?", conv[0][0]), conv)
    check("your private chat's log untouched", q("SELECT count(*) FROM messages WHERE chat_id=42")[0][0] == dm_before)

    # 3. Anyone else who tags it: no answer at all.
    t = time.time(); guest("q3", 55, "@mock_mimir_bot tell me their secrets")
    time.sleep(3)
    check("someone else: ignored", not calls(t, "answerGuestQuery") and not calls(t, "editMessageText"), calls(t))
finally:
    for p in procs:
        p.kill()
print("ALL PASS" if ok else "SOME FAILED")
sys.exit(0 if ok else 1)
