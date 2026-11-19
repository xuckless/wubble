#!/bin/sh
# Writes the clips `--smoke codecs` plays into DIR (default: ./codec-clips), with the system
# ffmpeg: two seconds each, 160x90, a tone on every track with sound. Nothing is committed: the
# probe takes them from `WUBBLE_CODEC_CLIPS=DIR`. A clip whose encoder this ffmpeg lacks is
# skipped (the probe reports it as missing).
set -u
dir=${1:-codec-clips}
mkdir -p "$dir" || exit 1
v="-f lavfi -i testsrc2=size=160x90:rate=30:duration=2"
a="-f lavfi -i sine=frequency=440:duration=2:sample_rate=48000"
make() { # name, ffmpeg args...
  name=$1; shift
  # shellcheck disable=SC2086
  if ffmpeg -hide_banner -loglevel error -y "$@" "$dir/$name" 2>/dev/null; then
    echo "made $name"
  else
    echo "skipped $name (encoder missing?)"; rm -f "$dir/$name"
  fi
}
h264="-c:v libx264 -profile:v high -pix_fmt yuv420p -g 30"
ffmpeg -hide_banner -encoders 2>/dev/null | grep -q libx264 || h264="-c:v libopenh264 -pix_fmt yuv420p -g 30"
# shellcheck disable=SC2086
{
make h264-aac.mp4 $v $a $h264 -c:a aac -movflags +faststart -shortest
make h264.mp4 $v $h264 -movflags +faststart
make hevc.mp4 $v -c:v hevc_nvenc -tag:v hvc1 -movflags +faststart
make vp8-vorbis.webm $v $a -c:v libvpx -b:v 200k -c:a libvorbis -shortest
make vp9-opus.webm $v $a -c:v libvpx-vp9 -b:v 200k -c:a libopus -shortest
make vp9-opus.mp4 $v $a -c:v libvpx-vp9 -b:v 200k -c:a libopus -movflags +faststart -shortest
make av1.mp4 $v -c:v libsvtav1 -movflags +faststart
make av1-opus.webm $v $a -c:v libsvtav1 -c:a libopus -shortest
make audio.mp3 $a -c:a libmp3lame
make audio.m4a $a -c:a aac
make audio.ogg $a -c:a libvorbis
make audio.opus $a -c:a libopus
make audio.flac $a -c:a flac
make audio.wav $a -c:a pcm_s16le
# HLS as X serves it: H.264 + AAC in fMP4 segments, and the older MPEG-TS shape.
mkdir -p "$dir/hls-fmp4" "$dir/hls-ts" "$dir/hls-vp9"
make hls-fmp4/media.m3u8 $v $a $h264 -c:a aac -shortest -f hls -hls_time 1 -hls_playlist_type vod \
  -hls_segment_type fmp4 -hls_fmp4_init_filename init.mp4
make hls-ts/media.m3u8 $v $a $h264 -c:a aac -shortest -f hls -hls_time 1 -hls_playlist_type vod
make hls-vp9/media.m3u8 $v $a -c:v libvpx-vp9 -b:v 200k -c:a libopus -shortest -f hls -hls_time 1 \
  -hls_playlist_type vod -hls_segment_type fmp4 -hls_fmp4_init_filename init.mp4
# MSE feeds: one fragmented MP4 (H.264 + AAC, X's shape) and one video-only WebM (VP9, YouTube's
# shape: a SourceBuffer per track).
make mse-h264-aac.mp4 $v $a $h264 -c:a aac -shortest -movflags frag_keyframe+empty_moov+default_base_moof
make mse-vp9.webm $v -c:v libvpx-vp9 -b:v 200k
}
