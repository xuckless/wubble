#!/usr/bin/env python3
"""Runs one browser on one benchmark served from loopback, prints one JSON line.

  runner.py SUITES LOGDIR <wubble|chromium|firefox> <speedometer|motionmark> [timeout_s]

SUITES holds `speedometer/` (Speedometer 3.1, `release/3.1`) and `motionmark/` (MotionMark 1.3)
checkouts; the browser's output goes to LOGDIR. `WUBBLE=path/to/wubble` picks the wubble binary
(default: `wubble` on PATH). Meant to run inside scripts/headless.sh. The page posts its score to
this server; meanwhile the browser's process tree is sampled every second (CPU ticks, PSS).
"""
import http.server, json, os, shutil, socketserver, subprocess, sys, tempfile, threading, time

from common import memory, tree

if len(sys.argv) < 5:
    sys.exit(__doc__)
SUITES, LOGDIR = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
WUBBLE = os.environ.get("WUBBLE", "wubble")
browser, suite = sys.argv[3], sys.argv[4]
limit = int(sys.argv[5]) if len(sys.argv) > 5 else 900
result = {}
done = threading.Event()

WRAP = """<!doctype html><meta charset=utf-8><title>bench</title>
<style>html,body{margin:0;height:100%%;overflow:hidden}iframe{border:0;width:1000px;height:680px;display:block}</style>
<iframe id=f src="%s"></iframe><script>
const f=document.getElementById('f');const suite=%s;
let raf=0,hz=null;(function t(){raf++;requestAnimationFrame(t)})();setTimeout(()=>{const a=raf;setTimeout(()=>{hz=raf-a},1000)},300);
function post(o){o.inner=[innerWidth,innerHeight];o.frame=[f.contentWindow.innerWidth,f.contentWindow.innerHeight];o.hz=hz;o.dpr=devicePixelRatio;o.ua=navigator.userAgent;
  fetch('/result',{method:'POST',body:JSON.stringify(o)});}
let started=false;
setInterval(()=>{try{const w=f.contentWindow,d=w.document;
 if(suite==='speedometer'){const c=w.benchmarkClient;
   if(c&&c._hasResults&&!started){started=true;
     const m=c._metrics||{};const s=m.Score||{};
     post({score:s.mean,delta:s.delta,geomean_ms:(m.Geomean||{}).mean,text:d.getElementById('result-number')?.textContent});}}
 else{
   if(!started&&w.benchmarkController&&d.readyState==='complete'){started=true;setTimeout(()=>w.benchmarkController.startBenchmark(),2000);}
   const t=d.querySelector('#results .score')?.textContent;
   if(t&&!window.sent){window.sent=true;
     const rows=[...d.querySelectorAll('#results-header tr')].map(r=>r.textContent.trim());
     const sc=[...d.querySelectorAll('#results-score tr')].map(r=>r.textContent.trim());
     post({text:t,confidence:d.querySelector('#results .confidence')?.textContent,tests:rows,scores:sc});}}
}catch(e){if(!window.err){window.err=true;fetch('/log',{method:'POST',body:String(e)});}}},500);
</script>"""
TARGET = {"speedometer": "/speedometer/index.html?startAutomatically=true",
          "motionmark": "/motionmark/MotionMark/index.html"}


class H(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=SUITES, **k)

    def log_message(self, *a):
        pass

    def do_GET(self):
        if self.path.startswith("/wrap"):
            body = (WRAP % (TARGET[suite], json.dumps(suite))).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            super().do_GET()

    def do_POST(self):
        data = self.rfile.read(int(self.headers.get("Content-Length", 0))).decode()
        self.send_response(204)
        self.end_headers()
        if self.path == "/result":
            result.update(json.loads(data))
            done.set()
        else:
            sys.stderr.write("page: " + data + "\n")


class S(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True


srv = S(("127.0.0.1", 0), H)
port = srv.server_address[1]
threading.Thread(target=srv.serve_forever, daemon=True).start()
url = f"http://127.0.0.1:{port}/wrap"
prof = tempfile.mkdtemp(prefix="bench-prof-")
cmd = {
    "wubble": [WUBBLE, "--profile", prof, url],
    "chromium": ["chromium-browser", f"--user-data-dir={prof}", "--no-first-run",
                 "--no-default-browser-check", "--ozone-platform=wayland",
                 "--window-size=1280,800", url],
    "firefox": ["firefox", "--new-instance", "--profile", prof, "--width", "1280",
                "--height", "800", url],
}[browser]
env = dict(os.environ)
if browser == "firefox":
    env["MOZ_ENABLE_WAYLAND"] = "1"
os.makedirs(LOGDIR, exist_ok=True)
log = open(os.path.join(LOGDIR, f"log-{browser}-{suite}.txt"), "w")
t0 = time.time()
p = subprocess.Popen(cmd, env=env, stdout=log, stderr=log, start_new_session=True)
TICK = os.sysconf("SC_CLK_TCK")


def pss(pids):
    return sum(memory(p)[1] for p in pids)


seen, peak, n = {}, 0, 0
while not done.is_set() and time.time() - t0 < limit and p.poll() is None:
    done.wait(1)
    t = tree(p.pid)
    seen.update(t)  # last ticks of every process ever seen, exited ones included
    n += 1
    if n % 5 == 0:
        peak = max(peak, pss(t))
wall = time.time() - t0
t = tree(p.pid)
seen.update(t)
peak = max(peak, pss(t))
out = {"browser": browser, "suite": suite, "ok": done.is_set(), "wall_s": round(wall, 1),
       "cpu_s": round(sum(seen.values()) / TICK, 1), "peak_pss_mb": round(peak / 1024),
       "procs": len(t), **result}
try:
    os.killpg(p.pid, 15)
    p.wait(10)
except Exception:
    try:
        os.killpg(p.pid, 9)
    except Exception:
        pass
shutil.rmtree(prof, ignore_errors=True)
print(json.dumps(out), flush=True)
