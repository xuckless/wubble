# System requirements (draft)

Draft from the first Linux-vs-Firefox run (2026-11-25, `scripts/bench/linux-vs-firefox.py`, results in
`.local/bench/20261125-042321/` on the owner's Linux box: i7-14700F, RTX 5060, driver 615.71.09, Fedora 44).
Each line says where it comes from: **measured**, **hard** (a dependency the build or code imposes) or **guess**.
macOS lines come later, from the Mac harness.

## Linux

| | Minimum | Recommended | Source |
|---|---|---|---|
| OS, C library | glibc 2.43 and libstdc++ with GLIBCXX_3.4.29 (Fedora 44 or another 2026 distro) | the same | hard: the release binary links `GLIBC_2.43` symbols (`acosf`, `asinf`, `atan2f`, `remainder`) because it is built on Fedora 44; wubble-helper needs 2.34. Building on an older-glibc host would widen this |
| CPU | x86-64 with SSE3, 4 threads | 4+ cores, 2020 or later | hard (Chromium's baseline); threads and class are guesses from about 10 processes and 0.35 to 0.4 of a desktop core for 1080p60 software video (measured) |
| Memory | 4 GB | 8 GB or more | guess, from one tab measured at 670 to 740 MB PSS (1.3 to 1.4 GB RSS for the process tree) |
| GPU | Vulkan 1.1 with dmabuf import (`VK_EXT_external_memory_dma_buf`, `VK_EXT_image_drm_format_modifier`, `VK_EXT_queue_family_foreign`) | the same | hard: Vulkan is the only import backend on Linux (welding `vulkan_dmabuf.rs`); the version is a guess |
| GPU driver | NVIDIA proprietary (tested 615.71.09; minimum unknown, guess 555 or later); Intel and AMD on Mesa anv or radv (guess Mesa 23 or later, untested) | the same; hardware video decode on NVIDIA needs `libva-nvidia-driver` and the setting | measured (NVIDIA); guesses (minimums, Mesa) |
| Display server | Wayland (tested on GNOME) | Wayland | measured; X11 is linked but untested (guess) |
| Disk | 628 MB for the app, plus the profile | 2 GB free | measured (0.6.1 installed); a fresh profile is tens of MB (guess), a lived-in one 755 MB, mostly CEF's cache (measured) |
| Display | any; pages paint at up to 240 frames a second | | hard: `MAX_FRAME_RATE` in `wubble-gfx/src/pacing.rs` |
