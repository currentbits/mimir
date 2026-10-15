"""A Telegram message to a bot that's busy (issue #12), end to end against scripts/mock-telegram.py and fake-ai.py:
a slow routine in the same chat (typing at once, one "Finishing a routine first…", then the reply), and a reply
waiting on an approval (another chat isn't held up). Then steering (issue #90): a message sent mid-reply changes the
reply in progress (the draft says it changed course, one reply takes both in), and /stop still cuts one off.

    cargo build -p mimir && cp $CARGO_TARGET_DIR/debug/mimir /tmp/busy/mimir   # worktrees share the target: copy it out
    python3 scripts/e2e-busy-chat.py /tmp/busy              # MOCK_NO_RICH=1: as on a Bot API before 10.1
"""
import json, os, re, sqlite3, subprocess, sys, time, urllib.request

W = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
DIR = sys.argv[1] if len(sys.argv) > 1 else "/tmp/busy"
HOME, LOG, BIN, TG, AI = f"{DIR}/home", f"{DIR}/calls.jsonl", f"{DIR}/mimir", 18081, 18899
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


def msg(text, chat=42):
    push({"message": {"message_id": int(time.time() * 1000) % 10**6, "date": int(time.time()), "chat": {"id": chat, "type": "private"}, "from": {"id": chat}, "text": text}})


def calls(since):
    return [c for c in map(json.loads, open(LOG)) if c["t"] >= since] if os.path.exists(LOG) else []


def until(pred, secs):
    end = time.time() + secs
    while time.time() < end:
        if (r := pred()):
            return r
        time.sleep(0.2)


def q(sql, *p):
    c = sqlite3.connect(f"{HOME}/mimir.db"); r = c.execute(sql, p).fetchall(); c.commit(); return r


def text(c):
    # A rich message's HTML, tags dropped.
    return c.get("text") or re.sub(r"<[^>]+>", "", c.get("rich_message", {}).get("html", "")).replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")


def texts(since, chat=42):
    # MOCK_NO_RICH=1 (an API before 10.1): the rich attempts it rejected aren't in the chat.
    sent = lambda c: not (os.environ.get("MOCK_NO_RICH") and "rich_message" in c)
    return [text(c) for c in calls(since) if c["method"] in ("sendMessage", "editMessageText", "sendMessageDraft", "sendRichMessage", "sendRichMessageDraft") and str(c.get("chat_id")) == str(chat) and sent(c)]


try:
    code = until(lambda: (m := re.search(r"/pair (\S+)", open(f"{DIR}/serve.log").read())) and m[1], 15)
    t = time.time(); msg(f"/pair {code}")
    check("paired", until(lambda: any("aired" in x for x in texts(t)), 10), texts(t))
    ai = lambda: open(f"{DIR}/ai.log").read()

    # 1. A slow routine in this very chat: typing at once, one note after a few seconds, then the reply.
    rid = q("INSERT INTO routines(chat_id, cron, prompt, next_fire) VALUES (42, '0 8 * * *', 'SLOW 12', ?) RETURNING id", int(time.time()) - 1)[0][0]
    check("routine running", until(lambda: "SLOW 12" in ai(), 45), ai())
    t = time.time(); msg("hello one")
    typing = until(lambda: [c for c in calls(t) if c["method"] == "sendChatAction"], 3)
    check("same chat: typing at once", typing and typing[0]["t"] - t < 2, calls(t))
    note = until(lambda: [c for c in calls(t) if "Finishing a routine first" in text(c)], 10)
    check("same chat: busy note after a few seconds", note and 3 < note[0]["t"] - t < 7, calls(t))
    check("same chat: no reply before the routine's", not any("OK: hello one" in x for x in texts(t)), texts(t))
    got = until(lambda: any("OK: hello one" in x for x in texts(t)), 20)
    check("same chat: reply once the routine is done", got, texts(t))
    check("same chat: note sent once", sum("Finishing" in x for x in texts(t)) == 1, texts(t))

    # 2. A reply in chat 42 waiting on an approval: chat 43 isn't held up.
    t = time.time(); msg(f"DELETE ROUTINE {rid}")
    a = until(lambda: q("SELECT id, tg_message_id FROM approvals WHERE status='pending' AND chat_id=42"), 15)
    check("chat 42 turn waits on approval", a, q("SELECT * FROM approvals"))
    code2 = re.findall(r"next pairing code is (\S+)", open(f"{DIR}/serve.log").read())[-1]
    t = time.time(); msg(f"/pair {code2}", 43)
    check("chat 43 paired", until(lambda: any("aired" in x for x in texts(t, 43)), 10), texts(t, 43))
    t = time.time(); msg("hello two", 43)
    check("other chat: reply while 42 waits on approval", until(lambda: any("OK: hello two" in x for x in texts(t, 43)), 10), texts(t, 43))
    check("other chat: no busy note", not any("Finishing" in x for x in texts(t, 43)), texts(t, 43))
    t = time.time()
    push({"callback_query": {"id": "cb1", "from": {"id": 42}, "data": f"ap:{a[0][0]}:y", "message": {"message_id": int(a[0][1] or 0), "chat": {"id": 42}}}})
    check("same chat: the approved one finishes", until(lambda: any("DONE routine_delete" in x for x in texts(t)), 20), texts(t))

    # 3. Steering: a message sent while the reply is being written changes it, instead of waiting its turn.
    time.sleep(1)
    t = time.time(); msg("SLOW 4 plan my month")
    check("slow reply started", until(lambda: "plan my month" in ai(), 10), ai())
    time.sleep(1); msg("actually, only this week")
    check("draft says it changed course", until(lambda: any("Changing course" in x for x in texts(t)), 6), texts(t))
    got = until(lambda: [x for x in texts(t) if x.startswith("OK:")], 20)
    check("one reply, taking both in", got and "only this week" in got[-1], texts(t))
    time.sleep(1)
    check("no second reply, no busy note", sum(x.startswith("OK:") for x in texts(t)) == 1 and not any("Finishing" in x for x in texts(t)), texts(t))
    said = [r[0] for r in q("SELECT content FROM messages WHERE chat_id=42 ORDER BY id DESC LIMIT 3")][::-1]
    check("both messages in the chat's log, then the reply", said[:2] == ["SLOW 4 plan my month", "actually, only this week"] and said[2].startswith("OK:"), said)

    # 4. Stop still works, steered or not: what was asked (both messages) is kept.
    t = time.time(); msg("SLOW 8 a long one")
    until(lambda: "a long one" in ai(), 10)
    time.sleep(0.5); msg("and another thing")
    time.sleep(1); msg("/stop")
    check("stopped", until(lambda: any("Stopped" in x for x in texts(t)), 8), texts(t))
    said = [r[0] for r in q("SELECT content FROM messages WHERE chat_id=42 ORDER BY id DESC LIMIT 3")][::-1]
    check("stop keeps both messages", said[:2] == ["SLOW 8 a long one", "and another thing"] and "Stopped" in said[2], said)
finally:
    for p in procs:
        p.kill()
print("ALL PASS" if ok else "SOME FAILED")
sys.exit(0 if ok else 1)
