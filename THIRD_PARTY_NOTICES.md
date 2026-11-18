# Third-party notices

Code copied or ported into wubble, with its source and licence. Dependencies pulled
in through Cargo are not listed here.

| What | Where in wubble | Source | Licence |
|---|---|---|---|
| Fredoka variable font (`Fredoka[wdth,wght].ttf`, renamed `Fredoka-Variable.ttf`, unmodified) | `crates/wubble-ui/ui/fonts/` (licence: `OFL-Fredoka.txt`) | google/fonts `ofl/fredoka` @ a60a77e14f28abd4ef243a1b5dfc48df0cec5205 | SIL OFL 1.1 |
| Geist variable font (`Geist[wght].ttf`, renamed `Geist-Variable.ttf`, unmodified) | `crates/wubble-ui/ui/fonts/` (licence: `OFL-Geist.txt`) | vercel/geist-font `fonts/Geist/variable` @ v1.9.2 | SIL OFL 1.1 |
| Geist Mono variable font (`GeistMono[wght].ttf`, renamed `GeistMono-Variable.ttf`, unmodified) | `crates/wubble-ui/ui/fonts/` (licence: `OFL-Geist.txt`) | vercel/geist-font `fonts/GeistMono/variable` @ v1.9.2 | SIL OFL 1.1 |
| Key handler (Up/Down/Escape) and scroll-into-view logic, adapted in Slint | `crates/wubble-ui/ui/components/command-bar.slint` | slint-ui/slint 1.19.0 (i-slint-compiler) `widgets/common/combobox-base.slint` (popup-key-handler), `listview.slint` (bring-into-view) | GPL-3.0-only (Slint) |
