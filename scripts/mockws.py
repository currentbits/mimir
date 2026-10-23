"""The smallest WebSocket server the channel mocks need (text frames, no extensions), stdlib only.

    ws = upgrade(handler)   # inside a BaseHTTPRequestHandler.do_GET; then ws.send(obj), ws.recv() -> obj or None

The handler class needs protocol_version = "HTTP/1.1" (a 101 on HTTP/1.0 is refused).
"""
import base64, hashlib, json, struct, threading


class WS:
    def __init__(self, rfile, wfile):
        self.r, self.w, self.lock = rfile, wfile, threading.Lock()

    def send(self, obj):
        b = json.dumps(obj).encode()
        head = bytes([0x81]) + (bytes([len(b)]) if len(b) < 126 else bytes([126]) + struct.pack(">H", len(b)) if len(b) < 65536 else bytes([127]) + struct.pack(">Q", len(b)))
        with self.lock:
            self.w.write(head + b); self.w.flush()

    def close(self, code=1000):
        with self.lock:
            self.w.write(bytes([0x88, 2]) + struct.pack(">H", code)); self.w.flush()

    def recv(self):
        """The next text frame as JSON; None once the client closes or goes. Pings are answered, other frames skipped."""
        while True:
            head = self.r.read(2)
            if len(head) < 2:
                return None
            op, n = head[0] & 0x0F, head[1] & 0x7F
            if n == 126: n = struct.unpack(">H", self.r.read(2))[0]
            elif n == 127: n = struct.unpack(">Q", self.r.read(8))[0]
            mask = self.r.read(4) if head[1] & 0x80 else b"\0\0\0\0"
            data = bytes(c ^ mask[i % 4] for i, c in enumerate(self.r.read(n)))
            if op == 8:
                return None
            if op == 9:
                with self.lock:
                    self.w.write(bytes([0x8A, len(data)]) + data); self.w.flush()
            elif op == 1:
                return json.loads(data)


def upgrade(h):
    accept = base64.b64encode(hashlib.sha1((h.headers["Sec-WebSocket-Key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
    h.send_response(101); h.send_header("Upgrade", "websocket"); h.send_header("Connection", "Upgrade"); h.send_header("Sec-WebSocket-Accept", accept); h.end_headers()
    h.close_connection = True
    return WS(h.rfile, h.wfile)
