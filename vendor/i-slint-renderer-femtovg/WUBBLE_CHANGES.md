# wubble changes to i-slint-renderer-femtovg

Vendored from crates.io `i-slint-renderer-femtovg` 1.19.0 (repository
slint-ui/slint, `internal/renderers/femtovg`), used by wubble under
GPL-3.0-only, one of its licence options (`LICENSES/`). Copied unmodified:
`Cargo.toml` (the registry's normalised manifest), `README.md`, `LICENSES/`,
the sources and `tests/`. Removed: the registry's `Cargo.lock`,
`Cargo.toml.orig`, `.cargo_vcs_info.json` and `.cargo-ok`. Added:
`rustfmt.toml` (no formatting, so a diff against 1.19.0 holds only the changes
below) and this file.

The workspace uses it through `[patch.crates-io]` in the root `Cargo.toml`
and excludes it from its members, so `cargo clippy --workspace` does not lint
it. Its own `tests/image_source.rs` needs dev-dependencies the published
manifest leaves out; wubble's check of the changes is
`crates/wubble-ui/tests/crisp_text.rs` (`cargo test -p wubble-ui --test crisp_text`,
offscreen on a GPU adapter, skipped without one).

The copy moves with the `slint` pin: on a Slint bump, start from the new
release, re-apply each change below, or drop the patch once upstream has it.

## Changes

Every change is marked `wubble change` in the source.

1. **Text origin on whole pixels.** `itemrenderer.rs`: `draw_text` and
   `draw_text_input` align the text item's origin to the device pixel grid
   (`pixel_align_text_origin`), where upstream aligned its alignment anchor
   (the centre of a centred text, the right or bottom edge of an end-aligned
   one). With the anchor aligned, an odd-sized item (a 23 px wide segmented
   label, a 17 px high row text centred vertically) had its origin on a half
   pixel; femtovg places each glyph quad at a whole-pixel offset from the
   origin and samples its glyph atlas with nearest filtering, so the quads sat
   exactly between texels and each glyph fell to one side or the other: columns
   dropped or doubled ('Full' drawn as 'Fuli', 'Ctrl+T' broken) and letters a
   pixel up or down within a line (the "wobbly" text). The glyphs' own
   fractional x positions still render through femtovg's subpixel glyph
   variants, which assume a whole-pixel origin.
2. **Path origin on whole pixels.** `itemrenderer.rs` `draw_path`: the path's
   origin is pixel-aligned like an image's (upstream #6455), so an icon drawn
   on a pixel grid renders the same wherever layout puts it.
