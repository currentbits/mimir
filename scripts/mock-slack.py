"""Minimal mock of the Slack Web API + Socket Mode, for end-to-end tests of mimir's Slack channel.

auth.test, apps.connections.open (-> ws://127.0.0.1:<port>/socket, upgraded on GET), chat.postMessage/update/
postEphemeral/startStream (always answered with no "ts", so a stream falls back to postMessage) and anything
else POSTed get a generic {"ok": true}; conversations.replies (GET) answers a one-message thread, slowed by
/control/slow. Every Web API call is appended to calls.jsonl with a `t` timestamp.

POST /control/push {type, payload, retry_after?} sends a Socket Mode envelope (events_api's payload is
{"event": {...}}, interactive's is the block_actions payload itself) over the one open socket, logs it
("call": "push") and the bot's ack ("call": "ack", once it arrives), and returns {"envelope_id"}. retry_after:
Slack resends the same payload after that many seconds, new envelope_id, retry_attempt=1 (as Slack does when
an ack is late) -- regardless of whether the first was acked, since that's the case a dedupe test needs.

POST /control/slow {method, secs}: the next call to that Web API method (conversations.replies) sleeps first,
to make an inline handler slow, the way a slow file download or thread lookup would.
"""
import json, os, sys, threading, time, urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.path.dirname(__file__))
import mockws

PORT = sys.argv[1]
LOG = sys.argv[2] if len(sys.argv) > 2 else "calls.jsonl"
BOT_USER = "UBOT1"

lock = threading.Lock()
seq = [1_700_000_000_000_000]  # microseconds, for fake Slack ts values
next_env = [0]
conn = {"ws": None}
slow = {}


def log(obj):
    with lock:
        open(LOG, "a").write(json.dumps({"t": round(time.time(), 3), **obj}) + "\n")


def next_ts():
    with lock:
        seq[0] += 1
        v = seq[0]
    return f"{v // 1_000_000}.{v % 1_000_000:06d}"


def send_ws(obj):
    # Waits a little for the socket Mimir opens on apps.connections.open, in case this races the test.
    for _ in range(100):
        if conn["ws"]:
            conn["ws"].send(obj)
            return
        time.sleep(0.1)
    raise RuntimeError("no Socket Mode connection yet")


def resend(data):
    time.sleep(data["retry_after"])
    eid = f"env{next_env[0]}"; next_env[0] += 1
    env = {"envelope_id": eid, "type": data["type"], "payload": data["payload"], "retry_attempt": 1}
    log({"call": "push", "envelope_id": eid, "type": data["type"], "retry_attempt": 1})
    send_ws(env)


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def reply(self, obj):
        b = json.dumps(obj).encode()
        self.send_response(200); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)

    def do_GET(self):
        if self.path.startswith("/socket"):
            ws = mockws.upgrade(self)
            conn["ws"] = ws
            while (obj := ws.recv()) is not None:
                log({"call": "ack", "envelope_id": obj.get("envelope_id")})
            return
        path, _, qs = self.path.partition("?")
        method = path.lstrip("/")
        q = {k: v[0] for k, v in urllib.parse.parse_qs(qs).items()}
        log({"call": "get", "method": method, **q})
        if method in slow:
            time.sleep(slow.pop(method))
        if method == "conversations.replies":
            return self.reply({"ok": True, "messages": [{"user": BOT_USER, "bot_id": "B1", "text": "(thread root)"}]})
        if method == "conversations.history":
            return self.reply({"ok": True, "messages": [], "has_more": False})
        if method in ("users.info", "conversations.info"):
            return self.reply({"ok": True, "user": {"name": "tester"}, "channel": {"name": "general"}})
        self.reply({"ok": True})

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length") or 0)) or b"{}"
        data = json.loads(body)
        if self.path == "/control/push":
            eid = f"env{next_env[0]}"; next_env[0] += 1
            env = {"envelope_id": eid, "type": data["type"], "payload": data["payload"]}
            log({"call": "push", "envelope_id": eid, "type": data["type"]})
            send_ws(env)
            if data.get("retry_after"):
                threading.Thread(target=resend, args=(data,), daemon=True).start()
            return self.reply({"envelope_id": eid})
        if self.path == "/control/slow":
            slow[data["method"]] = data.get("secs", 3)
            return self.reply({"ok": True})
        method = self.path.lstrip("/")
        log({"call": "post", "method": method, **data})
        if method == "auth.test":
            return self.reply({"ok": True, "user_id": BOT_USER, "user": "mimir", "team": "Test Team"})
        if method == "apps.connections.open":
            return self.reply({"ok": True, "url": f"ws://127.0.0.1:{PORT}/socket"})
        if method == "chat.postMessage":
            return self.reply({"ok": True, "ts": next_ts(), "channel": data.get("channel")})
        if method == "chat.update":
            return self.reply({"ok": True, "ts": data.get("ts")})
        # chat.postEphemeral, chat.startStream (no "ts": draft_post falls back to chat.postMessage),
        # chat.appendStream, chat.stopStream, assistant.threads.setStatus, files.*: a plain ok is enough.
        self.reply({"ok": True})


ThreadingHTTPServer(("127.0.0.1", int(PORT)), H).serve_forever()
