#!/usr/bin/env python3
"""Loopback fixtures for the media downloader, by hand (`wubble --smoke media=probe --url ...`).

    scripts/media-fixtures.py make DIR     # real clips with the system ffmpeg: MP4, WebM, MP3, HLS (TS,
                                           # fMP4, separate audio, AES-128, a DRM-marked one, a live
                                           # one, one with a missing segment), DASH (template and
                                           # timeline, segment list, one file) and an MSE page's clip
    scripts/media-fixtures.py serve DIR PORT   # 127.0.0.1 only; each request is one line in DIR/server.log

Pages (`/NAME.html`): plain redirect query nolen cookie cookie-slow cookie-hls cookie-hls-pair reset iframe mse hls-ts hls-fmp4 hls-aes hls-drm
hls-live hls-fail hls-audio hls-media dash-timeline dash-list dash-file dash-mse audio.
Nothing here is committed output: DIR is a scratch folder. Loopback only, no outside request.
"""
import http.server
import os
import re
import shutil
import socketserver
import subprocess
import sys
import urllib.parse

V = ["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25:duration=8"]
V2 = ["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=8"]
A = ["-f", "lavfi", "-i", "sine=frequency=440:duration=8:sample_rate=48000"]
H264 = ["-c:v", "libopenh264", "-pix_fmt", "yuv420p", "-g", "25", "-b:v", "400k"]
AAC = ["-c:a", "aac", "-b:a", "96k"]


def ff(*args):
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *args], check=True)


def make(out):
    os.makedirs(out, exist_ok=True)
    os.chdir(out)
    ff(*V, *A, *H264, *AAC, "-movflags", "+faststart", "-shortest", "clip.mp4")
    ff(*V2, *A, "-c:v", "libvpx-vp9", "-b:v", "300k", "-c:a", "libopus", "-shortest", "clip.webm")
    ff(*A, "-c:a", "libmp3lame", "-b:a", "128k", "song.mp3")
    # a fragmented MP4 for the MSE page
    ff(*V, *A, *H264, *AAC, "-movflags", "frag_keyframe+empty_moov+default_base_moof", "-shortest", "frag.mp4")
    # HLS: two video qualities with a separate audio group, TS and fMP4
    for kind, extra in (("ts", []), ("fmp4", ["-hls_segment_type", "fmp4"])):
        d = f"hls-{kind}"
        os.makedirs(d, exist_ok=True)
        ff(*V, *V, *A, "-map", "0:v", "-map", "1:v", "-map", "2:a",
           "-c:v", "libopenh264", "-pix_fmt", "yuv420p", "-g", "25",
           "-b:v:0", "600k", "-b:v:1", "200k", "-s:v:1", "320x180", *AAC,
           "-f", "hls", "-hls_time", "2", "-hls_playlist_type", "vod", *extra,
           "-master_pl_name", "master.m3u8",
           "-var_stream_map", "v:0,agroup:aud,name:v360 v:1,agroup:aud,name:v180 a:0,agroup:aud,name:en,default:yes",
           "-hls_segment_filename", f"{d}/%v-%03d." + ("ts" if kind == "ts" else "m4s"),
           f"{d}/%v.m3u8")
        if os.path.exists(f"{d}/master.m3u8"):
            pass
    # HLS muxed single-rendition playlist
    os.makedirs("hls-media", exist_ok=True)
    ff(*V, *A, *H264, *AAC, "-shortest", "-f", "hls", "-hls_time", "2", "-hls_playlist_type", "vod",
       "-hls_segment_filename", "hls-media/s%03d.ts", "hls-media/index.m3u8")
    # AES-128
    os.makedirs("hls-aes", exist_ok=True)
    with open("hls-aes/key.bin", "wb") as f:
        f.write(b"0123456789abcdef")
    with open("hls-aes/keyinfo", "w") as f:
        f.write("key.bin\nhls-aes/key.bin\n")
    ff(*V, *A, *H264, *AAC, "-shortest", "-f", "hls", "-hls_time", "2", "-hls_playlist_type", "vod",
       "-hls_key_info_file", "hls-aes/keyinfo", "-hls_segment_filename", "hls-aes/s%03d.ts", "hls-aes/index.m3u8")
    # DRM-marked, live, and a missing segment: edits of the muxed playlist
    base = open("hls-media/index.m3u8").read()
    os.makedirs("hls-drm", exist_ok=True)
    drm = base.replace("#EXT-X-VERSION", '#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://x",KEYFORMAT="com.apple.streamingkeydelivery"\n#EXT-X-VERSION', 1)
    open("hls-drm/index.m3u8", "w").write(drm.replace("s0", "../hls-media/s0"))
    os.makedirs("hls-live", exist_ok=True)
    live = base.replace("#EXT-X-ENDLIST\n", "").replace("#EXT-X-PLAYLIST-TYPE:VOD\n", "")
    open("hls-live/index.m3u8", "w").write(live.replace("s0", "../hls-media/s0"))
    os.makedirs("hls-fail", exist_ok=True)
    fail = base.replace("s002.ts", "missing.ts")
    open("hls-fail/index.m3u8", "w").write(fail.replace("s0", "../hls-media/s0"))
    # DASH: timeline template (+ separate audio), a segment list, one file per track
    for name, extra in (
        ("dash-timeline", ["-use_template", "1", "-use_timeline", "1"]),
        ("dash-list", ["-use_template", "0", "-use_timeline", "0"]),
        ("dash-file", ["-single_file", "1"]),
    ):
        os.makedirs(name, exist_ok=True)
        ff(*V, *A, *H264, *AAC, "-shortest", "-map", "0:v", "-map", "1:a", "-f", "dash",
           "-seg_duration", "2", "-adaptation_sets", "id=0,streams=v id=1,streams=a", *extra,
           f"{name}/manifest.mpd")


PAGES = {
    "plain": '<video src="/f/clip.mp4" autoplay muted loop controls width=480></video>',
    "redirect": '<video src="/redir/go" autoplay muted loop controls width=480></video>',
    "query": '<video src="/dl?id=5&name=holiday.mp4" autoplay muted loop controls width=480></video>',
    "nolen": '<video src="/nolen/clip.mp4" autoplay muted loop controls width=480></video>',
    "cookie": '<video src="/guard/clip.mp4" autoplay muted loop controls width=480></video>',
    "cookie-slow": '<video src="/gslow/clip.mp4" autoplay muted loop controls width=480></video>',
    "cookie-hls": '<video src="/gslow/hls-media/index.m3u8" autoplay muted loop controls width=480></video>',
    "cookie-hls-pair": '<video src="/gslow/hls-ts/master.m3u8" autoplay muted loop controls width=480></video>',
    "reset": '<video src="/reset/clip.mp4" autoplay muted loop controls width=480></video>',
    "iframe-x": '<iframe src="http://localhost:18080/plain-kit.html" width=520 height=300 style="border:0"></iframe>',
    "iframe-s": '<iframe src="/plain-kit.html" width=520 height=300 style="border:0"></iframe>',
    "plain-kit": '<video src="/f/clip.mp4" autoplay muted loop controls width=480></video><script>note("kit "+(typeof window[Symbol.for("wubble.kit")]))</script>',
    "iframe": '<iframe src="/plain.html" width=520 height=300 style="border:0"></iframe>',
    "audio": '<audio src="/f/song.mp3" autoplay muted loop controls></audio><p>an audio page',
    "mse": """<video id=v autoplay muted loop controls width=480></video><script>
const ms=new MediaSource();v.src=URL.createObjectURL(ms);
const T='video/mp4; codecs="avc1.42c01e,mp4a.40.2"';note('mse supported '+MediaSource.isTypeSupported(T)+' '+MediaSource.isTypeSupported('video/mp4; codecs="avc1.42E01E, mp4a.40.2"')+' '+MediaSource.isTypeSupported('video/mp4; codecs="avc1.42c01e"'));
ms.addEventListener('sourceopen',async()=>{note('sourceopen');const sb=ms.addSourceBuffer(T);
const r=await fetch('/f/frag.mp4');sb.appendBuffer(await r.arrayBuffer());});</script>""",
    "hls-ts": '<video src="/f/hls-ts/master.m3u8" autoplay muted loop controls width=480></video>',
    "hls-fmp4": '<video src="/f/hls-fmp4/master.m3u8" autoplay muted loop controls width=480></video>',
    "hls-aes": '<video src="/f/hls-aes/index.m3u8" autoplay muted loop controls width=480></video>',
    "hls-drm": '<video src="/f/hls-drm/index.m3u8" autoplay muted loop controls width=480></video>',
    "hls-live": '<video src="/f/hls-live/index.m3u8" autoplay muted controls width=480></video>',
    "hls-fail": '<video src="/f/hls-fail/index.m3u8" autoplay muted controls width=480></video>',
    "hls-media": '<video src="/f/hls-media/index.m3u8" autoplay muted loop controls width=480></video>',
    "hls-audio": '<video src="/f/hls-ts/master.m3u8" autoplay muted loop controls width=480></video>',
}
# DASH: the page plays the muxed clip, and a player script reads the manifest as dash.js would.
for _n in ("dash-timeline", "dash-list", "dash-file"):
    PAGES[_n] = (
        '<video src="/f/clip.mp4" autoplay muted loop controls width=480></video>'
        f"<script>fetch('/f/{_n}/manifest.mpd');</script>"
    )
# DASH through MSE: init and segments appended to a blob: video, as dash.js does
# a service worker answers /sw/clip.mp4 itself, from the clip: the page's video request is served
# by the worker, not the network (does detection still see it?)
PAGES["sw"] = """<video id=v autoplay muted loop controls width=480></video><script>
navigator.serviceWorker.register('/sw.js').then(()=>navigator.serviceWorker.ready).then(()=>
  fetch('/sw/clip.mp4').then(()=>{v.src='/sw/clip.mp4'})).catch(e=>note('sw '+e))</script>"""
PAGES["shadow"] = """<div id=h></div><script>
h.attachShadow({mode:'open'}).innerHTML='<video src="/f/clip.mp4" autoplay muted loop controls width=480></video>'</script>"""
PAGES["cached"] = '<video src="/cache/clip.mp4" autoplay muted loop controls width=480></video>'
PAGES["dash-mse"] = """<video id=v autoplay muted loop controls width=480></video><script>
(async()=>{const base='/f/dash-timeline/';const mpd=await (await fetch(base+'manifest.mpd')).text();
const ms=new MediaSource();v.src=URL.createObjectURL(ms);await new Promise(r=>ms.addEventListener('sourceopen',r));
const doc=new DOMParser().parseFromString(mpd,'application/xml');
for(const set of doc.getElementsByTagName('AdaptationSet')){const rep=set.getElementsByTagName('Representation')[0];
const t=set.getElementsByTagName('SegmentTemplate')[0];const id=rep.getAttribute('id');
const mime=(rep.getAttribute('mimeType')||set.getAttribute('mimeType'));
const sb=ms.addSourceBuffer(mime+'; codecs="'+(rep.getAttribute('codecs')||set.getAttribute('codecs'))+'"');
const add=async u=>{const b=await (await fetch(base+u)).arrayBuffer();sb.appendBuffer(b);await new Promise(r=>sb.addEventListener('updateend',r,{once:true}))};
await add(t.getAttribute('initialization').replace('$RepresentationID$',id));
let n=1;for(const s of t.getElementsByTagName('S')){const r=+(s.getAttribute('r')||0);for(let i=0;i<=r;i++){await add(t.getAttribute('media').replace('$RepresentationID$',id).replace(/\\$Number%05d\\$/,String(n++).padStart(5,'0')));}}}
ms.endOfStream();})();</script>"""


def page(name, body):
    # a page's own errors and notes reach the server log (GET /log?...)
    report = ("<script>const note=m=>fetch('/log?'+encodeURIComponent(m));"
              "addEventListener('error',e=>note('error '+e.message));"
              "addEventListener('unhandledrejection',e=>note('rejected '+e.reason));</script>")
    return (f"<!doctype html><meta charset=utf-8><title>{name} fixture</title>{report}"
            f"<body style=\"margin:20px;background:#222;color:#ddd\">{body}")


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    root = "."
    log_file = None

    def log_message(self, *a):
        pass

    def note(self, status):
        h = self.headers
        line = f"{self.command} {self.path} -> {status} range={h.get('Range')} cookie={h.get('Cookie')} referer={h.get('Referer')} ua={(h.get('User-Agent') or '')[:30]} ae={h.get('Accept-Encoding')}\n"
        with open(self.log_file, "a") as f:
            f.write(line)

    def send(self, status, headers, body=b"", length=True, slow=False, reset_at=None):
        self.note(status)
        self.send_response(status)
        for k, v in headers.items():
            self.send_header(k, v)
        if length:
            self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if self.command != "HEAD":
            try:
                if slow or reset_at is not None:
                    import time
                    for i in range(0, len(body), 16384):
                        if reset_at is not None and i >= reset_at:
                            self.close_connection = True
                            self.connection.close()  # the body ends short of its Content-Length
                            return
                        self.wfile.write(body[i : i + 16384])
                        self.wfile.flush()
                        time.sleep(0.05 if slow else 0)
                else:
                    self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

    mime = {".mp4": "video/mp4", ".m4s": "video/iso.segment", ".webm": "video/webm", ".mp3": "audio/mpeg",
            ".m3u8": "application/vnd.apple.mpegurl", ".mpd": "application/dash+xml", ".ts": "video/mp2t",
            ".key": "application/octet-stream", ".bin": "application/octet-stream", ".html": "text/html"}

    def file(self, rel, ranges=True, length=True, mime=None, slow=False, reset_at=None, cache=None):
        path = os.path.normpath(os.path.join(self.root, rel))
        if not path.startswith(os.path.abspath(self.root)) or not os.path.isfile(path):
            return self.send(404, {"Content-Type": "text/plain"}, b"not found")
        data = open(path, "rb").read()
        ctype = mime or self.mime.get(os.path.splitext(path)[1], "application/octet-stream")
        head = {"Content-Type": ctype}
        if cache:
            head["Cache-Control"] = cache
        rng = self.headers.get("Range")
        if ranges:
            head["Accept-Ranges"] = "bytes"
        m = re.match(r"bytes=(\d*)-(\d*)", rng or "")
        if ranges and m and (m.group(1) or m.group(2)):
            if m.group(1):
                a = int(m.group(1))
                b = int(m.group(2)) if m.group(2) else len(data) - 1
            else:
                a, b = max(0, len(data) - int(m.group(2))), len(data) - 1
            b = min(b, len(data) - 1)
            head["Content-Range"] = f"bytes {a}-{b}/{len(data)}"
            return self.send(206, head, data[a : b + 1], slow=slow, reset_at=reset_at)
        if not length:
            # no Content-Length: the body ends with the connection
            self.close_connection = True
            head["Connection"] = "close"
            return self.send(200, head, data, length=False)
        self.send(200, head, data, slow=slow, reset_at=reset_at)

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        u = urllib.parse.urlparse(self.path)
        p = urllib.parse.unquote(u.path)
        if p == "/":
            names = " ".join(f'<a href="/{n}.html">{n}</a>' for n in sorted(PAGES))
            return self.send(200, {"Content-Type": "text/html"}, page("index", names).encode())
        if p == "/remote.html":
            # a page that plays (kind=video) or reads (kind=fetch: a player's manifest request)
            # a public URL: only for hand runs against freely licensed test streams
            q = urllib.parse.parse_qs(u.query)
            target = (q.get("u") or [""])[0].replace('"', "")
            if q.get("kind", ["video"])[0] == "fetch":
                body = f"<p>a player's request</p><script>fetch(\"{target}\")</script>"
            else:
                body = f'<video autoplay muted loop controls width=480 src="{target}"></video>'
            return self.send(200, {"Content-Type": "text/html"}, page("remote", body).encode())
        m = re.fullmatch(r"/([\w-]+)\.html", p)
        if m and m.group(1) in PAGES:
            head = {"Content-Type": "text/html; charset=utf-8"}
            if m.group(1).startswith("cookie"):
                head["Set-Cookie"] = "sess=1; Path=/"
            return self.send(200, head, page(m.group(1), PAGES[m.group(1)]).encode())
        if p == "/sw.js":
            js = ("self.addEventListener('install',e=>self.skipWaiting());"
                  "self.addEventListener('activate',e=>e.waitUntil(clients.claim()));"
                  "self.addEventListener('fetch',e=>{if(new URL(e.request.url).pathname==='/sw/clip.mp4')"
                  "e.respondWith(fetch('/f/clip.mp4'))});")
            return self.send(200, {"Content-Type": "text/javascript"}, js.encode())
        if p.startswith("/cache/"):
            # a response the HTTP cache may serve again: a reload needs no network
            return self.file(p[7:], cache="max-age=3600")
        if p.startswith("/f/"):
            return self.file(p[3:])
        if p.startswith("/nolen/"):
            return self.file(p[7:], ranges=False, length=False)
        if p == "/redir/go":
            return self.send(302, {"Location": "/f/clip.mp4"})
        if p == "/dl":
            return self.file("clip.mp4", mime="video/mp4")
        if p.startswith("/reset/"):
            return self.file(p[7:], reset_at=100000)
        if p.startswith("/gslow/"):
            ok = "sess=1" in (self.headers.get("Cookie") or "")
            if not ok:
                return self.send(403, {"Content-Type": "text/plain"}, b"cookie needed")
            return self.file(p[7:], slow=True)
        if p.startswith("/guard/"):
            ok = "sess=1" in (self.headers.get("Cookie") or "") and (self.headers.get("Referer") or "").startswith(
                f"http://127.0.0.1:{self.server.server_address[1]}/cookie.html")
            if not ok:
                return self.send(403, {"Content-Type": "text/plain"}, b"cookie and referer needed")
            return self.file(p[7:])
        self.send(404, {"Content-Type": "text/plain"}, b"not found")


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True


def serve(root, port):
    Handler.root = os.path.abspath(root)
    Handler.log_file = os.path.join(Handler.root, "server.log")
    Server(("127.0.0.1", int(port)), Handler).serve_forever()


if __name__ == "__main__":
    if len(sys.argv) >= 3 and sys.argv[1] == "make":
        make(sys.argv[2])
    elif len(sys.argv) >= 4 and sys.argv[1] == "serve":
        serve(sys.argv[2], sys.argv[3])
    else:
        sys.exit(__doc__)
