#!/bin/sh
# mac-energy.sh /path/to/wubble.app [SECONDS] : what wubble costs on a Mac, per scenario, in one
# table. macOS tools only, no sudo: CPU time deltas of the whole process tree from `ps`, wubble's
# own `--measure` report for the frame counts, and `pmset -g therm` before and after (thermal or
# CPU speed limits, if macOS applied any). Each scenario runs on its own throwaway profile and
# opens its own window: leave the Mac alone while it runs (about 5 x (SECONDS + 15) s), on power
# or battery as you normally use it. A baseline of SECONDS comes first, with wubble not running.
#
#   scenario  idle    a still page (`--measure idle`)
#             video   a CSS animation and a canvas redrawn every frame (`--measure video`)
#             scroll  a page scrolled by script every frame (`--measure scroll`)
#             start   the start page (Ctrl+T's, with its living bubble); on a build without
#                     `--measure-page` (before 2026-11-21) ps only, no frame counts
#
# Columns: CPU seconds per second (1.00 = one core busy) for the tree and split main / GPU /
# renderer (Chromium's other helpers are in the tree's total only); WindowServer's over the same
# span (it composites wubble's window, so its cost is wubble's too: compare it with the baseline
# taken first, before wubble starts, with the same windows open); then from wubble's report:
# CEF frames/s, Slint renders/s, scene renders/s, wake-ups of the CEF pump/s.
set -u
app=${1:-}
secs=${2:-20}
[ -n "$app" ] && [ -d "$app" ] || { echo "usage: mac-energy.sh /path/to/wubble.app [SECONDS]" >&2; exit 2; }
bin="$app/Contents/MacOS/wubble"
[ -x "$bin" ] || { echo "mac-energy.sh: no $bin" >&2; exit 2; }
work=$(mktemp -d "${TMPDIR:-/tmp}/wubble-energy.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT INT TERM

# ps's cumulative CPU time ([[dd-]hh:]mm:ss.cc) in centiseconds
cs() { awk -F'[-:.]' '{ n = NF; t = $(n) + 100 * $(n - 1); if (n > 2) t += 6000 * $(n - 2);
  if (n > 3) t += 360000 * $(n - 3); if (n > 4) t += 8640000 * $(n - 4); print t }'; }

# The tree under $1: "pid kind centiseconds" per process; kind main, gpu, renderer or other.
tree() {
  ps -A -o pid=,ppid=,time=,command= | awk -v root="$1" '
    { pid[NR] = $1; ppid[NR] = $2; t[NR] = $3; $1 = $2 = $3 = ""; cmd[NR] = $0; n = NR }
    END {
      inside[root] = 1
      for (pass = 0; pass < 6; pass++) for (i = 1; i <= n; i++) if (inside[ppid[i]]) inside[pid[i]] = 1
      for (i = 1; i <= n; i++) if (inside[pid[i]]) {
        kind = "other"
        if (pid[i] == root) kind = "main"
        else if (cmd[i] ~ /--type=gpu-process/) kind = "gpu"
        else if (cmd[i] ~ /--type=renderer/) kind = "renderer"
        print pid[i], kind, t[i]
      }
    }' | while read -r p k t; do echo "$p $k $(echo "$t" | cs)"; done
}

# WindowServer's cumulative CPU time in centiseconds (0 if ps cannot see it).
winsrv() {
  t=$(ps -A -o time=,comm= | awk '$2 ~ /WindowServer$/ { print $1; exit }')
  if [ -n "$t" ]; then echo "$t" | cs; else echo 0; fi
}

# The CPU seconds per second of each kind between two tree files, over $3 seconds.
rates() {
  awk -v secs="$3" '
    FNR == NR { a[$1] = $3; next }
    { d = $3 - ($1 in a ? a[$1] : 0); s[$2] += d; s["tree"] += d }
    END { printf "%5.2f %5.2f %5.2f %5.2f", s["tree"] / 100 / secs, s["main"] / 100 / secs,
          s["gpu"] / 100 / secs, s["renderer"] / 100 / secs }' "$1" "$2"
}

# One scenario: $1 name, the rest wubble's arguments. Samples the tree after a warm-up of 8 s for
# $secs seconds; a --measure run is given a longer --seconds so it is still recording then.
run() {
  name=$1; shift
  case "$*" in
    # --measure runs on its own throwaway profile
    *--measure*) "$bin" "$@" >"$work/$name.out" 2>&1 & ;;
    *) mkdir -p "$work/profile-$name"
       "$bin" --profile "$work/profile-$name" "$@" >"$work/$name.out" 2>&1 & ;;
  esac
  w=$!
  sleep 8
  tree $w >"$work/$name.a"
  ws0=$(winsrv)
  sleep "$secs"
  tree $w >"$work/$name.b"
  ws1=$(winsrv)
  r=$(rates "$work/$name.a" "$work/$name.b" "$secs")
  ws=$(awk -v a="$ws0" -v b="$ws1" -v s="$secs" 'BEGIN { printf "%5.2f", (b - a) / 100 / s }')
  case "$*" in
    *--measure*) wait $w ;;
    *) kill $w 2>/dev/null; sleep 3; kill -9 $w 2>/dev/null; wait $w 2>/dev/null ;;
  esac
  rate() { awk -v k="$1" '$1 == k { print $3; exit }' "$work/$name.out"; }
  printf "%-7s %s %6s %8s %8s %8s %8s\n" "$name" "$r" "$ws" "$(rate cef_frames)" "$(rate slint_renders)" \
    "$(rate scene_renders)" "$(rate pump_runs)"
}

echo "wubble energy on $(sw_vers -productName 2>/dev/null) $(sw_vers -productVersion 2>/dev/null), $(sysctl -n machdep.cpu.brand_string 2>/dev/null), ${secs} s per scenario"
echo "thermal before:"; pmset -g therm 2>/dev/null | sed 's/^/  /'
echo
ws0=$(winsrv); sleep "$secs"; ws1=$(winsrv)
echo "WindowServer before wubble starts: $(awk -v a="$ws0" -v b="$ws1" -v s="$secs" 'BEGIN { printf "%.2f", (b - a) / 100 / s }') CPU s/s"
echo
echo "scenario  tree  main   gpu  rend winsrv  cef/s  slint/s  scene/s   pump/s"
m=$((secs + 12))
run idle --measure idle --seconds "$m" --warmup 4
run video --measure video --seconds "$m"
run scroll --measure scroll --seconds "$m"
# builds from 2026-11-21 on count the start page's frames too (`--measure-page`)
if "$bin" --help 2>/dev/null | grep -q -- --measure-page; then
  run start --measure idle --measure-page wubble://start --seconds "$m" --warmup 4
else
  run start wubble://start
fi
echo
echo "thermal after:"; pmset -g therm 2>/dev/null | sed 's/^/  /'
echo "wubble's full reports: kept only while this runs (in $work)."
