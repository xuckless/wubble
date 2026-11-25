#!/usr/bin/env python3
"""Compare the shell's embedded fonts with the faces the design ships (by hand).

    python3 scripts/check-fonts.py [DESIGN_ROOT]

Needs fonttools and brotli (`pip install fonttools brotli`). The design's faces are the
@fontsource-variable 5.3.0 packages of the owner's design repo (default
.local/the-wallpaper-project in the checkout), shipped as latin-subset woff2, which Slint cannot
import; wubble embeds the upstream variable TTFs. For each family this checks the version
string, the variation axes (tag, min, default, max; the design uses wght, Fredoka's wdth is not
selectable from Slint) and that every code point of the design's latin subset is in the TTF.
Exits 1 on any difference.
"""
import os
import sys
from pathlib import Path

from fontTools.ttLib import TTFont

REPO = Path(__file__).resolve().parent.parent
FONTS = REPO / "crates" / "wubble-ui" / "ui" / "fonts"
# family -> (wubble TTF, fontsource package)
FACES = {
    "Geist": ("Geist-Variable.ttf", "geist"),
    "Geist Mono": ("GeistMono-Variable.ttf", "geist-mono"),
    "Fredoka": ("Fredoka-Variable.ttf", "fredoka"),
}


def axes(font):
    return {a.axisTag: (a.minValue, a.defaultValue, a.maxValue) for a in font["fvar"].axes}


def version(font):
    return font["name"].getDebugName(5)


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else
                Path(__file__).resolve().parent.parent / ".local/the-wallpaper-project")
    failed = False
    for family, (ttf, package) in FACES.items():
        ours = TTFont(FONTS / ttf)
        files = root / "node_modules" / "@fontsource-variable" / package / "files"
        theirs = TTFont(files / f"{package}-latin-wght-normal.woff2")
        a, b = axes(ours), axes(theirs)
        # the design's wght axis must be covered; fontsource's wght file drops other axes
        wght_ok = "wght" in a and "wght" in b and a["wght"][0] <= b["wght"][0] and a["wght"][2] >= b["wght"][2]
        missing = set(theirs.getBestCmap()) - set(ours.getBestCmap())
        print(f"{family}: wubble {version(ours)!r} axes {a}")
        print(f"{family}: design {version(theirs)!r} axes {b}; glyphs {len(ours.getGlyphOrder())} vs "
              f"{len(theirs.getGlyphOrder())} (latin subset), code points missing: {len(missing)}")
        if not wght_ok or missing:
            failed = True
            print(f"{family}: DIFFERS (wght covered: {wght_ok})")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
