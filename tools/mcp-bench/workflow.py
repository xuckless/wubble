#!/usr/bin/env python3
# Drives a browsing workflow through an approved wubble MCP session (via mcpd.py) while
# sampler.sh records metrics; each step writes its name to OUTDIR/phase.
#
#   python3 -I workflow.py SOCK SESSION FIRST_TAB [OUTDIR]
#
# The session needs the sites below (see README.md). Videos are started through YouTube's own
# player API: the player pauses a <video> that a script plays behind its back.
import json, os, socket, sys, time

SOCK, S, FIRST = sys.argv[1], sys.argv[2], sys.argv[3]
OUT = sys.argv[4] if len(sys.argv) > 4 else "data"
os.makedirs(OUT, exist_ok=True)
LOG = open(os.path.join(OUT, "bench.log"), "a", buffering=1)

SITES = ["https://en.wikipedia.org/wiki/Web_browser", "https://github.com/rust-lang/rust",
         "https://news.ycombinator.com/", "https://www.bbc.com/news", "https://www.theverge.com/",
         "https://developer.mozilla.org/en-US/docs/Web/CSS/grid", "https://docs.rs/tokio/latest/tokio/",
         "https://www.rust-lang.org/", "https://www.apple.com/macbook-pro/",
         "https://en.wikipedia.org/wiki/Chromium_(web_browser)"]
# AV1 1080p60, VP9 1080p60, AV1 1080p30 (the codecs YouTube served on 2026-11-21)
VIDEOS = ["https://www.youtube.com/watch?v=aqz-KE-bpKQ", "https://www.youtube.com/watch?v=LXb3EKWsInQ",
          "https://www.youtube.com/watch?v=hlWiI4xVXKY"]


def log(*a):
    LOG.write(time.strftime("%H:%M:%S ") + " ".join(str(x) for x in a) + "\n")


def phase(name):
    open(os.path.join(OUT, "phase"), "w").write(name)
    log("PHASE", name, int(time.time()))


def call(tool, **args):
    args["session"] = S
    s = socket.socket(socket.AF_UNIX)
    s.connect(SOCK)
    s.sendall((json.dumps({"tool": tool, "args": args}) + "\n").encode())
    data = b""
    while True:
        c = s.recv(1 << 20)
        if not c:
            break
        data += c
    m = json.loads(data)
    if "error" in m:
        log("ERR", tool, m["error"])
        return None
    r = m["result"]
    txt = "\n".join(p.get("text", "") for p in r.get("content", []) if p.get("type") == "text")
    if r.get("isError"):
        log("TOOLERR", tool, txt[:300])
        return None
    return txt


def timed(tool, **args):
    t = time.time()
    r = call(tool, **args)
    return r, time.time() - t


def tab_of(r):
    try:
        return json.loads(r).get("tab")
    except (TypeError, ValueError):
        return None


def js(tab, script):
    return call("evaluate", tab=tab, script=script, timeout_ms=15000)


VSTATS = """(() => { const v = document.querySelector('video'); if (!v) return 'no video';
 const q = v.getVideoPlaybackQuality(); const p = document.getElementById('movie_player');
 let s = {}; try { s = p.getStatsForNerds ? p.getStatsForNerds() : {}; } catch (e) {}
 return {t: v.currentTime.toFixed(1), paused: v.paused, size: v.videoWidth + 'x' + v.videoHeight,
  frames: q.totalVideoFrames, dropped: q.droppedVideoFrames, codec: s.codecs || '', fmt: s.resolution || '',
  quality: p && p.getPlaybackQuality ? p.getPlaybackQuality() : ''}; })()"""

# Plays through the player API and reports how far the clip advanced in 4 s (0: not playing).
PLAY = """(async () => { const p = document.getElementById('movie_player');
 for (let i = 0; i < 40 && !(p && p.playVideo); i++) await new Promise(r => setTimeout(r, 250));
 try { p.setPlaybackQualityRange('hd1080','hd1080'); } catch (e) {}
 p.mute(); p.playVideo(); const v = document.querySelector('video'); const t0 = v.currentTime;
 await new Promise(r => setTimeout(r, 4000));
 return {state: p.getPlayerState(), advanced: +(v.currentTime - t0).toFixed(1)}; })()"""

PAUSE = "document.getElementById('movie_player').pauseVideo()"

# 1. idle: the first blank tab, nothing happening
phase("1-idle")
time.sleep(60)

# 2. browsing: open real sites, read and scroll each, then leave them alone
phase("2-browse-open")
tabs = []
r, dt = timed("navigate", tab=FIRST, url=SITES[0])
tabs.append(FIRST)
log("load", SITES[0], f"{dt:.2f}s")
for u in SITES[1:]:
    r, dt = timed("open_tab", url=u)
    t = tab_of(r)
    log("load", u, f"{dt:.2f}s", "tab", t)
    if t:
        tabs.append(t)
phase("2-browse-scroll")
for t in tabs:
    call("focus_tab", tab=t)
    for _ in range(4):
        call("scroll", tab=t, direction="down", amount=900)
        time.sleep(0.6)
    call("read_text", tab=t, max_chars=2000)
phase("2-browse-idle")
time.sleep(45)

# 3. one video shown, then fullscreen through YouTube's own key
phase("3-video1-load")
r, dt = timed("open_tab", url=VIDEOS[0])
v1 = tab_of(r)
log("load", VIDEOS[0], f"{dt:.2f}s", "tab", v1)
call("focus_tab", tab=v1)
log("play", js(v1, PLAY))
log("vstats-start", js(v1, VSTATS))
phase("3-video1-play")
time.sleep(60)
log("vstats-end", js(v1, VSTATS))
phase("3-video1-fullscreen")
call("press_key", tab=v1, key="f")
time.sleep(3)
log("fullscreen", js(v1, "!!document.fullscreenElement"))
time.sleep(45)
log("vstats-fs", js(v1, VSTATS))
call("press_key", tab=v1, key="Escape")
time.sleep(3)

# 4. three videos playing, one shown; then switching the shown tab
phase("4-video3-load")
vids = [v1]
for u in VIDEOS[1:]:
    r, dt = timed("open_tab", url=u)
    t = tab_of(r)
    log("load", u, f"{dt:.2f}s", "tab", t)
    if t:
        vids.append(t)
        call("focus_tab", tab=t)
        log("play", t, js(t, PLAY))
for t in vids:
    log("vstats", t, js(t, VSTATS))
phase("4-video3-play")
time.sleep(60)
for t in vids:
    log("vstats", t, js(t, VSTATS))
phase("4-video3-switch")
for _ in range(3):
    for t in vids:
        call("focus_tab", tab=t)
        time.sleep(5)
for t in vids:
    log("vstats", t, js(t, VSTATS))

# 5. mixed: the videos play in background tabs while browsing in front
phase("5-mixed")
for t in tabs[:6]:
    call("focus_tab", tab=t)
    for _ in range(3):
        call("scroll", tab=t, direction="down", amount=700)
        time.sleep(0.8)
    call("scroll", tab=t, direction="up", amount=3000)
    time.sleep(1)
for t in vids:
    log("vstats", t, js(t, VSTATS))

# 6. videos paused, every tab still open
phase("6-paused")
for t in vids:
    js(t, PAUSE)
time.sleep(45)

# 7. recovery: close everything but the first tab
phase("7-closing")
for t in tabs[1:] + vids:
    call("close_tab", tab=t)
call("navigate", tab=FIRST, url="https://www.rust-lang.org/")
phase("7-recovered")
time.sleep(60)
phase("done")
log("DONE")
