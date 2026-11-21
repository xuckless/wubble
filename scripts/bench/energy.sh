#!/usr/bin/env bash
# energy.sh BIN [SECONDS] : what wubble costs per scenario on Linux, one table: the whole process
# tree's CPU seconds per second (from /proc), its wake-ups per second (context switches of every
# thread: voluntary ones are sleeps ended by a wake-up), and from wubble's own `--measure` report
# CEF frames, Slint renders, scene renders and CEF pump runs per second and the GPU (nvidia-smi).
# Run it on the invisible display: scripts/headless.sh timeout 600 scripts/bench/energy.sh BIN
# The fixture pages (a small CSS animation, a focused text field's caret, a looping video) are
# served from loopback on a port the kernel picks.
set -u
bin=$1 secs=${2:-10}
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'kill $srv 2>/dev/null; rm -rf "$work"' EXIT
"$here/video-page.sh" "$work" >/dev/null || exit 1
cat >"$work/anim.html" <<'HTML'
<!doctype html><meta charset=utf-8><title>anim</title><style>
body{margin:24px;font:16px sans-serif}i{display:inline-block;width:16px;height:16px;background:#36c;
animation:s 1s linear infinite}@keyframes s{to{transform:rotate(360deg)}}</style>
<p>A still page with one 16 px spinner. <i></i></p>
HTML
cat >"$work/caret.html" <<'HTML'
<!doctype html><meta charset=utf-8><title>caret</title><style>body{margin:24px;font:16px sans-serif}</style>
<p>A still page with a focused text field (its caret blinks).</p><input autofocus>
HTML
(cd "$work" && exec python3 -u -m http.server 0 --bind 127.0.0.1) >"$work/server.log" 2>&1 &
srv=$!
for _ in $(seq 100); do port=$(sed -n 's/.*port \([0-9]*\).*/\1/p' "$work/server.log" | head -1); [ -n "$port" ] && break; sleep 0.05; done
base="http://127.0.0.1:$port"

tree() { local out=$1 k; for k in $(pgrep -P "$1"); do out="$out $(tree "$k")"; done; echo "$out"; }
# CPU ticks and context switches of every thread of the tree
sample() {
  local t=0 c=0 p f x
  for p in $(tree "$1"); do
    for f in /proc/$p/task/*; do
      set -- $(sed 's/.*) //' "$f/stat" 2>/dev/null); t=$((t + ${12:-0} + ${13:-0}))
      x=$(awk '/ctxt_switches/ {s += $2} END {print s + 0}' "$f/status" 2>/dev/null); c=$((c + ${x:-0}))
    done
  done
  echo "$t $c"
}
hz=$(getconf CLK_TCK)
has_page=$("$bin" --help 2>/dev/null | grep -- --measure-page)
printf "%-10s %6s %8s %7s %8s %8s %7s %s\n" scenario cpu_s/s wakeups/s cef/s slint/s scene/s pump/s gpu
run() {
  local name=$1; shift
  # builds before 2026-11-21 have no --measure-page
  case "$*" in *--measure-page*) [ -n "$has_page" ] || { printf "%-10s (this build has no --measure-page)\n" "$name"; return; } ;; esac
  "$bin" "$@" --seconds $((secs + 12)) >"$work/$name.out" 2>&1 &
  local w=$!
  sleep 9
  read -r t0 c0 < <(sample $w)
  sleep "$secs"
  read -r t1 c1 < <(sample $w)
  wait $w
  rate() { awk -v k="$1" '$1 == k { print $3; exit }' "$work/$name.out"; }
  printf "%-10s %6.2f %8.0f %7s %8s %8s %7s %s\n" "$name" \
    "$(echo "($t1 - $t0) / $hz / $secs" | bc -l)" "$(echo "($c1 - $c0) / $secs" | bc -l)" \
    "$(rate cef_frames)" "$(rate slint_renders)" "$(rate scene_renders)" "$(rate pump_runs)" \
    "$(sed -n 's/^gpu: //p' "$work/$name.out")"
  cp "$work/$name.out" "${ENERGY_KEEP:-/dev/null}.$name" 2>/dev/null
}
run idle --measure idle --warmup 4
run start --measure idle --warmup 4 --measure-page wubble://start
run anim --measure idle --warmup 4 --measure-page "$base/anim.html"
run caret --measure idle --warmup 4 --measure-page "$base/caret.html"
run scroll --measure scroll
run canvas --measure video
run video --measure idle --warmup 4 --measure-page "$base/video.html"
echo "load average $(cut -d' ' -f1-3 /proc/loadavg)"
