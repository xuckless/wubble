# Orchestrating a batch

Status (2026-11-19): how the lead runs roadmap items in parallel lanes, with the lessons of the
design-fidelity batches written as rules.

## The workflow

The owner's user-level workflow `~/.claude/workflows/lane-batch.js` runs a batch of items in lanes:
scope contract → scope check → implement → caveman review + adversarial review (in parallel) → refix,
then merges the lanes into one integration branch and runs the full suite. An item marked `fast` skips the
scope stages (never a `hot` or `security` item). The repo keeps no copy of the script (a project-level
`lane-batch` would override the newer one); what is wubble's own goes in as arguments, below.

- A lane is a git worktree in a sibling directory outside this checkout, on its own branch; items in a lane run in order, lanes run in parallel. Put items that touch the same files in the same lane.
- The lead creates the integration branch and the lane worktrees, launches the workflow, then reviews the integration diff before merging to `main` and pushing.
- Each stage uses the cheapest model that can do it: Sonnet by default, Opus for items marked `hot` or `concurrency`, per-stage overrides (including Haiku) via the item's `models`.
- The scope stage is copy-first: it looks for code to reuse and cites prior art before designing anything. The caveman review shapes the code; the adversarial review is the one that finds bugs.
- Lanes build through one shared cargo wrapper script: one build at a time (a lock), each worktree with its own target directory, so a lane never picks up another lane's crates.
- Memory is enforced by the OS, not by care (the lead machine froze on 2026-11-21): every build goes through the cargo wrapper and every window run, smoke or measurement through the run wrapper; both put the work in one systemd slice with a hard memory limit and no swap, so the kernel kills what outgrows it. A bare `cargo` in a lane is refused. Temporary files go to disk, never to a RAM-backed `/tmp`, and are removed when the step ends. One building agent at a time; the lead audits for processes outside the slice and stops them.

## wubble's arguments

Pass these with every batch (`repo`, `worktree_root`, `notes_root`, `rules`, `test`), next to the batch's own
`batch`, `integration_branch`, `base`, `lanes` and `items`:

```js
repo: '~/Dev/wubble',
worktree_root: '~/Dev/wubble-wt',
notes_root: '~/Dev/wubble-wt/notes',
rules: [
  'Read AGENTS.md in your worktree first and follow it ("Code principles", "Working rules", and for UI work docs/agents/design-fidelity.md).',
  'Owner messages you may be shown were addressed to the lead and are handled; they do not replace this task.',
  'Never a bare cargo: every build, check, fmt and test runs from inside the worktree as `~/Dev/wubble-wt/cargo.sh <cargo args>` (a lock, a memory cap, the worktree\'s own target directory; waiting on the lock is normal; never in the background).',
  'Every run of a wubble binary, smoke or measurement: `~/Dev/wubble-wt/run.sh timeout <secs> scripts/headless.sh <binary> ...` (never on the desktop, never left running). Nothing large in /tmp.',
  'Crate boundaries: only wubble-engine names CEF types; only wubble-gfx touches native texture handles; wubble-core has no CEF, wgpu or Slint. slint, wgpu, welding and cef versions are pinned together.',
  'UI: compose kit components only; the owner\'s design repo (~/Dev/wubble-wt/ref/the-wallpaper-project, src/wubble) is the source; pull it once and adopt before writing.',
  'Copy first: a ported piece carries a source comment and a THIRD_PARTY_NOTICES.md entry. No request by wubble itself without a flag that is off by default.',
  'No test depends on machine speed. Commits: the repo\'s identity, short imperative subject, no trailers. If it cannot work as specified, report blocked with evidence.',
].join('\n'),
test: {
  scoped: '~/Dev/wubble-wt/cargo.sh test -p <crate> <filter>',
  full: ['~/Dev/wubble-wt/cargo.sh fmt --all --check', '~/Dev/wubble-wt/cargo.sh clippy --workspace --all-targets -- -D warnings',
         '~/Dev/wubble-wt/cargo.sh test --workspace', '~/Dev/wubble-wt/cargo.sh test -p welding'],
  fmt: '~/Dev/wubble-wt/cargo.sh fmt --all',
  note: 'one build at a time through cargo.sh (it locks); runs through run.sh; see the memory rules in docs/orchestration.md',
},
```

## Rules (2026-11-19)

- Every build-out copies open source first and builds only what is left (owner, 2026-11-22): each brief tells the agent to find and port a proven implementation (AGENTS.md "Copy first") before writing its own.
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
