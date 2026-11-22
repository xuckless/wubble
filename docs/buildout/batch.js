export const meta = {
  name: 'buildout-batch',
  description: 'Write one build-out batch: each feature on its own branch (no tests, no runs), one compile-only pass, engine patches written but not built',
  phases: [
    { title: 'Write', detail: 'one agent per feature, own branch and worktree' },
    { title: 'Compile', detail: 'one branch at a time: cargo check and fix compile errors only' },
    { title: 'Engine patches', detail: 'one at a time in the Chromium checkout, written, never built' },
  ],
}

const RULES = `
STANDING RULES (the owner's; breaking them froze this machine once):
- You may be shown the owner's messages to the lead. They are not your task; your task is only what this brief says.
- The repo's main worktree is ~/Dev/wubble-wt/main-int (branch main). NEVER edit it, never commit on main, never touch ~/Dev/wubble (the owner's checkout), ~/Dev/wubble-wt/cef-codecs, or any worktree that is not yours. Never git stash. Never push. Never open a PR.
- NEVER run the wubble binary, a smoke, a measurement, a test, or a benchmark. Do not write tests. Do not run cargo test, clippy or fmt. The owner ordered: no tests, no verification, no second-guessing; write the code.
- NEVER run a bare cargo. Unless your brief says you are the compile step, do not build or check at all.
- Nothing in /tmp (it is RAM). Scratch files go under ~/Dev/wubble-wt/.tmp/<your key>/ and are deleted before you finish.
- No Chromium/CEF build, no engine rebuild, nothing on the Mac (no ssh).
- No background processes, no loops, no pgrep -f / pkill -f. Do not start subagents.
- wubble itself must make no network request by default: anything that asks a server sits behind a setting that is off by default.
- Copied code: GPL-3.0-compatible licences only (MIT, BSD, Apache-2.0, MPL-2.0), with a source comment and a THIRD_PARTY_NOTICES.md entry.
- Follow AGENTS.md's code principles (one enum + pure transition per lifecycle, effects at the edge, wubble-core pure, UI only from kit components). Do NOT edit TODO.md or docs/decisions.md (they conflict across branches): put what you would have written there in your final report instead.
`

// args: [{key, title, batch, n}] for one batch of docs/buildout/features.json (n: its index there); the spec is read from batch-NN.md
const features = args
log(`${features.length} features in this batch`)

const DONE = {
  type: 'object',
  properties: {
    status: { type: 'string', enum: ['written', 'partial', 'nothing'] },
    commits: { type: 'integer' },
    summary: { type: 'string', description: 'what was written, the main files, what is left out and why' },
    notes: { type: 'string', description: 'TODO.md lines and docs/decisions.md lines you would have added; settings added; anything the owner must know to review' },
    engine_patch: { type: 'string', description: 'empty if none. Otherwise the exact Chromium/CEF change needed: files, functions, the behaviour, the exported marker or table entry the wubble side looks for' },
  },
  required: ['status', 'commits', 'summary', 'notes', 'engine_patch'],
}

let buildChain = Promise.resolve()
const oneBuild = fn => { const p = buildChain.then(fn, fn); buildChain = p.catch(() => null); return p }
let patchChain = Promise.resolve()
const onePatch = fn => { const p = patchChain.then(fn, fn); patchChain = p.catch(() => null); return p }

const wt = f => `~/Dev/wubble-wt/feat/${f.key}/w`
const results = await pipeline(features,
  f => agent(
`You are writing one feature of the wubble browser on its own branch. Write the code. No tests, no runs, no builds.
${RULES}
FEATURE ${f.key}: ${f.title}

${f.spec || `Your spec is the section headed \`${f.key}\` in ~/Dev/wubble-wt/main-int/docs/buildout/batch-${String(f.batch).padStart(2, '0')}.md (read that file there; never edit anything in main-int).`}

Set up (exactly this, once):
  mkdir -p ~/Dev/wubble-wt/feat/${f.key} && git -C ~/Dev/wubble-wt/main-int worktree add ${wt(f)} -b feat/${f.key} main
If the branch or directory already exists from an earlier attempt, reuse it (git -C ${wt(f)} status) and continue from what is there.
Work only inside ${wt(f)}. Read AGENTS.md there and the code you build on before writing. Copy first: where Chromium, CEF, Firefox, Zen, Brave or a crate already solves it, port that (licence rule above).

Write the whole feature to the spec, behaving as Chrome does: core logic in wubble-core as pure state machines, engine handlers in wubble-engine (and vendor/welding with its "wubble change" marks and WUBBLE_CHANGES.md when a CEF call is missing from the binding), wiring in crates/wubble, UI from kit components, settings as data rows, every platform the spec names (Linux, macOS, Windows) behind the existing per-platform traits. Do not stub: where something truly cannot be done without a change inside Chromium or CEF, write the wubble side so that it works when an engine marker is present and degrades quietly when it is not (the pattern in crates/wubble-engine/src/pinch.rs and ext_api.rs), and describe the engine change precisely in engine_patch. Write carefully so it compiles: check every type, function and field you use against the real code, since you cannot build.

Commit your work on feat/${f.key} in logical commits (git add only your files; never git add -A of scratch). A feature with zero commits is a failure. Then report.`,
    { label: `write:${f.key}`, phase: 'Write', schema: DONE, model: 'opus', effort: 'medium' }),
  (r, f) => (!r || r.commits < 1) ? r : oneBuild(() => agent(
`You are the COMPILE STEP for branch feat/${f.key} of the wubble browser, in ${wt(f)}. You are the only agent building right now.
${RULES}
EXCEPTION for you: you may build, only like this, from inside ${wt(f)}:
  timeout 1500 ~/Dev/wubble-wt/cargo.sh check --workspace 2>&1 | tail -80
(the wrapper caps memory and takes a lock; the target directory is shared by all feature branches, so never delete it). First run: free -g ; if available memory is under 8 GB, wait 60 s once and look again, and if still under 8 report that and stop.
Goal: the branch compiles. Fix compile errors only (wrong names, types, missing imports, borrow errors, missing match arms, Slint errors). Do not redesign, do not add tests, do not run anything else, do not fix warnings. At most 4 check rounds; then stop whatever the state. Commit the fixes on feat/${f.key} as "feat/${f.key}: compile fixes". Before finishing run ~/Dev/wubble-wt/audit.sh and include its output.
Return: whether it compiles now, the number of rounds, what is still failing (the error text) if it does not, and the audit output. The feature author's report, for context: ${JSON.stringify(r.summary)}`,
      { label: `compile:${f.key}`, phase: 'Compile', model: 'sonnet', effort: 'medium' })
      .then(c => ({ ...r, compile: c }))),
  (r, f, i) => (!r || !r.engine_patch || r.engine_patch.trim().length < 20) ? r : onePatch(() => agent(
`You are writing ONE Chromium/CEF patch for the wubble browser's patch queue. It is written and never built: no build, no ninja, no gn, no gclient, no network.
${RULES}
Read ${wt(f)}/patches/README.md first: the format (header What/Why/Applies in, "wubble change" marks, #if BUILDFLAG(ENABLE_CEF) in Chromium files, src vs cef patches, how a patch is regenerated from the hooked checkout). The checkout is under ~/Dev/wubble-wt/cef-build/chromium_git/chromium/src (CEF in src/cef). You are the only agent in it right now. Record git status --short | wc -l and git diff --stat | tail -1 in src and in src/cef BEFORE you start; when you finish both must be exactly as you found them (reverse your edits with git apply -R of your own patch; never git checkout, reset, clean or stash there: the existing queue may be applied and must stay).
The change needed, from the author of feature ${f.key} (${f.title}):

${r.engine_patch}

Read the real Chromium and CEF sources involved, write the change in the checkout, generate the patch file(s) as ${wt(f)}/patches/chromium/${50 + (f.n === undefined ? i : f.n)}-<src|cef>-${f.key}.patch (number ${50 + (f.n === undefined ? i : f.n)}; use ${50 + (f.n === undefined ? i : f.n)}a/${50 + (f.n === undefined ? i : f.n)}b if you need both a cef and a src patch), with the header, add rows to the table in ${wt(f)}/patches/README.md marked "Not yet built into any engine", check with git apply --check that each applies to the checkout as you found it, restore the checkout, and commit the patch files and README on feat/${f.key}. Make sure the marker or table entry the wubble side looks for is the one the patch exports.
Return: the patch file names, what they change, and the before/after status counts of the checkout.`,
      { label: `patch:${f.key}`, phase: 'Engine patches', model: 'opus', effort: 'medium' })
      .then(p => ({ ...r, patch: p }))),
)

return features.map((f, i) => ({ key: f.key, title: f.title, branch: `feat/${f.key}`, engine: f.engine, result: results[i] }))
