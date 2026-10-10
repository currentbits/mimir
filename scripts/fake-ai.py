"""A scripted fake AI with an OpenAI-compatible /chat/completions, for testing what bots do with their tools end to end
without a real model. Point a bot at it: AI account "OpenAI-compatible", base URL http://127.0.0.1:8899/v1, any key.

    python3 scripts/fake-ai.py [port]

The bot's name comes from its instructions ("Your name is …"). A bot named Search answers "SEARCH-ANSWER: …" to any
task. Any other bot looks at your latest message for a word in capitals and calls the tool it stands for, then
replies "DONE <tool>: <the tool's result>":

    ASK SEARCH → ask_bot    MAKE BOT → bot_create    ROUTINE → routine_create    REMEMBER → memory_store
    FORGET <n> → memory_forget(n)    FILE → file_write    LIST FILES → file_list    DELETE ROUTINE <n> → routine_delete(n)

Anything else gets "OK: <your message>". Each request is logged to stdout.
"""
import json, re, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8899
SCRIPT = [
    ("ASK SEARCH", "ask_bot", lambda m: {"bot": "Search", "task": "Check the title tag of heymimir.app"}),
    ("MAKE BOT", "bot_create", lambda m: {"name": "Scout", "role": "You watch prices and only speak up when one drops."}),
    ("DELETE ROUTINE", "routine_delete", lambda m: {"id": int(re.search(r"DELETE ROUTINE (\d+)", m).group(1))}),
    ("ROUTINE", "routine_create", lambda m: {"cron": "0 8 * * *", "prompt": "Brief me on tech news"}),
    ("REMEMBER", "memory_store", lambda m: {"type": "knowledge", "title": "Dentist", "body": "Dr. Lee in Stellenbosch", "source": "user said"}),
    ("FORGET", "memory_forget", lambda m: {"id": int(re.search(r"FORGET (\d+)", m).group(1))}),
    ("LIST FILES", "file_list", lambda m: {}),
    ("FILE", "file_write", lambda m: {"path": "notes/trip.md", "content": "- passport\n", "mode": "create"}),
]


def text_of(c):
    return c if isinstance(c, str) else " ".join(p.get("text", "") for p in c or [] if isinstance(p, dict))


class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass

    def reply(self, msg):
        b = json.dumps({"id": "fake", "object": "chat.completion", "model": "fake", "choices": [{"index": 0, "message": msg, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 10, "completion_tokens": 5}}).encode()
        self.send_response(200); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)

    def do_GET(self):  # model list
        b = json.dumps({"data": [{"id": "fake"}]}).encode()
        self.send_response(200); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)

    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers.get("content-length") or 0)) or b"{}")
        msgs = req.get("messages", [])
        system = " ".join(text_of(m.get("content")) for m in msgs if m.get("role") == "system")
        bot = (re.search(r"Your name is ([^.]+)\.", system) or [None, "?"])[1]
        users = [text_of(m.get("content")) for m in msgs if m.get("role") == "user"]
        last_user = users[-1] if users else ""
        last = msgs[-1] if msgs else {}
        print(f"[{bot}] last={last.get('role')} user={last_user[-80:]!r}", flush=True)
        if bot == "Search":
            return self.reply({"role": "assistant", "content": f"SEARCH-ANSWER: the title tag is fine (asked: {last_user[-60:]})"})
        if last.get("role") == "tool":
            # What the tool said, back to the person.
            name = next((c["function"]["name"] for m in reversed(msgs) if m.get("role") == "assistant" for c in m.get("tool_calls") or []), "?")
            return self.reply({"role": "assistant", "content": f"DONE {name}: {text_of(last.get('content'))[:300]}"})
        for word, tool, args in SCRIPT:
            if word in last_user:
                return self.reply({"role": "assistant", "content": None, "tool_calls": [
                    {"id": "call1", "type": "function", "function": {"name": tool, "arguments": json.dumps(args(last_user))}}]})
        return self.reply({"role": "assistant", "content": f"OK: {last_user[-100:]}"})


print(f"fake AI on http://127.0.0.1:{PORT}/v1", flush=True)
ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
