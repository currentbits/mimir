"""Minimal mock of signal-cli-rest-api (MODE=json-rpc) for end-to-end tests of mimir's Signal channel.

    python3 scripts/mock-signal.py <port> [log]     # then MIMIR_SIGNAL_API=http://127.0.0.1:<port> SIGNAL_NUMBER=+15550000

GET /v1/accounts lists the bot's number (+15550000, or MOCK_NUMBER). GET /v1/receive/<number> is the WebSocket that
delivers envelopes; POST /control/push {envelope} queues one. POST /v2/send and PUT /v1/typing-indicator/<number> are
appended to calls.jsonl (each attachment as {"mime", "filename", "size"}). GET /v1/attachments/<id> serves MOCK_FILE,
else the id as text (dashes as spaces).
"""
import base64, hashlib, json, os, queue, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18082
LOG = sys.argv[2] if len(sys.argv) > 2 else "calls.jsonl"
NUMBER = os.environ.get("MOCK_NUMBER", "+15550000")
inbox, stamp = queue.Queue(), [1_700_000_000_000]


def attachment(uri):
    head, _, data = uri.partition(",")
    meta = dict(p.split("=", 1) for p in head.split(";") if "=" in p)
    return {"mime": head[5:].split(";")[0], "filename": meta.get("filename"), "size": len(base64.b64decode(data))}


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a): pass

    def reply(self, obj, code=200):
        b = json.dumps(obj).encode()
        self.send_response(code); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)

    def log(self, method, data):
        with open(LOG, "a") as f: f.write(json.dumps({"t": time.time(), "method": method, **data}) + "\n")

    def body(self):
        return json.loads(self.rfile.read(int(self.headers.get("content-length") or 0)) or b"{}")

    def do_GET(self):
        if self.path == "/v1/accounts": return self.reply([NUMBER])
        if self.path == f"/v1/receive/{NUMBER}" and self.headers.get("upgrade", "").lower() == "websocket":
            key = self.headers["sec-websocket-key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
            self.send_response(101); self.send_header("upgrade", "websocket"); self.send_header("connection", "Upgrade")
            self.send_header("sec-websocket-accept", base64.b64encode(hashlib.sha1(key.encode()).digest()).decode()); self.end_headers()
            while True:
                env = inbox.get()
                b = json.dumps({"envelope": env, "account": NUMBER}).encode()
                head = bytes([0x81, len(b)]) if len(b) < 126 else bytes([0x81, 126]) + len(b).to_bytes(2, "big") if len(b) < 65536 else bytes([0x81, 127]) + len(b).to_bytes(8, "big")
                try:
                    self.wfile.write(head + b); self.wfile.flush()
                except OSError:
                    inbox.put(env); return
        if self.path.startswith("/v1/attachments/"):
            b = open(os.environ["MOCK_FILE"], "rb").read() if os.environ.get("MOCK_FILE") else self.path.rsplit("/", 1)[-1].replace("-", " ").encode()
            self.send_response(200); self.send_header("content-length", str(len(b))); self.end_headers(); return self.wfile.write(b)
        self.reply({"error": "not found"}, 404)

    def do_POST(self):
        data = self.body()
        if self.path == "/control/push":
            inbox.put(data); return self.reply({})
        if self.path == "/v2/send":
            stamp[0] += 1
            self.log("send", {**data, "base64_attachments": [attachment(a) for a in data.get("base64_attachments") or []], "timestamp": stamp[0]})
            return self.reply({"timestamp": str(stamp[0])}, 201)
        self.reply({"error": "not found"}, 404)

    def do_PUT(self):
        data = self.body()
        if self.path == f"/v1/typing-indicator/{NUMBER}":
            self.log("typing", data)
            self.send_response(204); self.send_header("content-length", "0"); return self.end_headers()
        self.reply({"error": "not found"}, 404)


ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
