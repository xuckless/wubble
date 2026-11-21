#!/bin/sh
# Builds, signs, notarises and packs the macOS release, on the owner's Mac (docs/release.md):
#
#   CEF_PATH=<the codec CEF> WUBBLE_SIGN_IDENTITY="Developer ID Application: ..." \
#   WUBBLE_NOTARY_PROFILE=<notarytool keychain profile> scripts/release/package-macos.sh [--no-build]
#
# writes into dist/:
#   wubble-<V>-macos-<arch>.tar.xz   the update archive: the signed, notarised, stapled wubble.app
#   wubble-<V>-macos-<arch>.dmg      the disk image for a manual install, signed and notarised
#   *.sha256
#
# The identity and the notary profile come only from the environment (store the profile once
# with `xcrun notarytool store-credentials <name> --apple-id ... --team-id ...`). Nothing edits
# the bundle after it is signed: the updater and install.sh copy it as is.
#
# The app carries wubble-ffmpeg in Contents/MacOS (a stream-copy-only ffmpeg, built here by
# build-ffmpeg.sh: it needs only Xcode's command line tools), signed with the hardened runtime like
# every other executable, so notarisation passes.
#
# Not run in CI; written without a Mac at hand: check each step on first use.
set -eu

repo=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo"
build=1
case "${1:-}" in
--no-build) build=0 ;;
"") ;;
*)
    echo "usage: $0 [--no-build]" >&2
    exit 2
    ;;
esac

say() { printf 'package-macos: %s\n' "$*"; }
die() {
    printf 'package-macos: %s\n' "$*" >&2
    exit 1
}

[ "$(uname -s)" = Darwin ] || die "runs on macOS only"
[ -n "${WUBBLE_SIGN_IDENTITY:-}" ] || die "set WUBBLE_SIGN_IDENTITY to a \"Developer ID Application: ...\" identity"
[ -n "${WUBBLE_NOTARY_PROFILE:-}" ] || die "set WUBBLE_NOTARY_PROFILE to a notarytool keychain profile"
[ -n "${CEF_PATH:-}" ] || die "set CEF_PATH to the codec CEF build (docs/research/cef-codec-build.md)"
for tool in cargo codesign xcrun hdiutil ditto shasum tar git; do
    command -v "$tool" >/dev/null 2>&1 || die "needs $tool"
done

case $(uname -m) in
arm64) arch=aarch64 ;;
x86_64) arch=x86_64 ;;
*) die "unknown processor $(uname -m)" ;;
esac
platform=macos-$arch
version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
[ -n "$version" ] || die "no [workspace.package] version in Cargo.toml"
commit=$(git rev-parse HEAD)
cef_dir=
for d in "$CEF_PATH"/*/cef_macos_"$arch"; do
    [ -f "$d/archive.json" ] && cef_dir=$d
done
[ -n "$cef_dir" ] || die "no <version>/cef_macos_$arch/archive.json under CEF_PATH=$CEF_PATH"
say "CEF: $cef_dir"

target=${CARGO_TARGET_DIR:-$repo/target}
bin=$target/dist
if [ "$build" = 1 ]; then
    say "building wubble $version (profile dist)"
    cargo build --locked --profile dist -p wubble -p wubble-helper -p wubble-bundle
fi
rm -rf "$bin/bundle"
app=$("$bin/wubble-bundle")
[ -d "$app" ] || die "wubble-bundle made no app ($app)"
say "bundled $app"

say "ffmpeg for stream copy (scripts/release/build-ffmpeg.sh; downloads the pinned source once)"
"$repo/scripts/release/build-ffmpeg.sh" --out "$repo/dist/ffmpeg"
cp "$repo/dist/ffmpeg/wubble-ffmpeg" "$app/Contents/MacOS/wubble-ffmpeg"
ffmpeg_version=$(sed -n 's/^ffmpeg \([^ ]*\) .*/\1/p' "$repo/dist/ffmpeg/ffmpeg-build.txt")
ffmpeg_config=$(sed -n 's/^ffmpeg [^ ]* [^ ]* [^ ]* //p' "$repo/dist/ffmpeg/ffmpeg-build.txt")

res=$app/Contents/Resources
cp "$repo/dist/ffmpeg/COPYING.LGPLv2.1" "$res/COPYING.LGPLv2.1"
printf '{"version":"%s","platform":"%s","commit":"%s"}\n' "$version" "$platform" "$commit" >"$res/release.json"
cp LICENSE THIRD_PARTY_NOTICES.md "$res/"
cp crates/wubble-engine/assets/dictionaries/README_en_US.txt "$res/DICTIONARY_NOTICE_en_US.txt"
if [ -f "$cef_dir/CREDITS.html" ]; then cp "$cef_dir/CREDITS.html" "$res/CREDITS.html"; fi
cef_name=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$cef_dir/archive.json")
cat >"$res/SOURCE.md" <<EOF
# Source code

wubble is free software under the GNU General Public License, version 3 only (LICENSE).

The complete corresponding source of this build ($version, commit $commit) is the tag
v$version of https://github.com/xuckless/wubble. It embeds the Chromium Embedded Framework
built from $cef_name with proprietary codecs (docs/research/cef-codec-build.md in that source);
Chromium's source and licences: https://chromium.googlesource.com/chromium/src and CREDITS.html.

wubble-ffmpeg (Contents/MacOS) is FFmpeg $ffmpeg_version, a separate program under the GNU Lesser
General Public License 2.1 or later (COPYING.LGPLv2.1), built unmodified from the upstream release
https://ffmpeg.org/releases/ffmpeg-$ffmpeg_version.tar.xz with scripts/release/build-ffmpeg.sh in
the wubble source (configured: $ffmpeg_config). It copies a video's streams between containers and
changes nothing in them. To use another build, set WUBBLE_FFMPEG to its path.
EOF

entitlements=$repo/scripts/release/macos/entitlements.plist
sign() {
    say "signing ${1#"$app"/}"
    codesign --force --timestamp --options runtime --entitlements "$entitlements" \
        --sign "$WUBBLE_SIGN_IDENTITY" "$1"
}
# The ffmpeg runs plain: the hardened runtime and a timestamp, none of the browser's entitlements.
say "signing Contents/MacOS/wubble-ffmpeg"
codesign --force --timestamp --options runtime --sign "$WUBBLE_SIGN_IDENTITY" "$app/Contents/MacOS/wubble-ffmpeg"
# Inside out: the framework's libraries, the framework, each helper app, then the app.
frameworks=$app/Contents/Frameworks
find "$frameworks" -name '*.dylib' -type f | while read -r lib; do sign "$lib"; done
for fw in "$frameworks"/*.framework; do sign "$fw"; done
for helper in "$frameworks"/*.app; do sign "$helper"; done
sign "$app"
codesign --verify --deep --strict --verbose=2 "$app"

notarise() { # file to submit
    say "notarising $(basename "$1") (this waits for Apple)"
    xcrun notarytool submit "$1" --keychain-profile "$WUBBLE_NOTARY_PROFILE" --wait
}
dist=$repo/dist
mkdir -p "$dist"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
ditto -c -k --keepParent "$app" "$work/wubble.zip"
notarise "$work/wubble.zip"
xcrun stapler staple "$app"
spctl --assess --type execute --verbose "$app" || die "Gatekeeper refuses the app"

name=wubble-$version-$platform
say "packing the update archive"
(cd "$(dirname "$app")" && COPYFILE_DISABLE=1 tar --no-mac-metadata -cJf "$dist/$name.tar.xz" wubble.app)

say "making the disk image"
mkdir -p "$work/dmg"
ditto "$app" "$work/dmg/wubble.app"
ln -s /Applications "$work/dmg/Applications"
rm -f "$dist/$name.dmg"
hdiutil create -volname wubble -srcfolder "$work/dmg" -format UDZO -ov "$dist/$name.dmg"
codesign --force --timestamp --sign "$WUBBLE_SIGN_IDENTITY" "$dist/$name.dmg"
notarise "$dist/$name.dmg"
xcrun stapler staple "$dist/$name.dmg"

(cd "$dist" && shasum -a 256 "$name.tar.xz" >"$name.tar.xz.sha256" && shasum -a 256 "$name.dmg" >"$name.dmg.sha256")
say "done:"
(cd "$dist" && ls -l "$name".*)
