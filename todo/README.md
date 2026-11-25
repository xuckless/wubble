# The roadmap

wubble's only roadmap. The root [TODO.md](../TODO.md) holds what is being worked on and researched now and points here.

| Folder | What is in it |
|---|---|
| [in-progress/](in-progress/) | work that has started, one file each (branch, state, what is left), and the owner's open decisions |
| [upcoming/](upcoming/) | one file per area, numbered in work order: `01-` first |
| [shelved/](shelved/) | parked by the owner; each file starts with the owner's reason in one line |
| [done/](done/) | one file per release, one line per shipped item |

Engine rebuild split (owner, 2026-11-25): the area files in `upcoming/` hold only what builds on today's engine;
[upcoming/engine-build.md](upcoming/engine-build.md) holds everything that waits for the one engine build;
[shelved/engine-build.md](shelved/engine-build.md) holds the shelved items that would need it.

The standing risks: [risks.md](risks.md). Batch 1's writer reports: [upcoming/batch-1-reports.md](upcoming/batch-1-reports.md).

## Legend and format
- `[ ]` open · `[~]` in progress.
- An item from the old roadmap is one line, kept as written. A build-out feature (2026-11-22) is a `##` section: its
  title and key, whether it needs an engine patch, its state ("not written", or "written on `feat/<key>`, awaiting
  integration"), and the spec paragraph as written then.
- A new item goes to the file of its area.

## How an item moves
- upcoming → in-progress: when work on it starts, its line or section moves into a file in `in-progress/` (one per piece of work, naming its branch).
- in-progress → done: when it ships, one line goes into `done/<version>.md` and the item is deleted (`git log` keeps it).
- to shelved and back: only by the owner, the reason in the file's first line. Shelved hardening and review findings are worked on demand and closed when fixed.
- Settled owner decisions leave `in-progress/decisions.md` for [docs/decisions.md](../docs/decisions.md), one line each.

## How features are built (owner, standing rule)
1. Written without compile and test, one branch per feature (`feat/<key>`).
2. Then an integration pass per feature: compile, test, caveman review then adversarial review, merge. "1 feature gets
   integrated at once", grouped by feature, never by individual commit.

Batch 1's written branches are integrated this way, one feature at a time.

## Engine builds (owner, 2026-11-23)
As few as possible: one, when CPU needs it. Engine patches are written and collected under `patches/chromium/` without
building; the first CEF rebuild happens only when the under-5 % CPU patch is ready, and it carries every collected
patch. A feature that needs an engine patch lives in [upcoming/engine-build.md](upcoming/engine-build.md), not in its area file.

`in-progress/` is the working memory (owner, 2026-11-25): what is loaded for the current round, optimization and features together, moved there from `upcoming/` when chosen.
