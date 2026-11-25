"""What the hand-run harnesses share (standard library only): a browser's process tree and its
memory from /proc, a loopback server whose pages report to `/log?k=v`, and wubble's
`WUBBLE_TRACE=1` page-layer summary lines. `runner.py` and `linux-vs-firefox.py` import it;
`scripts/mac-checks/run.py` can take its server and summary parser from here
(`sys.path.insert(0, str(HERE.parent / "bench"))`): the same code, ported from it.
"""
import http.server
import os
import re
import threading
import time
import urllib.parse


def tree(root):
    """{pid: user + system ticks} of `root` and every process under it."""
    kids = {}
    for pid in filter(str.isdigit, os.listdir("/proc")):
        try:
            with open(f"/proc/{pid}/stat") as f:
                rest = f.read().rsplit(")", 1)[1].split()
            kids.setdefault(int(rest[1]), []).append((int(pid), int(rest[11]) + int(rest[12])))
        except OSError:
            pass
    out, todo = {}, [root]
    while todo:
        q = todo.pop()
        for pid, ticks in kids.get(q, []):
            out[pid] = ticks
            todo.append(pid)
    try:
        with open(f"/proc/{root}/stat") as f:
            rest = f.read().rsplit(")", 1)[1].split()
        out[root] = int(rest[11]) + int(rest[12])
    except OSError:
        pass
    return out


def memory(pid):
    """(RSS, PSS) of one process in kB, from `smaps_rollup`; (0, 0) once it is gone."""
    got = {}
    try:
        with open(f"/proc/{pid}/smaps_rollup") as f:
            for line in f:
                key = line.split(":", 1)[0]
                if key in ("Rss", "Pss"):
                    got[key] = int(line.split()[1])
    except OSError:
        pass
    return got.get("Rss", 0), got.get("Pss", 0)


def ident(pid):
    """A process's name and command line, joined: what a kind rule matches against."""
    try:
        with open(f"/proc/{pid}/comm") as f:
            comm = f.read().strip()
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            args = f.read().decode(errors="replace").replace("\0", " ")
    except OSError:
        return ""
    return f"{comm}\0{args}"


class Loopback:
    """Serves `root` on 127.0.0.1 at a port the kernel picks; what a page sends to `/log?k=v`
    is kept in order (`mark`, `last`). Ported from scripts/mac-checks/run.py's `Quiet`."""

    def __init__(self, root):
        self.events, self.lock = [], threading.Lock()
        loop = self

        class Handler(http.server.SimpleHTTPRequestHandler):
            def __init__(self, *a, **k):
                super().__init__(*a, directory=str(root), **k)

            def log_message(self, *a):
                pass

            def do_GET(self):
                if not self.path.startswith("/log?"):
                    return super().do_GET()
                q = urllib.parse.parse_qs(self.path[5:], keep_blank_values=True)
                with loop.lock:
                    loop.events.append({k: v[0] for k, v in q.items()} | {"_t": time.time()})
                self.send_response(204)
                self.end_headers()

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def mark(self):
        with self.lock:
            return len(self.events)

    def since(self, mark, key):
        with self.lock:
            return [e for e in self.events[mark:] if key in e]

    def last(self, mark, key, wait=1.5):
        """The latest event since `mark` carrying `key`, waiting up to `wait` s for one."""
        end = time.time() + wait
        while True:
            hits = self.since(mark, key)
            if hits or time.time() > end:
                return hits[-1] if hits else None
            time.sleep(0.1)

    def close(self):
        self.server.shutdown()


SUMMARY = re.compile(r"page layer: ([\d.]+)s: (.*?); pool_busy (\d+); (.*); window (\w+), pointer (.*)$")


def summaries(lines):
    """wubble's per-second page-layer lines (`WUBBLE_TRACE=1`): counts (`cef_frames`,
    `slint_renders`, ...), pool_busy, each tab's route."""
    out = []
    for m in map(SUMMARY.search, lines):
        if not m:
            continue
        counts = {}
        for part in m.group(2).split(", "):
            k, _, n = part.rpartition(" ")
            if k and n.isdigit():
                counts[k] = int(n)
        tabs = dict(re.findall(r"tab (\d+) (\w+)", m.group(4)))
        out.append({"counts": counts, "busy": int(m.group(3)), "tabs": tabs})
    return out
