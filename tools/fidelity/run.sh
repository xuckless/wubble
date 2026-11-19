#!/bin/sh
# The design-fidelity harness in one command (tools/fidelity/README.md):
#   tools/fidelity/run.sh [OUT]
# serves the design clone through the harness's vite, shoots every scene of scenes.json and every
# specimen of specimens.json headless at each DPR (in parallel browsers, through the web cache),
# shoots wubble's gallery on the design's parity profile at the web window's size with the
# backdrop frozen, scores every pair into OUT/scores.csv, OUT/scores.md and OUT/sheets/, and
# writes the sign-off page OUT/report.html. OUT defaults to ~/Dev/wubble-wt/notes/fidelity/runs/<date>.
# Environment: WUBBLE_DESIGN (the clone), FIDELITY_DPR (default 1,2), FIDELITY_SKIP (any of
# web,wubble: reuse that side's shots already in OUT), FIDELITY_JOBS (parallel browsers, default
# capture.mjs's), FIDELITY_CACHE (the web cache, default ~/Dev/wubble-wt/notes/fidelity/web-cache;
# `off` disables it), FIDELITY_BASELINE (the run the report compares with, default the L0
# baseline), FIDELITY_WUBBLE_RECTS (space-separated rects files for anatomy crops), CARGO
# (default ~/Dev/wubble-wt/cargo.sh, else cargo). The gallery opens a window: run it in a
# desktop session.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
out=${1:-$HOME/Dev/wubble-wt/notes/fidelity/runs/$(date +%Y-%m-%d)}
design=${WUBBLE_DESIGN:-$HOME/Dev/wubble-wt/ref/the-wallpaper-project}
dprs=${FIDELITY_DPR:-1,2}
skip=${FIDELITY_SKIP:-}
port=${FIDELITY_PORT:-5199}
cache=${FIDELITY_CACHE:-$HOME/Dev/wubble-wt/notes/fidelity/web-cache}
baseline=${FIDELITY_BASELINE:-$HOME/Dev/wubble-wt/notes/fidelity/baseline}
cargo=${CARGO:-$HOME/Dev/wubble-wt/cargo.sh}
[ -x "$cargo" ] || cargo=cargo
mkdir -p "$out"
out=$(cd "$out" && pwd)
t0=$(date +%s)
export WUBBLE_DESIGN=$design

node "$here/extract.mjs" --check || echo "run.sh: the generated kit is stale against the clone (scores still run)"

cap="--out $out/web --dpr $dprs"
if [ "$cache" = off ]; then cap="$cap --no-cache"; else cap="$cap --cache $cache"; fi
[ -z "${FIDELITY_JOBS:-}" ] || cap="$cap --jobs $FIDELITY_JOBS"

# the parity manifest is served by the harness's vite from the design: cached beside the web
# shots under the design state and vite.config.mjs, so a run whose web shots all hit needs no vite
parity_key=$(node --input-type=module -e "import { designState, harnessHash, sha1 } from '$here/lib/cache.mjs'; console.log(sha1(designState('$design') + harnessHash(['$here/vite.config.mjs'])))")
parity_cached="$cache/parity-$parity_key.json"
need_web=1
case ",$skip," in *,web,*) need_web=0 ;; esac
need_vite=1
if [ "$cache" != off ] && [ -f "$parity_cached" ]; then
  if [ $need_web = 0 ] || node "$here/capture.mjs" $cap --probe-cache; then need_vite=0; fi
fi

if [ $need_vite = 1 ]; then
  # the design, served with the fixed clock, the frozen-backdrop switch and the parity pages
  (cd "$design" && WUBBLE_DESIGN_ROOT="$design" FIDELITY_PORT=$port exec setsid npx vite --config "$here/vite.config.mjs") > "$out/vite.log" 2>&1 &
  vite=$!
  trap 'kill -- -$vite 2>/dev/null || kill $vite 2>/dev/null || true' EXIT INT TERM
  i=0
  until curl -sf "http://127.0.0.1:$port/__parity/manifest.json" > "$out/parity.json"; do
    i=$((i + 1)); [ $i -lt 120 ] || { echo "run.sh: vite did not come up (see $out/vite.log)"; exit 1; }
    sleep 0.5
  done
  [ "$cache" = off ] || { mkdir -p "$cache"; cp "$out/parity.json" "$parity_cached"; }
else
  echo "run.sh: every web shot is in the cache; no vite"
  cp "$parity_cached" "$out/parity.json"
fi

if [ $need_web = 1 ]; then
  node "$here/capture.mjs" $cap --base "http://127.0.0.1:$port" || echo "run.sh: some web scenes failed (see above)"
fi
echo "run.sh: web side done after $(( $(date +%s) - t0 )) s"

# wubble at the web window's logical size per DPR (the design draws the browser as a window on
# a desktop; scenes.json `viewportByDpr` keeps the 2x window on the screen)
case ",$skip," in *,wubble,*) ;; *)
  (cd "$repo" && "$cargo" build --workspace)
  for dpr in $(echo "$dprs" | tr ',' ' '); do
    rm -rf "$out/wubble/${dpr}x"
    size=$(node -e "const m=require('$out/web/${dpr}x/sidebar-extended.json');console.log(Math.round(m.frame[2])+'x'+Math.round(m.frame[3]))" 2>/dev/null || echo 1288x816)
    (cd "$repo" && SLINT_SCALE_FACTOR=$dpr WUBBLE_SHOT_SIZE=$size timeout 300 "$cargo" run -p wubble -- \
      --shot-dir "$out/wubble/${dpr}x" --frozen-backdrop --parity "$out/parity.json" --smoke gallery) \
      > "$out/wubble-${dpr}x.log" 2>&1 || echo "run.sh: wubble gallery at ${dpr}x failed (see $out/wubble-${dpr}x.log)"
  done
esac

rects=""
for f in ${FIDELITY_WUBBLE_RECTS:-}; do rects="$rects --wubble-rects $f"; done
node "$here/score.mjs" --web "$out/web" --wubble "$out/wubble" --out "$out" --dpr "$dprs" $rects | tee "$out/scores.md"
node "$here/report.mjs" "$out" --baseline "$baseline"
echo "run.sh: done after $(( $(date +%s) - t0 )) s"
