"""mimir's Discord channel, end to end against scripts/mock-discord.py and fake-ai.py (issue #385):
dedupe of a replayed MESSAGE_CREATE, an approval card answered by a button tap, interactions answered within
3s, and a heartbeat during a long handler. Also flags two open bugs as KNOWN (not FAIL) so a fix later turns
them into real checks: #375 (a "yes" reply to a card denies it) and #378/#213 (inline handling blocks
interactions and heartbeats).

    CARGO_TARGET_DIR=/tmp/mimir-d53a cargo build -p mimir   # if the binary isn't already built
    python3 scripts/e2e-discord.py /tmp/e2e-discord
"""
import json, os, re, shutil, sqlite3, subprocess, sys, time, urllib.request

W = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
DIR = sys.argv[1] if len(sys.argv) > 1 else "/tmp/e2e-discord"
HOME, LOG, BIN, MOCK, AI = f"{DIR}/home", f"{DIR}/calls.jsonl", f"{DIR}/mimir", 18086, 18896
CHANNEL, USER = "200", "500"
subprocess.run(["rm", "-rf", HOME, LOG]); os.makedirs(HOME)
shutil.copy2("/tmp/mimir-d53a/debug/mimir", BIN); os.chmod(BIN, 0o755)
procs = [subprocess.Popen([sys.executable, f"{W}/scripts/mock-discord.py", str(MOCK), LOG]),
         subprocess.Popen([sys.executable, f"{W}/scripts/fake-ai.py", str(AI)], stdout=open(f"{DIR}/ai.log", "w"))]
time.sleep(1.5)
env = {**os.environ, "MIMIR_HOME": HOME, "MIMIR_DISCORD_API": f"http://127.0.0.1:{MOCK}", "MIMIR_DISCORD_GATEWAY": f"ws://127.0.0.1:{MOCK}/gateway",
       "DISCORD_BOT_TOKEN": "test", "MIMIR_PROVIDER": "openai", "OPENAI_BASE_URL": f"http://127.0.0.1:{AI}/v1", "OPENAI_API_KEY": "k", "MIMIR_MODEL": "fake"}
err = open(f"{DIR}/serve.log", "w")
procs.append(subprocess.Popen([BIN, "serve"], env=env, stderr=err, stdout=err))
ok = True
_mid = [3000]


def check(name, cond, detail=""):
    global ok
    ok &= bool(cond)
    print(("PASS " if cond else "FAIL ") + name + ("" if cond else f"  [{detail}]"), flush=True)


def known(issue, name, cond, detail=""):
    # An open bug (#issue): reported, not failed. When it passes, the bug is fixed: turn this into a check().
    print(("FIXED? " if cond else "KNOWN ") + f"#{issue} {name}" + ("" if cond else f"  [{detail}]"), flush=True)


def post(path, obj):
    urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{MOCK}{path}", json.dumps(obj).encode()))


def dispatch(t, d):
    post("/control/dispatch", {"t": t, "d": d})


def dm(content, mid=None, ref=None):
    if mid is None:
        _mid[0] += 1
        mid = str(_mid[0])
    d = {"id": mid, "content": content, "channel_id": CHANNEL, "author": {"id": USER, "username": "tester"},
         "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S+00:00", time.gmtime())}
    if ref:
        d["referenced_message"] = {"id": ref, "author": {"id": "1"}}
    dispatch("MESSAGE_CREATE", d)
    return mid


def press(custom_id):
    _mid[0] += 1
    dispatch("INTERACTION_CREATE", {"id": f"int{_mid[0]}", "token": f"tok{_mid[0]}", "channel_id": CHANNEL, "data": {"custom_id": custom_id}, "user": {"id": USER}})


def calls(since):
    return [c for c in map(json.loads, open(LOG)) if c["t"] >= since] if os.path.exists(LOG) else []


def sent(since):
    return [c for c in calls(since) if c["method"] == "post" and c.get("path", "").startswith(f"/channels/{CHANNEL}/messages")]


def texts(since):
    return [c["body"].get("content", "") for c in sent(since)]


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
    t = time.time(); dm(f"/pair {code}")
    check("paired", until(lambda: any("Paired" in x for x in texts(t)), 10), texts(t))

    # 1. The same MESSAGE_CREATE delivered twice (a reconnect's catch-up, or a gateway resend): answered once.
    # Ids must keep increasing (mark_seen rejects an id at or below one already seen, like a real snowflake would).
    _mid[0] += 1; dupe_id = str(_mid[0])
    t = time.time(); dm("hello dedupe", mid=dupe_id); dm("hello dedupe", mid=dupe_id)
    check("dedupe: answered once", until(lambda: any("OK: hello dedupe" in x for x in texts(t)), 10), texts(t))
    time.sleep(1)
    check("dedupe: not twice", sum("OK: hello dedupe" in x for x in texts(t)) == 1, texts(t))

    chat = q("SELECT chat_id FROM chats WHERE channel='discord' AND external=?", CHANNEL)[0][0]

    # 2. An approval card, answered by a button tap (INTERACTION_CREATE), then the reply goes on.
    rid1 = q("INSERT INTO routines(chat_id, cron, prompt, next_fire) VALUES (?, '0 8 * * *', 'brief me', ?) RETURNING id", chat, int(time.time()) + 86400)[0][0]
    t = time.time(); dm(f"DELETE ROUTINE {rid1}")
    a1 = until(lambda: q("SELECT id FROM approvals WHERE status='pending' AND chat_id=?", chat), 15)
    check("card posted", until(lambda: [c for c in sent(t) if c["body"].get("components")], 10), sent(t))
    check("approval pending", a1, q("SELECT * FROM approvals"))
    t2 = time.time(); press(f"ap:{a1[0][0]}:y")
    cb = until(lambda: [c for c in calls(t2) if c["method"] == "callback"], 10)
    check("interaction answered", cb and "Approved" in json.dumps(cb[0]["body"]), cb)
    check("interaction within 3s", cb and cb[0]["t"] - t2 < 3, cb)
    check("tool ran once approved", until(lambda: any("DONE routine_delete" in x for x in texts(t)), 15), texts(t))

    # 3 & 4. A busy handler: slow catch-up on reconnect blocks interactions and heartbeats inline (issue #378, #213).
    rid3 = q("INSERT INTO routines(chat_id, cron, prompt, next_fire) VALUES (?, '0 8 * * *', 'brief me', ?) RETURNING id", chat, int(time.time()) + 86400)[0][0]
    t = time.time(); dm(f"DELETE ROUTINE {rid3}")
    a3 = until(lambda: q("SELECT id FROM approvals WHERE status='pending' AND chat_id=?", chat), 15)
    check("second approval pending", a3, q("SELECT * FROM approvals"))

    post("/control/slow", {"ms": 4000})
    t_close = time.time(); post("/control/close", {})
    t_ident = until(lambda: [c for c in calls(t_close) if c["method"] == "identify"], 15)
    check("reconnected after the gateway dropped", t_ident, calls(t_close))
    t_catchup = until(lambda: [c for c in calls(t_ident[0]["t"]) if c["method"] == "catchup"], 10)
    check("catch-up ran on reconnect", t_catchup, calls(t_ident[0]["t"] if t_ident else t_close))

    t_dispatch = time.time(); press(f"ap:{a3[0][0]}:y")
    cb2 = until(lambda: [c for c in calls(t_dispatch) if c["method"] == "callback"], 10)
    latency = (cb2[0]["t"] - t_dispatch) if cb2 else None
    known(378, "an interaction is answered within 3s even while a handler runs inline", latency is not None and latency < 3, f"{latency}s" if latency else "no callback in 10s")

    window_end = cb2[0]["t"] if cb2 else time.time()
    hb = sorted(c["t"] for c in calls(t_close) if c["method"] == "heartbeat" and c["t"] <= window_end)
    edges = [t_ident[0]["t"]] + hb + [window_end] if t_ident else hb
    gap = max((b - a for a, b in zip(edges, edges[1:])), default=0)
    known(378, "no heartbeat gap much longer than the 1s interval during a long inline handler", gap < 2.0, f"gap {gap:.1f}s")

    # A half-open socket: heartbeats go unacknowledged, so the bot should reconnect.
    t = time.time(); post("/control/noack", {"on": True})
    known(213, "unacknowledged heartbeats (no op 11) make it reconnect", until(lambda: [c for c in calls(t) if c["method"] == "identify"], 6), "no new IDENTIFY in 6s")
    post("/control/close", {})
    # The second drop in two minutes: the reconnect backs off 10 s.
    check("reconnects once the socket drops", until(lambda: [c for c in calls(t) if c["method"] == "identify"], 30), [c["method"] for c in calls(t)])
    time.sleep(1)

    # Known bug #375: an explicit reply saying "yes" to a pending card must not deny it.
    rid4 = q("INSERT INTO routines(chat_id, cron, prompt, next_fire) VALUES (?, '0 8 * * *', 'brief me', ?) RETURNING id", chat, int(time.time()) + 86400)[0][0]
    t = time.time(); dm(f"DELETE ROUTINE {rid4}")
    card4 = until(lambda: [c for c in sent(t) if c["body"].get("components")], 15)
    a4 = until(lambda: q("SELECT id FROM approvals WHERE status='pending' AND chat_id=?", chat), 15)
    check("third approval card posted", card4 and a4, (card4, a4))
    dm("yes", ref=card4[0]["id"] if card4 else None)
    time.sleep(2)
    status4 = q("SELECT status FROM approvals WHERE id=?", a4[0][0])[0][0] if a4 else "?"
    known(375, "a 'yes' reply to a card doesn't deny it", bool(a4) and status4 != "denied", status4)
finally:
    for p in procs:
        p.kill()
print("ALL PASS" if ok else "SOME FAILED")
sys.exit(0 if ok else 1)
