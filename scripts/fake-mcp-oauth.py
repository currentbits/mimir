"""A fake remote MCP server that wants an OAuth sign-in, for testing Mimir's MCP authorization without Notion or Linear.

    python3 scripts/fake-mcp-oauth.py [port]     (default 8977)

/mcp answers 401 with a resource_metadata hint until a valid bearer token comes. It serves protected-resource and
authorization-server metadata, dynamic registration (native apps only, as some services insist on for a localhost
redirect), an /authorize page that approves at once (redirects back with a code and, RFC 9207, its issuer), and
/token (PKCE S256 checked; tokens last 2 s; refresh tokens rotate). POST /revoke ends the current access token early,
to test the retry. The MCP side has one tool, "echo".
"""
import base64, hashlib, json, secrets, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs, urlencode

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8977
BASE = f"http://127.0.0.1:{PORT}"
clients, codes, access, refresh = {}, {}, {}, set()


class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass

    def send(self, code, body=None, headers=()):
        data = json.dumps(body).encode() if body is not None else b""
        self.send_response(code)
        for k, v in headers: self.send_header(k, v)
        if body is not None: self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data))); self.end_headers(); self.wfile.write(data)

    def body(self):
        return self.rfile.read(int(self.headers.get("content-length") or 0)).decode()

    def do_GET(self):
        u = urlparse(self.path); q = {k: v[0] for k, v in parse_qs(u.query).items()}
        if u.path == "/.well-known/oauth-protected-resource/mcp":
            return self.send(200, {"resource": f"{BASE}/mcp", "authorization_servers": [BASE], "scopes_supported": ["read"]})
        if u.path == "/.well-known/oauth-authorization-server":
            return self.send(200, {"issuer": BASE, "authorization_endpoint": f"{BASE}/authorize", "token_endpoint": f"{BASE}/token",
                                   "registration_endpoint": f"{BASE}/register", "code_challenge_methods_supported": ["S256"],
                                   "authorization_response_iss_parameter_supported": True})
        if u.path == "/authorize":
            c = clients.get(q.get("client_id"))
            assert c and q["redirect_uri"] in c["redirect_uris"] and q["code_challenge_method"] == "S256", q
            assert q.get("resource") == f"{BASE}/mcp" and q.get("scope") == "read", q
            code = secrets.token_hex(8); codes[code] = q
            return self.send(302, None, [("location", q["redirect_uri"] + "?" + urlencode({"code": code, "state": q["state"], "iss": BASE}))])
        self.send(404, {})

    def tokens(self):
        a, r = secrets.token_hex(8), secrets.token_hex(8)
        access[a] = time.time() + 2; refresh.add(r)
        return self.send(200, {"access_token": a, "refresh_token": r, "token_type": "Bearer", "expires_in": 2})

    def do_POST(self):
        u = urlparse(self.path); raw = self.body()
        if u.path == "/register":
            c = json.loads(raw)
            if c.get("application_type") != "native": return self.send(400, {"error": "invalid_redirect_uri"})
            cid = secrets.token_hex(6); clients[cid] = c
            return self.send(201, {"client_id": cid, **c})
        if u.path == "/token":
            f = {k: v[0] for k, v in parse_qs(raw).items()}
            if f.get("resource") != f"{BASE}/mcp": return self.send(400, {"error": "invalid_target"})
            if f["grant_type"] == "authorization_code":
                q = codes.pop(f.get("code"), None)
                ok = q and q["client_id"] == f["client_id"] and q["redirect_uri"] == f["redirect_uri"] and \
                    base64.urlsafe_b64encode(hashlib.sha256(f["code_verifier"].encode()).digest()).rstrip(b"=").decode() == q["code_challenge"]
                return self.tokens() if ok else self.send(400, {"error": "invalid_grant"})
            if f["grant_type"] == "refresh_token" and f.get("refresh_token") in refresh:
                refresh.discard(f["refresh_token"]); return self.tokens()
            return self.send(400, {"error": "invalid_grant"})
        if u.path == "/revoke":
            access.clear(); return self.send(200, {})
        if u.path == "/mcp":
            tok = (self.headers.get("authorization") or "").removeprefix("Bearer ")
            if access.get(tok, 0) < time.time():
                return self.send(401, {"error": "invalid_token"},
                                 [("www-authenticate", f'Bearer resource_metadata="{BASE}/.well-known/oauth-protected-resource/mcp"')])
            m = json.loads(raw)
            if "id" not in m: return self.send(202)
            res = {"initialize": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "1"}},
                   "tools/list": {"tools": [{"name": "echo", "description": "Says it back", "inputSchema": {"type": "object"}}]},
                   "tools/call": {"content": [{"type": "text", "text": json.dumps(m.get("params", {}).get("arguments"))}]}}.get(m["method"], {})
            return self.send(200, {"jsonrpc": "2.0", "id": m["id"], "result": res})
        self.send(404, {})


print(f"fake MCP with OAuth on {BASE}/mcp", flush=True)
ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
