#!/bin/sh
# Release check (docs/release.md step 7): an installed wubble's first start opens no tab but
# the start page. Starts HOME's installed wubble once (a fresh install, or one with a staged
# update), headless, then compares the saved session's tabs with those before: every new one
# must be wubble://start.
#   scripts/release/first-start-check.sh THROWAWAY_HOME [SECONDS]
# Linux only; needs sqlite3 and scripts/headless.sh. Never point it at your own HOME.
set -eu
home=${1:?usage: first-start-check.sh THROWAWAY_HOME [SECONDS]}
secs=${2:-40}
[ "$home" != "$HOME" ] || { echo "first-start-check: not your own HOME" >&2; exit 2; }
db=$home/.local/share/wubble/wubble.db
here=$(cd "$(dirname "$0")/../.." && pwd)
tabs() { [ -f "$db" ] && sqlite3 "$db" "select url from tabs order by url" || true; }
before=$home/.first-start-tabs
tabs >"$before"
env -u XDG_DATA_HOME -u XDG_CONFIG_HOME -u XDG_CACHE_HOME -u XDG_STATE_HOME HOME="$home" \
    timeout "$secs" "$here/scripts/headless.sh" "$home/.local/bin/wubble" >/dev/null 2>&1 || true
new=$(tabs | grep -vxF -f "$before" || true)
rm -f "$before"
odd=$(printf '%s\n' "$new" | grep -vx -e '' -e 'wubble://start' || true)
if [ -n "$odd" ]; then
    printf 'first-start-check: FAILED: the first start opened\n%s\n' "$odd" | cut -c1-200 >&2
    exit 1
fi
echo "first-start-check: ok ($(printf '%s' "$new" | grep -c . || true) new tab(s), all wubble://start)"
