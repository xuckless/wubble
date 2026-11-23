#!/usr/bin/env bash
# A profile-guided build of wubble (hand run; untested, written without a run).
#
# Three steps: build with instrumentation, run the frame path's own workloads on the invisible
# display so the profile is of page frames and shell redraws, build again with the profile.
# The result lands in target/pgo/<triple>/dist; compare it with the plain dist build through
# profile.sh and `--measure` before any release uses it.
#
#   scripts/bench/pgo.sh [SECONDS_PER_WORKLOAD]
#
# CARGO names the cargo to use (a wrapper that caps memory, on the lead machine).
# Needs `llvm-profdata` of the toolchain's LLVM: `rustup component add llvm-tools`.
set -euo pipefail

cd "$(dirname "$0")/../.."
seconds=${1:-20}
cargo=${CARGO:-cargo}
triple=$(rustc -vV | sed -n 's/^host: //p')
profdata=$(find "$(rustc --print sysroot)" -name llvm-profdata -type f | head -n 1)
[ -n "$profdata" ] || { echo "pgo: no llvm-profdata (rustup component add llvm-tools)" >&2; exit 1; }

raw=target/pgo/raw
merged=target/pgo/wubble.profdata
rm -rf "$raw" && mkdir -p "$raw"

# An explicit --target keeps the flags off build scripts and proc macros.
build() {
    RUSTFLAGS="$1" "$cargo" build --profile dist --target "$triple" --target-dir target/pgo \
        -p wubble -p wubble-helper
}

build "-Cprofile-generate=$PWD/$raw"
bin=target/pgo/$triple/dist/wubble

# The workloads: what the main process does per frame. One row each; add a scenario as a row.
while read -r args; do
    # A workload that fails still leaves its profile; the build below says if none did.
    scripts/headless.sh timeout $((seconds + 60)) "$bin" $args --seconds "$seconds" || true
done <<'EOF'
--measure idle
--measure idle --measure-page wubble://start
--measure scroll
--measure video
--measure typing
--measure resize
EOF

"$profdata" merge -o "$merged" "$raw"
build "-Cprofile-use=$PWD/$merged -Cllvm-args=-pgo-warn-missing-function"
echo "pgo: $bin"
