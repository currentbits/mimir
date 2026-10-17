"""A scripted fake AI with an OpenAI-compatible /chat/completions, for testing what bots do with their tools end to end
without a real model. Point a bot at it: AI account "OpenAI-compatible", base URL http://127.0.0.1:8899/v1, any key.

    python3 scripts/fake-ai.py [port]

The bot's name comes from its instructions ("Your name is …"). A bot named Search answers "SEARCH-ANSWER: …" to any
task. Any other bot looks at your latest message for a word in capitals and calls the tool it stands for, then
replies "DONE <tool>: <the tool's result>":

    REMIND SAM → routine_create for Sam    HOUSEHOLD → memory_store to the household
    EMAIL (email not connected) → points to Settings → Email, from its <not_connected> instructions
    ASK SEARCH → ask_bot    MAKE BOT → bot_create    ROUTINE → routine_create    REMEMBER → memory_store
    FORGET <n> → memory_forget(n)    CHOOSE → ask_choice    FILE → file_write    LIST FILES → file_list    DELETE ROUTINE <n> → routine_delete(n)
    SEND FILE → send_file(notes/trip.md)    CARD → card_write(Shopping: oat milk, eggs)

A routine asked to say URGENT when the person's rule holds, about something DELAYED, says so.
Anything else gets "OK: <your message>". A message with SLOW in it is answered after 3 seconds (any bot), for
switching away mid-reply; "SLOW <n>" waits n seconds. Each request is logged to stdout.

It also stands in for ChatGPT's sign-in and Responses API ("Sign in with ChatGPT"): start the server with
MIMIR_CHATGPT_AUTH=http://127.0.0.1:8899 MIMIR_CHATGPT_API=http://127.0.0.1:8899/v1. The fake sign-in code is
"nonce:<the nonce from the sign-in address>". Requests that break the route's rules (stored items, tools outside a
namespace, forbidden fields) get the 400 the real one would; web search is "not offered", and LIMIT in a message
answers with the plan's usage limit.
"""
import base64, json, re, sys, time, urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8899
SCRIPT = [
    ("REMIND SAM", "routine_create", lambda m: {"at": time.strftime("%Y-%m-%d 18:00", time.localtime(time.time() + 86400)), "prompt": "Remind Sam to take the bins out", "for": "Sam"}),
    ("HOUSEHOLD", "memory_store", lambda m: {"type": "task", "title": "Shopping list", "body": "milk, bread", "household": True}),
    ("ASK SEARCH", "ask_bot", lambda m: {"bot": "Search", "task": "Check the title tag of heymimir.app"}),
    ("MAKE BOT", "bot_create", lambda m: {"name": "Scout", "role": "You watch prices and only speak up when one drops."}),
    ("DELETE ROUTINE", "routine_delete", lambda m: {"id": int(re.search(r"DELETE ROUTINE (\d+)", m).group(1))}),
    ("ROUTINE", "routine_create", lambda m: {"cron": "0 8 * * *", "prompt": "Brief me on tech news"}),
    ("REMEMBER", "memory_store", lambda m: {"type": "knowledge", "title": "Dentist", "body": "Dr. Lee in Stellenbosch", "source": "user said"}),
    ("FORGET", "memory_forget", lambda m: {"id": int(re.search(r"FORGET (\d+)", m).group(1))}),
    ("CHOOSE", "ask_choice", lambda m: {"question": "Which day?", "options": ["Thursday", "Friday"]}),
    ("LIST FILES", "file_list", lambda m: {}),
    ("SEND FILE", "send_file", lambda m: {"path": "notes/trip.md"}),
    ("CARD", "card_write", lambda m: {"title": "Shopping", "body": "- [ ] oat milk\n- [ ] eggs"}),
    ("FILE", "file_write", lambda m: {"path": "notes/trip.md", "content": "- passport\n", "mode": "create"}),
]


def text_of(c):
    t = c if isinstance(c, str) else " ".join(p.get("text", "") for p in c or [] if isinstance(p, dict))
    # Each message starts with "Now: <date and time>"; the echo is of what the person said.
    return re.sub(r"^Now: [^\n]*\n\n", "", t)


class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass

    def reply(self, msg):
        b = json.dumps({"id": "fake", "object": "chat.completion", "model": "fake", "choices": [{"index": 0, "message": msg, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 10, "completion_tokens": 5}}).encode()
        self.send_response(200); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)

    def send_json(self, status, v):
        b = json.dumps(v).encode()
        self.send_response(status); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)

    def do_GET(self):  # model list (a ChatGPT plan's has slugs, names and visibility)
        b = json.dumps({"data": [{"id": "fake", "slug": "fake-plan", "display_name": "Fake Plan", "visibility": "list"}, {"id": "hidden", "slug": "hidden", "visibility": "hide"}]}).encode()
        self.send_response(200); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)

    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("content-length") or 0))
        if self.path.startswith("/api/accounts/oauth/"):
            return self.oauth(dict(urllib.parse.parse_qsl(raw.decode())))
        req = json.loads(raw or b"{}")
        if self.path.endswith("/responses"):
            return self.respond_api(req)
        msgs = req.get("messages", [])
        system = " ".join(text_of(m.get("content")) for m in msgs if m.get("role") == "system")
        bot = (re.search(r"Your name is ([^.]+)\.", system) or [None, "?"])[1]
        users = [text_of(m.get("content")) for m in msgs if m.get("role") == "user"]
        last_user = users[-1] if users else ""
        last = msgs[-1] if msgs else {}
        print(f"[{bot}] last={last.get('role')} user={last_user[-80:]!r}", flush=True)
        if "SLOW" in last_user and last.get("role") == "user":
            time.sleep(int((re.search(r"SLOW (\d+)", last_user) or [0, 3])[1]))
        if bot == "Search":
            return self.reply({"role": "assistant", "content": f"SEARCH-ANSWER: the title tag is fine (asked: {last_user[-60:]})"})
        # The one-time "get to know me" pass: one suggestion per email sender line it was shown.
        if "get to know them" in system:
            senders = re.findall(r"^- [^|]+\| ([^|]+) \|", last_user, re.M)
            return self.reply({"role": "assistant", "content": json.dumps([{"action": "add", "type": "knowledge", "title": f"Writes to you: {s.strip()}", "body": f"{s.strip()} emails you.", "reason": "Found in your email: a recent message"} for s in senders[:3]])})
        # Asked for email while it isn't connected: the bot points at the setting, as its instructions say.
        if "EMAIL" in last_user and "<not_connected>" in system and "Settings → Email" in system:
            return self.reply({"role": "assistant", "content": "Your email isn't connected yet. Connect it in Settings → Email and I'll try again."})
        if last.get("role") == "tool":
            # What the tool said, back to the person.
            name = next((c["function"]["name"] for m in reversed(msgs) if m.get("role") == "assistant" for c in m.get("tool_calls") or []), "?")
            return self.reply({"role": "assistant", "content": f"DONE {name}: {text_of(last.get('content'))[:300]}"})
        if "DELAYED" in last_user and "own rule holds now" in last_user:
            return self.reply({"role": "assistant", "content": "URGENT\nYour flight is DELAYED by 3 hours."})
        for word, tool, args in SCRIPT:
            if word in last_user:
                return self.reply({"role": "assistant", "content": None, "tool_calls": [
                    {"id": "call1", "type": "function", "function": {"name": tool, "arguments": json.dumps(args(last_user))}}]})
        return self.reply({"role": "assistant", "content": f"OK: {last_user[-100:]}"})

    # ---- Sign in with ChatGPT ----

    def oauth(self, form):
        if self.path.endswith("/revoke"):
            return self.send_json(200, {})
        if form.get("grant_type") == "refresh_token":
            return self.send_json(200, {"access_token": "fake-access", "refresh_token": "r2", "expires_in": 3600})
        code = form.get("code", "")
        if not code.startswith("nonce:") or not form.get("code_verifier") or form.get("resource") != "https://api.openai.com/v1":
            return self.send_json(400, {"error": "invalid_grant"})
        b64 = lambda v: base64.urlsafe_b64encode(json.dumps(v).encode()).decode().rstrip("=")
        claims = {"iss": f"http://127.0.0.1:{PORT}", "aud": form["client_id"], "nonce": code[6:], "exp": int(time.time()) + 3600, "email": "you@example.com", "sub": "u1"}
        print(f"[chatgpt] signed in, client {form['client_id']}", flush=True)
        self.send_json(200, {"access_token": "fake-access", "refresh_token": "r1", "id_token": f"e30.{b64(claims)}.sig", "expires_in": 3600, "token_type": "Bearer"})

    def respond_api(self, req):
        if self.headers.get("authorization") != "Bearer fake-access":
            return self.send_json(401, {"detail": "bad token"})
        bad = [k for k in ("temperature", "max_output_tokens", "previous_response_id", "user", "metadata") if k in req]
        items = req.get("input", [])
        tools = req.get("tools", [])
        if req.get("store") is not False or req.get("stream") is not True or bad or "instructions" not in req:
            return self.send_json(400, {"error": {"code": "subscription_sharing_unsupported_capability", "message": f"preview rules: {bad}"}})
        if any(t.get("type") == "web_search" for t in tools):
            return self.send_json(400, {"error": {"code": "subscription_sharing_unsupported_capability", "message": "web search isn't offered here"}})
        if any(t.get("type") == "function" for t in tools):
            return self.send_json(400, {"error": {"code": "subscription_sharing_unsupported_capability", "message": "function tools must be in a namespace"}})
        if any(isinstance(i, dict) and "id" in i for i in items):
            return self.send_json(400, {"error": {"code": "invalid_request_error", "message": "Items are not persisted when store is false"}})
        bot = (re.search(r"Your name is ([^.]+)\.", req["instructions"]) or [None, "?"])[1]
        users = [text_of(i.get("content")) for i in items if i.get("role") == "user"]
        last_user, last = (users[-1] if users else ""), (items[-1] if items else {})
        print(f"[{bot} via chatgpt] last={last.get('type') or last.get('role')} user={last_user[-80:]!r}", flush=True)
        self.send_response(200); self.send_header("content-type", "text/event-stream"); self.end_headers()
        ev = lambda v: self.wfile.write(f"event: {v['type']}\ndata: {json.dumps(v)}\n\n".encode())
        done = {"type": "response.completed", "response": {"model": "fake-plan", "usage": {"input_tokens": 20, "output_tokens": 4}, "output": []}}
        if "LIMIT" in last_user:
            return ev({"type": "response.failed", "response": {"error": {"code": "subscription_sharing_usage_limit_exceeded", "message": "weekly cap"}}})
        def say(text):
            for part in (text[:5], text[5:]):
                ev({"type": "response.output_text.delta", "delta": part})
            ev({"type": "response.output_item.done", "item": {"type": "message", "id": "msg_1", "role": "assistant", "content": [{"type": "output_text", "text": text}]}})
            ev(done)
        if last.get("type") == "function_call_output":
            name = next((i["name"] for i in reversed(items) if i.get("type") == "function_call"), "?")
            return say(f"DONE {name}: {str(last.get('output'))[:300]}")
        for word, tool, args in SCRIPT:
            if word in last_user:
                ev({"type": "response.output_item.done", "item": {"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "opaque"}})
                ev({"type": "response.output_item.done", "item": {"type": "function_call", "id": "fc_1", "call_id": "call1", "namespace": "mimir", "name": tool, "arguments": json.dumps(args(last_user))}})
                return ev(done)
        say(f"OK: {last_user[-100:]}")


print(f"fake AI on http://127.0.0.1:{PORT}/v1", flush=True)
ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
