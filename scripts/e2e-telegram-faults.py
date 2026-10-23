"""Telegram when it misbehaves (issue #385), end to end: `mimir server` with two bots on scripts/mock-telegram.py and
fake-ai.py. Pairing across bots, a 429 with retry_after, "file is too big" from getFile, the owner's photo in a group,
draft edit pacing in a group, a failure on part 2 of a split reply, a restart mid-batch (unconfirmed updates replayed),
and an approval card whose send fails.

A KNOWN line is an open bug, checked for the right behaviour but not failing the run; FIXED? means it now passes, so
turn it into a check().

    cargo build -p mimir && cp $CARGO_TARGET_DIR/debug/mimir /tmp/tgfaults/mimir   # worktrees share the target: copy it out
    python3 scripts/e2e-telegram-faults.py /tmp/tgfaults
"""
import json, os, re, sqlite3, subprocess, sys, time, urllib.error, urllib.request

W = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
DIR = sys.argv[1] if len(sys.argv) > 1 else "/tmp/tgfaults"
HOME, LOG, BIN, TG, AI, PORT = f"{DIR}/home", f"{DIR}/calls.jsonl", f"{DIR}/mimir", 18084, 18898, 18791
TOKEN, GROUP = "e2e-admin-token-0123456789abcdef", -100777
subprocess.run(["rm", "-rf", HOME, LOG]); os.makedirs(HOME)
procs = [subprocess.Popen([sys.executable, f"{W}/scripts/mock-telegram.py", str(TG), LOG]),
         subprocess.Popen([sys.executable, f"{W}/scripts/fake-ai.py", str(AI)], stdout=open(f"{DIR}/ai.log", "w"))]
env = {**os.environ, "MIMIR_HOME": HOME, "MIMIR_ADMIN_TOKEN": TOKEN, "MIMIR_TELEGRAM_API": f"http://127.0.0.1:{TG}"}
err = open(f"{DIR}/server.log", "a")
server = lambda: subprocess.Popen([BIN, "server", "--listen", f"127.0.0.1:{PORT}"], env=env, stderr=err, stdout=err)
procs.append(server())
ok = True


def check(name, cond, detail=""):
    global ok
    ok &= bool(cond)
    print(("PASS " if cond else "FAIL ") + name + ("" if cond else f"  [{detail}]"), flush=True)


def known(issue, name, cond, detail=""):
    # An open bug (#issue): reported, not failed. When it passes, the bug is fixed: turn this into a check().
    print(("FIXED? " if cond else "KNOWN ") + f"#{issue} {name}" + ("" if cond else f"  [{detail}]"), flush=True)


def until(pred, secs):
    end = time.time() + secs
    while time.time() < end:
        if (r := pred()):
            return r
        time.sleep(0.2)


def post(url, body, headers={}):
    try:
        with urllib.request.urlopen(urllib.request.Request(url, json.dumps(body).encode(), {"content-type": "application/json", **headers})) as r:
            return r.status, r.read().decode()
    except (urllib.error.URLError, ConnectionError) as e:
        return getattr(e, "code", 0), ""


def api(cmd, args={}):
    return json.loads(post(f"http://127.0.0.1:{PORT}/api/{cmd}", args, {"authorization": f"Bearer {TOKEN}"})[1] or "null")


def up():
    try:
        return urllib.request.urlopen(f"http://127.0.0.1:{PORT}/healthz").status == 200
    except OSError:
        return False


def push(u):
    post(f"http://127.0.0.1:{TG}/control/push", u)


def fail(**rule):
    post(f"http://127.0.0.1:{TG}/control/fail", rule)


def update(chat=42, bot="test", group=False, **m):
    c = {"id": chat, "type": "supergroup", "title": "Team"} if group else {"id": chat, "type": "private"}
    return {"bot": bot, "message": {"message_id": int(time.time() * 1000) % 10**6, "date": int(time.time()), "chat": c, "from": {"id": 42}, **m}}


def msg(text, **kw):
    push(update(text=text, **kw))


def calls(since):
    return [c for c in map(json.loads, open(LOG)) if c["t"] >= since] if os.path.exists(LOG) else []


def text(c):
    return c.get("text") or re.sub(r"<[^>]+>", "", c.get("rich_message", {}).get("html", ""))


def sent(since, chat=42, bot="test"):
    # What reached the chat: messages and edits Telegram accepted.
    return [c for c in calls(since) if c["method"] in ("sendMessage", "editMessageText", "sendRichMessage") and str(c.get("chat_id")) == str(chat) and c["bot"] == bot and "failed" not in c]


def texts(since, **kw):
    return [text(c) for c in sent(since, **kw)]


def code(bot_id):
    return until(lambda: next((s["pair_code"] for s in api("statuses") or [] if s.get("bot_id") == bot_id and s.get("pair_code")), None), 15)


try:
    until(up, 15)
    fake = {"persona": "", "provider": "openai", "model": "fake", "base_url": f"http://127.0.0.1:{AI}/v1", "tools": "", "look": ""}
    api("onboarding_done")
    api("set_api_key", {"kind": "openai", "key": "test-key"})
    api("save_bot", {"bot": {"id": 1, "name": "Ada", **fake}, "secrets": {"telegram_token": "test"}})
    bea = api("save_bot", {"bot": {"id": 0, "name": "Bea", **fake}, "secrets": {"telegram_token": "beta"}})
    t = time.time(); msg(f"/pair {code(1)}")
    check("paired with Ada", until(lambda: any("aired" in x for x in texts(t)), 15), texts(t))

    # Pairing is per person today (#382): the chat paired with Ada reaches Bea too. Pinned until that's decided.
    t = time.time(); msg("hello bea", bot="beta")
    check("a chat paired with bot A reaches bot B (per person, #382)", until(lambda: any("OK: hello bea" in x for x in texts(t, bot="beta")), 15), texts(t, bot="beta"))

    # A 429 on the reply: Telegram says when to try again.
    fail(method="sendRichMessage", match="hello limit", code=429, description="Too Many Requests: retry after 2", retry_after=2)
    t = time.time(); msg("hello limit")
    got = until(lambda: [c for c in sent(t) if "OK: hello limit" in text(c)], 15)
    check("a reply after a 429 arrives", got, calls(t))
    hit = [c for c in calls(t) if c.get("failed", "").startswith("Too Many")]
    known(379, "a 429 waits retry_after before trying again", got and hit and got[0]["t"] >= hit[0]["t"] + 1.8, [(c["method"], c["t"]) for c in calls(t)])
    known(380, "…and doesn't fall back to HTML on it (Telegram may have taken it)", got and "rich_message" in got[0], got and got[0]["method"])

    # Files Telegram won't hand over: an image sent as a 25 MB file, and one whose size it didn't say.
    t = time.time(); push(update(document={"file_id": "bigpic", "file_name": "scan.png", "mime_type": "image/png", "file_size": 25_000_000}))
    check("a file over 20 MB gets the 'over 20 MB' note", until(lambda: any("20 MB" in x for x in texts(t)), 10), texts(t))
    check("…without trying to download it", not [c for c in calls(t) if c["method"] == "getFile"], calls(t))
    fail(method="getFile", match="nosize", description="Bad Request: file is too big")
    t = time.time(); push(update(document={"file_id": "nosize", "file_name": "scan.png", "mime_type": "image/png"}))
    check("getFile was asked", until(lambda: [c for c in calls(t) if c["method"] == "getFile"], 10), calls(t))
    known(384, "(left over after #384 closed) getFile's 'file is too big' gets the 'over 20 MB' note too", until(lambda: any("20 MB" in x for x in texts(t)), 8), texts(t))

    # A group: paired by the owner, who sends a photo there.
    t = time.time(); msg(f"/pair {code(1)}", chat=GROUP, group=True)
    check("group paired", until(lambda: any("aired" in x for x in texts(t, chat=GROUP)), 15), texts(t, chat=GROUP))
    t = time.time(); push(update(chat=GROUP, group=True, caption="@mock_mimir_bot what's this?", photo=[{"file_id": "grouppic", "width": 90, "height": 90}]))
    check("the owner's photo in a group is fetched", until(lambda: [c for c in calls(t) if c.get("file_id") == "grouppic"], 8), calls(t))

    # A reply streamed into a group: the draft is edited while it's written.
    t = time.time(); msg("@mock_mimir_bot TRICKLE 10", chat=GROUP, group=True)
    until(lambda: [c for c in sent(t, chat=GROUP) if "word" in text(c) and "▍" not in text(c)], 30)
    edits = [c["t"] for c in sent(t, chat=GROUP) if c["method"] == "editMessageText" and "▍" in text(c)]
    gaps = [round(b - a, 1) for a, b in zip(edits, edits[1:])]
    check("group draft edited as it's written", len(edits) >= 2, calls(t))
    known(379, "group drafts edit at most every 3 s (about 20 a minute)", gaps and min(gaps) >= 2.9, gaps)

    # A reply in parts: part 2 fails (both as HTML and as plain text).
    fail(method="sendRichMessage", match="Paragraph", description="Bad Request: can't parse rich message")
    fail(method="sendMessage", match="Paragraph", skip=1, times=2, code=502, description="Bad Gateway")
    t_long = time.time(); msg("LONG 6000")
    check("part 2 was tried and failed", until(lambda: len([c for c in calls(t_long) if c.get("failed") == "Bad Gateway"]) == 2, 15), calls(t_long))

    # A restart mid-batch: a message answered, then a photo whose download hangs, and the app quits meanwhile.
    fail(method="getFile", match="slowpic", delay=8)
    t = time.time(); push([update(text="hello replay"), update(photo=[{"file_id": "slowpic", "width": 90, "height": 90}])])
    check("answered before the restart", until(lambda: any("OK: hello replay" in x for x in texts(t)), 10), texts(t))
    check("the photo's download hangs", until(lambda: [c for c in calls(t) if c["method"] == "getFile"], 5), calls(t))
    procs[-1].kill(); procs[-1].wait(); procs[-1] = server()
    until(up, 15)
    t2 = time.time(); msg("hello again")
    check("answers after the restart", until(lambda: any("OK: hello again" in x for x in texts(t2)), 20), texts(t2))
    time.sleep(3)
    known(380, "a restart doesn't answer the last batch again", sum("OK: hello replay" in x for x in texts(t)) == 1, f'{sum("OK: hello replay" in x for x in texts(t))} replies')
    firsts = [x for x in texts(t_long) if "Paragraph 1:" in x]
    check("the split reply was resent after the restart", any("Paragraph 15:" in x for x in texts(t_long)), texts(t_long))
    known(380, "a part-way failure resends only the parts not sent", len(firsts) == 1, f"part 1 sent {len(firsts)} times")

    # An approval card whose send fails: it's sent again.
    db = sqlite3.connect(f"{HOME}/mimir.db")
    rid = db.execute("INSERT INTO routines(chat_id, cron, prompt, next_fire) VALUES (42, '0 8 * * *', 'brief me', ?) RETURNING id", (int(time.time()) + 86400,)).fetchall()[0][0]
    db.commit()
    fail(method="sendMessage", match="Approve this action?", code=502, description="Bad Gateway")
    t = time.time(); msg(f"DELETE ROUTINE {rid}")
    check("the card's send failed", until(lambda: [c for c in calls(t) if "Approve this action?" in text(c) and c.get("failed")], 15), [c["method"] for c in calls(t)])
    known(377, "a card whose send fails is sent again", until(lambda: [c for c in sent(t) if "Approve this action?" in text(c)], 15), [(c["method"], c.get("failed")) for c in calls(t)])
finally:
    for p in procs:
        p.kill()
print("ALL PASS" if ok else "SOME FAILED")
sys.exit(0 if ok else 1)
