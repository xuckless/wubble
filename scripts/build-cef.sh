#!/bin/sh
# Builds CEF with proprietary codecs (H.264, AAC, MPEG-TS, native HLS; HEVC through a platform
# decoder only) at the exact version the pinned `cef` crate expects (same commit, so the same
# version string and API hashes), and installs it where cef-dll-sys finds it through CEF_PATH.
# Linux x64 only; the macOS arm64 recipe is in docs/decisions.md "Video codecs" and docs/research/cef-codec-build.md.
#
#   scripts/build-cef.sh WORK [STEP...]   # steps: fetch hook patch build distrib install (default: all)
#
# `patch` applies wubble's own queue, patches/chromium/*.patch in name order, after CEF's hook
# (patches/README.md); a patch already applied is skipped, so the step can run again. `install`
# writes to CEF_INSTALL (default WORK/cef_path), so a patched build can sit beside a CEF_PATH in use:
#   CEF_INSTALL=DIR scripts/build-cef.sh WORK patch build distrib install
#
# WORK ends up at ~38 GB: depot_tools 1.3, the Chromium + CEF checkout 29, the build output 7,
# the distribution 0.7, the installed copy 0.6. On 28 cores / 31 GB: fetch 8 min, hook 1 min,
# build 2 h 16 min (-j 16), distrib 1 min. No root needed: Chromium builds with its bundled
# clang and a Debian sysroot. Then build wubble with
#   CEF_PATH=WORK/cef_path cargo build --workspace
# Shipping a codec build needs the AVC and AAC patent licences: keep it opt-in until decided.
set -eu
mkdir -p "${1:?usage: build-cef.sh WORK [STEP...]}"
work=$(cd "$1" && pwd); shift
steps=${*:-fetch hook patch build distrib install}
install_to=${CEF_INSTALL:-$work/cef_path}
queue="$(cd "$(dirname "$0")/.." && pwd)/patches/chromium"
jobs=${CEF_BUILD_JOBS:-16} # compile jobs; 31 GB RAM held 16 with the desktop in use

# The cef crate's version (Cargo.lock: cef-dll-sys 156.6.0+156.0.30), its CEF commit and branch.
cef_version=156.0.30
cef_commit=6028375
cef_branch=8078

export PATH="$work/depot_tools:$PATH"
export DEPOT_TOOLS_METRICS=0 DEPOT_TOOLS_UPDATE=0 DEPOT_TOOLS_REPORT_BUILD=0
export CEF_USE_GN=1 GN_OUT_CONFIGS=Release_GN_x64 CEF_ARCHIVE_FORMAT=tar.bz2
# A plain optimised build: official, but no thin LTO/CFI (they would not link in 31 GB), no
# PGO profiles, no symbols, two links at once.
export GN_DEFINES="is_official_build=true use_thin_lto=false is_cfi=false chrome_pgo_phase=0 \
symbol_level=0 blink_symbol_level=0 v8_symbol_level=0 use_sysroot=true \
proprietary_codecs=true ffmpeg_branding=Chrome concurrent_links=2"
src="$work/chromium_git/chromium/src"

for step in $steps; do
  echo "== $step"
  case $step in
  fetch)
    [ -d "$work/depot_tools" ] ||
      git clone -q https://chromium.googlesource.com/chromium/tools/depot_tools.git "$work/depot_tools"
    (cd "$work/depot_tools" && ./ensure_bootstrap) # gn and the pinned python need it
    mkdir -p "$work/automate"
    curl -sSfL -o "$work/automate/automate-git.py" \
      "https://raw.githubusercontent.com/chromiumembedded/cef/$cef_branch/tools/automate/automate-git.py"
    nice -n 10 python3 "$work/automate/automate-git.py" --download-dir="$work/chromium_git" \
      --depot-tools-dir="$work/depot_tools" --branch=$cef_branch --checkout=$cef_commit \
      --no-chromium-history --no-build --no-distrib --x64-build --no-debug-build
    ;;
  hook) # applies CEF's patches to Chromium and runs gn gen with GN_DEFINES
    (cd "$src/cef" && nice -n 10 python3 tools/gclient_hook.py)
    ;;
  patch) # reverse check, check, apply: CEF tools/git_util.py git_apply_patch_file (BSD-3-Clause)
    for p in "$queue"/*.patch; do
      case $(basename "$p") in
      *-src-*) dir=$src ;;
      *-cef-*) dir=$src/cef ;;
      *) echo "$p: name it NN-src-... or NN-cef-..."; exit 2 ;;
      esac
      if git -C "$dir" apply -p1 --ignore-whitespace --reverse --check "$p" 2>/dev/null; then
        echo "$(basename "$p"): already applied"
      elif git -C "$dir" apply -p1 --ignore-whitespace --check "$p"; then
        git -C "$dir" apply -p1 --ignore-whitespace "$p" && echo "$(basename "$p"): applied"
      else
        echo "$(basename "$p"): does not apply"; exit 1
      fi
    done
    ;;
  build) # not CEF's `cef` group: its sample apps include gtk.h, absent with the sysroot
    (cd "$src" && nice -n 10 autoninja -j "$jobs" -C out/Release_GN_x64 libcef chrome_sandbox)
    ;;
  distrib)
    (cd "$src/cef/tools" && nice -n 10 python3 make_distrib.py --output-dir="$work/distrib" \
      --allow-partial --ninja-build --x64-build --minimal --no-docs --no-symbols)
    ;;
  install) # the layout download-cef's extract_target_archive makes, plus its archive.json
    archive=$(ls "$work"/distrib/cef_binary_${cef_version}+*_linux64_minimal.tar.bz2)
    name=$(basename "$archive" .tar.bz2)
    dest="$install_to/$cef_version"
    rm -rf "$dest" && mkdir -p "$dest"
    tar -xjf "$archive" -C "$dest"
    mv "$dest/$name/Release" "$dest/cef_linux_x86_64"
    mv "$dest/$name"/Resources/* "$dest/cef_linux_x86_64/"
    for f in CMakeLists.txt cmake include libcef_dll CREDITS.html; do
      mv "$dest/$name/$f" "$dest/cef_linux_x86_64/"
    done
    rm -rf "${dest:?}/$name"
    printf '{\n  "type": "minimal",\n  "name": "%s",\n  "sha1": "%s"\n}' "$(basename "$archive")" \
      "$(sha1sum "$archive" | cut -d' ' -f1)" > "$dest/cef_linux_x86_64/archive.json"
    echo "CEF_PATH=$install_to"
    ;;
  *) echo "unknown step $step"; exit 2 ;;
  esac
done
