#!/bin/sh
# Builds wubble's bundled ffmpeg (`wubble-ffmpeg`) from the pinned upstream source, for this machine
# (linux-x86_64 or macos-aarch64): an LGPL-2.1-or-later build that only copies streams between
# containers (docs/decisions.md "Media joining"). No encoders, no network, no system libraries, no
# GPL or nonfree parts. Run by package-linux.sh and package-macos.sh; by hand:
#
#   scripts/release/build-ffmpeg.sh [--out DIR]   # DIR/wubble-ffmpeg, DIR/ffmpeg-build.txt, DIR/COPYING.LGPLv2.1
#
# Needs a C compiler (gcc or Xcode's clang), make, curl, tar, xz and sha256sum or shasum: nothing
# from Homebrew. The tarball goes to dist/ffmpeg-src (kept); a result already built from the same
# pins is reused. Nothing here runs in a normal `cargo build`.
set -eu

# Bump the two together, from https://ffmpeg.org/releases/ (the release key is
# FCF9 86EA 15E6 E293 A564 4F10 B432 2F04 D676 58D8: check the .asc once when you bump).
VERSION=8.1.3
SHA256=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3

die() { echo "build-ffmpeg: $*" >&2; exit 1; }
say() { echo "build-ffmpeg: $*" >&2; }

repo=$(cd "$(dirname "$0")/../.." && pwd)
out=$repo/dist/ffmpeg
while [ $# -gt 0 ]; do
    case $1 in
        --out) [ $# -ge 2 ] || die "--out needs a folder"; out=$2; shift 2 ;;
        *) die "unknown argument $1 (usage: build-ffmpeg.sh [--out DIR])" ;;
    esac
done

# The whole configuration: what a stream copy needs and nothing else. Demuxers and muxers for
# MP4/MOV (also fragmented), MPEG-TS, Matroska/WebM and bare audio; the parsers that read a
# stream's headers without decoding it; the bitstream filters (aac_adtstoasc is what puts ADTS AAC
# from an HLS segment into an MP4); the AAC decoder, which the AAC-in-TS probe needs for the
# sample rate. Everything else, `--disable-everything`.
CONFIG="--disable-everything --disable-autodetect --disable-network --disable-doc --disable-debug
 --disable-ffplay --disable-ffprobe --disable-avdevice --disable-swscale --disable-swresample
 --disable-x86asm --enable-small --enable-static --disable-shared
 --enable-protocol=file,pipe
 --enable-demuxer=mov,mpegts,matroska,aac,mp3,ac3,eac3,flac,ogg
 --enable-muxer=mp4,matroska,webm,ipod,adts,mp3
 --enable-parser=h264,hevc,aac,aac_latm,ac3,mpegaudio,vp8,vp9,av1,opus,vorbis,flac,mpeg4video
 --enable-bsf=aac_adtstoasc,extract_extradata,vp9_superframe,h264_mp4toannexb,hevc_mp4toannexb
 --enable-decoder=aac,aac_latm"
CONFIG=$(echo $CONFIG) # one line, for the stamp and the notices

os=$(uname -s)
arch=$(uname -m)
case $os-$arch in
    Linux-x86_64) platform=linux-x86_64; extra="" ;;
    Darwin-arm64) platform=macos-aarch64; extra="--cc=clang --extra-cflags=-mmacosx-version-min=12.0 --extra-ldflags=-mmacosx-version-min=12.0" ;;
    *) die "no build for $os $arch (linux-x86_64 and macos-aarch64 only)" ;;
esac
stamp="ffmpeg $VERSION $SHA256 $platform $CONFIG $extra"

if [ -x "$out/wubble-ffmpeg" ] && [ -f "$out/ffmpeg-build.txt" ] \
    && [ "$(cat "$out/ffmpeg-build.txt")" = "$stamp" ]; then
    say "up to date: $out/wubble-ffmpeg"
    exit 0
fi

for tool in cc make curl tar xz; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is needed"
done
if command -v sha256sum >/dev/null 2>&1; then
    sum() { sha256sum "$1" | cut -d' ' -f1; }
else
    sum() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

src=$repo/dist/ffmpeg-src
mkdir -p "$src" "$out"
tarball=$src/ffmpeg-$VERSION.tar.xz
if [ ! -f "$tarball" ] || [ "$(sum "$tarball")" != "$SHA256" ]; then
    say "downloading ffmpeg $VERSION"
    curl -fsSL -o "$tarball" "https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz"
fi
[ "$(sum "$tarball")" = "$SHA256" ] || die "$tarball does not match the pinned SHA-256"

work=$(mktemp -d "${TMPDIR:-/tmp}/wubble-ffmpeg.XXXXXX")
trap 'rm -rf "$work"' EXIT
tar -xf "$tarball" -C "$work" --strip-components=1
cd "$work"
say "configuring ($platform)"
# shellcheck disable=SC2086
./configure --prefix=/nonexistent $CONFIG $extra >configure.log 2>&1 || { tail -20 configure.log >&2; die "configure failed"; }
grep -q '^License: LGPL version 2.1 or later' configure.log || die "the build is not plain LGPL-2.1+"
jobs=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)
say "building"
make -j"$jobs" >make.log 2>&1 || { tail -20 make.log >&2; die "make failed"; }
strip ffmpeg
cp ffmpeg "$out/wubble-ffmpeg"
cp COPYING.LGPLv2.1 "$out/COPYING.LGPLv2.1"
printf '%s\n' "$stamp" >"$out/ffmpeg-build.txt"
say "built $out/wubble-ffmpeg ($(wc -c <"$out/wubble-ffmpeg" | tr -d ' ') bytes)"
