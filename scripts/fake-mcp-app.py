"""A fake stdio tool server with an interactive card (MCP Apps), for trying cards without Figma, Notion or Canva.

    Settings → Tool servers: command python3, arguments scripts/fake-mcp-app.py, name demo

"show" (read-only) answers a city's weather as text and as a card: ui://demo/weather. The card says hello once the app
sends it the call's input and answer, and has three buttons: Refresh calls "show" again (read-only: no ask once you
confirm it read-only), Save calls "save" (a write: asks like any other), Ask puts a message in the composer.
With scripts/fake-ai.py, "SHOW CARD" makes a bot call demo__show.
"""
import json, sys

HTML = r"""<!doctype html>
<html><head><style>
body { font: 14px system-ui; margin: 0; padding: 14px; color: #222; } .dark body { color: #eee; }
button { margin-right: 6px; } #out { margin-top: 8px; color: #666; }
</style></head><body>
<b id="title">Waiting for the host…</b>
<div id="out"></div>
<p><button id="refresh">Refresh</button><button id="save">Save</button><button id="ask">Ask</button></p>
<script>
let next = 1; const waiting = {};
const send = (method, params) => new Promise((ok, fail) => { const id = next++; waiting[id] = [ok, fail]; parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*") });
const note = (method, params) => parent.postMessage({ jsonrpc: "2.0", method, params }, "*");
const out = t => document.getElementById("out").textContent = t;
addEventListener("message", e => {
  const m = e.data;
  if (m.id && waiting[m.id]) { const [ok, fail] = waiting[m.id]; delete waiting[m.id]; m.error ? fail(m.error) : ok(m.result); return }
  if (m.method === "ui/notifications/tool-input") document.title = "input " + JSON.stringify(m.params.arguments);
  if (m.method === "ui/notifications/tool-result") document.getElementById("title").textContent = "Weather in " + m.params.structuredContent.city + ": " + m.params.structuredContent.temp + "°";
});
const text = r => (r.content || []).map(c => c.text).join(" ");
document.getElementById("refresh").onclick = () => send("tools/call", { name: "show", arguments: { city: "Paris" } }).then(r => out("refreshed: " + text(r)), e => out("error: " + e.message));
document.getElementById("save").onclick = () => send("tools/call", { name: "save", arguments: { city: "Paris" } }).then(r => out((r.isError ? "not saved: " : "saved: ") + text(r)), e => out("error: " + e.message));
document.getElementById("ask").onclick = () => send("ui/message", { role: "user", content: { type: "text", text: "What should I wear in Paris?" } }).then(() => out("in the composer"));
send("ui/initialize", { protocolVersion: "2026-01-26", clientInfo: { name: "demo", version: "1" }, capabilities: {}, appCapabilities: {} }).then(r => {
  if (r.hostContext.theme === "dark") document.documentElement.className = "dark";
  note("ui/notifications/initialized", {});
  note("ui/notifications/size-changed", { height: document.documentElement.scrollHeight });
  // CSP check: the card names no connectDomains, so this must fail.
  fetch("https://example.com").then(() => out("fetch went through (CSP missing!)"), () => {});
});
</script></body></html>"""

for line in sys.stdin:
    m = json.loads(line)
    if "id" not in m:
        continue
    p, r = m.get("params", {}), None
    if m["method"] == "initialize":
        r = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}, "resources": {}}, "serverInfo": {"name": "demo", "version": "1"}}
    elif m["method"] == "tools/list":
        r = {"tools": [
            {"name": "show", "description": "A city's weather", "inputSchema": {"type": "object", "properties": {"city": {"type": "string"}}},
             "annotations": {"readOnlyHint": True}, "_meta": {"ui": {"resourceUri": "ui://demo/weather"}}},
            {"name": "save", "description": "Save a city", "inputSchema": {"type": "object", "properties": {"city": {"type": "string"}}}},
        ]}
    elif m["method"] == "tools/call":
        city = p.get("arguments", {}).get("city", "Cape Town")
        if p["name"] == "show":
            r = {"content": [{"type": "text", "text": f"{city}: 21°"}], "structuredContent": {"city": city, "temp": 21}}
        else:
            r = {"content": [{"type": "text", "text": f"saved {city}"}]}
    elif m["method"] == "resources/read":
        r = {"contents": [{"uri": p["uri"], "mimeType": "text/html;profile=mcp-app", "text": HTML}]}
    print(json.dumps({"jsonrpc": "2.0", "id": m["id"], "result": r}), flush=True)
