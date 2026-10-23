"""Minimal mock of Discord's REST API (api/v10) and Gateway for e2e tests of mimir's Discord channel.

REST: GET /users/@me; GET /channels/<id>/messages (catch-up paging, slowed by /control/slow); POST
/channels/<id>/messages (plain or multipart upload); PATCH /channels/<id>/messages/<id>; POST
/channels/<id>/typing; POST /interactions/<id>/<token>/callback. Every call is appended to calls.jsonl with
a 't' timestamp, logged when the request arrives (before any slowdown), including the message id assigned.

Gateway: GET /gateway upgrades (scripts/mockws.py). Sends HELLO (heartbeat_interval 1000ms), waits for
IDENTIFY, sends READY, then only acks heartbeats (op 1 -> op 11; no RESUME, matching discord.rs) and logs
each one as "heartbeat".

Control (for the test to drive it): POST /control/dispatch {t, d} sends a gateway event ({"op":0,"t","d"})
down the open connection. POST /control/slow {ms} delays the next catch-up GET by ms (issue #378: an inline
handler that blocks the event loop). POST /control/close drops the gateway connection so mimir reconnects.
POST /control/noack {on} stops acking heartbeats (a half-open socket, issue #213); the next connection acks again.
"""
import email.parser, json, os, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.path.dirname(__file__))
import mockws

LOG = sys.argv[2] if len(sys.argv) > 2 else "calls.jsonl"
BOT_ID, BOT_NAME = "1", "mock_mimir_bot"
lock = threading.Lock()
next_id = [2000]
slow_ms, noack = [0], [False]
conn = {"ws": None, "seq": 0}


def log(method, **extra):
    with open(LOG, "a") as f:
        f.write(json.dumps({"t": round(time.time(), 3), "method": method, **extra}) + "\n")


def new_id():
    with lock:
        next_id[0] += 1
        return str(next_id[0])


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def reply(self, obj, status=200):
        b = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def body(self):
        n = int(self.headers.get("content-length") or 0)
        raw = self.rfile.read(n) if n else b""
        ct = self.headers.get("content-type", "")
        if ct.startswith("multipart/form-data"):
            msg = email.parser.BytesParser().parsebytes(b"content-type: " + ct.encode() + b"\r\n\r\n" + raw)
            payload = {}
            for part in msg.get_payload():
                name = part.get_param("name", header="content-disposition")
                if part.get_filename():
                    payload[name] = {"filename": part.get_filename(), "size": len(part.get_payload(decode=True) or b"")}
                else:
                    payload[name] = part.get_payload(decode=True).decode()
            return json.loads(payload.get("payload_json", "{}")), payload
        return (json.loads(raw) if raw else {}), None

    def do_GET(self):
        if self.path == "/gateway":
            return self.gateway()
        if self.path == "/users/@me":
            log("me")
            return self.reply({"id": BOT_ID, "username": BOT_NAME, "bot": True})
        if self.path.startswith("/channels/") and "/messages" in self.path:
            log("catchup", path=self.path)
            if slow_ms[0]:
                time.sleep(slow_ms[0] / 1000)
            return self.reply([])
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        data, files = self.body()
        if self.path == "/control/dispatch":
            self.dispatch(data["t"], data.get("d", {}))
            return self.reply({})
        if self.path == "/control/slow":
            slow_ms[0] = data.get("ms", 0)
            return self.reply({})
        if self.path == "/control/noack":
            noack[0] = data.get("on", True)
            return self.reply({})
        if self.path == "/control/close":
            if conn["ws"]:
                conn["ws"].close()
            return self.reply({})
        if self.path.startswith("/interactions/"):
            log("callback", path=self.path, body=data)
            return self.reply({})
        if self.path.startswith("/channels/") and self.path.endswith("/messages"):
            mid = new_id()
            log("post", path=self.path, body=data, files=files, id=mid)
            return self.reply({"id": mid, "channel_id": self.path.split("/")[2], "content": data.get("content", ""), "author": {"id": BOT_ID}})
        if self.path.endswith("/typing"):
            log("typing", path=self.path)
            return self.reply({})
        self.send_response(404)
        self.end_headers()

    def do_PATCH(self):
        data, _ = self.body()
        mid = self.path.rsplit("/", 1)[-1]
        log("patch", path=self.path, body=data, id=mid)
        return self.reply({"id": mid, "content": data.get("content", "")})

    def dispatch(self, t, d):
        with lock:
            conn["seq"] += 1
            seq = conn["seq"]
        log("dispatch", event=t)
        if conn["ws"]:
            conn["ws"].send({"op": 0, "t": t, "s": seq, "d": d})

    def gateway(self):
        ws = mockws.upgrade(self)
        conn["ws"], noack[0] = ws, False
        ws.send({"op": 10, "d": {"heartbeat_interval": 1000}})
        ws.recv()  # IDENTIFY
        log("identify")
        ws.send({"op": 0, "t": "READY", "s": 0, "d": {"user": {"id": BOT_ID, "username": BOT_NAME}}})
        while True:
            v = ws.recv()
            if v is None:
                break
            if v.get("op") == 1:
                log("heartbeat")
                if not noack[0]:
                    ws.send({"op": 11})
        if conn["ws"] is ws:
            conn["ws"] = None


ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
