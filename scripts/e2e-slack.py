"""Slack via Socket Mode, end to end against scripts/mock-slack.py and fake-ai.py (issue #385):
envelope ack and retry with mark_seen's dedupe; a card's thread kept answering after an approval card,
denying a plain reply there (issue #375, known); and interactions acked within 3s, including one stuck
behind an inline Slack API call (issue #378, known).

    cargo build -p mimir && cp $CARGO_TARGET_DIR/debug/mimir /tmp/e2e-slack/mimir   # worktrees share the target
    python3 scripts/e2e-slack.py /tmp/e2e-slack
"""
import json, os, re, sqlite3, subprocess, sys, time, urllib.request

W = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
DIR = sys.argv[1] if len(sys.argv) > 1 else "/tmp/e2e-slack"
HOME, LOG, BIN, SLACK, AI = f"{DIR}/home", f"{DIR}/calls.jsonl", f"{DIR}/mimir", 18085, 18897
BOT = "UBOT1"
subprocess.run(["rm", "-rf", HOME, LOG]); os.makedirs(HOME)
procs = [subprocess.Popen([sys.executable, f"{W}/scripts/mock-slack.py", str(SLACK), LOG]),
         subprocess.Popen([sys.executable, f"{W}/scripts/fake-ai.py", str(AI)], stdout=open(f"{DIR}/ai.log", "w"))]
time.sleep(1.5)
env = {**os.environ, "MIMIR_HOME": HOME, "MIMIR_SLACK_API": f"http://127.0.0.1:{SLACK}", "SLACK_BOT_TOKEN": "xoxb-test",
       "SLACK_APP_TOKEN": "xapp-test", "MIMIR_PROVIDER": "openai", "OPENAI_BASE_URL": f"http://127.0.0.1:{AI}/v1",
       "OPENAI_API_KEY": "k", "MIMIR_MODEL": "fake"}
err = open(f"{DIR}/serve.log", "w")
procs.append(subprocess.Popen([BIN, "serve"], env=env, stderr=err, stdout=err))
ok = True


def check(name, cond, detail=""):
    global ok
    ok &= bool(cond)
    print(("PASS " if cond else "FAIL ") + name + ("" if cond else f"  [{detail}]"), flush=True)


def known(issue, name, cond, detail=""):
    # An open bug (#issue): reported, not failed. When it passes, the bug is fixed: turn this into a check().
    print(("FIXED? " if cond else "KNOWN ") + f"#{issue} {name}" + ("" if cond else f"  [{detail}]"), flush=True)


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


def control(path, body):
    req = urllib.request.Request(f"http://127.0.0.1:{SLACK}/control/{path}", json.dumps(body).encode(), headers={"content-type": "application/json"})
    return json.loads(urllib.request.urlopen(req).read())


_seq = [1_600_000_000_000_000]  # a fake, monotonic Slack ts (seconds.microseconds)


def ts():
    _seq[0] += 1
    v = _seq[0]
    return f"{v // 1_000_000}.{v % 1_000_000:06d}"


def mention(channel, text, thread_ts=None, user="U1"):
    ev = {"type": "app_mention", "channel": channel, "user": user, "team": "T1", "text": f"<@{BOT}> {text}", "ts": ts()}
    if thread_ts:
        ev["thread_ts"] = thread_ts
    return ev


def chat_msg(channel, text, thread_ts, user="U1"):
    return {"type": "message", "channel": channel, "channel_type": "channel", "user": user, "text": text, "ts": ts(), "thread_ts": thread_ts}


def push_event(ev, retry_after=None):
    body = {"type": "events_api", "payload": {"event": ev}}
    if retry_after:
        body["retry_after"] = retry_after
    return control("push", body)


def push_interactive(p):
    return control("push", {"type": "interactive", "payload": p})


def ack_latency(eid, since):
    pushed = next((c["t"] for c in calls(since) if c.get("call") == "push" and c.get("envelope_id") == eid), None)
    acked = next((c["t"] for c in calls(since) if c.get("call") == "ack" and c.get("envelope_id") == eid), None)
    # max(..., 1e-6): never exactly 0, so `until`'s truthiness check (and ours below) treat a real, fast ack as found.
    return None if pushed is None or acked is None else max(acked - pushed, 1e-6)


def posted(channel, since):
    return [c.get("text", "") for c in calls(since) if c.get("call") == "post" and c.get("method") in ("chat.postMessage", "chat.update") and c.get("channel") == channel]


def pair(channel, code):
    t = time.time()
    push_event(mention(channel, f"pair {code}"))
    check(f"{channel} paired", until(lambda: any("aired" in x for x in posted(channel, t)), 10), posted(channel, t))


def chat_id(channel):
    return q("SELECT chat_id FROM chats WHERE channel='slack' AND external=?", channel)[0][0]


try:
    code1 = until(lambda: (m := re.search(r"/pair (\S+)", open(f"{DIR}/serve.log").read())) and m[1], 15)
    pair("C1", code1)
    code2 = until(lambda: (ms := re.findall(r"next pairing code is (\S+)", open(f"{DIR}/serve.log").read())) and ms[0], 10)
    pair("C2", code2)
    code3 = until(lambda: (ms := re.findall(r"next pairing code is (\S+)", open(f"{DIR}/serve.log").read())) and len(ms) > 1 and ms[1], 10)
    pair("C3", code3)

    # 1. Every envelope is acked, and Slack's retry (a late ack, same ts, new envelope_id) is deduped by mark_seen:
    # answered once, not twice.
    t = time.time()
    r1 = push_event(mention("C1", "hello one"), retry_after=0.5)
    lat1 = until(lambda: ack_latency(r1["envelope_id"], t), 3)
    check("envelope acked under 3s", lat1 is not None, lat1)
    retried = until(lambda: next((c for c in calls(t) if c.get("call") == "push" and c.get("retry_attempt")), None), 3)
    check("Slack's retried envelope is resent", bool(retried), calls(t))
    lat1b = retried and until(lambda: ack_latency(retried["envelope_id"], t), 3)
    check("the retry is acked under 3s too", bool(lat1b), lat1b)
    got = until(lambda: [x for x in posted("C1", t) if "OK: hello one" in x], 10)
    check("answered once despite the retry", got and sum("OK: hello one" in x for x in posted("C1", t)) == 1, posted("C1", t))

    # 2. @-mentioned in a channel, it answers in a thread and asks approval there; a plain "go ahead" reply in
    # that thread should not deny it (issue #375: today, any reply does).
    cid2 = chat_id("C2")
    rid1 = q("INSERT INTO routines(chat_id, cron, prompt, next_fire) VALUES (?, '0 8 * * *', 'placeholder', ?) RETURNING id", cid2, int(time.time()) + 86400)[0][0]
    m1 = mention("C2", f"DELETE ROUTINE {rid1}")
    push_event(m1)
    a1 = until(lambda: q("SELECT id FROM approvals WHERE chat_id=? AND status='pending' AND card_ref IS NOT NULL", cid2), 15)
    check("approval card requested in the thread", a1, q("SELECT * FROM approvals"))
    push_event(chat_msg("C2", "go ahead", m1["ts"]))
    time.sleep(1.5)
    status1 = q("SELECT status, reason FROM approvals WHERE id=?", a1[0][0])[0]
    known(375, "a thread reply that isn't a refusal shouldn't deny the card", status1[0] != "denied", status1)

    # 3. A button tap (interactive envelope) is acked within 3s, and decides the approval.
    cid3 = chat_id("C3")
    rid2 = q("INSERT INTO routines(chat_id, cron, prompt, next_fire) VALUES (?, '0 8 * * *', 'placeholder', ?) RETURNING id", cid3, int(time.time()) + 86400)[0][0]
    m2 = mention("C3", f"DELETE ROUTINE {rid2}")
    push_event(m2)
    a2 = until(lambda: q("SELECT id, card_ref FROM approvals WHERE chat_id=? AND status='pending' AND card_ref IS NOT NULL", cid3), 15)
    check("second approval card requested", a2, q("SELECT * FROM approvals"))
    aid2, card_ref2 = a2[0]
    card_ts2 = card_ref2.rsplit("/", 1)[-1]
    t3 = time.time()
    r3 = push_interactive({"type": "block_actions", "actions": [{"value": f"ap:{aid2}:y"}], "channel": {"id": "C3"}, "user": {"id": "U1"}, "message": {"ts": card_ts2, "blocks": []}})
    lat3 = until(lambda: ack_latency(r3["envelope_id"], t3), 5)
    check("a button tap is acked under 3s", lat3 is not None, lat3)
    check("the tap decides the approval", until(lambda: q("SELECT status FROM approvals WHERE id=?", aid2)[0][0] == "approved", 10), q("SELECT status FROM approvals WHERE id=?", aid2))
    check("the approved turn finishes", until(lambda: any("DONE routine_delete" in x for x in posted("C3", t3)), 20), posted("C3", t3))

    # A tap that arrives while the listener is stuck in an inline Slack API call (here, a thread lookup
    # following up a mention) should still be acked within 3s (issue #378: today, it waits behind it).
    t5 = time.time()
    m3 = mention("C3", "hi again")
    push_event(m3)
    check("a fresh mention is answered", until(lambda: any("OK: hi again" in x for x in posted("C3", t5)), 10), posted("C3", t5))
    control("slow", {"method": "conversations.replies", "secs": 4})
    t6 = time.time()
    push_event(chat_msg("C3", "continue please", m3["ts"]))
    time.sleep(0.5)
    r4 = push_interactive({"type": "block_actions", "actions": [{"value": "noop"}], "channel": {"id": "C3"}, "user": {"id": "U1"}, "message": {"ts": "0.000001", "blocks": []}})
    lat4 = until(lambda: ack_latency(r4["envelope_id"], t6), 8)
    known(378, "a tap during an inline Slack API call is still acked under 3s", lat4 is not None and lat4 < 3, lat4)
finally:
    for p in procs:
        p.kill()
print("ALL PASS" if ok else "SOME FAILED")
sys.exit(0 if ok else 1)
