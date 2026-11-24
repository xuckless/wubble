#!/usr/bin/env bash
# Hand-run memory comparison (docs/todo/performance.md "Performance pass", Memory): starts another browser on a
# throwaway profile with N tabs of the same static article `--measure memory` uses, each tab on
# its own site (t<i>.localhost, served on loopback by python3), waits SETTLE seconds, sums PSS
# from /proc/<pid>/smaps_rollup over the browser's process tree (the method wubble's
# `memreport` uses), prints one CSV row and ends the browser. Linux only; nothing is fetched
# from outside, and the browser's own background traffic is the browser's.
#
#   scripts/mem-compare.sh chromium-browser 10        # or firefox; SETTLE=20 to wait longer
set -euo pipefail

browser=${1:?browser command (chromium-browser, google-chrome, firefox, zen)}
tabs=${2:?tab count}
settle=${SETTLE:-15}
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'kill "${pid:-}" "${server:-}" 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$work"' EXIT

mkdir -p "$work/www/memory" "$work/profile"
cp "$root/crates/wubble/assets/article.html" "$work/www/memory/index.html"
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$work/www" >/dev/null 2>&1 &
server=$!

urls=()
for ((i = 0; i < tabs; i++)); do urls+=("http://t$i.localhost:$port/memory/"); done
case "$browser" in
  *firefox* | *zen*)
    "$browser" --new-instance --profile "$work/profile" "${urls[@]}" >/dev/null 2>&1 &
    ;;
  *)
    "$browser" --user-data-dir="$work/profile" --no-first-run --no-default-browser-check \
      "${urls[@]}" >/dev/null 2>&1 &
    ;;
esac
pid=$!
sleep "$settle"

# The tree under the started process, by parent pid.
tree() {
  local all=("$1") i=0
  while ((i < ${#all[@]})); do
    for kid in $(ps -o pid= --ppid "${all[$i]}" 2>/dev/null); do all+=("$kid"); done
    i=$((i + 1))
  done
  echo "${all[@]}"
}
pss=0 rss=0 n=0
declare -A kind
for p in $(tree "$pid"); do
  read -r r s < <(awk '/^Rss:/{r=$2} /^Pss:/{s=$2} END{print r+0, s+0}' "/proc/$p/smaps_rollup" 2>/dev/null || echo "0 0")
  rss=$((rss + r)) pss=$((pss + s)) n=$((n + 1))
  # Chromium's --type=, Firefox's trailing process kind (tab, gpu, socket, ...); else browser.
  k=$(tr '\0' ' ' <"/proc/$p/cmdline" 2>/dev/null | grep -oE -- '--type=[a-z-]+|-(isForBrowser|appDir)|[a-z]+$' | tail -1 || true)
  kind[${k:-browser}]=$((${kind[${k:-browser}]:-0} + s))
done
for k in "${!kind[@]}"; do echo "  $k: $((kind[$k] / 1024)) MiB PSS" >&2; done
version=$("$browser" --version 2>/dev/null | head -1 | tr ',' ' ')
echo "browser,version,tabs,processes,pss_mib,rss_mib"
echo "$browser,$version,$tabs,$n,$((pss / 1024)),$((rss / 1024))"
