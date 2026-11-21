#!/usr/bin/env python3
"""breakdown.py <NAME.folded> : CPU samples by process, and wubble's main thread by what it does.

Reads fold.py's output. A sample's group is the first rule (top to bottom) whose pattern is a
frame of its stack; inside the Slint render the part is the deepest frame matching one of the
part rules, and a render sample matching none is the item tree walk and property evaluation.
Shares are of the main thread's samples. The rules name functions of the pinned Slint 1.19,
Skia and wgpu: a renamed function moves its samples to "other", so check that row first.
"""
import re
import sys
from collections import Counter

if len(sys.argv) != 2:
    sys.exit(__doc__)

GROUPS = [
    ("page frame arrival (on_accelerated_paint: import and blit; present_panes)",
     r"on_accelerated_paint|Host>::present_panes"),
    ("Slint render of the window, total", r"WinitWindowAdapter>::draw$|SkiaRenderer>::render_to_canvas"),
    ("CEF message pump (Chromium in-process and host work)", r"Host>::pump$|Engine>::pump$"),
    ("event loop, Wayland, other", r"."),
]
PARTS = [
    ("shell step and scene (the BeforeRendering hook)", r"wubble_ui::driver::install"),
    ("Skia flush to the GPU (with the NVIDIA driver)", r"GrDirectContext_flush|DirectContext::flush|flush_and_submit|GrDrawingManager::flush"),
    ("present and surface acquire", r"Queue>::present$|get_current_texture|Skia present ordering"),
    ("opacity, clip and layer bookkeeping", r"visit_opacity|visit_clip|visit_layer|SkCanvas::restore|SkCanvas::saveLayer|save_state|restore_state|getLocalClipBounds"),
    ("recording paths", r"SkiaItemRenderer>::draw_path"),
    ("recording rectangles, images, shadows", r"SkiaItemRenderer>::draw_(border_)?rectangle|SkiaItemRenderer>::draw_image|SkiaItemRenderer>::draw_box_shadow|SkiaItemRenderer>::draw_window_background"),
    ("recording text", r"SkiaItemRenderer>::draw_text|draw_glyph_run"),
    ("partial rendering: dirty region", r"compute_dirty_regions|apply_dirty_region"),
    ("copy of the retained frame to the window", r"copy_retained"),
]

procs, main, groups, parts = Counter(), 0, Counter(), Counter()
for line in open(sys.argv[1], errors="replace"):
    stack, _, n = line.rstrip("\n").rpartition(" ")
    n = int(n)
    frames = stack.split(";")
    procs[frames[0]] += n
    if frames[0] != "wubble main process" or len(frames) < 2 or frames[1] != "main thread":
        continue
    main += n
    group = next(g for g, rule in GROUPS if any(re.search(rule, f) for f in frames))
    groups[group] += n
    if group.startswith("Slint render"):
        part = "item tree walk and property evaluation"
        for f in frames:
            for name, rule in PARTS:
                if re.search(rule, f):
                    part = name
        parts[part] += n

total = sum(procs.values())
print("| User CPU samples by process | share |\n|---|---|")
for p, n in procs.most_common():
    print(f"| {p} | {100 * n / total:.1f} % |")
print(f"\n| Main thread ({main} of the main process's "
      f"{procs['wubble main process']} samples) | share |\n|---|---|")
for g, _ in GROUPS:
    print(f"| {g} | {100 * groups[g] / max(main, 1):.1f} % |")
    if g.startswith("Slint render"):
        for p, n in parts.most_common():
            print(f"| - {p} | {100 * n / max(main, 1):.1f} % |")
