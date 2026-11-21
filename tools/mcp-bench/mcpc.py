#!/usr/bin/env python3
# One tool call through mcpd.py's socket; prints the text parts, saves images to shot-<ms>.png.
#
#   python3 -I mcpc.py SOCK TOOL '{"session": "...", ...}'
import base64, json, socket, sys, time

s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
args = json.loads(sys.argv[3] if len(sys.argv) > 3 else "{}")
s.sendall((json.dumps({"tool": sys.argv[2], "args": args}) + "\n").encode())
data = b""
while True:
    chunk = s.recv(1 << 20)
    if not chunk:
        break
    data += chunk
m = json.loads(data)
if "error" in m:
    print("ERROR", json.dumps(m["error"]))
    sys.exit(1)
r = m["result"]
for part in r.get("content", []):
    if part.get("type") == "text":
        print(part["text"])
    elif part.get("type") == "image":
        fn = f"shot-{int(time.time() * 1000)}.png"
        open(fn, "wb").write(base64.b64decode(part["data"]))
        print("image:", fn)
if r.get("isError"):
    sys.exit(1)
