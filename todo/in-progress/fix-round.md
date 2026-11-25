# Fix round (2026-11-25)

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md). From the owner's hand checks of 0.6.2 on macOS; built and tested now (owner: "build and test").

- [ ] After a drag and drop the pointer needs one extra click before it acts normally again (the stuck drag itself is fixed in 0.6.2)
- [ ] Automatic picture-in-picture from a split pane: a video playing in a pane of a three-pane split does not pop out into picture-in-picture when the owner switches to another tab (it does for a single page)
- [ ] Built-in pages (Settings, the start page and the other shell-drawn `InternalPage`s) as split panes: today refused by design (`Model::is_internal`, "never a split pane")
- [~] macOS: Cmd+Delete in a page text field does not delete to the start of the line (owner, 2026-11-25, seen in the Google search box); check the other macOS editing keys too (Option+Delete, Cmd+Left/Right, Option+Left/Right, Cmd+Up/Down, Emacs keys Ctrl+A/E/K/D/H). Written blind on sprint/perf, not compiled: CEF's windowless `SendKeyEvent` carries no AppKit editing commands, so those keys go as DevTools `Input.dispatchKeyEvent` with `commands` (`wubble_core::edit_keys`, `input::edit_command_key`)
