#!/usr/bin/env python3
"""wubble against Firefox on Linux, state by state: the Mac harness's states
(scripts/mac-checks on branch mac-checks) on the invisible display. Hand run only.

    ~/Dev/wubble/.local/bin/run.sh timeout 900 scripts/headless.sh \\
      python3 scripts/bench/linux-vs-firefox.py --wubble target/release/wubble

States: a still page (idle), a looping 1080p60 VP9 video, the video under the find bar and under
the command bar (Firefox: its find bar, its address bar), scrolling a long page (wheel notches),
typing into a textarea. Each runs `--seconds` (default 20) in a fresh launch of each browser on a
throwaway profile, after the page reports it loaded and `--warmup` seconds more; wubble runs
with WUBBLE_TRACE=1, Firefox with telemetry, updates and every Mozilla service off (user.js).

Input: mutter's RemoteDesktop on the headless display's own bus (found through its mutter
process; never the desktop's), a virtual keyboard and pointer that reach Wayland windows
(xdotool would reach only X11 ones). It needs python3-gobject, as headless.sh does.

Once a second: CPU % of one core per process group of the browser's tree (main, GPU process,
renderer/content, other) from /proc, the tree's RSS and PSS, the GPU (nvidia-smi: whole-GPU
busy and board power, the tree's own SM % from pmon where the driver gives it; else sysfs
gpu_busy_percent) and package power (powercap RAPL, where energy_uj is readable). From wubble's
trace: cef_frames and slint_renders a second (printed only on the page-layer path: not on Linux).
From the pages: video frames decoded and dropped, how far the page scrolled, how much text
arrived, whether the find and command keys took the page's focus.

Results go to --out (default ~/Dev/wubble/.local/bench/<time>/): samples.csv (one row a second),
results.md (per state, wubble against Firefox: median and p90, verdict), logs/. Nothing is
asserted: numbers for docs/measurements.md. Standard library only.
"""
import argparse
import csv
import datetime
import glob
import json
import os
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

import common

HERE = Path(__file__).resolve().parent
TICK = os.sysconf("SC_CLK_TCK")
KINDS = ("main", "gpu", "renderer", "other")
SIZE = tuple(int(x) for x in os.environ.get("WUBBLE_HEADLESS_SIZE", "1360x880").split("x"))

# --- the fixture pages; each says when it loaded and what it saw, through `/log`
HEAD = """<!doctype html><meta charset=utf-8><title>%s</title><style>body{margin:24px;font:16px sans-serif;background:#f4f4f4}</style>
<script>const log = q => fetch('/log?' + new URLSearchParams(q)).catch(() => {});
const every = (ms, f) => { let last = 0; return (...a) => { const n = Date.now(); if (n - last >= ms) { last = n; f(...a); } }; };
addEventListener('load', () => log({page: document.title}));
addEventListener('blur', () => log({blur: innerHeight}));</script>
"""
PROSE = "<p>" + "A still paragraph of text, nothing on the page moves or asks for frames. " * 6 + "</p>"
PAGES = {
    "still": PROSE * 4,
    # video-page.sh's clip and shape: 1080p60 VP9 looping muted at 854x480
    "video": """<video id=v src="clip-1080p60.webm" autoplay muted loop playsinline style="width:854px;height:480px;display:block"></video>
<script>const v = document.getElementById('v');
v.addEventListener('timeupdate', every(1000, () => { const q = v.getVideoPlaybackQuality();
  log({video: q.totalVideoFrames, dropped: q.droppedVideoFrames}); }));</script>""",
    "long": "".join(f"<h2>Section {i}</h2>{PROSE}<div style='height:120px;background:hsl({i * 7 % 360} 60% 70%)'></div>"
                    for i in range(300))
            + "<script>addEventListener('scroll', every(1000, () => log({y: Math.round(scrollY)})));</script>",
    "type": """<textarea id=t autofocus spellcheck=false style="box-sizing:border-box;width:100%;height:80vh;font:16px monospace"></textarea>
<script>const t = document.getElementById('t'); addEventListener('load', () => t.focus());
t.addEventListener('input', every(1000, () => log({len: t.value.length})));</script>""",
}

# --- the states: (page, keys once it shows, per browser; what drives it while it is measured)
STATES = {
    "idle": ("still", {}, None),
    "video": ("video", {}, None),
    "video+find": ("video", {"wubble": "ctrl+f", "firefox": "ctrl+f"}, None),
    "video+command": ("video", {"wubble": "ctrl+k", "firefox": "ctrl+l"}, None),
    "scroll": ("long", {}, "scroll"),
    "typing": ("type", {}, "type"),
}
TEXT = "the quick brown fox jumps over the lazy dog and types on\n"

# Firefox on a fresh profile: no first-run pages, no telemetry, updates or Mozilla services.
FIREFOX_PREFS = {
    "browser.shell.checkDefaultBrowser": False,
    "browser.startup.homepage_override.mstone": "ignore",
    "startup.homepage_welcome_url": "",
    "startup.homepage_welcome_url.additional": "",
    "browser.aboutwelcome.enabled": False,
    "datareporting.policy.firstRunURL": "",
    "datareporting.policy.dataSubmissionEnabled": False,
    "datareporting.healthreport.uploadEnabled": False,
    "toolkit.telemetry.enabled": False,
    "toolkit.telemetry.unified": False,
    "toolkit.telemetry.archive.enabled": False,
    "toolkit.telemetry.server": "",
    "app.shield.optoutstudies.enabled": False,
    "app.normandy.enabled": False,
    "app.update.auto": False,
    "app.update.disabledForTesting": True,
    "extensions.update.enabled": False,
    "extensions.systemAddon.update.enabled": False,
    "extensions.getAddons.cache.enabled": False,
    "extensions.pocket.enabled": False,
    "browser.search.update": False,
    "browser.region.update.enabled": False,
    "browser.region.network.url": "",
    "browser.safebrowsing.malware.enabled": False,
    "browser.safebrowsing.phishing.enabled": False,
    "browser.safebrowsing.downloads.enabled": False,
    "browser.safebrowsing.provider.mozilla.updateURL": "",
    "browser.safebrowsing.provider.google4.updateURL": "",
    "browser.newtabpage.enabled": False,
    "browser.newtabpage.activity-stream.feeds.telemetry": False,
    "browser.newtabpage.activity-stream.telemetry": False,
    "browser.urlbar.quicksuggest.enabled": False,
    "browser.discovery.enabled": False,
    "network.captive-portal-service.enabled": False,
    "network.connectivity-service.enabled": False,
    "network.prefetch-next": False,
    "network.dns.disablePrefetch": True,
    "geo.provider.network.url": "",
    "media.gmp-manager.url": "",
    "media.gmp-gmpopenh264.autoupdate": False,
    "services.settings.server": "http://127.0.0.1:9/",  # remote settings: nowhere
    "browser.sessionstore.resume_from_crash": False,
    "browser.tabs.warnOnClose": False,
    "media.autoplay.default": 0,
}

# --- the browsers: command, environment, process groups (matched on comm, then the command line)
BROWSERS = {
    "wubble": {
        "cmd": lambda b, prof, url: [b, "--profile", prof, url],
        "env": {"WUBBLE_TRACE": "1"},
        # the zygote's children keep its command line: the GPU process has GpuWatchdog, a
        # renderer (the sandboxed zygote's) the child IO thread a zygote itself lacks
        "kinds": (("gpu", ("--type=gpu-process",)), ("gpu", ("GpuWatchdog",)), ("renderer", ("--type=renderer",)),
                  ("renderer", ("--type=zygote", "Chrome_ChildIOT"))),
    },
    "firefox": {
        # wubble's first window is 1280x800 (kit derived.slint), so Firefox's is too
        "cmd": lambda b, prof, url: [b, "--no-remote", "-profile", prof, "--width", "1280", "--height", "800", url],
        "env": {"MOZ_ENABLE_WAYLAND": "1"},
        # no GPU process on Linux (WebRender composites in the main process); video decodes in RDD (other)
        "kinds": (("gpu", ("GPU Process",)), ("renderer", ("Isolated Web Co",)), ("renderer", ("Web Content",)),
                  ("renderer", ("file:// Content",))),
        "prefs": FIREFOX_PREFS,
    },
}

# The seat: a RemoteDesktop session on the headless mutter's bus; ops come one per line on stdin.
SEAT = r"""
import sys
from gi.repository import Gio, GLib
bus = Gio.DBusConnection.new_for_address_sync(sys.argv[1],
    Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)
RD = "org.gnome.Mutter.RemoteDesktop"
def call(path, iface, method, sig=None, args=(), reply=None):
    return bus.call_sync(RD, path, iface, method, GLib.Variant(sig, args) if sig else None,
                         GLib.VariantType(reply) if reply else None, 0, 5000, None)
session = call("/org/gnome/Mutter/RemoteDesktop", RD, "CreateSession", reply="(o)").unpack()[0]
S = RD + ".Session"
call(session, S, "Start")
OPS = {
    "key": ("NotifyKeyboardKeysym", "(ub)", lambda a: (int(a[0]), a[1] == "1")),
    "wheel": ("NotifyPointerAxisDiscrete", "(ui)", lambda a: (0, int(a[0]))),
    "move": ("NotifyPointerMotionRelative", "(dd)", lambda a: (float(a[0]), float(a[1]))),
    "button": ("NotifyPointerButton", "(ib)", lambda a: (int(a[0]), a[1] == "1")),
}
for line in sys.stdin:
    op, *a = line.split()
    method, sig, conv = OPS[op]
    call(session, S, method, sig, conv(a))
call(session, S, "Stop")
"""
KEYSYMS = {"ctrl": 0xFFE3, "shift": 0xFFE1, "alt": 0xFFE9, "escape": 0xFF1B, "\n": 0xFF0D}
BTN_LEFT = 0x110


class Seat:
    def __init__(self):
        name = os.environ.get("WAYLAND_DISPLAY", "")
        if not name.startswith("wubble-headless-"):
            sys.exit("linux-vs-firefox: run it inside scripts/headless.sh (see --help)")
        self.p = subprocess.Popen(["python3", "-I", "-c", SEAT, self.bus(name)], stdin=subprocess.PIPE, text=True)
        self.send(("move", -10000, -10000), ("move", SIZE[0] // 2, SIZE[1] // 2))  # the display's centre

    @staticmethod
    def bus(name):
        """The headless mutter's session bus: from the environment of the mutter serving `name`."""
        for pid in filter(str.isdigit, os.listdir("/proc")):
            try:
                with open(f"/proc/{pid}/cmdline", "rb") as f:
                    args = f.read().decode(errors="replace").split("\0")
                if "mutter" not in os.path.basename(args[0]) or name not in args:
                    continue
                with open(f"/proc/{pid}/environ", "rb") as f:
                    env = dict(e.split("=", 1) for e in f.read().decode(errors="replace").split("\0") if "=" in e)
                return env["DBUS_SESSION_BUS_ADDRESS"]
            except (OSError, KeyError):
                continue
        sys.exit(f"linux-vs-firefox: no mutter serving {name}")

    def send(self, *ops):
        self.p.stdin.write("".join(" ".join(map(str, op)) + "\n" for op in ops))
        self.p.stdin.flush()

    def key(self, chord):
        syms = [KEYSYMS.get(k) or ord(k) for k in ([chord] if len(chord) == 1 else chord.split("+"))]
        self.send(*[("key", s, 1) for s in syms], *[("key", s, 0) for s in reversed(syms)])

    def click(self):
        self.send(("move", 1, 1), ("move", -1, -1), ("button", BTN_LEFT, 1), ("button", BTN_LEFT, 0))

    def close(self):
        self.p.stdin.close()
        self.p.wait(5)


def drive_scroll(seat, secs):
    """Ten wheel notches a second, the direction turned every 5 s."""
    end, step, turn = time.monotonic() + secs, 1, time.monotonic() + 5
    while time.monotonic() < end:
        seat.send(("wheel", step))
        time.sleep(0.1)
        if time.monotonic() >= turn:
            step, turn = -step, turn + 5


def drive_type(seat, secs):
    """Ten keys a second, lines of lower-case words."""
    end, i = time.monotonic() + secs, 0
    while time.monotonic() < end:
        seat.key(TEXT[i % len(TEXT)])
        i += 1
        time.sleep(0.1)


DRIVERS = {None: lambda seat, secs: time.sleep(secs), "scroll": drive_scroll, "type": drive_type}


class Gpu:
    """NVIDIA: two nvidia-smi streams (whole-GPU busy % and board W each second; per-process SM %
    from pmon); else the first readable sysfs gpu_busy_percent (amdgpu). None where nothing reads."""

    def __init__(self):
        self.util = self.watts = None
        self.sm, self.procs = {}, []
        self.sysfs = [p for p in sorted(glob.glob("/sys/class/drm/card*/device/gpu_busy_percent")) if os.access(p, os.R_OK)]
        if shutil.which("nvidia-smi"):
            self.stream(["nvidia-smi", "--query-gpu=utilization.gpu,power.draw", "--format=csv,noheader,nounits",
                         "-lms", "1000"], self.on_query)
            self.stream(["nvidia-smi", "pmon", "-s", "u", "-d", "1"], self.on_pmon)

    def stream(self, cmd, fn):
        p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.procs.append(p)
        threading.Thread(target=lambda: [fn(line) for line in p.stdout], daemon=True).start()

    @staticmethod
    def num(s):
        try:
            return float(s)
        except ValueError:
            return None

    def on_query(self, line):
        f = [x.strip() for x in line.split(",")]
        if len(f) == 2:
            self.util, self.watts = self.num(f[0]), self.num(f[1])

    def on_pmon(self, line):
        f = line.split()
        if len(f) > 3 and not line.startswith("#") and f[1].isdigit():
            self.sm[int(f[1])] = (time.time(), self.num(f[3]))

    def read(self, pids):
        """(busy %, W, the tree's SM %) now."""
        util = self.util
        if util is None and self.sysfs:
            with open(self.sysfs[0]) as f:
                util = float(f.read())
        now = time.time()
        own = [v for p, (t, v) in list(self.sm.items()) if p in pids and now - t < 2.5 and v is not None]
        return util, self.watts, sum(own) if own else None

    def sources(self):
        return ("nvidia-smi" if self.procs else "sysfs gpu_busy_percent" if self.sysfs else None)

    def close(self):
        for p in self.procs:
            p.terminate()


class Rapl:
    """Package power from powercap (`intel-rapl:N`; AMD's RAPL driver registers there too); none
    when energy_uj is root-only, as Fedora ships it."""

    def __init__(self):
        self.zones = []
        for d in sorted(glob.glob("/sys/class/powercap/intel-rapl:[0-9]")):
            try:
                self.zones.append((d, int(Path(d, "max_energy_range_uj").read_text()), int(Path(d, "energy_uj").read_text())))
            except (OSError, ValueError):
                pass
        self.last = time.monotonic()

    def watts(self):
        if not self.zones:
            return None
        now, total, zones = time.monotonic(), 0, []
        for d, top, prev in self.zones:
            e = int(Path(d, "energy_uj").read_text())
            total += (e - prev) % top
            zones.append((d, top, e))
        dt, self.last, self.zones = now - self.last, now, zones
        return total / 1e6 / dt


class Sampler(threading.Thread):
    """One row a second for the tree under `root`: CPU % of one core per kind, RSS, PSS, GPU, package W."""

    def __init__(self, root, kinds, gpu, rapl, count):
        super().__init__(daemon=True)
        self.root, self.rules, self.gpu, self.rapl, self.count = root, kinds, gpu, rapl, count
        self.rows, self.kind_of, self.stop_flag = [], {}, threading.Event()

    def kind(self, pid):
        """Kept once known; "other" is asked again (a new process's threads start after it)."""
        if self.kind_of.get(pid, "other") == "other":
            who = common.ident(pid)
            self.kind_of[pid] = "main" if pid == self.root else \
                next((k for k, needles in self.rules if all(n in who for n in needles)), "other")
        return self.kind_of[pid]

    def run(self):
        prev, t0 = common.tree(self.root), time.monotonic()
        self.rapl.watts()
        while len(self.rows) < self.count and not self.stop_flag.wait(1.0):
            cur, t1 = common.tree(self.root), time.monotonic()
            dt, row = t1 - t0, {f"cpu_{k}": 0.0 for k in KINDS}
            for pid, ticks in cur.items():
                row[f"cpu_{self.kind(pid)}"] += (ticks - prev.get(pid, 0)) / TICK / dt * 100
            mem = [common.memory(p) for p in cur]
            util, watts, sm = self.gpu.read(set(cur))
            row |= {"cpu_total": sum(row[f"cpu_{k}"] for k in KINDS), "procs": len(cur),
                    "rss_mb": sum(m[0] for m in mem) / 1024, "pss_mb": sum(m[1] for m in mem) / 1024,
                    "gpu_util": util, "gpu_sm": sm, "gpu_w": watts, "pkg_w": self.rapl.watts()}
            self.rows.append(row)
            prev, t0 = cur, t1


COLS = ["round", "state", "browser", "sec", "cpu_total"] + [f"cpu_{k}" for k in KINDS] + \
       ["procs", "rss_mb", "pss_mb", "gpu_util", "gpu_sm", "gpu_w", "pkg_w"]


def write_prefs(prof, prefs):
    Path(prof, "user.js").write_text("".join(f'user_pref("{k}", {json.dumps(v)});\n' for k, v in prefs.items()))


def stop(p):
    for sig, wait in ((signal.SIGTERM, 15), (signal.SIGKILL, 5)):
        try:
            os.killpg(p.pid, sig)
            p.wait(wait)
            return
        except (ProcessLookupError, subprocess.TimeoutExpired):
            continue


def rate(events, key):
    """Per second of a counter the page reports, over the measured window."""
    if len(events) < 2:
        return None
    return (int(events[-1][key]) - int(events[0][key])) / (events[-1]["_t"] - events[0]["_t"])


def run_one(a, ctx, browser, state, rnd):
    page, keys, driver = STATES[state]
    b = BROWSERS[browser]
    prof = tempfile.mkdtemp(prefix=f"bench-{browser}-")
    if "prefs" in b:
        write_prefs(prof, b["prefs"])
    logpath = ctx["out"] / "logs" / f"{browser}-{state}-{rnd}.log"
    loop, seat, notes = ctx["loop"], ctx["seat"], []
    mark = loop.mark()
    with logpath.open("w") as log:
        p = subprocess.Popen(b["cmd"](ctx["bin"][browser], prof, f"{loop.base}/{page}.html"),
                             env=os.environ | b["env"], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    try:
        end = time.time() + 60
        while not any(e["page"] == page for e in loop.since(mark, "page")) and time.time() < end and p.poll() is None:
            time.sleep(0.2)
        if not any(e["page"] == page for e in loop.since(mark, "page")):
            notes.append("the page never loaded: not measured")
            return [], notes
        seat.click()  # focus: the window, and on the typing page its textarea
        time.sleep(a.warmup)
        if browser in keys:  # a bar that takes the keyboard takes it from the page: its window blurs
            k = loop.mark()
            seat.key(keys[browser])
            time.sleep(2)
            notes.append(f"{keys[browser]} took the page's focus" if loop.since(k, "blur")
                         else f"{keys[browser]}: THE PAGE KEPT ITS FOCUS (no bar took the keyboard?)")
        else:
            time.sleep(2)
        lines = len(logpath.read_text(errors="replace").splitlines())
        ev = loop.mark()
        sampler = Sampler(p.pid, b["kinds"], ctx["gpu"], ctx["rapl"], a.seconds)
        sampler.start()
        DRIVERS[driver](seat, a.seconds)
        sampler.join(3)  # its last second
        sampler.stop_flag.set()
        sampler.join()
        with logpath.open("a") as log:  # how the tree was grouped, to check the kind rules
            log.writelines(f"bench: {k} {pid} {common.ident(pid).split(chr(0))[0]}\n"
                           for pid, k in sorted(sampler.kind_of.items()))
        # what the page and the trace saw in the window
        video = loop.since(ev, "video")
        if page == "video":
            fps, drop = rate(video, "video"), rate(video, "dropped")
            notes.append("video: no report" if fps is None else f"video {fps:.0f} frames/s decoded, {drop:.1f} dropped/s")
        if driver == "scroll":
            ys = [int(e["y"]) for e in loop.since(ev, "y")]
            notes.append(f"scrolled to y {min(ys)}..{max(ys)}" if ys else "NOT SCROLLED (no wheel reached the page)")
        if driver == "type":
            got = loop.since(ev, "len")
            notes.append(f"{got[-1]['len']} characters reached the page" if got else "NOTHING TYPED reached the page")
        sums = common.summaries(logpath.read_text(errors="replace").splitlines()[lines:])
        if sums:  # wubble's, printed only on the page-layer path (none on Linux, Decided "Page layer default")
            med = lambda k: statistics.median(s["counts"].get(k, 0) for s in sums)
            notes.append(f"cef_frames {med('cef_frames'):.0f}/s, slint_renders {med('slint_renders'):.0f}/s")
            ctx["summaries"] = True
        for i, row in enumerate(sampler.rows):
            ctx["csv"].writerow({"round": rnd, "state": state, "browser": browser, "sec": i + 1} | row)
        ctx["csvfile"].flush()
        return sampler.rows, notes
    finally:
        stop(p)
        shutil.rmtree(prof, ignore_errors=True)
        time.sleep(2)


def fmt(v):
    return "n/a" if v is None else f"{v:.1f}" if v < 100 else f"{v:.0f}"


def p90(xs):
    return statistics.quantiles(xs, n=10)[-1] if len(xs) > 1 else xs[0]


# (column, sample key, statistic, counts in the verdict); lower is better for each
METRICS = [
    ("CPU % median", "cpu_total", statistics.median, True),
    ("CPU % p90", "cpu_total", p90, True),
    ("main", "cpu_main", statistics.median, False),
    ("GPU proc", "cpu_gpu", statistics.median, False),
    ("renderer", "cpu_renderer", statistics.median, False),
    ("other", "cpu_other", statistics.median, False),
    ("PSS MB", "pss_mb", statistics.median, True),
    ("RSS MB", "rss_mb", statistics.median, False),
    ("GPU busy %", "gpu_util", statistics.median, True),
    ("GPU own SM %", "gpu_sm", statistics.median, True),
    ("GPU W", "gpu_w", statistics.median, False),
    ("package W", "pkg_w", statistics.median, True),
]


def stat(rows, key, fn):
    xs = [r[key] for r in rows if r.get(key) is not None]
    return fn(xs) if xs else None


def write_report(a, ctx, results):
    shown = [m for m in METRICS if any(stat(rs, m[1], m[2]) is not None for rs, _ in results.values())]
    out = [f"# wubble against Firefox, Linux ({ctx['started']})", "",
           f"wubble: {ctx['version']['wubble']} ({a.wubble}); Firefox: {ctx['version']['firefox']}.",
           f"Machine: {ctx['machine']}. Invisible display {SIZE[0]}x{SIZE[1]} at 60 Hz (scripts/headless.sh), "
           f"windows 1280x800. {a.seconds} s per state after load + {a.warmup} s, {a.rounds} round(s), "
           f"a fresh launch and profile each. Load average: {ctx['load0']} at the start, {ctx['load1']} at the end.",
           "", "Each cell is wubble / Firefox; CPU is % of one core over the whole process tree (main, GPU "
           "process, renderers, other); memory over the tree; GPU busy and W are the whole GPU (the desktop's "
           "share included, the same for both); package W the whole CPU package. Verdict: wubble better when "
           "it is lower on every bold column that has numbers for both.", "",
           "| State | " + " | ".join(f"**{m[0]}**" if m[3] else m[0] for m in shown) + " | Verdict |",
           "|---" * (len(shown) + 2) + "|"]
    for state in a.states:
        cells, worse = [], []
        for name, key, fn, judged in shown:
            w, f = (stat(results.get((state, b), ([], []))[0], key, fn) for b in BROWSERS)
            cells.append(f"{fmt(w)} / {fmt(f)}")
            if judged and w is not None and f is not None and w > f:
                worse.append(name)
        has = all(results.get((state, b), ([], []))[0] for b in BROWSERS)
        verdict = "n/a" if not has else f"wubble worse ({', '.join(worse)})" if worse else "wubble better"
        out.append(f"| {state} | " + " | ".join(cells) + f" | {verdict} |")
    out += ["", "| State | Browser | What it saw |", "|---|---|---|"]
    for state in a.states:
        for b in BROWSERS:
            out.append(f"| {state} | {b} | {'; '.join(results.get((state, b), ([], ['not run']))[1]) or '-'} |")
    out += ["", "Not readable on this machine: " + (", ".join(ctx["unreadable"]) or "nothing") + ".",
            "Raw: samples.csv (one row a second), logs/ (each launch's output; wubble's with its trace)."]
    (ctx["out"] / "results.md").write_text("\n".join(out) + "\n")


def results_dir(a):
    if a.out:
        return Path(a.out)
    common_dir = subprocess.run(["git", "-C", str(HERE), "rev-parse", "--path-format=absolute", "--git-common-dir"],
                                capture_output=True, text=True).stdout.strip()
    base = Path(common_dir).parent / ".local" if common_dir else Path.home() / "wubble-bench"
    return base / "bench" / datetime.datetime.now().strftime("%Y%m%d-%H%M%S")


def machine():
    cpu = next((l.split(":", 1)[1].strip() for l in open("/proc/cpuinfo") if l.startswith("model name")), "?")
    gpu = subprocess.run(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"],
                         capture_output=True, text=True).stdout.strip() if shutil.which("nvidia-smi") else "?"
    return f"{cpu}, {gpu or '?'}, Linux {os.uname().release}"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--wubble", required=True, help="the wubble binary (a release build)")
    ap.add_argument("--firefox", default="/usr/bin/firefox")
    ap.add_argument("--seconds", type=int, default=20, help="measured seconds per state (default 20)")
    ap.add_argument("--warmup", type=int, default=10, help="seconds after the page loaded before a state starts")
    ap.add_argument("--rounds", type=int, default=1, help="every state this many times, the browsers' order alternating")
    ap.add_argument("--states", default=",".join(STATES), help="comma-separated, of: " + ", ".join(STATES))
    ap.add_argument("--out", help="results folder (default ~/Dev/wubble/.local/bench/<time>)")
    a = ap.parse_args()
    a.states = a.states.split(",")
    if unknown := [s for s in a.states if s not in STATES]:
        ap.error(f"unknown states: {', '.join(unknown)}")
    a.wubble = str(Path(a.wubble).resolve())
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))  # timeout's TERM: the finally below stops the browser

    out = results_dir(a)
    (out / "logs").mkdir(parents=True, exist_ok=True)
    fixture = Path(tempfile.mkdtemp(prefix="bench-fixture-"))
    subprocess.run([str(HERE / "video-page.sh"), str(fixture)], check=True, stdout=subprocess.DEVNULL)
    for name, body in PAGES.items():  # video.html replaces video-page.sh's: the same shape, reporting
        (fixture / f"{name}.html").write_text(HEAD % name + body)
    ctx = {"out": out, "bin": {"wubble": a.wubble, "firefox": a.firefox}, "loop": common.Loopback(fixture),
           "seat": Seat(), "gpu": Gpu(), "rapl": Rapl(), "machine": machine(),
           "started": datetime.datetime.now().strftime("%Y-%m-%d %H:%M"),
           "load0": " ".join(open("/proc/loadavg").read().split()[:3]),
           "version": {b: subprocess.run([ctx_bin, "--version"], capture_output=True, text=True).stdout.strip() or "?"
                       for b, ctx_bin in (("wubble", a.wubble), ("firefox", a.firefox))}}
    ctx["unreadable"] = [what for what, ok in (
        ("package energy (powercap energy_uj is root-only)", ctx["rapl"].zones),
        ("GPU busy (no nvidia-smi, no readable gpu_busy_percent)", ctx["gpu"].sources())) if not ok]
    print(f"linux-vs-firefox: {ctx['version']['wubble']} against {ctx['version']['firefox']} -> {out}", flush=True)
    results = {}
    with (out / "samples.csv").open("w", newline="") as f:
        ctx["csvfile"], ctx["csv"] = f, csv.DictWriter(f, COLS)
        ctx["csv"].writeheader()
        try:
            for rnd in range(1, a.rounds + 1):
                order = list(BROWSERS) if rnd % 2 else list(reversed(BROWSERS))
                for state in a.states:
                    for browser in order:
                        rows, notes = run_one(a, ctx, browser, state, rnd)
                        acc = results.setdefault((state, browser), ([], []))
                        acc[0].extend(rows)
                        acc[1].extend(f"r{rnd}: {n}" if a.rounds > 1 else n for n in notes)
                        cpu = stat(rows, "cpu_total", statistics.median)
                        print(f"  {state:14} {browser:8} CPU {fmt(cpu)} %  {'; '.join(notes)}", flush=True)
        except KeyboardInterrupt:
            print("linux-vs-firefox: stopped", flush=True)
        finally:
            ctx["load1"] = " ".join(open("/proc/loadavg").read().split()[:3])
            if not ctx.get("summaries"):
                ctx["unreadable"].append("wubble's cef_frames and slint_renders (its trace prints them only on the "
                                         "page-layer path, which Linux does not take)")
            if all(r.get("gpu_sm") is None for rs, _ in results.values() for r in rs) and ctx["gpu"].procs:
                ctx["unreadable"].append("the browsers' own GPU share (pmon gave no per-process SM %)")
            write_report(a, ctx, results)
            for k in ("seat", "gpu", "loop"):
                ctx[k].close()
            shutil.rmtree(fixture, ignore_errors=True)
    print(f"linux-vs-firefox: {out / 'results.md'}", flush=True)


if __name__ == "__main__":
    main()
