"""A bot on WhatsApp, end to end: `mimir server` with a fake Graph API (in this script) and fake-ai.py, messages sent to
its /whatsapp webhook signed as Meta signs them. Meta's webhook check, an unsigned call refused, pairing, typing, a
reply, a question's buttons tapped, an approval card, a forward, a file sent to the bot and one sent back.

    cargo build -p mimir && cp $CARGO_TARGET_DIR/debug/mimir /tmp/whatsapp/mimir   # worktrees share the target: copy it out
    python3 scripts/e2e-whatsapp.py /tmp/whatsapp
"""
import hashlib, hmac, json, os, re, sqlite3, subprocess, sys, threading, time, urllib.error, urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

W = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
DIR = sys.argv[1] if len(sys.argv) > 1 else "/tmp/whatsapp"
HOME, BIN, GRAPH, AI, PORT = f"{DIR}/home", f"{DIR}/mimir", 18083, 18899, 18790
TOKEN, SECRET, ANA = "e2e-admin-token-0123456789abcdef", "app-secret", "3161"
subprocess.run(["rm", "-rf", HOME]); os.makedirs(HOME)
CALLS, ids = [], iter(range(1, 10**6))


class Graph(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def reply(self, body, ctype="application/json"):
        b = body if isinstance(body, bytes) else json.dumps(body).encode()
        self.send_response(200); self.send_header("content-type", ctype); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)

    def do_GET(self):
        if self.path.startswith("/v23.0/100?"):
            return self.reply({"id": "100", "display_phone_number": "+1 555-0000"})
        if m := re.fullmatch(r"/v23.0/(\d+)", self.path):
            return self.reply({"url": f"http://127.0.0.1:{GRAPH}/file/{m[1]}"})
        if self.path.startswith("/file/"):
            return self.reply(b"Dentist is on Tuesday at nine", "text/plain")
        self.send_response(404); self.end_headers()

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        if self.path == "/v23.0/100/media":
            CALLS.append({"t": time.time(), "media": len(body), "name": b'filename="trip.md"' in body})
            return self.reply({"id": "9001"})
        c = json.loads(body); c["t"] = time.time(); CALLS.append(c)
        self.reply({"messaging_product": "whatsapp", "messages": [{"id": f"wamid.out{next(ids)}"}]})


threading.Thread(target=ThreadingHTTPServer(("127.0.0.1", GRAPH), Graph).serve_forever, daemon=True).start()
procs = [subprocess.Popen([sys.executable, f"{W}/scripts/fake-ai.py", str(AI)], stdout=open(f"{DIR}/ai.log", "w"))]
env = {**os.environ, "MIMIR_HOME": HOME, "MIMIR_ADMIN_TOKEN": TOKEN, "MIMIR_WHATSAPP_API": f"http://127.0.0.1:{GRAPH}/v23.0"}
err = open(f"{DIR}/server.log", "w")
procs.append(subprocess.Popen([BIN, "server", "--listen", f"127.0.0.1:{PORT}"], env=env, stderr=err, stdout=err))
ok = True


def check(name, cond, detail=""):
    global ok
    ok &= bool(cond)
    print(("PASS " if cond else "FAIL ") + name + ("" if cond else f"  [{detail}]"), flush=True)


def until(pred, secs):
    end = time.time() + secs
    while time.time() < end:
        if (r := pred()):
            return r
        time.sleep(0.2)


def http(path, body=None, headers={}):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", body, {"content-type": "application/json", **headers})
    try:
        with urllib.request.urlopen(req) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def api(cmd, args={}):
    return json.loads(http(f"/api/{cmd}", json.dumps(args).encode(), {"authorization": f"Bearer {TOKEN}"})[1])


def hook(message, sign=SECRET):
    message = {"from": ANA, "id": f"wamid.in{next(ids)}", "timestamp": str(int(time.time())), **message}
    value = {"messaging_product": "whatsapp", "metadata": {"phone_number_id": "100"}, "contacts": [{"wa_id": ANA, "profile": {"name": "Ana"}}], "messages": [message]}
    body = json.dumps({"object": "whatsapp_business_account", "entry": [{"id": "1", "changes": [{"field": "messages", "value": value}]}]}).encode()
    sig = "sha256=" + hmac.new(sign.encode(), body, hashlib.sha256).hexdigest()
    return http("/whatsapp", body, {"x-hub-signature-256": sig})[0]


def say(text, **extra):
    return hook({"type": "text", "text": {"body": text}, **extra})


def tap(data):
    return hook({"type": "interactive", "interactive": {"type": "button_reply", "button_reply": {"id": data, "title": "x"}}})


def sent(since):
    return [c for c in CALLS if c["t"] >= since and c.get("to") == ANA]


def texts(since):
    return [c["text"]["body"] if c["type"] == "text" else c["interactive"]["body"]["text"] for c in sent(since) if c.get("type") in ("text", "interactive")]


try:
    until(lambda: http("/healthz")[0] == 200 if os.path.exists(f"{HOME}/mimir.db") else None, 15)
    fake = {"provider": "openai", "model": "fake", "base_url": f"http://127.0.0.1:{AI}/v1", "tools": "", "look": ""}
    api("onboarding_done")
    api("set_api_key", {"kind": "openai", "key": "test-key"})
    api("save_bot", {"bot": {"id": 1, "name": "Ada", "persona": "", **fake}, "secrets": {"whatsapp_number_id": "100", "whatsapp_token": "tok", "whatsapp_app_secret": SECRET}})
    check("connected", until(lambda: "whatsapp connected as +1 555-0000" in open(f"{DIR}/server.log").read(), 15), open(f"{DIR}/server.log").read())
    check("Meta's webhook check", http("/whatsapp?hub.mode=subscribe&hub.verify_token=100&hub.challenge=abc") == (200, "abc"))
    check("…not with another token", http("/whatsapp?hub.mode=subscribe&hub.verify_token=nope&hub.challenge=abc")[0] == 403)
    check("a call not signed with the app's secret is refused", hook({"type": "text", "text": {"body": "hi"}}, sign="wrong") == 401)

    code = until(lambda: next((s["pair_code"] for s in api("statuses") if s.get("bot_id") == 1 and s.get("pair_code")), None), 10)
    t = time.time(); say(f"pair {code}")
    check("paired", until(lambda: any("Paired" in x for x in texts(t)), 10), texts(t))

    t = time.time(); say("hello there")
    check("typing first", until(lambda: [c for c in CALLS if c["t"] >= t and "typing_indicator" in c], 5))
    check("reply", until(lambda: "OK: hello there" in texts(t), 15), texts(t))
    sqlq = lambda sql, *p: sqlite3.connect(f"{HOME}/mimir.db").execute(sql, p).fetchall()
    check("the chat is named for the person", sqlq("SELECT title FROM chats WHERE external=?", ANA) == [("WhatsApp Ana",)], sqlq("SELECT title FROM chats"))

    # A question: its choices are buttons, and tapping one answers it.
    t = time.time(); say("CHOOSE a day")
    q = until(lambda: [c for c in sent(t) if c.get("type") == "interactive"], 15)
    check("choices as buttons", q and [b["reply"]["title"] for b in q[0]["interactive"]["action"]["buttons"]] == ["Thursday", "Friday", "Something else"], sent(t))
    t = time.time(); tap(q[0]["interactive"]["action"]["buttons"][1]["reply"]["id"])
    check("a tap picks", until(lambda: "You chose: Friday" in texts(t), 10), texts(t))

    # An approval card: buttons if they fit, else numbered; either way it's answered.
    chat = sqlq("SELECT chat_id FROM chats WHERE external=?", ANA)[0][0]
    db = sqlite3.connect(f"{HOME}/mimir.db")
    rid = db.execute("INSERT INTO routines(chat_id, cron, prompt, next_fire) VALUES (?, '0 8 * * *', 'brief me', ?) RETURNING id", (chat, int(time.time()) + 86400)).fetchall()[0][0]
    db.commit()
    t = time.time(); say(f"DELETE ROUTINE {rid}")
    card = until(lambda: [c for c in sent(t) if c["type"] in ("text", "interactive") and "Approve this action?" in json.dumps(c, ensure_ascii=False)], 15)
    check("a card", card, sent(t))
    t = time.time()
    if card and card[0]["type"] == "interactive":
        tap(card[0]["interactive"]["action"]["buttons"][0]["reply"]["id"])
    else:
        say("1")
    check("approved", until(lambda: any(x.startswith("✅ Approved") for x in texts(t)), 10), texts(t))
    check("the reply goes on", until(lambda: any("DONE routine_delete" in x for x in texts(t)), 15), texts(t))

    # A forward is someone else's words: the conversation has read outside content.
    t = time.time(); say("dinner at 8?", context={"forwarded": True})
    check("a forward is answered", until(lambda: any(x.startswith("OK:") for x in texts(t)), 15), texts(t))
    check("…as outside content", sqlq("SELECT COUNT(*) FROM conv_taint WHERE chat_id=?", chat)[0][0] > 0)

    # Files both ways.
    t = time.time(); hook({"type": "document", "document": {"id": "4242", "mime_type": "text/plain", "filename": "note.txt"}})
    check("a file is read", until(lambda: any(x.startswith("OK:") for x in texts(t)), 15), texts(t))
    check("…and its text reaches the AI", "Dentist is on Tuesday at nine" in open(f"{DIR}/ai.log").read())
    # The chat has read outside content, so writing a file asks first.
    t = time.time(); say("FILE")
    card = until(lambda: [c for c in sent(t) if "Approve this action?" in json.dumps(c, ensure_ascii=False)], 15)
    t = time.time()
    tap(card[0]["interactive"]["action"]["buttons"][0]["reply"]["id"]) if card and card[0]["type"] == "interactive" else say("1")
    check("written", until(lambda: any("DONE file_write" in x for x in texts(t)), 15), texts(t))
    t = time.time(); say("SEND FILE")
    check("a file sent back", until(lambda: [c for c in CALLS if c["t"] >= t and c.get("media") and c["name"]] and [c for c in sent(t) if c.get("type") == "document"], 15), CALLS[-3:])
finally:
    for p in procs:
        p.kill()
print("ALL PASS" if ok else "SOME FAILED")
sys.exit(0 if ok else 1)
