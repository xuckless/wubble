#!/usr/bin/env python3
"""Regenerate every app icon from the masters in assets/icons/*.svg.

    python3 scripts/make-icons.py [--sheet contact-sheet.png]

Needs Python 3 with Pillow and a Chromium (`chromium-browser`, `chromium` or
`google-chrome`, or $CHROMIUM) to rasterise the SVGs: headless Chromium draws the
masters' gradients, masks and filters correctly, which ImageMagick's built-in SVG
renderer does not. Each master is rendered once at 1024 px and area-downscaled
(Lanczos, premultiplied alpha) to every size, so the output is the same on every run.

Masters (artwork from the owner's brand kit, the Icons board and the brand sheet):
  wubble-mac.svg         macOS plate: 824 px squircle on a 1024 px canvas (10 % margin)
  wubble-mac-small.svg   the same plate, thicker W and no highlights, for 16 and 32 px
  wubble-linux.svg       Linux plate with the base edge; also the scalable icon
  wubble-mark.svg        the bare bubble (Windows has no plate), 128 px and up
  wubble-mark-64.svg     48 to 96 px: thicker rim and W, highlight dot dropped
  wubble-mark-32.svg     under 48 px: thicker W, no highlights
  wubble-mark-16.svg     16 px: sheen flattened to one periwinkle

Outputs, all under assets/icons/:
  linux/hicolor/<n>x<n>/apps/wubble.png, linux/hicolor/scalable/apps/wubble.svg
  macos/mac/wubble.icns   (the layout cef's bundler wants: <resources>/mac/wubble.icns)
  windows/wubble.ico
"""
import argparse
import os
import shutil
import struct
import subprocess
import sys
import tempfile
from io import BytesIO
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent / "assets" / "icons"
MASTER = 1024

# size -> master, per platform. The brand kit: under 48 px the highlight dot goes and
# the W thickens; at 16 px the sheen flattens to one periwinkle.
LINUX = {512: "wubble-linux", 256: "wubble-linux", 128: "wubble-linux", 64: "wubble-linux",
         48: "wubble-linux", 32: "wubble-mark-32", 24: "wubble-mark-32", 16: "wubble-mark-16"}
WINDOWS = {256: "wubble-mark", 128: "wubble-mark", 64: "wubble-mark-64", 48: "wubble-mark-64",
           32: "wubble-mark-32", 24: "wubble-mark-32", 16: "wubble-mark-16"}
# .icns members: pixel size -> master (the @2x members are the next size up).
MAC = {1024: "wubble-mac", 512: "wubble-mac", 256: "wubble-mac", 128: "wubble-mac",
       64: "wubble-mac", 32: "wubble-mac-small", 16: "wubble-mac-small"}
# icns type -> pixel size (modern PNG members).
ICNS_TYPES = [("icp4", 16), ("icp5", 32), ("ic11", 32), ("ic12", 64), ("ic07", 128),
              ("ic13", 256), ("ic08", 256), ("ic14", 512), ("ic09", 512), ("ic10", 1024)]


def chromium():
    for c in (os.environ.get("CHROMIUM"), "chromium-browser", "chromium", "google-chrome"):
        if c and shutil.which(c):
            return c
    sys.exit("make-icons: no Chromium found (set $CHROMIUM)")


def render(name, work, browser):
    """The master as a 1024 px RGBA image with a transparent background."""
    svg = ROOT / f"{name}.svg"
    page = work / f"{name}.html"
    shot = work / f"{name}.png"
    page.write_text(
        '<!doctype html><html><body style="margin:0;background:transparent">'
        f'<img src="{svg.as_uri()}" width="{MASTER}" height="{MASTER}" style="display:block">'
        "</body></html>"
    )
    subprocess.run(
        [browser, "--headless", "--disable-gpu", "--no-sandbox", "--hide-scrollbars",
         "--force-device-scale-factor=1", "--default-background-color=00000000",
         f"--window-size={MASTER},{MASTER}", f"--screenshot={shot}", page.as_uri()],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
    return Image.open(shot).convert("RGBA")


def at(masters, name, size):
    img = masters[name]
    return img if size == MASTER else img.resize((size, size), Image.LANCZOS)


def png_bytes(img):
    out = BytesIO()
    img.save(out, "PNG", optimize=True)
    return out.getvalue()


def write_icns(path, images):
    """The icns container: a header and one PNG member per type. Pillow's own
    writer omits the 16 and 32 px members and takes only one master."""
    members = b"".join(
        t.encode() + struct.pack(">I", 8 + len(d)) + d
        for t, d in ((t, png_bytes(images[s])) for t, s in ICNS_TYPES))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"icns" + struct.pack(">I", 8 + len(members)) + members)


def write_ico(path, images):
    """The ico container with a PNG member per size (Pillow's writer resizes one image)."""
    sizes = sorted(images)
    blobs = [png_bytes(images[s]) for s in sizes]
    head = struct.pack("<HHH", 0, 1, len(sizes))
    offset = 6 + 16 * len(sizes)
    entries = b""
    for s, b in zip(sizes, blobs):
        entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(b), offset)
        offset += len(b)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(head + entries + b"".join(blobs))


def contact_sheet(path, linux, windows, mac):
    """Every size of every platform on a light and a dark background, for looking at."""
    rows = [("linux", linux), ("windows", windows), ("macos", mac)]
    pad, label_h = 16, 14
    width = pad + sum(max(s, 40) + pad for s in sorted({*linux, *windows, *mac}))
    height = pad
    for _, imgs in rows:
        height += max(imgs) + label_h + pad
    sheet = Image.new("RGBA", (2 * width, height), (0, 0, 0, 0))
    for half, bg in enumerate(((244, 245, 249, 255), (24, 26, 33, 255))):
        draw = ImageDraw.Draw(sheet)
        draw.rectangle([half * width, 0, (half + 1) * width, height], fill=bg)
        ink = (90, 96, 114, 255) if half == 0 else (163, 169, 184, 255)
        y = pad
        for label, imgs in rows:
            x = half * width + pad
            draw.text((x, y), label, fill=ink)
            y += label_h
            for s in sorted(imgs, reverse=True):
                sheet.alpha_composite(imgs[s], (x, y))
                draw.text((x, y + imgs[s].height + 1 if s <= 64 else y + 2), str(s), fill=ink)
                x += max(s, 40) + pad
            y += max(imgs) + pad
    sheet.save(path)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--sheet", type=Path, help="also write a contact sheet PNG here")
    args = ap.parse_args()
    browser = chromium()

    wanted = sorted({*LINUX.values(), *WINDOWS.values(), *MAC.values()})
    with tempfile.TemporaryDirectory() as tmp:
        masters = {n: render(n, Path(tmp), browser) for n in wanted}

    linux = {s: at(masters, n, s) for s, n in LINUX.items()}
    windows = {s: at(masters, n, s) for s, n in WINDOWS.items()}
    mac = {s: at(masters, n, s) for s, n in MAC.items()}

    hicolor = ROOT / "linux" / "hicolor"
    shutil.rmtree(hicolor, ignore_errors=True)
    for s, img in linux.items():
        d = hicolor / f"{s}x{s}" / "apps"
        d.mkdir(parents=True)
        (d / "wubble.png").write_bytes(png_bytes(img))
    scalable = hicolor / "scalable" / "apps"
    scalable.mkdir(parents=True)
    shutil.copyfile(ROOT / "wubble-linux.svg", scalable / "wubble.svg")
    write_icns(ROOT / "macos" / "mac" / "wubble.icns", mac)
    write_ico(ROOT / "windows" / "wubble.ico", windows)
    if args.sheet:
        args.sheet.parent.mkdir(parents=True, exist_ok=True)
        contact_sheet(args.sheet, linux, windows, mac)
    print("icons written under", ROOT)


if __name__ == "__main__":
    main()
