# Surface manager (the surface store)

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md).

One store for every shell surface ([docs/research/surface-store.md](../../docs/research/surface-store.md)). The owner
calls it the "surface manager" in conversation; code and docs keep `wubble_core::overlay` and "surface store".

## State (2026-11-24)
- Shipped in 0.6.2, tested on Linux and macOS: the table, hover, page-attached and drag parts; of the overlays, downloads and Spaces.
- On `sprint/shell` (2026-11-25), written blind (never compiled): `untested/store-step2` merged with main, then the command bar, focus and keys, the bridge's removal, the lock at none, `pointer_at`'s caller and the gallery's closes. The doubt list is in `docs/research/surface-store.md`.
- Decision pending: one despawn path for keyed host state (below).

## Open
- [~] Overlays written blind (never compiled) on branch `untested/store-step2`, with the content map: extensions, menu, permission, dialog, find bar, sheets, region picker, link hints, notice, error and Boost panels: merged into sprint/shell with main (one compile error found by reading and fixed: two `Host::agent_answer`); compile, test on Linux and macOS
- [~] The command bar through the store. Written blind on sprint/shell, not compiled: a `STORED` kind; the driver keeps its motion and says when it moved (`command-bar-changed`), the host opens or closes the store's entry (`Host::bar_state_changed`), its exit ends with `Mounted::command_bar`; the start page's capsule stays the page's field (no store entry, no refusal)
- [~] Focus and key ownership through the store. Written blind on sprint/shell, not compiled: `focus-owner` (app.slint) is written from `Ui::key_owner` after every step (`settle_layers`); `settle-focus` reads it; the eleven `changed *-open` triggers and the shot picker's are gone
- [~] Remove the `Env::others` / `Env::machine` bridge. Written blind on sprint/shell, not compiled (core, host, the store's and video's tests; `Host::act` takes no window)
- [~] The surface lock's last lines. Written blind on sprint/shell, not compiled: the popovers' hover thumb is the kit's `HoverList`; the baseline is empty
- [~] A host caller for `pointer_at`. Written blind on sprint/shell, not compiled: a drop reaches the page under it only where the store gives the pointer to the page (`page_under`)
- [~] Gallery and `--shot` scenes as action lists. Written blind on sprint/shell, not compiled: a scene starts by closing every overlay the store holds open, and opens the autofill parts through it; `--shot` has no scenes of its own
- [ ] Hand-check on Wayland and macOS the owner's decisions: Escape on an unmoved tab press no longer cancels the click; a lost window focus (pointer outside) cancels a tab drag
- [ ] After the store work, decide (owner, 2026-11-23: "add if we need to"): one despawn path for keyed host state, the one ECS idea worth having. Every per-tab and per-window table in the host is in one registered list that the single "tab gone" / "window gone" effect clears, with a test that fails when a keyed table is not in the list. Three bugs of this shape were found on 2026-11-23 (the clipboard read mark, the input-method flag, private site choices). No ECS framework: state stays one enum per lifecycle and one reducer
