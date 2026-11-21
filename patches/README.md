# Patches

wubble's own changes to the code it builds on, applied by the build scripts. Each patch is one
concern, starts with a header (`What:`, `Why:`, `Applies in:`), marks its code `wubble change`, and
keeps Chromium's or CEF's style. Code in Chromium files sits under `#if BUILDFLAG(ENABLE_CEF)`.

## `chromium/`: CEF 156 (Chromium 156.0.8078.96, CEF 6028375)

Applied in name order by `scripts/build-cef.sh WORK patch`, after CEF's hook has applied CEF's own
patches (the step is idempotent: a patch whose reverse applies is skipped, CEF's
`tools/git_util.py` rule). Names are `NN-<src|cef>-<concern>.patch`: `src` patches apply in
`chromium/src`, `cef` patches in `chromium/src/cef`, both with `git apply -p1`. The queue is applied
as a whole: later patches build on earlier ones, and `10-cef-tabs-windows` lists every file of the
CEF module in `BUILD.gn`.

| Patch | Concern |
|---|---|
| `10-cef-tabs-windows.patch` | The C table `cef_wubble_ext_v1` and the registry: wubble's windows as window controllers per profile, tab window ids, tab events (P1, CEF side) |
| `11-src-tabs-windows.patch` | `tabs.query`, `windows.getAll/getLastFocused`, the current window, tab objects, `tabs.update {active}`, `tabs.remove`, `TabsEventRouter` on wubble's tabs (P1, Chromium side) |
| `20-cef-opened-tabs.patch` | Open requests to wubble and their answers; `clients.openWindow` (P2, CEF side) |
| `21-src-opened-tabs.patch` | `tabs.create` and `runtime.openOptionsPage` through wubble; `windows.create` refused (P2, Chromium side) |
| `30-cef-action.patch` | `chrome.action`: click, state, change notifications (P3) |
| `40-cef-touchpad-pinch.patch` | Windowless views apply a touchpad pinch the page left alone (the OSR view's missing `ForwardTouchpadZoomEventIfNecessary`), and export the marker `cef_wubble_touchpad_pinch_v1`; wubble pinches through DevTools `Input.synthesizePinchGesture` when it finds it (`crates/wubble-engine/src/pinch.rs`), with two synthetic fingers otherwise. **Not yet built into any engine** (2026-11-21): its road is unit-tested only |

wubble finds the table at run time (`crates/wubble-engine/src/ext_api.rs`); a stock CEF lacks it
and wubble runs without these APIs. Patch 40 has its own marker, so the pinch does not depend on
the extension patches being there, nor they on it. Build a patched CEF beside the one in use (never over it):

    CEF_INSTALL=DIR nice timeout 4h scripts/build-cef.sh WORK patch build distrib install
    CEF_PATH=DIR cargo build --workspace

Linux only so far; the macOS and Windows builds of the queue are a TODO item. Design and the API
coverage: [docs/research/extensions.md](../docs/research/extensions.md), section 7.

To change a patch: edit the files in the hooked checkout, then regenerate the patch from the
pristine (CEF-patched) file with `diff -u --label a/<path> --label b/<path>` for `src`, `git diff`
in `chromium/src/cef` for `cef`, keeping the header; check that the whole queue still applies to a
fresh hook and reverses on the patched tree.
