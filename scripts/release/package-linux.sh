#!/bin/sh
# Builds and packs the Linux release (docs/release.md):
#
#   CEF_PATH=<the codec CEF> scripts/release/package-linux.sh [--no-build] [--allow-prebuilt]
#
# writes into dist/:
#   wubble-<V>-linux-x86_64.tar.xz         the bundle (the update archive and what install.sh unpacks)
#   wubble-<V>-linux-x86_64.debug.tar.xz   the split debug symbols (not shipped in the bundle)
#   *.sha256                               `sha256sum` lines for each
#
# The bundle's one top folder holds wubble, wubble-helper, the CEF runtime, release.json, the
# desktop entry and icons under share/, and the notices (LICENSE, THIRD_PARTY_NOTICES.md,
# CREDITS.html, SOURCE.md). wubble finds libcef.so through its RUNPATH ($ORIGIN). chrome-sandbox
# ships without the setuid bit (an install without root cannot set it): CEF then uses the
# user-namespace sandbox.
#
# --no-build reuses the binaries already in the target directory; --allow-prebuilt packs a build
# on the prebuilt CEF (no H.264 or AAC), for a test only.
set -eu

repo=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo"
build=1
prebuilt=0
for arg in "$@"; do
    case $arg in
    --no-build) build=0 ;;
    --allow-prebuilt) prebuilt=1 ;;
    *)
        echo "usage: $0 [--no-build] [--allow-prebuilt]" >&2
        exit 2
        ;;
    esac
done

say() { printf 'package-linux: %s\n' "$*"; }
die() {
    printf 'package-linux: %s\n' "$*" >&2
    exit 1
}

for tool in cargo objcopy strip tar xz sha256sum git; do
    command -v "$tool" >/dev/null 2>&1 || die "needs $tool"
done

version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
[ -n "$version" ] || die "no [workspace.package] version in Cargo.toml"
platform=linux-x86_64
[ "$(uname -m)" = x86_64 ] || die "builds $platform only, on x86_64"
commit=$(git rev-parse HEAD)
if [ -n "$(git status --porcelain)" ]; then
    say "warning: the work tree has changes; the bundle names commit $commit"
fi

# The CEF the build links: the own build with proprietary codecs (docs/decisions.md "Release codecs").
cef_dir=
if [ -n "${CEF_PATH:-}" ]; then
    for d in "$CEF_PATH"/*/cef_linux_x86_64; do
        [ -f "$d/archive.json" ] && cef_dir=$d
    done
    [ -n "$cef_dir" ] || die "no <version>/cef_linux_x86_64/archive.json under CEF_PATH=$CEF_PATH"
    say "CEF: $cef_dir"
elif [ "$prebuilt" = 1 ]; then
    say "warning: no CEF_PATH: the prebuilt CEF, without H.264 or AAC; not for release"
else
    die "set CEF_PATH to the codec CEF build (scripts/build-cef.sh), or pass --allow-prebuilt for a test"
fi

target=${CARGO_TARGET_DIR:-$repo/target}
bin=$target/dist
if [ "$build" = 1 ]; then
    say "building wubble $version (profile dist, symbols kept for splitting)"
    CARGO_PROFILE_DIST_STRIP=none CARGO_PROFILE_DIST_DEBUG=line-tables-only \
        cargo build --locked --profile dist -p wubble -p wubble-helper
fi
for b in wubble wubble-helper; do
    [ -x "$bin/$b" ] || die "$bin/$b is missing (build first, or drop --no-build)"
done
# Without CEF_PATH the build copied the prebuilt runtime beside the binaries.
[ -n "$cef_dir" ] || cef_dir=$bin

dist=$repo/dist
top=wubble-$version-$platform
stage=$dist/stage/$top
debug=$dist/stage/$top.debug
rm -rf "$dist/stage"
mkdir -p "$stage" "$debug"

say "splitting symbols"
for b in wubble wubble-helper; do
    objcopy --only-keep-debug "$bin/$b" "$debug/$b.debug"
    cp "$bin/$b" "$stage/$b"
    strip --strip-debug --strip-unneeded "$stage/$b"
    objcopy --add-gnu-debuglink="$debug/$b.debug" "$stage/$b"
done

say "copying the CEF runtime"
for f in libcef.so libvk_swiftshader.so libvulkan.so.1 vk_swiftshader_icd.json chrome-sandbox \
    icudtl.dat v8_context_snapshot.bin chrome_100_percent.pak chrome_200_percent.pak resources.pak; do
    [ -e "$cef_dir/$f" ] || die "$cef_dir/$f is missing"
    cp "$cef_dir/$f" "$stage/$f"
done
cp -R "$cef_dir/locales" "$stage/locales"
chmod 0755 "$stage/chrome-sandbox"

say "writing release.json, the desktop entry and the notices"
printf '{"version":"%s","platform":"%s","commit":"%s"}\n' "$version" "$platform" "$commit" >"$stage/release.json"
mkdir -p "$stage/share/applications" "$stage/share/icons"
cp assets/icons/linux/wubble.desktop "$stage/share/applications/wubble.desktop"
cp -R assets/icons/linux/hicolor "$stage/share/icons/hicolor"
cp LICENSE "$stage/LICENSE"
cp THIRD_PARTY_NOTICES.md "$stage/THIRD_PARTY_NOTICES.md"
if [ -f "$cef_dir/CREDITS.html" ]; then
    cp "$cef_dir/CREDITS.html" "$stage/CREDITS.html"
elif [ "$prebuilt" = 1 ]; then
    say "warning: no CREDITS.html beside the prebuilt runtime; a test bundle only"
else
    die "no CREDITS.html (Chromium's licences) in $cef_dir"
fi
cef_name=unknown
if [ -f "$cef_dir/archive.json" ]; then
    cef_name=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$cef_dir/archive.json")
fi
cat >"$stage/SOURCE.md" <<EOF
# Source code

wubble is free software under the GNU General Public License, version 3 only (LICENSE).

The complete corresponding source of this build ($version, commit $commit) is the tag
v$version of https://github.com/xuckless/wubble (git clone, then git checkout v$version).
It embeds the Chromium Embedded Framework built from $cef_name with proprietary codecs, by
scripts/build-cef.sh in that source (docs/research/cef-codec-build.md); Chromium's own source
and licences: https://chromium.googlesource.com/chromium/src and CREDITS.html.
EOF

say "packing"
tar_owner=
if tar --version 2>/dev/null | grep -q GNU; then
    tar_owner="--owner=0 --group=0 --numeric-owner --sort=name"
fi
archive=$dist/$top.tar.xz
# shellcheck disable=SC2086 # tar_owner is a list of options or empty
tar $tar_owner -C "$dist/stage" -cf - "$top" | xz -T0 -9 >"$archive"
# shellcheck disable=SC2086
tar $tar_owner -C "$dist/stage" -cf - "$top.debug" | xz -T0 -6 >"$dist/$top.debug.tar.xz"
(cd "$dist" && sha256sum "$top.tar.xz" >"$top.tar.xz.sha256" && sha256sum "$top.debug.tar.xz" >"$top.debug.tar.xz.sha256")
rm -rf "$dist/stage"

say "done:"
(cd "$dist" && ls -l "$top".*)
