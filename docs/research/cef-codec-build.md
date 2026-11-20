# CEF 156 with proprietary codecs: Linux x64 build (2026-11-19)

Question: How do we build CEF 156.0.30 with proprietary codecs (H.264, AAC, HLS) and point wubble at it?
Status: Linux x64 built and smoked (`scripts/build-cef.sh`, `CEF_PATH`; ~38 GB, ~2.5 h); macOS arm64 not started; shipping a codec build is an open owner decision.

Result: `cef_binary_156.0.30+g6028375+chromium-156.0.8078.96_linux64_minimal`, built on this
box (Fedora 44, 28 cores, 31 GB RAM, NVIDIA RTX 5060), no root, no host packages installed.
H.264, AAC, MSE H.264+AAC, MPEG-TS and native HLS play in wubble; HEVC does not (see below).

The scripted form of everything here is `scripts/build-cef.sh WORK [fetch hook build distrib install]`
(branch `m6/codecs`). WORK used here: a directory named `cef-build` beside the checkout; paths below are relative to it.

## Where things are

| What | Path | Size |
|---|---|---|
| depot_tools | `cef-build/depot_tools` | 1.3 GB |
| Chromium + CEF checkout (no history) | `cef-build/chromium_git` (CEF at `chromium/src/cef`) | 29 GB |
| Build output | `cef-build/chromium_git/chromium/src/out/Release_GN_x64` | 6.9 GB |
| Distribution (dir + tarball) | `cef-build/distrib/cef_binary_156.0.30+g6028375+chromium-156.0.8078.96_linux64_minimal{,.tar.bz2}` | 717 MB (tarball 160 MB) |
| Installed for cef-dll-sys | `cef-build/cef_path/156.0.30/cef_linux_x86_64` | 557 MB |
| Total | `cef-build` | 38 GB |

libcef.so: 474 MB, no debug info (the prebuilt's is 1.48 GB with debug info).

## Version: why the crate accepts it, honestly

The pinned crate is `cef-dll-sys 156.6.0+156.0.30`; its build script resolves `CEF_PATH/<build
metadata = 156.0.30>/cef_linux_x86_64` and calls `download_cef::check_archive_json`, which reads
`archive.json`'s `name`, strips `cef_binary_` and `+...`, and requires that version `<= 156.0.30`.
We built the very commit the prebuilt came from (CEF branch 8078, commit `6028375`, Chromium tag
156.0.8078.96), so `make_distrib` named the archive exactly like the prebuilt and
`cef_version.h` says `156.0.30+g6028375+chromium-156.0.8078.96`. `archive.json` is written with
that real name and the sha1 of our own tarball (`19a3b9ef2c897676cedaf261aa10b70cebff9ba1`).
Checked against the prebuilt: same file set in `include/`, `libcef_dll/`, `cmake/`; identical
`CEF_API_HASH_*` and version defines; contents identical apart from whitespace (our translated
C API headers were not clang-formatted, see problems). Runtime: `--smoke boot` prints
`cef boot ok 156.0.30+g6028375+chromium-156.0.8078.96`.

## Recipe (commands that worked, in order)

```sh
W=$PWD/cef-build; mkdir -p $W && cd $W
git clone -q https://chromium.googlesource.com/chromium/tools/depot_tools.git
mkdir -p automate && curl -sSfL -o automate/automate-git.py \
  https://raw.githubusercontent.com/chromiumembedded/cef/8078/tools/automate/automate-git.py
cat > env.sh <<'EOF'
export CEF_BUILD=/path/to/cef-build   # the WORK directory
export PATH="$CEF_BUILD/depot_tools:$PATH"
export DEPOT_TOOLS_METRICS=0 DEPOT_TOOLS_UPDATE=0 DEPOT_TOOLS_REPORT_BUILD=0
export CEF_USE_GN=1 CEF_ARCHIVE_FORMAT=tar.bz2
export GN_DEFINES="is_official_build=true use_thin_lto=false is_cfi=false chrome_pgo_phase=0 symbol_level=0 blink_symbol_level=0 v8_symbol_level=0 use_sysroot=true proprietary_codecs=true ffmpeg_branding=Chrome concurrent_links=2"
EOF
. ./env.sh
(cd depot_tools && ./ensure_bootstrap)   # do this first (see problems)

# 1. fetch: 06:02 -> 06:11 (8 min sync incl. hooks: sysroot, clang, node...), 30 GB
nohup nice -n 10 python3 automate/automate-git.py --download-dir=$W/chromium_git \
  --depot-tools-dir=$W/depot_tools --branch=8078 --checkout=6028375 --no-chromium-history \
  --no-build --no-distrib --x64-build --no-debug-build > fetch.log 2>&1 &

# 2. hook: applies CEF's 118 patches to Chromium, translates the API, runs gn gen (~1 min)
cd chromium_git/chromium/src/cef && GN_OUT_CONFIGS=Release_GN_x64 python3 tools/gclient_hook.py
#   args.gn then has proprietary_codecs=true ffmpeg_branding="Chrome" enable_widevine=true ...;
#   gn says use_vaapi=true enable_hls_demuxer=true enable_platform_hevc=true

# 3. build: 2 h 16 min at -j 16 (siso, local), ~60k steps, out/ 6.9 GB
cd .. && nohup nice -n 10 autoninja -j 16 -C out/Release_GN_x64 libcef chrome_sandbox > $W/build.log 2>&1 &

# 4. distribution: 57 s
cd cef/tools && python3 make_distrib.py --output-dir=$W/distrib --allow-partial --ninja-build \
  --x64-build --minimal --no-docs --no-symbols

# 5. install in download-cef's layout (what cef-dll-sys would unpack): scripts/build-cef.sh $W install
#    Release/* + Resources/* + CMakeLists.txt cmake include libcef_dll CREDITS.html
#    -> $W/cef_path/156.0.30/cef_linux_x86_64, plus archive.json {type: minimal, name, sha1}
```

Memory during the build: 16 clang jobs at nice 10 left 8-15 GB available with the desktop
(and a running wubble) in use; swap (7 GB) was already full before the build began. The load
stayed usable. `concurrent_links=2` and no LTO: the one big link (libcef.so) was not a problem.

## Pointing wubble at it

```sh
# from the wubble checkout
CEF_PATH=$W/cef_path cargo build --workspace
# build script prints: Using versioned CEF path from environment: .../cef_path/156.0.30/cef_linux_x86_64
# and copies libcef.so etc. beside the binaries; unsetting CEF_PATH and rebuilding goes back to
# the prebuilt (its copy in OUT_DIR is reused). No code change needed.
```

`CEF_DOWNLOAD_URL` (a mirror with the Spotify index layout) would be the alternative for CI or
other machines; not done.

## Smokes on the codec build (Linux, Wayland, NVIDIA)

- `--smoke boot`: exit 0, `cef boot ok 156.0.30+g6028375+chromium-156.0.8078.96`
- `--smoke frames=120`: exit 0, `pixels ok`, `glass ok quality=Full`, `frames=121 accelerated=121 cpu_fallback=0`
- `--smoke nav`: exit 0, `nav ok`
- `--smoke codecs` and `--smoke codecs=bare` (clips from `scripts/codec-clips.sh`): exit 0, `smoke: codecs ok`
- Sandbox: on (no `--no-sandbox`); renderer and GPU-side zygote children run with
  `Seccomp: 2` in their own user and PID namespaces, as with the prebuilt. `chrome-sandbox` is
  shipped but not setuid; the namespace sandbox does not need it.

## Codec matrix, prebuilt vs codec build

`ok` = plays (time advances) for clips; `isTypeSupported` for support rows (native HLS: `canPlayType`).

| Row | Prebuilt 156.0.30 | Codec build |
|---|---|---|
| support `avc1.64001f, mp4a.40.2` | no | yes (canPlay probably, MSE yes) |
| support `avc1.42E01E` / `avc1.64001f` | no | yes, decodingInfo supported smooth |
| support `mp4a.40.2` | no | yes |
| support `hvc1.1.6.L93.B0` | no | no |
| support `video/mp2t; avc1+mp4a` (MSE) | no | yes |
| support `application/vnd.apple.mpegurl` | no (canPlay "") | yes (canPlay maybe) |
| h264-aac.mp4 | error 4 | plays |
| h264.mp4 | error 4 | plays |
| audio.m4a (AAC) | error 4 | plays |
| hevc.mp4 | error 4 | error 4 |
| mse h264+aac fmp4 (X's shape) | isTypeSupported false | plays |
| native hls h264 fmp4 | error 4 | plays |
| native hls h264 ts | error 4 | plays |
| native hls vp9 fmp4 | error 4 | plays |
| VP8/Vorbis, VP9/Opus (WebM, MP4), AV1 (WebM, MP4), MP3, Ogg, Opus, FLAC, WAV, MSE VP9 | play | play |
| bundled clip: Range, no Range, CORS, blob, data, iframe, autoplay muted | play | play |
| MSE `audio/ogg vorbis`, `audio/flac`, `audio/wav` | not MSE types | same |

HEVC: Chromium on Linux has no software HEVC decoder; `enable_platform_hevc` only lets the
platform (VA-API) decode it, and on this NVIDIA setup `canPlayType('hvc1...')` stays empty, so
Chromium found no hardware HEVC decoder here (why exactly was not checked) (would need nvidia-vaapi-driver with HEVC and Chromium's VA-API path
enabled for the GPU process). On macOS, VideoToolbox should decode HEVC once the build has it.

`--smoke codecs` change (branch `m6/codecs`): when the `avc1.42E01E` support row is yes, the
H.264/AAC/MPEG-TS/HLS rows (and their clips when present) are asserted; HEVC stays reported
only. On the prebuilt it prints `proprietary codecs in this build: no, reported` and passes as
before.

## Problems and fixes

1. `gclient_hook.py` failed in `gn gen` with "python3_bin_reldir.txt not found": with
   `DEPOT_TOOLS_UPDATE=0` automate-git never bootstraps depot_tools' python. Fix:
   `cd depot_tools && ./ensure_bootstrap` (it must run from inside depot_tools; from outside
   it fails on relative paths). Side effect of the first, failed hook run: the CEF translator
   had already written the C API headers without clang-format, so they differ from the
   prebuilt's in whitespace only (hashes identical). Bootstrapping before the hook avoids it.
2. The `cef` target (what automate-git builds) failed at step 59308 of 59539 in `cefclient`:
   `'gtk/gtk.h' file not found`. With `use_sysroot=true` CEF sets `cef_use_gtk=false` and so
   gives its samples no GTK include path, but cefclient still includes gtk.h. The samples are
   not in the minimal distribution: build `libcef chrome_sandbox` instead (23 s more; libcef.so
   had already linked).
3. Fedora is not a supported build host; nothing else was needed (no `install-build-deps`,
   no sudo): the Debian bullseye sysroot, clang, node, gn, siso all came from the checkout.
   Host tools used: python3 3.14, git, curl.
4. autoninja now drives siso (`use_siso=true`), local only (`--offline -local_jobs=16`).

Privacy: no git identity used or set, `DEPOT_TOOLS_METRICS=0`, `DEPOT_TOOLS_REPORT_BUILD=0`,
`DEPOT_TOOLS_UPDATE=0`, no global git config change. Network: chromium.googlesource.com,
GitHub (CEF), Google storage/CIPD for the sysroot and toolchains.

## macOS arm64 on the Mac (not started)

Same recipe with these differences:
- Full Xcode (not only the command line tools) with the macOS 26.5 SDK
  (`build/config/mac/mac_sdk.gni`: `mac_sdk_official_version = "26.5"`), `xcode-select` pointing at it.
- Disk: about the same (~40 GB) plus Xcode; RAM: lower `-j` if the Mac has less than 32 GB.
- GN_DEFINES without `use_sysroot=true`; `GN_OUT_CONFIGS=Release_GN_arm64`;
  automate-git with `--arm64-build`; `make_distrib.py ... --arm64-build`; build
  `autoninja -C out/Release_GN_arm64 libcef` (no chrome_sandbox on macOS; check what the
  minimal distrib wants: the framework plus the helper apps come from the `cef` targets on
  macOS, so try `cef` first there, it has no gtk problem).
- Install: the archive is `cef_binary_156.0.30+g6028375+chromium-156.0.8078.96_macosarm64_minimal.tar.bz2`;
  download-cef's layout is `CEF_PATH/156.0.30/cef_macos_aarch64` = its `Release` dir (the
  framework, Resources stay inside it) plus CMakeLists.txt, cmake, include, libcef_dll,
  CREDITS.html, and archive.json. `scripts/build-cef.sh` does Linux only; its install step
  minus the Resources move is what macOS needs.
- Then `cargo build --workspace && cargo run -p wubble-bundle` with `CEF_PATH` set, and the
  console smoke loop from AGENTS.md (codecs included). HEVC should play there (VideoToolbox).
