"""A bot on Signal, end to end against scripts/mock-signal.py and fake-ai.py: pairing, a reply with typing first, a
question's choices as a numbered list answered by number, an approval card answered by number, a file sent to the
bot and one sent back.

    cargo build -p mimir && cp $CARGO_TARGET_DIR/debug/mimir /tmp/signal/mimir   # worktrees share the target: copy it out
    python3 scripts/e2e-signal.py /tmp/signal
"""
import json, os, re, sqlite3, subprocess, sys, time, urllib.request

W = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
DIR = sys.argv[1] if len(sys.argv) > 1 else "/tmp/signal"
HOME, LOG, BIN, SIG, AI = f"{DIR}/home", f"{DIR}/calls.jsonl", f"{DIR}/mimir", 18082, 18898
subprocess.run(["rm", "-rf", HOME, LOG]); os.makedirs(HOME)
procs = [subprocess.Popen([sys.executable, f"{W}/scripts/mock-signal.py", str(SIG), LOG]),
         subprocess.Popen([sys.executable, f"{W}/scripts/fake-ai.py", str(AI)], stdout=open(f"{DIR}/ai.log", "w"))]
time.sleep(1.5)
env = {**os.environ, "MIMIR_HOME": HOME, "MIMIR_SIGNAL_API": f"http://127.0.0.1:{SIG}", "SIGNAL_NUMBER": "+15550000",
       "MIMIR_PROVIDER": "openai", "OPENAI_BASE_URL": f"http://127.0.0.1:{AI}/v1", "OPENAI_API_KEY": "k", "MIMIR_MODEL": "fake"}
err = open(f"{DIR}/serve.log", "w")
procs.append(subprocess.Popen([BIN, "serve"], env=env, stderr=err, stdout=err))
ok = True


def check(name, cond, detail=""):
    global ok
    ok &= bool(cond)
    print(("PASS " if cond else "FAIL ") + name + ("" if cond else f"  [{detail}]"), flush=True)


def msg(text, attachments=None):
    d = {"message": text, "timestamp": int(time.time() * 1000)}
    if attachments:
        d["attachments"] = attachments
    e = {"source": "+3161", "sourceNumber": "+3161", "sourceUuid": "u-ana", "sourceName": "Ana", "timestamp": d["timestamp"], "dataMessage": d}
    urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{SIG}/control/push", json.dumps(e).encode()))


def calls(since, method="send"):
    return [c for c in map(json.loads, open(LOG)) if c["t"] >= since and c["method"] == method] if os.path.exists(LOG) else []


def texts(since):
    return [c["message"] for c in calls(since) if c["recipients"] == ["u-ana"]]


def until(pred, secs):
    end = time.time() + secs
    while time.time() < end:
        if (r := pred()):
            return r
        time.sleep(0.2)


def q(sql, *p):
    c = sqlite3.connect(f"{HOME}/mimir.db"); r = c.execute(sql, p).fetchall(); c.commit(); return r


try:
    code = until(lambda: (m := re.search(r"/pair (\S+)", open(f"{DIR}/serve.log").read())) and m[1], 15)
    check("connected", until(lambda: "signal connected as +15550000" in open(f"{DIR}/serve.log").read(), 15), open(f"{DIR}/serve.log").read())
    t = time.time(); msg(f"pair {code}")
    check("paired", until(lambda: any("Paired" in x for x in texts(t)), 10), texts(t))

    t = time.time(); msg("hello there")
    check("typing first", until(lambda: calls(t, "typing"), 5), calls(t, "typing"))
    check("reply", until(lambda: any(x == "OK: hello there" for x in texts(t)), 15), texts(t))
    check("the chat is named for the person", q("SELECT title FROM chats WHERE external='u-ana'") == [("Signal Ana",)], q("SELECT title FROM chats"))

    # A question's choices: numbered, and a number answers it.
    t = time.time(); msg("CHOOSE a day")
    asked = until(lambda: [x for x in texts(t) if "1. Thursday\n2. Friday\n3. Something else" in x], 15)
    check("choices numbered", asked, texts(t))
    t = time.time(); msg("2")
    check("a number picks", until(lambda: "You chose: Friday" in texts(t), 10), texts(t))
    check("…and is answered like the choice", until(lambda: any(x.startswith("OK:") and "Friday" in x for x in texts(t)), 15), texts(t))
    t = time.time(); msg("2")
    check("used up: a later number is just a message", until(lambda: "OK: 2" in texts(t), 15), texts(t))

    # An approval card: numbered too.
    chat = q("SELECT chat_id FROM chats WHERE external='u-ana'")[0][0]
    rid = q("INSERT INTO routines(chat_id, cron, prompt, next_fire) VALUES (?, '0 8 * * *', 'brief me', ?) RETURNING id", chat, int(time.time()) + 86400)[0][0]
    t = time.time(); msg(f"DELETE ROUTINE {rid}")
    card = until(lambda: [x for x in texts(t) if "Approve this action?" in x], 15)
    check("card with numbers", card and "1. Approve\n2. Deny" in card[0], texts(t))
    t = time.time(); msg("1")
    check("approved by number", until(lambda: any(x.startswith("✅ Approved") for x in texts(t)), 10), texts(t))
    check("the reply goes on", until(lambda: any("DONE routine_delete" in x for x in texts(t)), 15), texts(t))

    # Files both ways.
    t = time.time(); msg("what's in it?", [{"contentType": "text/plain", "filename": "note.txt", "id": "Dentist-is-on-Tuesday-at-nine", "size": 29}])
    check("a file is read", until(lambda: any(x.startswith("OK:") for x in texts(t)), 15), texts(t))
    check("…and its text reaches the AI", "Dentist is on Tuesday at nine" in open(f"{DIR}/ai.log").read(), "")
    t = time.time(); msg("FILE")
    # After reading a file the chat has read outside content, so writing one asks first.
    until(lambda: any("Approve this action?" in x for x in texts(t)), 15)
    t = time.time(); msg("1")
    check("written", until(lambda: any("DONE file_write" in x for x in texts(t)), 15), texts(t))
    t = time.time(); msg("SEND FILE")
    sent = until(lambda: [a for c in calls(t) for a in c["base64_attachments"]], 15)
    check("a file sent back", sent and sent[0]["filename"] == "trip.md" and sent[0]["size"] > 0, calls(t))
finally:
    for p in procs:
        p.kill()
print("ALL PASS" if ok else "SOME FAILED")
sys.exit(0 if ok else 1)
