"""Minimal mock of the Telegram Bot API for end-to-end tests of mimir's Telegram channel.

POST /bot<token>/<method> handles getMe, getUpdates (long-poll), sendMessage, editMessageText,
sendChatAction, answerCallbackQuery, sendMessageDraft (private chats, non-zero draft_id; MOCK_NO_DRAFTS=1
rejects it, like an older API), sendRichMessage, sendRichMessageDraft and editMessageText with rich_message
(MOCK_NO_RICH=1 rejects them, like an API before 10.1), answerGuestQuery (an inline_message_id to edit), and multipart uploads (sendPhoto, sendDocument: logged with each part's name and size,
and a file's name). Every call is appended to calls.jsonl.
POST /control/push {update} queues an update for getUpdates. getFile + GET /file/bot<token>/<path> serve MOCK_FILE.
"""
import email.parser, json, os, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[2] if len(sys.argv) > 2 else "calls.jsonl"
updates, lock, next_id, next_msg = [], threading.Lock(), [1], [100]

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def reply(self, obj):
        b = json.dumps(obj).encode()
        self.send_response(200); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length") or 0)) or b"{}"
        if self.headers.get("content-type", "").startswith("multipart/form-data"):
            # An upload: each text part's value, each file part as {"filename", "size"}.
            msg = email.parser.BytesParser().parsebytes(b"content-type: " + self.headers["content-type"].encode() + b"\r\n\r\n" + body)
            data = {}
            for part in msg.get_payload():
                raw = part.get_payload(decode=True) or b""
                data[part.get_param("name", header="content-disposition")] = {"filename": part.get_filename(), "size": len(raw)} if part.get_filename() else raw.decode()
        else:
            data = json.loads(body)
        if self.path == "/control/push":
            with lock:
                data["update_id"] = next_id[0]; next_id[0] += 1; updates.append(data)
            return self.reply({"ok": True})
        method = self.path.rsplit("/", 1)[-1]
        if method != "getUpdates":
            with open(LOG, "a") as f: f.write(json.dumps({"t": round(time.time(), 2), "method": method, **data}) + "\n")
        if method == "getMe": return self.reply({"ok": True, "result": {"id": 1, "is_bot": True, "username": "mock_mimir_bot"}})
        if method == "getUpdates":
            deadline = time.time() + min(int(data.get("timeout", 0)), 2)
            while True:
                with lock:
                    ready = [u for u in updates if u["update_id"] >= data.get("offset", 0)]
                if ready or time.time() > deadline: return self.reply({"ok": True, "result": ready})
                time.sleep(0.1)
        if os.environ.get("MOCK_NO_RICH") and (method.startswith("sendRich") or "rich_message" in data):
            return self.reply({"ok": False, "description": "Not Found: method not found"})
        if method == "sendRichMessageDraft": method = "sendMessageDraft"
        if method in ("sendMessage", "editMessageText", "sendRichMessage"):
            if "<b>" in data.get("text", "") and data.get("parse_mode") != "HTML":
                return self.reply({"ok": False, "description": "HTML without parse_mode"})
            with lock:
                mid = data.get("message_id") or next_msg[0]
                if method != "editMessageText": next_msg[0] += 1
            return self.reply({"ok": True, "result": {"message_id": mid, "chat": {"id": data.get("chat_id")}}})
        if method == "sendMessageDraft":
            if os.environ.get("MOCK_NO_DRAFTS") or not data.get("draft_id") or int(data.get("chat_id", 0)) <= 0:
                return self.reply({"ok": False, "description": "Bad Request: drafts are for private chats with a draft_id"})
            return self.reply({"ok": True, "result": True})
        if method == "answerGuestQuery":
            return self.reply({"ok": True, "result": {"inline_message_id": "guest-" + data.get("guest_query_id", "")}})
        if method == "getFile":
            return self.reply({"ok": True, "result": {"file_id": data.get("file_id"), "file_path": "photos/" + data.get("file_id", "x") + ".png"}})
        return self.reply({"ok": True, "result": True})
    def do_GET(self):
        # Downloads of files "sent to the bot": every file is the image in MOCK_FILE (default: the app icon).
        if "/file/bot" not in self.path:
            self.send_response(404); self.end_headers(); return
        body = open(os.environ.get("MOCK_FILE", os.path.join(os.path.dirname(__file__), "../crates/mimir-desktop/icons/128x128.png")), "rb").read()
        self.send_response(200); self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)

ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
