#!/bin/sh
# The same pages in plain Chromium, headless, for comparison with `scripts/measure-load.sh`.
# The page reports its own timings (assets/probe.js) to the load server, which wubble serves
# (`--measure load --serve-only`). One fresh --user-data-dir per cold sample; the warm samples
# follow in the same process (`?chain=N`: the page loads itself again N times). A warm-up
# page (`/blank?next=PAGE`) loads first, as in wubble's run, so that a cold sample is not also the
# first page of a fresh browser.
#
#   scripts/measure-load-chromium.sh PROCESSES OUT.csv [WARM_RUNS]
#
# CHROMIUM: the binary, default chromium-browser. WUBBLE_CMD as in measure-load.sh. Hand run only.
set -eu
processes=${1:?processes}
out=${2:?out.csv}
warm=${3:-5}
chromium=${CHROMIUM:-chromium-browser}
cmd=${WUBBLE_CMD:-target/release/wubble}
tmp=$(mktemp -d)
# shellcheck disable=SC2086
$cmd --measure load --serve-only 0 > "$tmp/serve.log" 2>&1 &
server=$!
trap 'kill $server 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do
    grep -q '^serving' "$tmp/serve.log" && break
    sleep 0.2
done
port=$(sed -n 's/^serving http:\/\/127.0.0.1:\([0-9]*\)$/\1/p' "$tmp/serve.log")
echo "browser,variant,page,temp,run,fp,fcp,dcl,load,build,s0,host_first_frame,served,persisted,process" > "$out"
echo "$($chromium --version)" >&2
for page in /article /images /scripts /blockable /bigdom; do
    i=0
    while [ "$i" -lt "$processes" ]; do
        before=$(grep -c '^timing,' "$tmp/serve.log" || true)
        want=$((before + 2 + warm))
        $chromium --headless=new --user-data-dir="$tmp/profile-$i" --no-first-run \
            --no-default-browser-check --disable-extensions \
            "http://127.0.0.1:$port/blank?next=$page%3Fchain%3D$warm" > /dev/null 2>&1 &
        pid=$!
        for _ in $(seq 1 300); do
            [ "$(grep -c '^timing,' "$tmp/serve.log" || true)" -ge "$want" ] && break
            sleep 0.1
        done
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
        rm -rf "$tmp/profile-$i" || true
        grep '^timing,' "$tmp/serve.log" | tail -n +"$((before + 2))" | head -n "$((1 + warm))" |
            awk -F, -v i="$i" 'BEGIN{OFS=","} {run = NR - 2; temp = NR == 1 ? "cold" : "warm"; if (NR == 1) run = 0;
                print "chromium", "none", $2, temp, run, $3, $4, $5, $6, $7, $8, "", "", $9, i}' >> "$out"
        i=$((i + 1))
    done
done
echo "wrote $out"
