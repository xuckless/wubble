# Surface store: one store for every shell surface

Plan by a design agent, 2026-11-23, read from `feat/page-layer` (nothing run). Extends `wubble_core::overlay` (no second framework).

(inf) = inferred.

## Inventory: 43 rows, 52 surfaces. `overlay` covers 15 kinds; the rest bypass it.
In `overlay` today: context menu (surfaces.rs:678), permission prompt (:681), find bar (:714), dialog/sign-in (:689), bookmark/print/agent/extension sheets (pages.rs:792, print.rs:358, agent.rs:1669, ext_ui.rs:624), extensions popover (ext_ui.rs:613), extension action popup (:687), downloads popover, Spaces popup, command bar (wubble-ui driver.rs:756), region picker (builtins.rs:995), link hints (:505), notice (videos.rs:1111).
Bypass it: error panel (surfaces.rs:699), `<select>` popup (:719), toast/Esc hint (:950-957), tooltip (host/tooltip.rs:179), tab preview card (preview.rs:104), link status bubble (panes.rs:576), autofill chip/menu/prompt (autofill.rs:340), video controls (videos.rs:803/889), PiP strip, PiP card in Main (videos.rs:1282), reader controls (builtins.rs:989), boost panel (boost.rs:480), glance card (panes.rs:717), swipe chip (gesture.rs:191), exit page tail (snapshots.rs:101), page entrance, sidebar peek, seam bow (app.slint:1622), split ring (panes.rs:657), tab drag ghost (tab_drag.rs:231), drop zone (:253), page drag ghost + drop mark (drag.rs:598/642/868), autoscroll cursor (autoscroll.rs:119), DevTools dock, agent specimen.
Truth only inside Slint (11): downloads popover (app.slint:1607,1725 writes `downloads-open`), Spaces popup (app.slint:1786/1791/1797 + layers.rs:125, tab_drag.rs:171, drag.rs:349, agent.rs:5384: 5 writers), toast (toast.slint:26-43 `shown` + Timer; `toast-shown` 1500 ms in derived.slint), PiP strip (pip-controls.slint:33 `shown: area.has-hover`), sidebar peek cues (app.slint:862-873), exit tails of permission prompt, find bar (`bar.mounted`), extensions popover (`panel.mounted`), region picker (`marquee.fade`), autofill (autofill-menu.slint:37-41), video controls (video-controls.slint:49-55 `any-hover`, `dock.progress`), reader controls (reader-controls.slint:26), boost panel (boost-panel.slint:53); the focus chain `settle-focus` (app.slint:745-842) is a second key owner. Escape handled inside Slint in downloads-popover:44, extensions-popover:45, shot-overlay:37.
Also found: `GlassKeys.extension-sheet` and `.agent-sheet` are both 1000092 (model.slint:230/243): latent glass-ownership bug.

## The store (wubble-core, pure)
```rust
pub struct Row { kind: Kind, scope: Scope /*Window|Tab|Pane*/, z: u16,
  group: Group /*Exclusive|Modal|Holds|Coexist|HoverSlot(u8)*/,
  keys: Keys /*Take|OnAsk|Never*/, pointer: Pointer /*Scrim|DismissOutside|OwnRect|PassThrough|Grab*/,
  cover: Cover /*None|Part|Whole*/, glass: bool,
  life: Life /*Modal|Anchored|Timed(u64)|Hover(Timing)|Drag|Tail*/,
  on_leave: Leave, on_nav: bool, windows: Kinds, escape: Esc /*Never|ShellOnly|Any*/ }
pub const ROWS: &[Row];   // adding a surface = a row + a Kind variant
pub struct Layer { pub kind: Kind, pub tab: Option<TabId> }
pub enum Phase { Waiting { due: u64 }, Open { until: Option<u64> }, Leaving }  // absent = closed
pub struct Ui { windows: BTreeMap<WindowId, Win>, tabs: BTreeMap<TabId, Vec<(Kind, Phase)>> }
pub struct Win { kind: WindowKind, shown: Option<TabId>, keys: Holder /*Page|Shell*/, open: Vec<(Kind, Phase)>, rects: BTreeMap<Layer, Rect> }
pub enum Action { Open{w,layer,now}, Close(Layer), Hover{w,layer,on,now}, Exited(Layer), Tick{now},
  Escape{w,from_page}, Shown{w,tab}, Navigated(TabId), TabGone(TabId), Keys{w,holder}, Placed{w,layer,rect} }
pub enum Effect { Mount(Layer), Unmount(Layer), Cancel(Layer), Arm(Option<u64>), Focus(WindowId, KeyOwner) }
pub fn reduce(ui: Ui, a: Action) -> (Ui, Vec<Effect>);
pub fn stack(ui, w) -> Vec<Layer>;
pub enum Coverage { None, Part, Pixels }
pub fn covers(ui, w, pane: TabId) -> Coverage;
pub fn key_owner(ui, w) -> KeyOwner;  pub fn focus_return(ui, w) -> KeyOwner;
pub fn pointer_owner(ui, w, p: Point) -> PointerOwner;
pub fn escape_target(ui, w, from_page: bool) -> Option<Layer>;
pub fn idle(ui) -> bool;   // no Waiting, no Leaving, no deadline
pub fn ax_roots(ui, w) -> AxRoots { modal: Option<Layer>, roots: Vec<Layer> }
```
- One `Ui` for the app beside `Model` (not persisted); window-scoped entries keyed by window, tab-scoped by tab (a tab keeps its find bar when it changes window).
- Content stays with its machines (menu.rs, prompt.rs, print ...), which lose their Open/Closed variant; the host keeps one `BTreeMap<Layer, Content>` written only by Mount/Unmount (as `views` follows CreateTab/CloseTab).
- Surfaces another owner already has (glance = Model.glances, PiP card = pip_tab, page fullscreen = Screen) are not stored: selectors take a `Facts` argument.
- Timers: `Waiting{due}` / `Open{until}`; reduce returns `Arm(min deadline)`; the host runs ONE slint::Timer and sends `Tick{now}`. Replaces the three HoverClocks, the toast's Slint Timer, the video idle timer. Durations from core.
- Hover arrives as discrete enter/leave actions (pip `area.has-hover`, video `any-hover`, peek cues become `hovered(bool)` callbacks). An exit tail is `Leaving` until the kit's `changed mounted`/`progress == 0` sends `Exited(layer)`; a tail counts as covering.
- Out on purpose: per-frame values (springs, exit_t, page_t, tween progress, autoscroll stretch, glass alpha), field text and queries, rows and content.

## Flow and migration
Slint callbacks + engine events -> actions -> reduce -> `bind::render_ui` writes one `UiView` (a phase per kind + key-owner) -> host runs effects. `settle-focus` becomes one `changed ui.key-owner`.
1. (unblocks the OS layer) ROWS, Coverage, `covers` over a DERIVED stack (overlay_stack widened to every row; derivation stores no copy, so one-owner holds; mirroring is ruled out). Make the Slint-only truths host-visible: pip hover callback, toast timer to the host, `Exited` callbacks for tails. `Need::Covered = covers != None`, evaluated on each page Frame and in BeforeRendering. ~8 files, +600/-50.
2. The 15 overlay kinds + error panel + boost panel store-owned; delete overlay_stack and the machines' Open variants; `downloads-open`/`spaces-open`/`extensions-open` become `in` properties; the 5 Spaces writers become actions; settle-focus and the three Slint Escape handlers go. ~15 files, +500/-700.
3. Hover and timed: tooltip, preview card, status bubble, toast, notice, video controls, pip strip, peek; delete HoverClock and the toast Timer. ~8 files.
4. Page-attached: select popup, autofill, reader, glance via Facts, Placed rects. ~6 files.
5. Drag-bound: both ghosts, drop zone, autoscroll, swipe chip, region picker, hints (Pointer::Grab). ~6 files.
6. Gallery and --shot scenes become action lists; delete direct set_downloads_open/set_spaces_open (gallery.rs:437).

## Tests
- Table: one row per kind x action -> (stack, effects), including refused opens and ignored events in Leaving.
- Property walk over seeded random action sequences: at most one Modal/Holds per window; key_owner in stack or the page; escape_target(Some) closes something, None reaches the page; covers != None whenever a covering row is visible; idle implies Arm(None) and no Leaving; Mount/Unmount balanced.
- `surface_lock.rs` (kit_lock style, shrinking baseline): fails when a ROWS component's open/shown/if/visible reads anything but `root.ui.*`; has-hover feeds shown; a Timer exists; a `root.*-open =` write; an Escape key-pressed outside the kit; two GlassKeys share a value. Checks ROWS against Kind and GlassKeys.

## Leaving the OS layer without a tuned number
Cause: AfterRendering fires inside render_to_canvas, BEFORE frame.present() (vendor lib.rs:877 vs wgpu_30_surface present); counting renders stands in for "presented".
Design: `Route::Leaving { until: u64 }` (until = the CEF paint generation, NativeFrame::generation, handed over by Present/PresentLatest); `Event::Presenting { drawn: Option<u64> }` fired from Slint's pre_present_callback with the generation snapshotted at BeforeRendering (small wubble change in the vendored renderer); Hide only when drawn >= until; `presentsWithTransaction = YES` on the CAMetalLayer (wgpu-hal 30 reads it, metal/surface.rs:400, then waitUntilScheduled; drawable.present(), metal/mod.rs:814-822); hide without its own explicit transaction so hide and drawable land in one Core Animation commit.
Pure tests: never Hide with drawn missing or older; generations given while leaving; Capable(false)/Failed mid-leave; the visibility invariant walk. Once on a Mac: no stale flash leaving fullscreen on a still page; implicit commit per runloop pass under winit (inf); main-thread cost of waitUntilScheduled.

## Risks
Focus return after close (step 2); Escape order incl. the keyjacking rule; a11y on faded docks (reachable-hidden); kit lock (new callbacks compose-only); idle renders (Exited/hovered must not write every frame). Verify each step: cargo test, --smoke overlay/video/popup/autofill/windows/dnd, --measure idle --seconds 4, gallery shots by eye; steps 1 and 5 also on the Mac.

## Owner questions
1 one store for the app (rec.) or per window; 2 hover in the store as discrete enter/leave (rec. yes); 3 step 1 -> OS layer -> steps 2-6 (rec.) or everything first; 4 tails by kit `Exited` callback (rec.) or a duration in core; 5 keep the name `overlay` (rec.) or rename `surface`/`ui`; 6 error panel joins the exclusive group (rec. yes), boost Exclusive.

## Step 1 as written (agent, 2026-11-23): feat/page-layer, 7 commits 2d59269..db885ca, NEVER COMPILED
- 2d59269 core overlay::ROWS + covers: Kind 15 -> 46 (one per surface with its own visibility rule; Dialog = JS dialog + sign-in, Extensions = popover + action popup, Esc hint is a Toast; split seams, 3 autofill, 3 video kinds, DevTools close button added; link hints are page DOM: cover nothing). Row { scope, z, group, keys, pointer, cover, glass (GlassKeys name), life, windows, escape, on_leave, on_nav }; Kind::row() = &ROWS[kind as usize]; Stack gains `leaving`; covers(stack, facts, pane) -> Coverage; Facts { glass, glance, pip_card }.
- 89f4a61 page_layer: Layer { shown }, Leaving { until }, Frame { os, painted }, Presenting { drawn }; hide only when drawn >= until else Redraw; LEAVING_RENDERS gone; Needs::covered(Coverage).
- 87d4e3e toast timed by the host (one slint::Timer per window, WindowHost::toast; overlay::TOAST_SHOWN_MS / sponsor::TOAST_MS); `toast-shown` replaces serials + Slint Timer.
- 601a2b8 GlassKeys duplicate (1000092) renumbered 90-93.
- b4313f7 PiP strip hovered(bool) -> Host::pip_hovered -> `pip-strip`.
- a60dc4b overlay_stack -> surface_stack over every row; Stack::with_drawn; AppWindow.mounted (a `Mounted` struct collecting each kit component's own mounted/visible, read at derivation, no copy; drawn-but-not-open => leaving); Host::page_covered (#[expect(dead_code)] until wired); extensions-open is `in`. DEVIATION: one `mounted-changed()` callback instead of per-component `exited`; not yet connected (next task wires it to Event::Changed).
- db885ca tests/surface_lock.rs + baseline 15 (was 18; needs one run to confirm): binding (downloads-open 1, spaces-open 2), hover-shown 2 (popover scroll thumbs), timer 0, open-write (downloads 3, spaces 3, settings notices-open 1), escape 3 (downloads, extensions, shot-overlay); hard tests: no duplicate GlassKeys, every glass row names a real key. kit_lock.rs shares tests/lock/mod.rs.
- 25 surfaces have a Slint-only exit tail (plan said 8). Gap: the status bubble's final fade inside SplitView's `for` is not readable (step 4).
- Compile first: Slint `changed strip-mounted/tab-mounted/speeds-mounted/mounted =>` inside `if !root.pip`/`if root.pip` writing root private properties; the `mounted` struct binding reading `xxx-ui.visible`; `changed mounted` on a struct out property; `tip-ui.shown-opacity > 0`. Rust: #[expect(dead_code, reason)]; `&ROWS[self as usize]`; `..DISMISSED` bases in a const; slint::Model::row_count(&window.get_seams()); get_exit_page().size().width; DropMarkKind::None; get_mounted() fields; #[rustfmt::skip] on let; tests glass_owners.rs and toast.rs set toast-shown.
- Possible bug (unverified): set_page_fullscreen ignores which window owns the fullscreen tab.
- Step 2 first: downloads and Spaces openness host-owned (`in` properties + actions): clears 9 of 15 baseline entries; then replace Mounted for overlay kinds with Phase::Leaving + Exited driven by mounted-changed.
- AGENTS.md rule to add when it lands: "Every shell surface is a row in `wubble_core::overlay::ROWS`; its visibility is set by the host (its owner, later the store), never by Slint alone (no `has-hover`->shown, no shell `Timer`, no Slint-written `*-open`); `tests/surface_lock.rs` holds the count down."

## Step 2, rest (written blind)
Branch `untested/store-step2` from 7bae93c, NEVER COMPILED, no test run (rustfmt parse only). Commits C3 to C7; C8 (command bar) and C9 (focus) not written.
- C3 extensions: `Panel {List, Popup}` and the install prompt are `Option` contents in `ExtUi` (app-wide, keyed by the holder from `Ui::windows_with`); `panel_owner`/`prompt_owner` gone; `extensions-open` written only by `ext_drawn_in`; the popover draws for `Panel::List` only (`panel_drawn`); its Slint Escape handler is gone.
- C4 one content map (`layers::Contents`, `(window of a window overlay, Layer) -> Content`): Mount the only insert, Unmount the only removal (skipped when the layer reopened in the same step). Machines run over `Option<content>`; `Host::content_after` opens, rewrites or closes. `overlay::acting` names the window an action acts in; `Action::Ask` is an open no `refused_under` refuses (the private close question in a menu over a page's dialog). Menu, permission, dialog+auth, find bar (`find_last` stays in `Surfaces`, now `{popup, error, find_last}`).
- C5 bookmark sheet (`Content::Bookmark`), print sheet (`Content::Print`; a printing sheet a rule closed reopens; one the overlays refuse ends as its tab closing), agent sheet (`Content::Agent(SessionId)` in its window).
- C6 region picker (ShotEffect Show/HideOverlay open and close it; a refused one cancels the shot), link hints (`Content::Hints`), the notice (closed with the toast's act, `toast_taken`). settings.slint's `notices-open` is a disclosure, not an overlay: renamed `notices-listed` (one writer), baseline line removed.
- C7 error panel and Boost editor: their machines keep the content and open/close the layer (`layer_is`); views drawn only while the store has them open; a rule closing the editor closes it as its Close does.
- Left for C8: the command bar (driver phase -> store, `command-exited`, the wubble-ui tests). Left for C9: Effect::Focus, `key-owner`/`focus-owner` as `in`, `settle-focus` and the `changed *-open` triggers, `Env::others`/`Env::machine` (only the command bar uses them now), pointer ownership, the two hover-shown bindings.

TODO.md lines (for the owner to apply):
- Store step 2 (blind, untested/store-step2): compile, cargo test, smokes overlay/windows/parity/mcp/print by hand; then C8 command bar and C9 focus.

docs/decisions.md lines:
- 2026-11-23 Store overlays keep their content in one map on the host (`layers::Contents`), written only by the Mount and Unmount executors; machines step over `Option<content>`.
- 2026-11-23 A page's question asked by the app (the private window's close question) opens with `Action::Ask`, refused by no overlay.
- 2026-11-23 Panels whose machine owns the content and that no rule closes (error panel, Boost) mirror it into the store with `layer_is`, drawn from the store's phase.

AGENTS.md: no wording change until C9 lands (then the rule in Step 1's last line).
