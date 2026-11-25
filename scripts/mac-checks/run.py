#!/usr/bin/env python3
"""mac-checks: the owner's macOS hand-check list for the page layer
(docs/research/page-layer.md, "Owner's hand-check list for a default-on release"), driven.

    python3 scripts/mac-checks/run.py /path/to/wubble.app [--out DIR] [--only c1,c4,...]

It takes over the screen for about four minutes: it starts the app with WUBBLE_TRACE=1 on a
throwaway profile, moves its window to (60, 60) at 1400x900, and posts keys and mouse events to
it. Leave the Mac alone while it runs; Ctrl-C stops it and quits wubble.

Each check is judged from the trace log (the page layer's route lines and its per-second summary:
cef_frames, slint_renders, pool_busy, each pane's route) and from screenshots of the window
(`screencapture -l`), on fixture pages served from loopback whose background is a known colour,
so a pane is found and checked by its colour (present, where it should be, not black). Results
go to DIR (default ~/wubble-mac-checks/<time>): results.md (the table), wubble.log (stderr),
shots/*.png; the throwaway profile and the fixture are deleted at the end. Needs Accessibility and Screen Recording for the terminal that runs it, ffmpeg for
the video fixture (video checks are skipped without it), swiftc (Xcode or its command line tools).
Standard library only.
"""
import argparse
import plistlib
import urllib.parse
import datetime
import functools
import http.server
import os
import re
import shutil
import signal
import sqlite3
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent

# Fixture colours: page backgrounds no shell glass or tint produces. TOL is per channel.
A, B, C, D = "e0218a", "21c4e0", "e0c021", "5a21e0"
GREEN, BLUE = "00ff00", "0000ff"
TOL = 34
WIN = (60, 60, 1400, 900)  # where the window is put: x, y, w, h in points

# Element positions inside a fixture page, in CSS px (= points at 100 % zoom); page.html matches.
PLAY = (130, 68)
FULLSCREEN = (330, 68)
VIDEO = (40, 120, 480, 270)
SELECT = (140, 436)
MARK = (300, 436)
DRAG_SRC = (90, 510)
DROP_ZONE = (280, 520)
AFTER = (450, 510)
QUIET = (760, 300)  # page background, clear of every element

PAGE = """<!doctype html><meta charset=utf-8><title>mac-checks</title>
<style>
html,body{margin:0;height:100%;overflow:hidden}
body{font:15px -apple-system,sans-serif}
.a{position:absolute;box-sizing:border-box;border:0;margin:0}
button.a{font:15px -apple-system,sans-serif;background:#fff;color:#000;border-radius:6px}
</style>
<button class=a id=play style="left:40px;top:40px;width:180px;height:56px">play with sound</button>
<button class=a id=fs style="left:240px;top:40px;width:180px;height:56px">fullscreen</button>
<video class=a id=v loop playsinline style="left:40px;top:120px;width:480px;height:270px;background:#000"></video>
<select class=a id=sel style="left:40px;top:420px;width:200px;height:32px">
<option>one</option><option>two</option><option>three</option></select>
<div class=a id=mark style="left:260px;top:420px;width:80px;height:32px;background:#808080"></div>
<div class=a id=src draggable=true style="left:40px;top:480px;width:100px;height:60px;background:#ffff00"></div>
<div class=a id=zone style="left:200px;top:480px;width:160px;height:80px;background:#808080"></div>
<div class=a id=after style="left:400px;top:480px;width:100px;height:60px;background:#808080"></div>
<div class=a id=spin style="left:560px;top:48px;width:40px;height:40px;background:#fff;animation:spin 1s linear infinite"></div>
<style>@keyframes spin{to{transform:rotate(360deg)}}</style>
<script>
const q = new URLSearchParams(location.search);
if (q.get('from')) addEventListener('pageshow', () => fetch('/log?page=next').catch(() => {}));
document.body.style.background = '#' + (q.get('c') || 'ffffff');
const v = document.getElementById('v');
if (q.get('still')) document.getElementById('spin').remove();
if (q.get('v')) v.src = 'clip.webm'; else v.style.display = 'none';
const play = document.getElementById('play');
play.onclick = () => { if (v.paused) { v.muted = false; v.volume = 0.2; v.play(); } else v.pause(); };
v.onplay = () => play.style.background = '#00ff00';
v.onpause = () => play.style.background = '#ffffff';
document.getElementById('fs').onclick = () => v.requestFullscreen();
document.getElementById('sel').onchange = () => document.getElementById('mark').style.background = '#00ff00';
const zone = document.getElementById('zone');
document.getElementById('src').ondragstart = e => e.dataTransfer.setData('text/plain', 'mac-checks');
zone.ondragover = e => e.preventDefault();
zone.ondrop = e => { e.preventDefault(); zone.style.background = '#00ff00'; };
document.getElementById('after').onclick = e => e.target.style.background = '#0000ff';
</script>
"""

# The editing-keys fixture: one plain <input>, its value and selection sent to `/log` on every
# change; `pageshow` sends which page shows (Back from the bfcache too).
EDIT = """<!doctype html><meta charset=utf-8><title>edit</title>
<style>html,body{margin:0;height:100%;background:#e0218a;font:16px -apple-system,sans-serif}
input{position:absolute;left:40px;top:40px;width:400px;height:40px;font-size:18px}
a{position:absolute;left:40px;top:120px;display:block;width:200px;height:40px;background:#fff}</style>
<input id=f autocomplete=off spellcheck=false>
<a id=next href="page.html?c=21c4e0&from=edit">next page</a>
<script>
const page = new URLSearchParams(location.search).get('name') || 'edit';
const log = q => fetch('/log?' + new URLSearchParams(q)).catch(() => {});
const f = document.getElementById('f');
f.addEventListener('input', () => { document.title = f.value; log({v: f.value}); });
document.addEventListener('selectionchange', () =>
  document.activeElement === f && log({sel: f.selectionStart + ',' + f.selectionEnd, len: f.value.length}));
addEventListener('pageshow', () => log({page}));
</script>
"""

ROUTE = re.compile(r"page layer: tab (\d+): (\w+) -> (\w+) on (.*?); needs: (.*)$")
SUMMARY = re.compile(r"page layer: ([\d.]+)s: (.*?); pool_busy (\d+); (.*); window (\w+), pointer (.*)$")
FAILURE = re.compile(r"^wubble: page layer: ")

MANUAL = [
    "5  Input methods (Japanese, Chinese) and dead keys (option-e then e): candidate window at the caret, text in the page.",
    "6  Drag and drop with the OS: a file from Finder and a link into the page, a picture and selected text out of it.",
    "10 Picture-in-picture by eye: the video smooth in the Pip window and back in the page.",
    "11 Fullscreen video smooth by eye.",
    "12 A second display: move the window to it; unplug it or change its resolution: page sharp, placed right.",
    "15 Sleep and wake: the page live after wake, not a stale frame.",
    "16 `--shot` (a smoke run's own shot shows a page); the OS screenshot is checked here.",
    "18 On an external 60 Hz display: video at 60 a second (`cef_frames`), `pool_busy 0` (this run prints the numbers on the display it ran on).",
    "By eye throughout: pane corners, glass over pages, no seam; compare with Settings' layer row off.",
]


class Skip(Exception):
    pass


def sh(*cmd, check=True, capture=True):
    r = subprocess.run([str(c) for c in cmd], capture_output=capture, text=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"{' '.join(map(str, cmd))}: {r.stderr.strip()}")
    return r.stdout.strip() if capture else ""


class Log:
    """wubble's stderr, read as it grows; marks are line counts."""

    def __init__(self, path):
        self.path = path

    def lines(self):
        try:
            return self.path.read_text(errors="replace").splitlines()
        except FileNotFoundError:
            return []

    def mark(self):
        return len(self.lines())

    def since(self, mark):
        return self.lines()[mark:]

    @staticmethod
    def routes(lines):
        return [m.groups() for m in map(ROUTE.search, lines) if m]

    @staticmethod
    def summaries(lines):
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


KINDS = ("main", "gpu", "renderer", "other")


class Sampler(threading.Thread):
    """Once a second: each wubble process's CPU time, instructions and cycles retired, energy
    (proc_pid_rusage) and GPU time (the GPU driver's per-client AppUsage in the IO registry, what
    Activity Monitor's GPU column reads), by kind (main, gpu, renderer, other); the whole GPU's
    Device Utilization %; and the other processes using the GPU most (a video call's
    avconferenced, WindowServer), since they share it. No sudo. Rows go to samples.csv."""

    def __init__(self, h, path):
        super().__init__(daemon=True)
        self.h, self.path, self.rows, self.stop_flag = h, path, [], threading.Event()
        path.write_text("t,kind,cpu_pct,ginstr_per_s,gcycles_per_s,gpu_ms_per_s,energy_mj_per_s,gpu_util_pct,others\n")

    def tree(self):
        root = self.h.proc.pid if self.h.proc else None
        if root is None:
            return {}
        procs = []
        for line in sh("ps", "-A", "-o", "pid=,ppid=,command=", check=False).splitlines():
            f = line.split(None, 2)
            if len(f) == 3:
                procs.append((int(f[0]), int(f[1]), f[2]))
        inside, grew = {root}, True
        while grew:
            n = len(inside)
            inside |= {p for p, pp, _ in procs if pp in inside}
            grew = len(inside) > n
        kinds = {}
        for p, _, cmd in procs:
            if p in inside:
                kinds[p] = ("main" if p == root else "gpu" if "--type=gpu-process" in cmd
                            else "renderer" if "--type=renderer" in cmd else "other")
        return kinds

    @staticmethod
    def gpu():
        """({pid: (name, accumulated GPU ns)}, device utilization %)."""
        per = {}
        try:
            data = plistlib.loads(subprocess.run(["ioreg", "-r", "-c", "AGXDeviceUserClient", "-a"],
                                                 capture_output=True).stdout or b"<plist><array/></plist>")
        except Exception:
            data = []
        for c in data:
            m = re.match(r"pid (\d+), (.*)", c.get("IOUserClientCreator", ""))
            if not m:
                continue
            t = sum(u.get("accumulatedGPUTime", 0) for u in c.get("AppUsage", []))
            name, prev = per.get(int(m.group(1)), (m.group(2), 0))
            per[int(m.group(1))] = (name, prev + t)
        util = re.search(r'"Device Utilization %"=(\d+)', sh("ioreg", "-r", "-c", "IOAccelerator", "-l", check=False))
        return per, int(util.group(1)) if util else -1

    def snapshot(self):
        kinds = self.tree()
        ru = {}
        if kinds:
            for line in self.h.h("rusage", *kinds).splitlines():
                pid, cpu, ins, cyc, en = map(int, line.split())
                ru[pid] = (cpu, ins, cyc, en)
        gpu, util = self.gpu()
        return time.time(), kinds, ru, gpu, util

    def run(self):
        prev = None
        while not self.stop_flag.wait(1.0):
            try:
                cur = self.snapshot()
            except Exception:
                prev = None
                continue
            if prev:
                self.add(prev, cur)
            prev = cur

    def add(self, a, b):
        dt = b[0] - a[0]
        row = {"t": b[0], "util": b[4], "kinds": {k: [0.0] * 5 for k in KINDS}}
        for pid, kind in b[1].items():
            acc = row["kinds"][kind]
            if pid in a[2] and pid in b[2]:
                d = [y - x for x, y in zip(a[2][pid], b[2][pid])]
                acc[0] += d[0] / dt / 1e7          # CPU % of one core
                acc[1] += d[1] / dt / 1e9          # G instructions / s
                acc[2] += d[2] / dt / 1e9          # G cycles / s
                acc[4] += d[3] / dt / 1e6          # mJ / s (mW)
            if pid in a[3] and pid in b[3]:
                acc[3] += (b[3][pid][1] - a[3][pid][1]) / dt / 1e6  # GPU ms / s
        others = []
        for pid, (name, t) in b[3].items():
            if pid not in b[1] and pid in a[3]:
                ms = (t - a[3][pid][1]) / dt / 1e6
                if ms >= 5:
                    others.append((ms, name))
        row["others"] = sorted(others, reverse=True)[:3]
        self.rows.append(row)
        with self.path.open("a") as f:
            o = " ".join(f"{n}={ms:.0f}" for ms, n in row["others"])
            for k, v in row["kinds"].items():
                f.write(f"{b[0]:.1f},{k},{v[0]:.1f},{v[1]:.3f},{v[2]:.3f},{v[3]:.1f},{v[4]:.0f},{row['util']},{o}\n")

    def summary(self, t0, t1):
        rows = [r for r in self.rows if t0 < r["t"] <= t1 + 0.5]
        if not rows:
            return "no samples"
        avg = lambda k, i: sum(r["kinds"][k][i] for r in rows) / len(rows)
        cpu = " ".join(f"{k[:4]} {avg(k, 0):.0f}" for k in KINDS[:3])
        gpu = " ".join(f"{k[:4]} {avg(k, 3):.0f}" for k in KINDS[:3])
        ins = sum(avg(k, 1) for k in KINDS)
        cyc = sum(avg(k, 2) for k in KINDS)
        util = sum(r["util"] for r in rows) / len(rows)
        tally = {}
        for r in rows:
            for ms, n in r["others"]:
                tally[n] = tally.get(n, 0) + ms / len(rows)
        top = ", ".join(f"{n} {ms:.0f}" for n, ms in sorted(tally.items(), key=lambda x: -x[1])[:2])
        return (f"CPU% {cpu}; GPU ms/s {gpu}; {ins:.2f} Ginstr/s {cyc:.2f} Gcyc/s; "
                f"GPU busy {util:.0f}%" + (f" (others ms/s: {top})" if top else ""))


class Harness:
    def __init__(self, app, out):
        self.bin = Path(app) / "Contents/MacOS/wubble"
        if not os.access(self.bin, os.X_OK):
            sys.exit(f"mac-checks: no {self.bin}")
        self.out = out
        self.shots = out / "shots"
        self.shots.mkdir(parents=True, exist_ok=True)
        self.profile = out / "profile"
        self.log = Log(out / "wubble.log")
        self.proc = None
        self.win = None  # (id, x, y, w, h) of the main window
        self.n = 0
        self.results = []
        self.helper = out / "helper"
        sh("swiftc", "-O", "-o", self.helper, HERE / "helper.swift")
        trust = dict(l.split() for l in self.h("trust").splitlines())
        missing = [k for k, v in trust.items() if v != "1"]
        if missing:
            sys.exit("mac-checks: grant this terminal " + " and ".join(
                {"accessibility": "Accessibility", "screen": "Screen Recording"}[k] for k in missing)
                + " in System Settings > Privacy & Security, then run it again")

    # --- the helper and the window
    def h(self, *a):
        return sh(self.helper, *a)

    def front(self):
        self.h("activate", self.proc.pid)

    def key(self, chord, wait=0.0):
        self.h("key", chord)
        time.sleep(wait)

    def wins(self, any_level=False):
        out = []
        for line in self.h("wins", self.proc.pid).splitlines():
            i, x, y, w, hh, layer = map(int, line.split())
            if (any_level or layer == 0) and w > 120 and hh > 80:
                out.append((i, x, y, w, hh))
        return out

    def main_window(self, timeout=20):
        end = time.time() + timeout
        while time.time() < end:
            ws = self.wins()
            if ws:
                return max(ws, key=lambda w: w[3] * w[4])
            time.sleep(0.3)
        raise RuntimeError("no wubble window")

    def refresh(self):
        ids = {w[0]: w for w in self.wins()}
        if self.win and self.win[0] in ids:
            self.win = ids[self.win[0]]
        else:
            self.win = self.main_window()
        return self.win

    def shot(self, name, win=None):
        w = win or self.refresh()
        self.n += 1
        p = self.shots / f"{self.n:03d}-{name}.png"
        sh("screencapture", "-x", "-o", "-l", w[0], p)
        return p

    # --- pixels: image pixels in, points out where it says so
    def scale(self, png, win=None):
        w = win or self.win
        return int(self.h("stats", png, 0, 0, 1, 1).split()[-2]) / w[3]

    def bbox(self, png, colour, tol=TOL):
        """(x, y, w, h, fraction) in image pixels, or None."""
        f = self.h("bbox", png, colour, tol).split()
        return None if f[0] == "none" else tuple(map(int, f[:4])) + (float(f[4]),)

    def stats(self, png, rect, colour=None, tol=TOL):
        f = self.h("stats", png, *map(int, rect), *((colour, tol) if colour else ())).split()
        d = dict(zip(f[0::2], f[1::2]))
        return {"black": float(d["black"]), "match": float(d["match"]), "sd": float(d["sd"]), "mean": d["mean"]}

    def px(self, png, x, y):
        return self.h("px", png, int(x), int(y))

    def diff(self, a, b, rect):
        return float(self.h("diff", a, b, *map(int, rect)).split()[0])

    def page_point(self, png, colour, css):
        """Screen point of a CSS position on the page whose background is `colour`."""
        box = self.bbox(png, colour)
        if not box:
            raise RuntimeError(f"page {colour} not on screen")
        s = self.scale(png)
        return (self.win[1] + box[0] / s + css[0], self.win[2] + box[1] / s + css[1])

    def css_px(self, png, colour, css):
        """Image pixel of a CSS position on that page (for px/stats)."""
        box = self.bbox(png, colour)
        s = self.scale(png)
        return (box[0] + css[0] * s, box[1] + css[1] * s)

    @staticmethod
    def near(hexa, hexb, tol=TOL):
        a = [int(hexa[i:i + 2], 16) for i in (0, 2, 4)]
        b = [int(hexb[i:i + 2], 16) for i in (0, 2, 4)]
        return all(abs(x - y) <= tol for x, y in zip(a, b))

    # --- wubble
    def launch(self, url, trace=True):
        env = dict(os.environ)
        if trace:
            env["WUBBLE_TRACE"] = "1"
        f = open(self.log.path, "a")
        self.proc = subprocess.Popen([str(self.bin), "--profile", str(self.profile), url], stdout=f,
                                     stderr=subprocess.STDOUT, env=env, start_new_session=True)
        self.win = None
        self.main_window(30)
        time.sleep(2)
        self.front()
        self.h("ax", self.proc.pid, "pos", WIN[0], WIN[1])
        self.h("ax", self.proc.pid, "size", WIN[2], WIN[3])
        time.sleep(1.5)
        self.refresh()

    def quit(self):
        if self.proc and self.proc.poll() is None:
            os.killpg(self.proc.pid, signal.SIGTERM)
            try:
                self.proc.wait(10)
            except subprocess.TimeoutExpired:
                os.killpg(self.proc.pid, signal.SIGKILL)
                self.proc.wait()
        self.proc = None

    def open_tab(self, url):
        self.key("cmd+t", 1.0)
        self.key("cmd+l", 0.4)
        self.h("type", url)
        self.key("return", 2.5)

    def reset(self, tab="cmd+1"):
        """Back to a plain single pane: out of any split, nothing open, the first tab."""
        self.front()
        for k in ("escape", "escape", "cmd+shift+backslash"):
            self.key(k, 0.3)
        self.key(tab, 1.5)

    def record(self, item, name, ok, why, log_lines=()):
        routes = Log.routes(log_lines)
        sums = Log.summaries(log_lines)
        busy = sum(s["busy"] for s in sums)
        fails = [l for l in log_lines if FAILURE.search(l)]
        note = f"{len(routes)} route lines, pool_busy {busy}"
        if fails:
            ok = False
            note += f", {len(fails)} layer failures: {fails[0][:120]}"
        status = {True: "PASS", False: "FAIL", None: "EYE"}[ok]
        res = self.sampler.summary(self.t0, time.time()) if getattr(self, "sampler", None) else ""
        self.results.append((item, name, status, why, note, res))
        print(f"  {status:4} {item:>3} {name}: {why} ({note})\n           {res}", flush=True)


def check(item, name, needs_video=False):
    def wrap(fn):
        @functools.wraps(fn)
        def run(h, ctx):
            print(f"-- {item} {name}", flush=True)
            if needs_video and not ctx["video"]:
                h.results.append((item, name, "SKIP", "no ffmpeg for the video fixture", "", ""))
                return
            mark = h.log.mark()
            h.t0 = time.time()
            try:
                ok, why = fn(h, ctx)
            except Skip as e:
                h.results.append((item, name, "SKIP", str(e), "", ""))
                return
            except Exception as e:  # a broken step is that check's failure, the run goes on
                ok, why = False, f"error: {e}"
            h.record(item, name, ok, why, h.log.since(mark))
            try:
                stop_video(h, ctx)
                h.reset()
            except Exception:
                pass
        run.item, run.title = item, name
        return run
    return wrap


def layered(h, mark, tabs_at_least=1):
    """The latest summary since `mark`: how many panes are on Layer, and the routes."""
    sums = Log.summaries(h.log.since(mark))
    if not sums:
        return 0, "no summary line"
    tabs = sums[-1]["tabs"]
    n = sum(1 for r in tabs.values() if r == "Layer")
    return n, ", ".join(f"tab {t} {r}" for t, r in tabs.items())


def no_black(h, png, rect, limit=0.03):
    s = h.stats(png, rect)
    return s["black"] <= limit, s["black"]


def content_rect(h, png):
    s = h.scale(png)
    return (0, 0, h.win[3] * s, h.win[4] * s)


# ---------------------------------------------------------------------------------------- checks

@check(16, "OS screenshot shows the page")
def c16(h, ctx):
    p = h.shot("plain")
    box = h.bbox(p, A)
    ok = bool(box and box[4] > 0.3)
    return ok, f"page colour covers {box[4]:.0%} of the window" if box else "page colour not in the shot"


@check(4, "tab switch and closing the shown tab")
def c4(h, ctx):
    bad = []
    for chord, new, old in (("cmd+2", B, A), ("cmd+1", A, B)):
        h.key(chord)
        t0 = time.time()
        for i in range(4):
            p = h.shot(f"switch-{new}-{i}")
            at = time.time() - t0
            black = h.stats(p, ctx["pane"])["black"]
            nb, ob = h.bbox(p, new), h.bbox(p, old)
            if black > 0.03:
                bad.append(f"black {black:.0%} at {at:.2f}s")
            if at > 0.3 and (not nb or nb[4] < 0.3):
                bad.append(f"new page missing at {at:.2f}s")
            if at > 0.3 and ob and ob[4] > 0.02:
                bad.append(f"old page showing at {at:.2f}s")
        time.sleep(0.5)
    h.open_tab(ctx["url"](D))
    h.key("cmd+w")
    t0 = time.time()
    for i in range(4):
        p = h.shot(f"close-{i}")
        at = time.time() - t0
        if h.stats(p, ctx["pane"])["black"] > 0.03:
            bad.append(f"black after close at {at:.2f}s")
        dbox = h.bbox(p, D)
        if at > 0.3 and dbox and dbox[4] > 0.02:
            bad.append(f"closed page still showing at {at:.2f}s")
        if at > 0.3 and not any(h.bbox(p, c) for c in (A, B, C)):
            bad.append(f"no page after close at {at:.2f}s")
    return not bad, "; ".join(bad) or "new page at once, no black, old page gone (4 shots each)"


@check(1, "split view, two pages on two layers")
def c1(h, ctx):
    mark = h.log.mark()
    h.key("cmd+backslash", 2.5)
    p = h.shot("split")
    a, b = h.bbox(p, A), h.bbox(p, B)
    if not a or not b:
        return False, f"pane colours on screen: A {bool(a)}, B {bool(b)}"
    overlap = a[0] + a[2] > b[0] + 4 and b[0] + b[2] > a[0] + 4
    ok_black, black = no_black(h, p, (min(a[0], b[0]), min(a[1], b[1]),
                                      max(a[0] + a[2], b[0] + b[2]) - min(a[0], b[0]), max(a[3], b[3])))
    n, routes = layered(h, mark)
    ctx["split"] = (a, b)
    ok = not overlap and ok_black and a[4] > 0.1 and b[4] > 0.1 and n >= 2
    return ok, f"A {a[4]:.0%} B {b[4]:.0%}, overlap {overlap}, black {black:.1%}; {n} on Layer ({routes})"


@check(3, "split seam drag")
def c3(h, ctx):
    h.key("cmd+backslash", 2.5)
    p = h.shot("seam-before")
    a, b = h.bbox(p, A), h.bbox(p, B)
    if not a or not b:
        return False, "no split to drag"
    s = h.scale(p)
    left, right = (a, b) if a[0] < b[0] else (b, a)
    seam_x = h.win[1] + (left[0] + left[2] + right[0]) / 2 / s
    seam_y = h.win[2] + (left[1] + left[3] / 2) / s
    bad = []
    h.h("down", seam_x, seam_y)
    for i in range(1, 13):
        h.h("drag", seam_x - 12 * i, seam_y)
        time.sleep(0.05)
        if i in (4, 8):
            q = h.shot(f"seam-drag-{i}")
            l2, r2 = h.bbox(q, A if left is a else B), h.bbox(q, B if left is a else A)
            if not l2 or not r2:
                bad.append(f"a pane missing mid-drag ({i})")
                continue
            gap = (l2[0] + l2[2], r2[0] - (l2[0] + l2[2]))
            if gap[1] > 0 and h.stats(q, (gap[0], l2[1], gap[1], l2[3]))["black"] > 0.2:
                bad.append(f"black strip under the seam mid-drag ({i})")
    h.h("up", seam_x - 144, seam_y)
    mark = h.log.mark()
    time.sleep(2)
    q = h.shot("seam-after")
    l2, r2 = h.bbox(q, A if left is a else B), h.bbox(q, B if left is a else A)
    if not l2 or not r2:
        return False, "a pane missing after the drag"
    moved = (left[2] - l2[2]) / s
    gap = (r2[0] - (l2[0] + l2[2])) / s
    if not 60 <= moved <= 230:
        bad.append(f"left pane shrank {moved:.0f} pt for a 144 pt drag")
    if gap > 40:
        bad.append(f"gap {gap:.0f} pt between panes")
    n, routes = layered(h, mark)
    if n < 2:
        bad.append(f"after settling: {routes}")
    return not bad, "; ".join(bad) or f"left pane {moved:.0f} pt narrower, gap {gap:.0f} pt, both on Layer"


@check(2, "sidebar toggle, spring and peek")
def c2(h, ctx):
    h.key("cmd+backslash", 2.5)
    p0 = h.shot("sidebar-before")
    a0 = h.bbox(p0, A)
    bad = []
    h.key("cmd+alt+s")
    t0 = time.time()
    for i in range(5):
        p = h.shot(f"sidebar-spring-{i}")
        at = time.time() - t0
        a, b = h.bbox(p, A), h.bbox(p, B)
        if not a or not b:
            bad.append(f"a pane missing at {at:.2f}s")
        elif h.stats(p, (min(a[0], b[0]), a[1], max(a[0] + a[2], b[0] + b[2]) - min(a[0], b[0]), a[3]))["black"] > 0.03:
            bad.append(f"black in the panes at {at:.2f}s")
    mark = h.log.mark()
    time.sleep(2)
    p1 = h.shot("sidebar-hidden")
    a1 = h.bbox(p1, A)
    if a0 and a1 and abs(a1[0] - a0[0]) < 10:
        bad.append("pane did not move with the sidebar")
    n, routes = layered(h, mark)
    if n < 2:
        bad.append(f"after the spring: {routes}")
    # peek: the pointer at the window's left edge
    mark = h.log.mark()
    h.h("move", h.win[1] + 3, h.win[2] + h.win[4] / 2)
    time.sleep(1.2)
    p2 = h.shot("sidebar-peek")
    s = h.scale(p2)
    peek = h.diff(p1, p2, (0, 0, 320 * s, h.win[4] * s))
    unplaced = [r for r in Log.routes(h.log.since(mark)) if "Unplaced" in r[4]]
    if peek < 0.05:
        bad.append(f"no peek drawn ({peek:.0%} changed)")
    h.h("move", h.win[1] + h.win[3] / 2, h.win[2] + h.win[4] / 2)
    time.sleep(1)
    h.key("cmd+alt+s", 1.5)
    return not bad, "; ".join(bad) or (
        f"panes followed, no black; peek drawn ({peek:.0%} of the strip), "
        f"{'Unplaced route line seen' if unplaced else 'no Unplaced route line'}")


@check("b", "built-in page (Settings) in a split pane")
def cb(h, ctx):
    h.key("cmd+comma", 2.5)  # Settings opens as the last tab, below the video page (tab 3)
    h.key("cmd+3", 1.5)
    h.key("cmd+backslash", 3)
    p = h.shot("settings-split")
    box = h.bbox(p, C)
    s = h.scale(p)
    bad = []
    if box and box[4] >= 0.6:
        # builds up to 0.6.2 refuse it: `Model::split_with` turned internal pages away
        h.key("cmd+4", 1)
        h.key("cmd+w", 1)
        raise Skip("this build does not split a built-in page (Model::split_with refuses internal pages)")
    if not box or not 0.15 < box[4] < 0.6:
        bad.append("the page is not in a pane of its own")
    else:
        rest = (box[0] + box[2] + 16 * s, box[1], h.win[3] * s - (box[0] + box[2] + 16 * s), box[3])
        st = h.stats(p, rest)
        if st["black"] > 0.6 or st["sd"] < 8:
            bad.append(f"Settings pane flat or black (sd {st['sd']:.0f})")
    h.key("cmd+shift+backslash", 1)
    h.key("cmd+4", 1)
    h.key("cmd+w", 1)  # the settings tab
    return (False if bad else None), "; ".join(bad) or f"page and Settings side by side; check {p.name} by eye"


@check("s", "a still page reaches its layer")
def cs(h, ctx):
    mark = h.log.mark()
    h.open_tab(ctx["url"](D) + "&still=1")
    time.sleep(3)
    routes = [r for r in Log.routes(h.log.since(mark))]
    reached = any(r[2] == "Layer" for r in routes)
    last = routes[-1] if routes else None
    h.key("cmd+w", 1)
    return reached, ("on Layer" if reached else
                     f"stays {last[2]} (last route line: {last[1]} -> {last[2]} on {last[3][:50]})" if last else "no route line")


def stop_video(h, ctx):
    """Pause the fixture video if it shows and plays, so leaving its tab opens no Pip window."""
    if not ctx["video"]:
        return
    h.front()
    h.key("escape", 0.3)
    p = h.shot("stop-video")
    if h.bbox(p, C) and h.near(h.px(p, *h.css_px(p, C, PLAY)), GREEN):
        h.h("click", *h.page_point(p, C, PLAY))
        time.sleep(0.5)


def play_video(h, ctx):
    h.key("cmd+3", 1.5)
    p = h.shot("video-tab")
    if not h.near(h.px(p, *h.css_px(p, C, PLAY)), GREEN):
        h.h("click", *h.page_point(p, C, PLAY))
    time.sleep(2)


@check(7, "find bar and command bar over a playing video", needs_video=True)
def c7(h, ctx):
    play_video(h, ctx)
    p0 = h.shot("bars-before")
    s = h.scale(p0)
    box = h.bbox(p0, C)
    vx = box[0] + (VIDEO[0] + VIDEO[2] + 20) * s  # right of the video: page background only
    region = (vx, box[1], box[0] + box[2] - vx, box[3])
    mark = h.log.mark()
    h.key("cmd+f", 1.2)
    p1 = h.shot("find-bar")
    h.key("escape", 0.6)
    h.key("cmd+k", 1.2)
    p2 = h.shot("command-bar")
    h.key("escape", 1.2)
    lines = h.log.since(mark)
    routes = Log.routes(lines)
    sums = Log.summaries(lines)
    ctx["video_seconds"] += sums
    d1, d2 = h.diff(p0, p1, region), h.diff(p0, p2, (box[0], box[1], box[2], box[3] / 2))
    left = [r for r in routes if r[1] == "Layer"]
    bad = []
    if d1 < 0.003:
        bad.append("find bar not seen")
    if d2 < 0.01:
        bad.append("command bar not seen")
    if left:
        bad.append(f"page left its layer: {left[0][1]} -> {left[0][2]} on {left[0][3][:60]}")
    return not bad, "; ".join(bad) or f"both drawn ({d1:.1%}, {d2:.1%} changed), page stayed on its layer"


@check(8, "context menu over the page", needs_video=True)
def c8(h, ctx):
    play_video(h, ctx)
    p0 = h.shot("menu-before")
    x, y = h.page_point(p0, C, QUIET)
    h.h("rclick", x, y)
    time.sleep(0.8)
    p1 = h.shot("context-menu")
    h.key("escape", 0.6)
    s = h.scale(p1)
    ix, iy = h.css_px(p0, C, QUIET)
    d = h.diff(p0, p1, (ix, iy, 220 * s, 260 * s))
    return d > 0.05, f"{d:.0%} of the area by the pointer changed (menu drawn)" if d > 0.05 else "no menu seen"


@check(9, "<select> popup", needs_video=True)
def c9(h, ctx):
    play_video(h, ctx)
    p0 = h.shot("select-before")
    h.h("click", *h.page_point(p0, C, SELECT))
    time.sleep(0.8)
    p1 = h.shot("select-open")
    s = h.scale(p1)
    ix, iy = h.css_px(p0, C, (40, 452))
    d = h.diff(p0, p1, (ix, iy, 200 * s, 90 * s))
    h.key("down", 0.2)
    h.key("return", 0.8)
    p2 = h.shot("select-picked")
    mx, my = h.css_px(p2, C, MARK)
    picked = h.near(h.px(p2, mx, my), GREEN)
    ok = d > 0.05 and picked
    return ok, f"popup under the select {'seen' if d > 0.05 else 'not seen'} ({d:.0%}), pick {'landed' if picked else 'did not land'}"


@check(6, "drag and drop in the page, then the next click", needs_video=True)
def c6(h, ctx):
    play_video(h, ctx)
    p0 = h.shot("dnd-before")
    sx, sy = h.page_point(p0, C, DRAG_SRC)
    zx, zy = h.page_point(p0, C, DROP_ZONE)
    h.h("down", sx, sy)
    for i in range(1, 21):
        h.h("drag", sx + (zx - sx) * i / 20, sy + (zy - sy) * i / 20)
        time.sleep(0.03)
    p_mid = h.shot("dnd-dragging")
    h.h("up", zx, zy)
    time.sleep(0.8)
    p1 = h.shot("dnd-dropped")
    dropped = h.near(h.px(p1, *h.css_px(p1, C, DROP_ZONE)), GREEN)
    h.h("click", *h.page_point(p1, C, AFTER))
    time.sleep(0.6)
    p2 = h.shot("dnd-next-click")
    clicked = h.near(h.px(p2, *h.css_px(p2, C, AFTER)), BLUE)
    return dropped and clicked, f"drop {'landed' if dropped else 'did not land'}, next click {'worked' if clicked else 'did not work'}"


@check(11, "page fullscreen and back", needs_video=True)
def c11(h, ctx):
    play_video(h, ctx)
    p0 = h.shot("fs-before")
    pane0 = h.bbox(p0, C)
    normal = h.win
    h.h("click", *h.page_point(p0, C, FULLSCREEN))
    time.sleep(3)
    mark = h.log.mark()
    time.sleep(4)
    w = h.main_window()
    p1 = h.shot("fullscreen", w)
    s1 = h.stats(p1, (w[3] * 0.3 * h.scale(p1, w), w[4] * 0.3 * h.scale(p1, w),
                      w[3] * 0.4 * h.scale(p1, w), w[4] * 0.4 * h.scale(p1, w)))
    sums = Log.summaries(h.log.since(mark))
    ctx["video_seconds"] += sums
    frames = [x["counts"].get("cef_frames", 0) for x in sums]
    h.key("escape", 3)
    h.win = None
    p2 = h.shot("fs-after")
    pane2 = h.bbox(p2, C)
    bad = []
    if w[3] < normal[3] + 50:
        bad.append(f"window not fullscreen ({w[3]}x{w[4]})")
    if s1["sd"] < 20 or s1["black"] > 0.5:
        bad.append(f"fullscreen centre flat or black (sd {s1['sd']:.0f}, black {s1['black']:.0%})")
    if not pane2 or not pane0 or abs(pane2[0] - pane0[0]) > 8 or abs(pane2[2] - pane0[2]) > 8:
        bad.append("pane not back where it was")
    elif h.stats(p2, pane2[:4])["black"] > 0.03:
        bad.append("black in the pane after exit")
    fps = statistics.median(frames) if frames else 0
    return not bad, "; ".join(bad) or f"fullscreen {w[3]}x{w[4]}, cef_frames median {fps:.0f}/s, pane back"


@check(10, "picture-in-picture on a tab switch, and back", needs_video=True)
def c10(h, ctx):
    play_video(h, ctx)
    return pip_round(h, ctx, "cmd+1", "cmd+3", "pip")


@check("10b", "picture-in-picture from a split pane on a tab switch", needs_video=True)
def c10b(h, ctx):
    play_video(h, ctx)
    h.key("cmd+2", 1.5)
    h.key("cmd+backslash", 2.5)  # B beside the next tab down, the video
    p = h.shot("pip-split")
    if not (h.bbox(p, B) and h.bbox(p, C)):
        raise Skip("could not put the video tab in a split (needs tab 3 next to tab 2)")
    return pip_round(h, ctx, "cmd+1", "cmd+2", "pip-split")


def pip_round(h, ctx, away, back, name):
    main = h.refresh()[0]
    h.key(away, 3)
    others = [w for w in h.wins(any_level=True) if w[0] != main]
    bad = []
    if not others:
        # again with the pointer resting on the video first (the media strip's poll)
        h.key(back, 2)
        q = h.shot(f"{name}-hover")
        if not h.bbox(q, C):
            return False, "no Pip window on a plain switch; video page not back to retry"
        vx, vy = h.page_point(q, C, (VIDEO[0] + VIDEO[2] / 2, VIDEO[1] + VIDEO[3] / 2))
        h.h("move", vx, vy)
        time.sleep(2)
        h.key(away, 3)
        others = [w for w in h.wins(any_level=True) if w[0] != main]
        if not others:
            return False, "no Pip window on a tab switch, with or without the pointer on the video"
        bad.append("Pip window only after the pointer rested on the video, not on a plain switch")
    pip = min(others, key=lambda w: w[3] * w[4])
    p = h.shot(name, pip)
    s = h.stats(p, (0, 0, pip[3] * h.scale(p, pip), pip[4] * h.scale(p, pip)))
    if s["sd"] < 15 or s["black"] > 0.6:
        bad.append(f"Pip window flat or black (sd {s['sd']:.0f}, black {s['black']:.0%})")
    h.key(back, 3)
    left = [w for w in h.wins(any_level=True) if w[0] == pip[0]]
    q = h.shot(f"{name}-back")
    box = h.bbox(q, C)
    if left:
        bad.append("Pip window still open after switching back")
    if not box:
        bad.append("video page not shown after switching back")
    else:
        sc = h.scale(q)
        v = h.stats(q, (box[0] + VIDEO[0] * sc, box[1] + VIDEO[1] * sc, VIDEO[2] * sc, VIDEO[3] * sc))
        if v["sd"] < 15:
            bad.append("video area flat after coming back")
    return not bad, "; ".join(bad) or f"Pip window {pip[3]}x{pip[4]} showing the video, back in the page"


@check(13, "window resize")
def c13(h, ctx):
    pid = h.proc.pid
    bad = []
    for i in range(1, 11):
        h.h("ax", pid, "size", WIN[2] - 30 * i, WIN[3] - 15 * i)
        time.sleep(0.1)
        if i in (3, 7):
            p = h.shot(f"resize-{i}")
            if not h.bbox(p, A) or h.stats(p, content_rect(h, p))["black"] > 0.03:
                bad.append(f"page missing or black mid-resize ({i})")
    mark = h.log.mark()
    time.sleep(2.5)
    p = h.shot("resize-after")
    box = h.bbox(p, A)
    n, routes = layered(h, mark)
    if not box:
        bad.append("page missing after resize")
    if n < 1:
        bad.append(f"not back on the layer 2.5 s after: {routes}")
    h.h("ax", pid, "size", WIN[2], WIN[3])
    time.sleep(2)
    return not bad, "; ".join(bad) or f"page present through the resize, on its layer again ({routes})"


@check(14, "minimise and restore")
def c14(h, ctx):
    if ctx["video"]:
        play_video(h, ctx)
    colour = C if ctx["video"] else A
    pid = h.proc.pid
    h.h("ax", pid, "minimize")
    time.sleep(2)
    mark = h.log.mark()
    h.h("ax", pid, "restore")
    h.front()
    time.sleep(0.4)
    p1 = h.shot("restored-0.4s")
    time.sleep(1.5)
    p2 = h.shot("restored-2s")
    bad = []
    for p in (p1, p2):
        box = h.bbox(p, colour)
        if not box or h.stats(p, box[:4])["black"] > 0.03:
            bad.append(f"page missing or black in {p.name}")
    sums = Log.summaries(h.log.since(mark))
    if ctx["video"] and not any(s["counts"].get("cef_frames", 0) for s in sums):
        bad.append("no cef_frames after restore")
    return not bad, "; ".join(bad) or "page there at once and live"


@check(17, "a private window")
def c17(h, ctx):
    main = h.refresh()[0]
    mark = h.log.mark()
    h.key("cmd+shift+n", 2.5)
    new = [w for w in h.wins() if w[0] != main]
    if not new:
        return False, "no private window opened"
    pw = max(new, key=lambda w: w[3] * w[4])
    h.key("cmd+l", 0.4)
    h.h("type", ctx["url"](B))
    h.key("return", 3)
    p = h.shot("private", pw)
    box = h.bbox(p, B, 60)  # its tint may shift the colour
    sums = Log.summaries(h.log.since(mark))
    tabs = sums[-1]["tabs"] if sums else {}
    h.key("cmd+shift+w", 1.5)
    ok = bool(box and box[4] > 0.3)
    return (ok and None) if ok else False, (
        f"page in the private window ({box[4]:.0%} of it); routes {tabs}; tint and chip: check {p.name} by eye"
        if ok else "page not seen in the private window")


OVERLAYS = [  # (name, keys to open it, keys to close it); each held over the playing video
    ("find bar", ["cmd+f"], ["escape"]),
    ("command bar", ["cmd+k"], ["escape"]),
    ("context menu", ["rclick"], ["escape"]),
    ("zoom toast", ["cmd+equal"], ["cmd+0"]),
]
HOLD = 8


@check("o", "shell popups held over a playing video: no CPU or GPU spike", needs_video=True)
def co(h, ctx):
    """The 0.6.2 fix: glass over a page on its layer blurs a snapshot (4 a second) instead of
    taking the page off its layer and re-rendering the shell every frame."""
    play_video(h, ctx)
    p = h.shot("overlay-video")
    qx, qy = h.page_point(p, C, QUIET)
    h.h("move", qx + 200, qy + 200)

    def window(label):
        mark, t0 = h.log.mark(), time.time()
        time.sleep(HOLD)
        lines = h.log.since(mark)
        rows = [r for r in h.sampler.rows if t0 + 1 < r["t"] <= time.time()]
        n = max(len(rows), 1)
        cpu = sum(sum(r["kinds"][k][0] for k in KINDS) for r in rows) / n
        main = sum(r["kinds"]["main"][0] for r in rows) / n
        gpu = sum(sum(r["kinds"][k][3] for k in KINDS) for r in rows) / n
        sums = Log.summaries(lines)
        med = lambda c: statistics.median([x["counts"].get(c, 0) for x in sums]) if sums else 0
        left = [r for r in Log.routes(lines) if r[1] == "Layer"]
        return {"label": label, "cpu": cpu, "main": main, "gpu": gpu, "renders": med("slint_renders"),
                "snaps": med("layer_snapshots"), "frames": med("cef_frames"), "left": left}

    base = window("video alone")
    out, bad = [base], []
    for name, opens, closes in OVERLAYS:
        for k in opens:
            h.h("rclick", qx, qy) if k == "rclick" else h.key(k)
        time.sleep(0.8)
        h.shot(f"overlay-{name.replace(' ', '-')}")
        w = window(name)
        out.append(w)
        for k in closes:
            h.key(k, 0.8)
        if w["left"]:
            r = w["left"][0]
            bad.append(f"{name}: page left its layer ({r[1]} -> {r[2]} on {r[3][:40]})")
        if w["main"] > base["main"] * 1.5 + 10:
            bad.append(f"{name}: main CPU {w['main']:.0f}% against {base['main']:.0f}% alone")
        if w["gpu"] > base["gpu"] * 1.5 + 30:
            bad.append(f"{name}: wubble GPU {w['gpu']:.0f} ms/s against {base['gpu']:.0f} alone")
        if w["renders"] > 30:
            bad.append(f"{name}: {w['renders']:.0f} slint_renders a second")
    table = "; ".join(f"{w['label']}: main {w['main']:.0f}% tree {w['cpu']:.0f}% GPU {w['gpu']:.0f} ms/s, "
                      f"renders {w['renders']:.0f}/s, snapshots {w['snaps']:.0f}/s, frames {w['frames']:.0f}/s"
                      for w in out)
    ctx["overlays"] = out
    return not bad, ("; ".join(bad) + " | " if bad else "") + table


@check("e", "macOS editing keys in a plain <input>, and Cmd+Left as Back outside one")
def ce(h, ctx):
    h.open_tab(ctx["base"] + "/edit.html")
    p = h.shot("edit")
    if not h.bbox(p, A):
        return False, "edit page not on screen"
    h.h("click", *h.page_point(p, A, (240, 60)))
    time.sleep(0.4)
    bad, seen = [], []

    def step(label, keys, want, key="v"):
        if bad:  # after a failed step the field's state is unknown: the rest would only echo it
            return
        m = event_mark()
        for k in keys:
            if k.startswith("type:"):
                h.h("type", k[5:])
            else:
                h.key(k)
            time.sleep(0.15)
        e = last_event(m, key)
        got = e.get(key) if e else None
        if key == "sel" and e:
            got = f"{e['sel']}/{e['len']}"
        seen.append(f"{label}: {got!r}")
        if got != want:
            shot = h.shot(f"edit-failed-{label.replace(' ', '').replace(',', '-')}")
            bad.append(f"{label}: got {got!r}, want {want!r} (see {shot.name}; later steps not run)")

    step("type", ["type:hello world foo"], "hello world foo")
    step("Option+Delete", ["alt+delete"], "hello world ")
    step("Cmd+Delete", ["cmd+delete"], "")
    step("retype", ["type:hello world foo"], "hello world foo")
    step("Cmd+Left, type", ["cmd+left", "type:X"], "Xhello world foo")
    step("Shift+Cmd+Left", ["cmd+right", "shift+cmd+left"], "0,16/16", key="sel")
    step("Ctrl+A, type", ["right", "ctrl+a", "type:Y"], "YXhello world foo")
    step("Ctrl+E, type", ["ctrl+e", "type:Z"], "YXhello world fooZ")
    step("Ctrl+A, Ctrl+K", ["ctrl+a", "ctrl+k"], "")
    # Cmd+Left outside a text field: Back (from a fresh edit page if a step above went wrong)
    if bad:
        h.reset("cmd+1")
        h.open_tab(ctx["base"] + "/edit.html")
        p = h.shot("edit-again")
    m = event_mark()
    h.h("click", *h.page_point(p, A, (140, 140)))
    nxt = last_event(m, "page", 4)
    if not nxt or nxt["page"] != "next":
        bad.append("the link to the next page did not load")
    else:
        time.sleep(1)
        q = h.shot("edit-next")
        if h.bbox(q, B):
            h.h("click", *h.page_point(q, B, QUIET))  # page background: no field focused
        time.sleep(0.3)
        m = event_mark()
        h.key("cmd+left")
        back = last_event(m, "page", 4)
        seen.append(f"Cmd+Left on the page: {back['page'] if back else 'nothing'}")
        if not back or back["page"] != "edit":
            bad.append("Cmd+Left outside a field did not go Back")
    h.key("cmd+w", 1)
    return not bad, "; ".join(bad) or "; ".join(seen)


@check("r", "CEF frame rate matches the built-in display")
def cr(h, ctx):
    rates = [re.search(r"CEF frame rate (\d+) \(display (.*?) mHz\)", l) for l in h.log.lines()]
    rates = [m for m in rates if m]
    if not rates:
        return False, "no `CEF frame rate` line in the log"
    rate, display = int(rates[-1].group(1)), rates[-1].group(2)
    return rate == 120, f"CEF frame rate {rate} (display {display} mHz); want 120 on the built-in display"


CHECKS = [cr, ce, c16, cs, c4, c1, c3, c2, cb, c7, co, c8, c9, c6, c11, c10, c10b, c13, c14, c17]


# What fixture pages report to `/log?k=v` (an input's value, a selection, a page shown), in order.
EVENTS = []
EVENTS_LOCK = threading.Lock()


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        if self.path.startswith("/log?"):
            q = urllib.parse.parse_qs(self.path[5:], keep_blank_values=True)
            with EVENTS_LOCK:
                EVENTS.append({k: v[0] for k, v in q.items()})
            self.send_response(204)
            self.end_headers()
            return
        super().do_GET()


def event_mark():
    with EVENTS_LOCK:
        return len(EVENTS)


def last_event(mark, key, wait=1.5):
    """The latest event since `mark` carrying `key`, waiting up to `wait` s for one."""
    end = time.time() + wait
    while True:
        with EVENTS_LOCK:
            hits = [e for e in EVENTS[mark:] if key in e]
        if hits or time.time() > end:
            return hits[-1] if hits else None
        time.sleep(0.1)


def fixture(dirpath):
    dirpath.mkdir(parents=True, exist_ok=True)
    (dirpath / "page.html").write_text(PAGE)
    (dirpath / "edit.html").write_text(EDIT)
    clip = dirpath / "clip.webm"
    try:
        sh("ffmpeg", "-hide_banner", "-loglevel", "error", "-y",
           "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=60:duration=12",
           "-f", "lavfi", "-i", "sine=frequency=330:duration=12",
           "-c:v", "libvpx-vp9", "-b:v", "3M", "-deadline", "realtime", "-cpu-used", "8", "-row-mt", "1",
           "-g", "120", "-c:a", "libopus", "-b:a", "48k", "-af", "volume=0.08", clip)
        return True
    except (RuntimeError, FileNotFoundError):
        return False


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("app", help="path to wubble.app")
    ap.add_argument("--out", help="results folder (default ~/wubble-mac-checks/<time>)")
    ap.add_argument("--flame", type=int, default=0, metavar="SECONDS",
                    help="after the checks, profile wubble's main, GPU and renderer processes with `sample` "
                         "for SECONDS per scenario (video alone; find bar over it) and draw flame graphs "
                         "(needs FlameGraph: --flamegraph-dir, cloned once)")
    ap.add_argument("--flamegraph-dir", default=str(Path.home() / "wubble-mac-checks/tools/FlameGraph"),
                    help="a clone of https://github.com/brendangregg/FlameGraph")
    ap.add_argument("--only", help="comma-separated items to run, e.g. 1,4,10b")
    a = ap.parse_args()
    out = Path(a.out) if a.out else Path.home() / "wubble-mac-checks" / datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    out.mkdir(parents=True, exist_ok=True)
    h = Harness(a.app, out)
    video = fixture(out / "fixture")
    os.chdir(out / "fixture")
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Quiet)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_address[1]
    url = lambda c: f"http://127.0.0.1:{port}/page.html?c={c}" + ("&v=1" if c == C else "")
    ctx = {"video": video, "url": url, "video_seconds": [], "base": f"http://127.0.0.1:{port}"}
    version = sh(h.bin, "--version", check=False)
    print(f"mac-checks: {version} -> {out}", flush=True)
    try:
        # a first run creates the profile; then picture-in-picture on tab switch is turned on
        h.launch(url(A), trace=False)
        h.quit()
        db = h.profile / "wubble.db"
        with sqlite3.connect(db) as con:
            con.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('pip.on_tab_switch', '1')")
        h.launch(url(A))
        h.sampler = Sampler(h, out / "samples.csv")
        h.sampler.start()
        h.t0 = time.time()
        time.sleep(6)
        idle = h.sampler.summary(h.t0, time.time())
        print(f"idle, one still page: {idle}", flush=True)
        ctx["idle"] = idle
        for c in (B, C):
            h.open_tab(url(c))
        h.key("cmd+1", 2)
        p = h.shot("start")
        box = h.bbox(p, A)
        if not box:
            sys.exit(f"mac-checks: the fixture page is not on screen; see {p}")
        ctx["pane"] = box[:4]
        only = set(a.only.split(",")) if a.only else None
        for fn in CHECKS:
            if only and str(fn.item) not in only:
                continue
            fn(h, ctx)
        if a.flame and ctx["video"]:
            flames(h, ctx, a.flame, Path(a.flamegraph_dir))
    except KeyboardInterrupt:
        print("mac-checks: stopped", flush=True)
    finally:
        if getattr(h, "sampler", None):
            h.sampler.stop_flag.set()
        h.quit()
        server.shutdown()
        # the throwaway profile (CEF's cache) and the fixture clip: only the log and shots are kept
        shutil.rmtree(h.profile, ignore_errors=True)
        shutil.rmtree(out / "fixture", ignore_errors=True)
        h.helper.unlink(missing_ok=True)
    write_report(h, ctx, out, version)


def flames(h, ctx, secs, fg):
    """`sample` on the main process, the GPU process and the busiest renderer, at once, per
    scenario; folded per process and together (each stack under its process's name). Symbols
    come from the build's dSYM when Spotlight can find it (`mdfind` its UUID)."""
    collapse, draw = fg / "stackcollapse-sample.awk", fg / "flamegraph.pl"
    if not collapse.exists() or not draw.exists():
        print(f"mac-checks: no FlameGraph in {fg}; skipping flame graphs", flush=True)
        return
    out = h.out / "flame"
    out.mkdir(exist_ok=True)
    for scenario, opens, closes in (("video", [], []), ("video-find-bar", ["cmd+f"], ["escape"])):
        print(f"-- flame: {scenario} ({secs} s)", flush=True)
        h.reset("cmd+3")
        play_video(h, ctx)
        for k in opens:
            h.key(k, 1)
        kinds = h.sampler.tree()
        cpu = {}
        for line in h.h("rusage", *kinds).splitlines():
            pid, c = map(int, line.split()[:2])
            cpu[pid] = c
        pick = {"main": h.proc.pid}
        for kind in ("gpu", "renderer"):
            cands = [p for p, k in kinds.items() if k == kind]
            if cands:
                pick[kind] = max(cands, key=lambda p: cpu.get(p, 0))
        procs = {k: subprocess.Popen(["sample", str(p), str(secs), "-mayDie", "-file", str(out / f"{scenario}-{k}.sample.txt")],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                 for k, p in pick.items()}
        for p in procs.values():
            p.wait()
        for k in closes:
            h.key(k, 0.5)
        stop_video(h, ctx)
        draw_flames(out, scenario, list(pick), fg)
    print(f"flame graphs in {out}", flush=True)


# A stack whose leaf is one of these is a thread waiting, not working: left out of the on-CPU graphs.
WAITS = re.compile(r"(mach_msg2_trap|__psynch_cvwait|__psynch_mutexwait|__workq_kernreturn|kevent|kevent64|kevent_id|__accept|"
                   r"__semwait_signal|__ulock_wait|__select|poll|__recvfrom|read|__open|__sigsuspend|"
                   r"mach_wait_until|semaphore_wait_trap|semaphore_timedwait_trap|__wait4|start_wqthread|"
                   r"thread_start)$")


def draw_flames(out, scenario, kinds, fg):
    """`sample`'s text per process -> folded stacks -> per-process and combined SVGs: `-all` every
    thread (waits included, so it adds up to wall time), `-cpu` only stacks doing work. Rust names
    go through rustfilt when it is on PATH. CEF's framework ships stripped (exported symbols only):
    its frames are named after the nearest export, so read them as "somewhere in CEF"."""
    collapse, draw = fg / "stackcollapse-sample.awk", fg / "flamegraph.pl"
    demangle = shutil.which("rustfilt") or str(Path.home() / ".cargo/bin/rustfilt")
    together = []
    for k in kinds:
        txt = out / f"{scenario}-{k}.sample.txt"
        if not txt.exists():
            continue
        folded = subprocess.run(["awk", "-f", str(collapse), str(txt)], capture_output=True, text=True).stdout
        if os.access(demangle, os.X_OK):
            folded = subprocess.run([demangle], input=folded, capture_output=True, text=True).stdout
        (out / f"{scenario}-{k}.folded").write_text(folded)
        together += [f"{k};{line}" for line in folded.splitlines() if line.strip()]
    busy = [l for l in together if not WAITS.search(l.rsplit(" ", 1)[0].rsplit(";", 1)[-1].split("`")[-1])]
    for name, lines, what in (("all", together, "every thread, waits included"),
                              ("cpu", busy, "on CPU: waiting stacks left out")):
        with open(out / f"{scenario}-{name}.svg", "w") as f:
            subprocess.run(["perl", str(draw), "--title", f"wubble {scenario}: main, GPU and renderer ({what})",
                            "--width", "1800", "--minwidth", "0.5"],
                           input="\n".join(lines) + "\n", stdout=f, text=True)
        for k in kinds:
            mine = [l.split(";", 1)[1] for l in lines if l.startswith(k + ";")]
            with open(out / f"{scenario}-{k}-{name}.svg", "w") as f:
                subprocess.run(["perl", str(draw), "--title", f"wubble {scenario}: {k} process ({what})",
                                "--width", "1800", "--minwidth", "0.5"],
                               input="\n".join(mine) + "\n", stdout=f, text=True)


def write_report(h, ctx, out, version):
    lines = h.log.lines()
    fails = [l for l in lines if FAILURE.search(l)]
    panics = [l for l in lines if "panicked" in l]
    vs = ctx["video_seconds"]
    frames = [s["counts"].get("cef_frames", 0) for s in vs]
    rows = ["| item | check | result | why | log | resources (mean over the check) |", "|---|---|---|---|---|---|"]
    rows += [f"| {i} | {n} | {s} | {w} | {l} | {r} |" for i, n, s, w, l, r in h.results]
    text = [f"# mac-checks, {version}, {datetime.datetime.now():%Y-%m-%d %H:%M}", "",
            "PASS and FAIL are judged from the trace and the pixels; EYE passed what can be measured, the rest is a look at its shot.", "",
            *rows, "",
            f"Idle, one still page, before the checks: {ctx.get('idle', 'not measured')}.",
            "Resources: CPU % of one core and GPU ms per second for wubble's main, GPU and renderer processes; "
            "instructions and cycles retired by its whole tree; GPU busy is the whole GPU (Device Utilization %), "
            "shared with every app: the other processes using it most are named with their GPU ms per second. "
            "Per-second rows in samples.csv.", "",
            f"Whole run: {len(fails)} page layer failures, {len(panics)} panics, {len(Log.routes(lines))} route lines.",
            f"Video seconds measured: {len(vs)}; cef_frames median {statistics.median(frames) if frames else 0:.0f}, "
            f"min {min(frames) if frames else 0}; pool_busy total {sum(s['busy'] for s in vs)} (item 18 on this display).", "",
            "## By hand", "", *[f"- {m}" for m in MANUAL], "",
            f"Shots: {h.shots}; log: {h.log.path}"]
    (out / "results.md").write_text("\n".join(text) + "\n")
    print("\n".join(text))


if __name__ == "__main__":
    main()
