# Open decisions (owner)

Waiting for the owner; settled ones go to [docs/decisions.md](../../docs/decisions.md), one line each. The despawn
path for keyed host state is in [surface-manager.md](surface-manager.md); the approved pop-up minimum-size token is
in [02-tabs-windows.md](../upcoming/02-tabs-windows.md).

- Mac heat (measure first with `scripts/bench/mac-energy.sh`): an opaque window on macOS, a lower rate for the start page's liquid
- PR #1's defaults as merged: `install.sh` installs on the sha256 alone without `minisign` (a warning), page Escape, sentence-case labels, agent-tab refusals, the shortcut reset
- PR #2's choices as merged: compact mode is the sidebar toggle; the autofill key chip at the page's top left
- Attach FFmpeg's source tarball to each release, or keep the link in the notes
- The inset-shadow edge under partial rendering: snap the shadow to the pixel grid (moves its edges up to 3 levels) or leave it
- Hardware video decode: keep the opt-in row (unsandboxed GPU process) or remove it
- Start page liquid: stirring per second or per frame
- Ship codecs by default or opt-in (AVC/AAC patent exposure)
- Delete the 38 GB Chromium checkout used for the codec build
- Apple Passwords / passkeys entitlement request (the owner files it)
- Video strip: does a collapse carry over to later pages; Pip: a cropped video or the page's own element fullscreen
- Sidebar day labels ("Today": the design draws none); whether `cpu_fallback > 0` fails `--smoke frames`
- If Turso (pre-1.0) proves unstable: the schema stays SQLite-compatible so `rusqlite` can replace it
