#!/bin/sh
# video-page.sh [DIR] : the profiling fixture into DIR (default ./bench-fixture): an 8 s 1080p60
# VP9 clip (ffmpeg's testsrc2, no sound) and `video.html`, a still page that loops it muted at
# 854x480, the shape of the 2026-11-20 flame graph. Serve DIR with profile.sh -s DIR.
set -u
dir=${1:-bench-fixture}
mkdir -p "$dir" || exit 1
[ -s "$dir/clip-1080p60.webm" ] ||
  ffmpeg -hide_banner -loglevel error -y -f lavfi -i testsrc2=size=1920x1080:rate=60:duration=8 \
    -c:v libvpx-vp9 -b:v 4M -deadline realtime -cpu-used 8 -row-mt 1 -g 120 \
    "$dir/clip-1080p60.webm" || exit 1
cat >"$dir/video.html" <<'HTML'
<!doctype html><meta charset=utf-8><title>video</title>
<style>body{margin:24px;font:16px sans-serif;background:#f4f4f4}video{width:854px;height:480px;display:block}</style>
<p>A still page with one looping 1080p60 VP9 video, shown at 854x480.</p>
<video src="clip-1080p60.webm" autoplay muted loop playsinline></video>
HTML
echo "fixture in $dir"
