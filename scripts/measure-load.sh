#!/bin/sh
# `--measure load`, one process per cold sample (a fresh throwaway profile has an empty HTTP
# cache); each process also loads every page warm. Hand run only; prints nothing but progress.
#
#   scripts/measure-load.sh VARIANT PROCESSES OUT.csv [WARM_RUNS]
#
# VARIANT: blocker | blocker-off | no-hook | no-observer | observer-only (see `--load-variant`).
# WUBBLE_CMD: how to start wubble, default `target/release/wubble`; in a lane worktree
# `.local/bin/cargo.sh run --release -p wubble --`.
set -eu
variant=${1:?variant}
processes=${2:?processes}
out=${3:?out.csv}
warm=${4:-5}
cmd=${WUBBLE_CMD:-target/release/wubble}
tmp=$(mktemp -d)
echo "browser,variant,page,temp,run,fp,fcp,dcl,load,build,s0,host_first_frame,served,persisted,process" > "$out"
i=0
while [ "$i" -lt "$processes" ]; do
    # shellcheck disable=SC2086
    timeout 240 $cmd --measure load --load-variant "$variant" --runs "$warm" ${WUBBLE_LOAD_ARGS:-} --out "$tmp/one.csv" > "$tmp/log" 2>&1 || {
        echo "process $i failed (see $tmp/log)" >&2
        i=$((i + 1))
        continue
    }
    tail -n +2 "$tmp/one.csv" | sed "s/\$/,$i/" >> "$out"
    grep "^blocker decisions" "$tmp/log" || true
    i=$((i + 1))
done
echo "wrote $out"
