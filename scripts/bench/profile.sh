#!/usr/bin/env bash
# profile.sh [options] NAME PATH : wubble on a page served from loopback, `perf record` on its
# whole process tree, then the flame graph and the main thread's breakdown.
#
#   -b BIN   the wubble binary (default: wubble on PATH); built with symbols (`release` keeps them)
#   -s DIR   the directory served on loopback; PATH is a page in it (default: the fixture dir
#            video-page.sh writes, ./bench-fixture)
#   -o DIR   where the outputs go (default: ./profile-out); NAME prefixes each file
#   -t SEC   recording length (default 20)
#   -w SEC   wait after start before recording (default 10)
#   -g MODE  call graph: lbr (default: cycles:u with last-branch records) or dwarf (cpu-clock:u)
#   -p PERF  the perf binary (default: $PERF, else perf on PATH)
#   -f DIR   a FlameGraph checkout (flamegraph.pl); without one no SVG is drawn
#   -x ARG   one more argument for wubble (repeatable), e.g. -x --frozen-backdrop
#
# Run it on the invisible display: scripts/headless.sh timeout 120 scripts/bench/profile.sh ...
# The server takes a free port the kernel picks (never a guessed one) and the run stops unless
# the page was fetched and nothing it asked for (but a favicon) was missing, so a wrong page never gets profiled.
# Writes NAME.data (perf), NAME.pids (pid and process type), NAME.log (wubble's output),
# NAME.server.log (the requests), NAME.folded, NAME.svg / NAME-main.svg, NAME.breakdown.txt
# and NAME.cpu.txt (CPU over the recording from /proc, user + system, and the load average).
set -u
here=$(cd "$(dirname "$0")" && pwd)
bin=wubble serve=./bench-fixture out=./profile-out secs=20 wait=10 graph=lbr
perf=${PERF:-perf} flame='' extra=()
while getopts b:s:o:t:w:g:p:f:x: opt; do
  case $opt in
    b) bin=$OPTARG ;; s) serve=$OPTARG ;; o) out=$OPTARG ;; t) secs=$OPTARG ;;
    w) wait=$OPTARG ;; g) graph=$OPTARG ;; p) perf=$OPTARG ;; f) flame=$OPTARG ;;
    x) extra+=("$OPTARG") ;; *) sed -n '2,24p' "$0" >&2; exit 2 ;;
  esac
done
shift $((OPTIND - 1))
[ $# -eq 2 ] || { sed -n '2,24p' "$0" >&2; exit 2; }
name=$1 path=$2
mkdir -p "$out" || exit 1
out=$(cd "$out" && pwd)
case $graph in
  lbr) record=(-e cycles:u -F 999 --call-graph lbr) ;;
  dwarf) record=(-e cpu-clock:u -F 399 --call-graph dwarf,65528) ;;
  *) echo "profile.sh: -g lbr or dwarf" >&2; exit 2 ;;
esac

slog=$out/$name.server.log
(cd "$serve" && exec python3 -u -m http.server 0 --bind 127.0.0.1) >"$slog" 2>&1 &
srv=$!
port=''
for _ in $(seq 100); do
  port=$(sed -n 's/.*port \([0-9]*\).*/\1/p' "$slog" | head -1)
  [ -n "$port" ] && break
  sleep 0.05
done
[ -n "$port" ] || { echo "profile.sh: the server did not start" >&2; kill $srv; exit 1; }
url="http://127.0.0.1:$port/$path"
prof=$(mktemp -d)
cleanup() { kill -TERM "${w:-}" 2>/dev/null; sleep 2; kill -KILL "${w:-}" 2>/dev/null; kill $srv 2>/dev/null; rm -rf "$prof"; }
trap 'cleanup; exit 143' TERM INT HUP
"$bin" --profile "$prof" "${extra[@]}" "$url" >"$out/$name.log" 2>&1 &
w=$!
sleep "$wait"
# The page and everything it asked for came from this server, with 200 or 206.
if ! grep -q "\"GET /$path HTTP/1.1\" 200" "$slog"; then
  echo "profile.sh: wubble never fetched $url; see $slog" >&2; cleanup; exit 1
fi
if grep -v 'GET /favicon.ico ' "$slog" | grep -q '" 404 '; then
  echo "profile.sh: the page asked for a missing file; see $slog" >&2; cleanup; exit 1
fi
tree() { local out=$1 k; for k in $(pgrep -P "$1"); do out="$out,$(tree "$k")"; done; echo "$out"; }
pids=$(tree $w)
ticks() { local t=0 p; for p in ${1//,/ }; do set -- $(sed 's/.*) //' /proc/$p/stat 2>/dev/null); t=$((t + ${12:-0} + ${13:-0})); done; echo $t; }
for p in ${pids//,/ }; do
  echo "$p $(tr '\0' ' ' </proc/$p/cmdline | grep -o -- '--type=[a-z-]*' || echo main)"
done >"$out/$name.pids"
load0=$(cut -d' ' -f1-3 /proc/loadavg)
a=$(ticks "$pids") am=$(ticks $w)
"$perf" record "${record[@]}" -o "$out/$name.data" -p "$pids" -- sleep "$secs" 2>&1 | tail -1
b=$(ticks "$pids") bm=$(ticks $w)
load1=$(cut -d' ' -f1-3 /proc/loadavg)
hz=$(getconf CLK_TCK)
{
  echo "tree CPU $(( (b - a) * 100 / hz / secs ))% of a core (user+sys), main process $(( (bm - am) * 100 / hz / secs ))%"
  echo "load average before $load0, after $load1"
} | tee "$out/$name.cpu.txt"
cleanup
trap - TERM INT HUP

"$perf" script -F comm,pid,tid,ip,sym,dso -i "$out/$name.data" >"$out/$name.script.txt" 2>/dev/null
python3 "$here/fold.py" "$out/$name.script.txt" "$w" >"$out/$name.folded"
python3 "$here/breakdown.py" "$out/$name.folded" | tee "$out/$name.breakdown.txt"
if [ -n "$flame" ]; then
  "$flame/flamegraph.pl" --title "$name" "$out/$name.folded" >"$out/$name.svg"
  grep '^wubble main process;main thread' "$out/$name.folded" |
    "$flame/flamegraph.pl" --title "$name: main thread" >"$out/$name-main.svg"
fi
rm -f "$out/$name.script.txt"
