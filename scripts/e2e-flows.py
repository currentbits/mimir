"""Bot flows end to end, without a real AI: delegation, memory, routines, files, making and deleting bots, threads.

    python3 scripts/fake-ai.py 8899 &
    MIMIR_HOME=/tmp/flow-home MIMIR_ADMIN_TOKEN=testtoken-abcdefghijklmnop mimir server --listen 127.0.0.1:8789 &
    python3 scripts/e2e-flows.py

Sets up the bots itself (Ada and Search on the fake AI) in a fresh MIMIR_HOME; checks the database after each step.
"""
import json, sqlite3, threading, time, urllib.request

API = "http://127.0.0.1:8789/api/"
import os
DB = os.path.join(os.environ.get("MIMIR_HOME", "/tmp/flow-home"), "mimir.db")
ok_all = True


def snake(v):
    import re
    if isinstance(v, dict): return {re.sub(r"([A-Z])", lambda m: "_" + m.group(1).lower(), k): snake(x) for k, x in v.items()}
    return v


def api(cmd, args=None):
    r = urllib.request.Request(API + cmd, json.dumps(snake(args or {})).encode(), {"Authorization": "Bearer testtoken-abcdefghijklmnop", "Content-Type": "application/json"})
    out = json.loads(urllib.request.urlopen(r, timeout=120).read())
    if isinstance(out, dict) and "error" in out and len(out) == 1:
        raise RuntimeError(f"{cmd}: {out['error']}")
    return out


def q(sql, *p):
    return sqlite3.connect(DB).execute(sql, p).fetchall()


def check(name, cond, detail=""):
    global ok_all
    ok_all &= bool(cond)
    print(("PASS " if cond else "FAIL ") + name + (f"  [{detail}]" if detail and not cond else ""))


def send(bot, text, approve=None):
    """A message in the built-in chat; approve: True/False answers the approval it raises (from another thread)."""
    if approve is not None:
        def answer():
            for _ in range(60):
                time.sleep(0.5)
                for a in api("approvals"):
                    api("decide", {"id": a["id"], "approve": approve})
                    return
        threading.Thread(target=answer, daemon=True).start()
    return api("send_local", {"botId": bot, "text": text})


fake = {"provider": "openai", "model": "fake", "base_url": "http://127.0.0.1:8899/v1", "tools": "", "look": ""}
api("onboarding_done")
api("set_api_key", {"kind": "openai", "key": "test-key"})
api("save_bot", {"bot": {"id": 1, "name": "Ada", "persona": "", **fake}})
search_id = api("save_bot", {"bot": {"id": 0, "name": "Search", "persona": "SEO research", **fake}})
check("second bot made", isinstance(search_id, int) and search_id > 1, search_id)

r = send(1, "please ASK SEARCH about the title tag")
check("delegation: the answer comes back", "SEARCH-ANSWER" in json.dumps(r), r)
runs = q("SELECT bot_id, trigger, parent_run FROM runs ORDER BY id")
check("delegation: Search ran as a task from Ada", any(b == search_id and t == "delegated" and p for b, t, p in runs), runs)
check("delegation: the thread shows it under Ada", any("SEARCH-ANSWER" in c for (c,) in q("SELECT content FROM messages WHERE bot_id=1")))

r = send(1, "REMEMBER my dentist")
mem = q("SELECT id, title, superseded_by, invalid FROM entries WHERE title='Dentist'")
check("memory_store saves", len(mem) == 1, (r, mem))
r = send(1, f"FORGET {mem[0][0]}", approve=True)
check("memory_forget after approval", q("SELECT invalid FROM entries WHERE id=?", mem[0][0])[0][0] == 1 or "forgot" in json.dumps(r).lower(), (r, q("SELECT * FROM entries WHERE id=?", mem[0][0])))

r = send(1, "set a ROUTINE")
rts = q("SELECT id, prompt, bot_id FROM routines")
check("routine_create", len(rts) == 1 and rts[0][2] == 1, (r, rts))
r = send(1, f"DELETE ROUTINE {rts[0][0]}", approve=True)
check("routine_delete after approval", not q("SELECT id FROM routines"), (r, q("SELECT * FROM routines")))

r = send(1, "write a FILE")
files = api("files", {"chatId": 0})
check("file_write", any(f["path"] == "notes/trip.md" for f in files), (r, files))
r = send(1, "LIST FILES")
check("file_list sees it", "notes/trip.md" in json.dumps(r), r)

r = send(1, "MAKE BOT for prices", approve=False)
check("bot_create denied: no bot", not q("SELECT id FROM bots WHERE name='Scout'"), (r, q("SELECT id,name FROM bots")))
r = send(1, "MAKE BOT for prices", approve=True)
scout = q("SELECT id FROM bots WHERE name='Scout'")
check("bot_create approved: Scout exists", len(scout) == 1, (r, q("SELECT id,name FROM bots")))
check("bot_create: Scout shows in the bot list", any(b["name"] == "Scout" for b in api("bots")))

# Per-bot threads
mine = api("messages", {"chatId": 0, "botId": search_id})
ada = api("messages", {"chatId": 0, "botId": 1})
check("messages carry their id (the app follows long chats by it)", ada and all(isinstance(m.get("id"), int) for m in ada), ada[:1])
check("Search's own thread holds no Ada messages", all(m["bot_id"] == search_id for m in mine), [m["bot_id"] for m in mine])

# Deleting a bot takes its routines and runs with it; memories stay
api("add_routine", {"chatId": 0, "botId": scout[0][0], "cron": "0 9 * * *", "prompt": "check prices", "quiet": True})
api("delete_bot", {"id": scout[0][0]})
check("delete_bot: gone", not q("SELECT id FROM bots WHERE id=?", scout[0][0]))
check("delete_bot: its routines gone", not q("SELECT id FROM routines WHERE bot_id=?", scout[0][0]), q("SELECT * FROM routines"))
check("delete_bot: statuses don't list it", all(s.get("id") != scout[0][0] for s in (api("statuses") if isinstance(api("statuses"), list) else [])))

# Clearing history
n = api("clear_history", {"chatId": 0})
check("clear_history", not q("SELECT id FROM messages WHERE chat_id=0"), (n, q("SELECT count(*) FROM messages WHERE chat_id=0")))

print("ALL PASS" if ok_all else "SOME FAILED")
