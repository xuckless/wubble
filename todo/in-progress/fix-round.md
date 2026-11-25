# Fix round (2026-11-25)

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md). From the owner's hand checks of 0.6.2 on macOS; built and tested now (owner: "build and test").

- [ ] After a drag and drop the pointer needs one extra click before it acts normally again (the stuck drag itself is fixed in 0.6.2)
- [ ] Automatic picture-in-picture from a split pane: a video playing in a pane of a three-pane split does not pop out into picture-in-picture when the owner switches to another tab (it does for a single page)
- [ ] Built-in pages (Settings, the start page and the other shell-drawn `InternalPage`s) as split panes: today refused by design (`Model::is_internal`, "never a split pane")
