<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/readme/logo-dark.svg"/>
  <img src="docs/readme/logo-light.svg" alt="wubble" width="300"/>
</picture>

**A Chromium-based browser where the whole window is one scene**

<a href="https://github.com/xuckless/wubble/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/xuckless/wubble/ci.yml?branch=main&style=flat-square&label=CI"/></a> <a href="#license"><img alt="License: GPL-3.0-only" src="https://img.shields.io/badge/license-GPL--3.0--only-8fb4ff?style=flat-square"/></a>

[Build](#build) · [What works](#what-works-today) · [How it works](#how-it-works) · [Roadmap](TODO.md)

</div>

wubble is an open-source web browser for Linux and macOS, written in Rust. It has vertical tabs, Spaces, split view and a built-in content blocker, and it is built on Chromium (CEF) with a [Slint](https://slint.dev) shell drawn on the GPU with [wgpu](https://wgpu.rs).

> **Early.** wubble opens pages, keeps tabs and renders well on Linux, but it is not a daily browser yet. There are no releases; you build it from source.

## Build

```sh
cargo build --workspace     # first build downloads prebuilt CEF (about 1 GB)
cargo run -p wubble -- example.com
```

Needs stable Rust, CMake and Ninja. Developed on Linux (Wayland, Vulkan). macOS is being brought up; Windows is built in CI only.

## One scene, no seams

Chromium renders each page off-screen. The shell, written in [Slint](https://slint.dev), composites those pages as GPU textures next to its own glass, so the sidebar, the page and every animation are drawn in the same frame.

<img src="docs/readme/one-scene.svg" alt="A page texture slides from Chromium's off-screen surface into the wubble window, behind the glass sidebar, and the window is drawn as one frame" width="480"/>

- **Zero copy.** Page frames travel from Chromium to the screen as GPU textures. Nothing is read back to the CPU on the frame path.
- **Native chrome.** Menus, dialogs, downloads, permissions and settings are Slint, not web pages. DevTools and Chromium's own diagnostic pages are the exceptions.
- **A sidebar, not a toolbar.** Vertical tabs, Spaces, a command bar on <kbd>Ctrl</kbd>+<kbd>T</kbd> and a sidebar that collapses to a rail or hides and peeks from the edge.

### The sidebar

It has three modes: extended, a narrow rail, and hidden. When hidden, moving to the edge of the window peeks it back over the page.

<img src="docs/readme/sidebar.svg" alt="The sidebar cycling through extended, rail and hidden. Its width springs and settles each time, and a glowing ring at the window edge marks where the hidden sidebar peeks back in" width="480"/>

## What works today

On Linux, checked by the smoke runs in [docs/smokes.md](docs/smokes.md):

- Pages render through the zero-copy path, with colours checked against known swatches.
- Tabs that sleep when hidden and restore when opened; sessions, history and settings saved per profile.
- Split view with up to three pages side by side.
- Context menu, find in page, zoom, JS dialogs, permission prompts, downloads, error pages.
- A content blocker built on Brave's [adblock-rust](https://github.com/brave/adblock-rust). It downloads no lists unless you turn that on.
- An encrypted password store keyed from the OS keyring.
- A settings page (<kbd>Ctrl</kbd>+<kbd>,</kbd>) built from one table of settings, with a shortcut editor and per-Space name, icon and colour.

Not there yet: extensions, sync, HDR output, packaged builds. The order is in [TODO.md](TODO.md).

<kbd>Ctrl</kbd>+<kbd>T</kbd> new tab · <kbd>Ctrl</kbd>+<kbd>L</kbd> address · <kbd>Ctrl</kbd>+<kbd>W</kbd> close · <kbd>Ctrl</kbd>+<kbd>F</kbd> find · <kbd>Ctrl</kbd>+<kbd>\\</kbd> split · <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Z</kbd> sidebar

### Split view

Up to three pages side by side.

<img src="docs/readme/split.svg" alt="One page becoming two and then three panes. As each seam slides in, the line between panes bows like a rubber string and settles straight" width="480"/>

### Tabs that sleep

Tabs you are not looking at go to sleep, and selecting one restores it.

<img src="docs/readme/sleep.svg" alt="A vertical tab list where hidden tabs fade to asleep with small z marks. Selecting a sleeping tab moves the highlight to it, wakes the tab with a small bounce and brings its page back" width="480"/>

## How it works

<img src="docs/readme/pipeline.svg" alt="Frames travel as dots from Chromium through a shared GPU texture and a wgpu import into Slint. Below, the model emits effects that the engine and gfx crates carry out" width="480"/>

```
Chromium (CEF, windowless) -> shared GPU texture -> wgpu import -> Slint image
        model (tabs, Spaces, sleep) -> effects -> engine and gfx carry them out
```

| Crate | Role |
|---|---|
| `wubble-core` | Tabs, Spaces, sleep policy, command bar ranking, motion. Pure logic, no CEF, wgpu or Slint |
| `wubble-engine` | The only crate that names CEF types |
| `wubble-gfx` | Page frames to `wgpu` textures; the backdrop and glass renderer |
| `wubble-ui` | The Slint shell |
| `wubble-store` | Sessions, history, bookmarks and settings in one database |
| `wubble-blocker`, `wubble-vault`, `wubble-import` | Content blocking, passwords, import from other browsers |
| `wubble` | The binary that wires them together |

Anything with a lifecycle is one `enum` and one pure `transition(state, event) -> (state, effects)`; only the engine and gfx crates carry effects out.

## Develop

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p wubble -- --smoke frames=120    # opens a window, checks the pixels, exits
```

Working rules are in [AGENTS.md](AGENTS.md), the full command list in [docs/smokes.md](docs/smokes.md). Nothing in the tests depends on how fast the machine is.

Code adapted from other projects says so in its header; the full list is in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Built on [CEF](https://bitbucket.org/chromiumembedded/cef) through [cef-rs](https://github.com/tauri-apps/cef-rs), [Slint](https://slint.dev), [wgpu](https://wgpu.rs), a fork of [welding](https://crates.io/crates/welding) in `vendor/welding`, [adblock-rust](https://github.com/brave/adblock-rust) and [Turso](https://github.com/tursodatabase/turso).

## License

[GPL-3.0-only](LICENSE).
