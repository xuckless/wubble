# Bundled blocker data

Shipped inside the binary (`include_str!`), so blocking works on a first run with
nothing downloaded. Refreshed by hand, never by the build.

## `lists/` (snapshot of 2026-11-18)

One file per `catalog::LISTS` row with `bundled`, named `<id>.txt`, fetched from the
row's URL by `cargo run -p wubble-blocker --example snapshot` (uBO's lists from the
uAssetsCDN copies, which have `!#include` expanded).

| Files | Source | Licence |
|---|---|---|
| `ublock-filters`, `ublock-badware`, `ublock-privacy`, `ublock-unbreak`, `ublock-quick-fixes`, `ublock-cookies`, `privacy-removeparam` | uBlockOrigin/uAssets | GPL-3.0 |
| `easylist`, `easyprivacy`, `easylist-cookies` | easylist.to (via uBO's mirror for the cookie list) | GPL-3.0-or-later or CC BY-SA 3.0; taken under GPL-3.0 |

Not bundled (download only, `blocker.lists_auto_update`): `urlhaus-1` (12 MB, list
licence not established) and `plowe-0` (no redistribution licence found).

## `resources/` (uBlock Origin 765c51daa13143b5a7cbf60be4633fb8e7d324a5, 2026-11-18)

From a gorhill/uBlock checkout (GPL-3.0):

    node --permission --allow-fs-read=<uBlock> --allow-fs-read=tools \
      tools/blocker-resources/scriptlets.mjs <uBlock> > crates/wubble-blocker/data/resources/scriptlets.json
    cargo run -p wubble-blocker --example redirects -- <uBlock> \
      > crates/wubble-blocker/data/resources/redirects.json

`scriptlets.json`: uBO's `builtinScriptlets`; `requiresTrust` ones carry `permission: 1`,
which only uBO's own lists are parsed with. `redirects.json`: uBO's
`web_accessible_resources` through adblock's resource assembler.
