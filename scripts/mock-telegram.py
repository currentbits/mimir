"""Minimal mock of the Telegram Bot API for end-to-end tests of mimir's Telegram channel.

POST /bot<token>/<method> handles getMe, getUpdates (long-poll), sendMessage, editMessageText,
sendChatAction, answerCallbackQuery, sendMessageDraft (private chats, non-zero draft_id; MOCK_NO_DRAFTS=1
rejects it, like an older API), sendRichMessage, sendRichMessageDraft and editMessageText with rich_message
(MOCK_NO_RICH=1 rejects them, like an API before 10.1), answerGuestQuery (an inline_message_id to edit), and multipart uploads (sendPhoto, sendDocument: logged with each part's name and size,
and a file's name). Every call is appended to calls.jsonl.
POST /control/push {update} (or a list, one batch) queues updates for getUpdates (for one bot with "bot": <token>, else for any). As on
Telegram, getUpdates with an offset confirms (drops) the updates before it, so a restarted bot gets only unconfirmed ones.
getFile + GET /file/bot<token>/<path> serve MOCK_FILE. Each bot's getMe says @mock_<token>_bot ("test": @mock_mimir_bot).

POST /control/fail {method, times=1, skip=0, match?, code?, description?, retry_after?, delay?}: the next `times` calls
to `method` (whose JSON contains `match`, after `skip` such calls went through) wait `delay` seconds, then fail as
Telegram fails them when a code or description is given (400 by default; a 429 with parameters.retry_after), else go
through. Failed calls are logged with "failed": <description>.
"""
import email.parser, json, os, sys, threading, time, zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[2] if len(sys.argv) > 2 else "calls.jsonl"
updates, fails, lock, next_id, next_msg = [], [], threading.Lock(), [1], [100]

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def reply(self, obj, status=200):
        b = json.dumps(obj).encode()
        self.send_response(status); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)
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
                for u in data if isinstance(data, list) else [data]:
                    u["update_id"] = next_id[0]; next_id[0] += 1; updates.append(u)
            return self.reply({"ok": True})
        if self.path == "/control/fail":
            with lock: fails.append({"times": 1, "skip": 0, **data})
            return self.reply({"ok": True})
        bot, method = self.path.split("/bot", 1)[-1].split("/", 1)
        mine = lambda u: u.get("bot", bot) == bot
        with lock:
            rule = next((r for r in fails if r["method"] == method and r.get("match", "") in json.dumps(data, ensure_ascii=False)), None)
            if rule and rule["skip"] > 0: rule["skip"] -= 1; rule = None
            if rule:
                rule["times"] -= 1
                if rule["times"] <= 0: fails.remove(rule)
        delay = rule.get("delay", 0) if rule else 0
        if rule and not ("code" in rule or "description" in rule): rule = None
        if method != "getUpdates":
            with open(LOG, "a") as f: f.write(json.dumps({"t": round(time.time(), 2), "method": method, "bot": bot, **data, **({"failed": rule.get("description", "")} if rule else {})}) + "\n")
        time.sleep(delay)
        if rule:
            code = rule.get("code", 400)
            err = {"ok": False, "error_code": code, "description": rule.get("description") or f"Error {code}"}
            if "retry_after" in rule: err["parameters"] = {"retry_after": rule["retry_after"]}
            return self.reply(err, code)
        if method == "getMe": return self.reply({"ok": True, "result": {"id": 1 if bot == "test" else zlib.crc32(bot.encode()), "is_bot": True, "username": "mock_mimir_bot" if bot == "test" else f"mock_{bot}_bot"}})
        if method == "getUpdates":
            deadline = time.time() + min(int(data.get("timeout", 0)), 2)
            with lock:
                if data.get("offset"): updates[:] = [u for u in updates if not mine(u) or u["update_id"] >= data["offset"]]
            while True:
                with lock:
                    ready = [{k: v for k, v in u.items() if k != "bot"} for u in updates if mine(u) and u["update_id"] >= data.get("offset", 0)]
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
