#!/usr/bin/env python3
# Keeps one `wubble mcp` stdio connection open and serves tool calls on a unix socket, so a
# session (approved once on screen) can be driven by several short-lived scripts.
#
#   python3 -I mcpd.py SOCK [WUBBLE_BINARY]      (run in the background; see README.md)
#
# The default binary is the installed macOS app.
import json, os, socket, subprocess, sys, threading

sock_path = sys.argv[1]
binary = sys.argv[2] if len(sys.argv) > 2 else "/Applications/wubble.app/Contents/MacOS/wubble"
p = subprocess.Popen([binary, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
lock = threading.Lock()
pending = {}
next_id = [100]


def send(m):
    with lock:
        p.stdin.write(json.dumps(m) + "\n")
        p.stdin.flush()


def reader():
    for line in p.stdout:
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if "id" in m and m["id"] in pending:
            ev, box = pending[m["id"]]
            box.append(m)
            ev.set()
    for ev, box in list(pending.values()):
        box.append({"error": "the mcp process exited"})
        ev.set()


threading.Thread(target=reader, daemon=True).start()


def call(method, params):
    with lock:
        next_id[0] += 1
        i = next_id[0]
    ev, box = threading.Event(), []
    pending[i] = (ev, box)
    send({"jsonrpc": "2.0", "id": i, "method": method, "params": params})
    ev.wait()
    pending.pop(i, None)
    return box[0]


init = call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                           "clientInfo": {"name": "wubble-mcp-bench", "version": "0"}})
print(json.dumps(init)[:200], flush=True)
send({"jsonrpc": "2.0", "method": "notifications/initialized"})

if os.path.exists(sock_path):
    os.unlink(sock_path)
srv = socket.socket(socket.AF_UNIX)
srv.bind(sock_path)
srv.listen(8)


def serve(c):
    data = b""
    while not data.endswith(b"\n"):
        chunk = c.recv(65536)
        if not chunk:
            break
        data += chunk
    req = json.loads(data)
    res = call("tools/call", {"name": req["tool"], "arguments": req["args"], "_meta": {"progressToken": "p"}})
    c.sendall((json.dumps(res) + "\n").encode())
    c.close()


while True:
    c, _ = srv.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
