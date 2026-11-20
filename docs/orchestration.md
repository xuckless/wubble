# Orchestrating a batch

Status (2026-11-19): how the lead runs roadmap items in parallel lanes, with the lessons of the
design-fidelity batches written as rules.

## The workflow

`.claude/workflows/lane-batch.js` runs a batch of roadmap items in parallel lanes:
(research) → implement → correctness review + simplicity review → refix, then merges
the lanes into one integration branch and runs the full check round.

- A lane is a git worktree in a sibling directory outside this checkout, on its own branch; items in a lane run in order, lanes run in parallel. Put items that touch the same files in the same lane.
- The lead creates the integration branch and the lane worktrees, launches the workflow, then reviews the integration diff before merging to `main` and pushing.
- Each stage uses the cheapest model that can do it: Sonnet medium by default, Opus medium for items marked `hard`, per-stage overrides (including Haiku) via the item's `models`.
- Research runs for every item unless it sets `research: false`, and is copy-first: it looks for code to reuse before designing anything.
- Lanes build through one shared cargo wrapper script: one build at a time (a lock), each worktree with its own target directory, so a lane never picks up another lane's crates.

## Rules (2026-11-19)

- Look inside a long-running step (its last actions, commits, processes, load) instead of relaying "still running".
- A finished lane with zero commits is a failed lane: relaunch it at once.
- Lane agents may be shown the owner's messages to the lead: every brief starts by saying those are not their task.
- Merges gate on build, lint, tests and smokes only: never on a measurement or scoring run.
- Lanes get disjoint file ownership. Shared registries (`crates/wubble-ui/ui/components/controls/registry.slint`,
  `crates/wubble-ui/src/controls.rs`, `crates/wubble-ui/ui/app.slint`, `crates/wubble/src/gallery_smoke.rs`, generated files) belong to exactly one lane at a time.
- Every long command runs under `timeout`.
- Generated files are regenerated, never hand-merged.
- The lead verifies in a separate target directory and only then rebuilds the binary the owner runs.
- After each merge look at a screenshot of the plain window (nothing may be drawn over the page) and of every surface touched.
- A kit `GlassPanel` registered with the scene inside a fading wrapper must set `present:` from the wrapper's progress.
- Slint's `color.alpha` is 0..255: mix with `transparentize`, not `with-alpha(alpha * t)`.
- Keep at most about four building lanes and one CPU-heavy job at a time on the Linux box.
- Window tests run through `scripts/headless.sh`, never on the owner's desktop; the owner's runnable binary is rebuilt only when they allow it.
- A finished agent can leave wait loops running (`until ! pgrep -f ...` matches itself): after each report, list them and kill them by PID.
- An agent's "it measured fine" is one run: the lead re-measures on the merged result (the idle-redraw loop of 2026-11-20 showed 1 frame once, 240 the next run).
- When a lane conflicts with main, send it back to the agent that wrote it to merge and re-verify; do not hand-merge its files.
