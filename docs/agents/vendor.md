# Vendored forks

Moved out of [AGENTS.md](../../AGENTS.md) on 2026-11-23, unchanged. Read it before touching anything under `vendor/`.

`vendor/welding/` is our fork of `welding` (MPL-2.0), outside the workspace members: keep its
upstream style, mark each change `wubble change`, list it in `vendor/welding/WUBBLE_CHANGES.md`;
test with `cargo test -p welding`. `vendor/i-slint-renderer-skia/` is Slint's Skia renderer with partial
rendering on its wgpu surface, under the same rules (`WUBBLE_CHANGES.md` there); tested through wubble-ui's
`partial_render` test. `SLINT_SKIA_PARTIAL_RENDERING=off|verify` switches it off or checks every frame.
