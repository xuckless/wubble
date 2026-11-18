export const meta = {
  name: 'lane-batch',
  description: 'wubble batch: (research) -> implement -> correctness + simplicity review -> refix, per item in lane worktrees, then merge lanes into one integration branch and run the full check round',
  whenToUse: 'A batch of independent roadmap items. Pass args {batch, integration_branch, base, lanes, items}; see AGENTS.md "Orchestrating a batch".',
  phases: [
    { title: 'Research', detail: 'copy first: find code to reuse from other browsers/projects, verify APIs, write a plan' },
    { title: 'Implement', detail: 'per lane worktree, items in sequence' },
    { title: 'Review', detail: 'two lenses per item: correctness/bugs and simplicity' },
    { title: 'Refix', detail: 'apply confirmed findings, re-test, commit' },
    { title: 'Integrate', detail: 'merge lanes into the integration branch, full check round' },
  ],
}

// args: {
//   batch: 'm0-a',                          // name; research notes go to ~/Dev/wubble-wt/notes/<batch>/
//   plan_ref: 'TODO.md section "M0"',       // optional pointer the agents read
//   integration_branch: 'm0/int',           // must exist (created by the lead from base)
//   base: 'main',
//   lanes: [{ key: 'engine', wt: '~/Dev/wubble-wt/engine', branch: 'm0/engine', items: ['e1', 'e2'] }],
//   integration_extra: 'optional extra instructions for the integration agent',
//   integration_model: { model, effort },   // optional
//   items: { e1: { title, spec, hard: true, research: true, models: {...} } },
// }
// The lead creates the integration branch and one worktree per lane
// (git worktree add -b <branch> <wt> <base>) before launching.
//
// Models: minimum viable per stage. Defaults: Sonnet medium everywhere; an item marked
// `hard` gets Opus medium for implement, refix and the correctness review. Any stage can be
// overridden per item: models: { research, implement, refix, review_correctness, review_simplicity },
// each { model, effort } (e.g. { model: 'haiku', effort: 'low' } for mechanical items).
// Research runs for every item unless it sets research: false. Research is COPY FIRST: find
// existing code in other browsers/projects to copy or port before designing anything new.
// A lane stops at the first item that ends 'blocked': later items in a lane build on earlier ones.

const A = args || {}
if (!A.lanes || !A.items || !A.integration_branch) throw new Error('lane-batch needs args {batch, integration_branch, base, lanes, items}')
const NOTES = `~/Dev/wubble-wt/notes/${A.batch}`
const BASE = A.base || 'main'
const TARGET = '~/Dev/wubble/target'

const COMMON = `
You are working on wubble (a Chromium-based browser with a Slint shell, Rust workspace). Read AGENTS.md in your worktree first and follow it, especially "Code principles" and "Working rules". Roadmap and decisions: TODO.md${A.plan_ref ? ` (${A.plan_ref})` : ''}. Your branch starts from ${BASE}.

Hard rules:
- Work ONLY in your lane worktree (given below). Never edit ~/Dev/wubble. Research notes go to ${NOTES}/ (outside the repo; create it).
- Every cargo command runs from inside the worktree with \`CARGO_TARGET_DIR=${TARGET}\` set. That shared directory makes cargo serialise builds across lanes and avoids re-downloading CEF; "Blocking waiting for file lock" is expected, wait for it. Never run cargo in the background.
- Checks per change: \`cargo fmt --all\`, \`cargo clippy --workspace --all-targets -- -D warnings\`, \`cargo test --workspace\`. New logic in wubble-core needs unit tests. No sleeps, retries or timing assertions in tests.
- Running the app opens a window on the owner's desktop: only run it through a smoke mode that exits by itself, and never leave a process running.
- No network requests by wubble itself unless the spec says so. Dependencies: add only what the spec needs; slint / wgpu / welding / cef versions are pinned together, do not bump them.
- Crate boundaries: only wubble-engine names CEF types; only wubble-gfx touches native texture handles; wubble-core has no CEF, wgpu or Slint.
- Commits: plain \`git commit\` with the repo's configured identity, short imperative subject, NO trailers (no Co-Authored-By, no links). Never git stash, never push, never touch other branches.
- Copy first: where the research note names code to copy or port, do that rather than writing your own. Every copied or ported piece gets a source comment (project, path, licence) and an entry in THIRD_PARTY_NOTICES.md at the repo root.
- No test may depend on machine speed: no timing thresholds, fps assertions or benchmarks in `cargo test` or CI. Performance is measured by hand on the owner's Linux box or the macOS box, never asserted.
- If something cannot work as specified, stop and report status 'blocked' with evidence (error text, API docs, source lines). Do not fake a result or silently substitute a different approach.
`

const RESEARCH_SCHEMA = {
  type: 'object',
  properties: {
    note_path: { type: 'string' },
    plan: { type: 'string', description: 'concrete implementation plan: files, types, functions, in steps' },
    simplicity: { type: 'string', description: 'is the planned change overengineered? simpler option?' },
    risks: { type: 'array', items: { type: 'string' } },
    tests: { type: 'array', items: { type: 'string' } },
    scope_change: { type: 'string', description: 'anything in the spec that turned out wrong or impossible, else empty' },
  },
  required: ['note_path', 'plan', 'simplicity', 'risks', 'tests', 'scope_change'],
}
const IMPL_SCHEMA = {
  type: 'object',
  properties: {
    commits: { type: 'array', items: { type: 'string' }, description: 'sha + subject' },
    summary: { type: 'string' },
    tests: { type: 'string', description: 'which checks ran and their results (pass/fail counts, smoke output)' },
    deviations: { type: 'string' },
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
  },
  required: ['commits', 'summary', 'tests', 'deviations', 'status'],
}
const REVIEW_SCHEMA = {
  type: 'object',
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          severity: { type: 'string', enum: ['blocker', 'major', 'minor', 'nit'] },
          file: { type: 'string' },
          line: { type: 'integer' },
          summary: { type: 'string' },
          failure_scenario: { type: 'string' },
          suggested_fix: { type: 'string' },
        },
        required: ['severity', 'file', 'summary', 'failure_scenario', 'suggested_fix'],
      },
    },
    verdict: { type: 'string', enum: ['ship', 'fix-then-ship', 'rework'] },
  },
  required: ['findings', 'verdict'],
}
const REFIX_SCHEMA = {
  type: 'object',
  properties: {
    fixed: { type: 'array', items: { type: 'string' } },
    rejected: { type: 'array', items: { type: 'object', properties: { finding: { type: 'string' }, reason: { type: 'string' } }, required: ['finding', 'reason'] } },
    commits: { type: 'array', items: { type: 'string' } },
    tests: { type: 'string' },
  },
  required: ['fixed', 'rejected', 'commits', 'tests'],
}

const LENSES = [
  { key: 'correctness', stage: 'review_correctness', text: 'CORRECTNESS / BUG lens: try hard to break it. Logic errors and off-by-ones; state transitions that are missing, unreachable or wrong; unsafe and FFI misuse (CEF reference counts, raw handles, leaked file descriptors, panics crossing an FFI boundary); thread affinity (CEF UI thread vs the Slint event loop) and lifetime of anything shared between them; resize, zero-size, HiDPI and shutdown-order edge cases; error paths that are swallowed; numeric edge cases (NaN, huge dt, overflow); behaviour that contradicts the spec or a decision in TODO.md; tests that pass for the wrong reason or assert nothing; tests that depend on machine speed or timing; code that cannot compile on macOS or Windows because it is not cfg-gated.' },
  { key: 'simplicity', stage: 'review_simplicity', text: 'SIMPLICITY lens: is this the least code that meets the spec? Overengineering, premature abstraction, speculative options nobody asked for, dead code, needless dependencies, a simpler equivalent. Check AGENTS.md "Code principles" one by one: imperative UI where a Slint binding would do (1); an implicit state machine, i.e. behaviour depending on several booleans/Options or on callback order instead of one enum plus a pure transition function (2); effects executed inside core logic instead of returned as values (3); the same state owned in two places (4); duplicated code or duplicated knowledge such as a constant, token or table repeated (5). Also: hand-written code where the research note named something to copy or an existing crate does it; copied code missing attribution or a THIRD_PARTY_NOTICES.md entry; crate-boundary violations; comments that restate the code.' },
]

const tier = (it) => ({ model: it.hard ? 'opus' : 'sonnet', effort: 'medium' })
const pick = (it, stage, dflt) => {
  const m = (it.models || {})[stage] || {}
  return { model: m.model || dflt.model, effort: m.effort || dflt.effort }
}
const STAGE = (it) => ({
  research: pick(it, 'research', { model: 'sonnet', effort: 'medium' }),
  implement: pick(it, 'implement', tier(it)),
  refix: pick(it, 'refix', tier(it)),
  review_correctness: pick(it, 'review_correctness', tier(it)),
  review_simplicity: pick(it, 'review_simplicity', { model: 'sonnet', effort: 'medium' }),
})

const SPEC_AS_NOTE = { note_path: '(none: the spec is the plan)', plan: 'Follow the spec.', simplicity: '', risks: [], tests: [], scope_change: '' }

async function research(lane, id) {
  const it = A.items[id]
  if (it.research === false) return SPEC_AS_NOTE
  return agent(`${COMMON}\nLane worktree (read-only for you; do not edit code): ${lane.wt}\n\nRESEARCH: ${it.title}\nSpec: ${it.spec}\n\nCOPY FIRST. Before designing anything, find existing code that already does this and can be copied or ported: Chromium and CEF (cefclient/cefsimple, BSD), Firefox and Zen (MPL-2.0), Brave (MPL-2.0), Servo, the cef-rs and welding/grafting examples, Slint examples, Zed, and any well-used crate. Licence must be compatible with GPL-3.0-only (MIT, BSD, Apache-2.0, MPL-2.0, GPL-3.0 are fine; no proprietary or source-available code, and nothing from Arc). Prefer, in order: use a crate as a dependency; copy/port a proven implementation with attribution; write new code only for what is left. For each thing to copy give the exact source (repo URL, path, function, commit or version) and its licence. Then verify every assumption in the spec against real sources: the crate sources under ~/.cargo/registry/src (exact types, functions, feature flags), docs.rs, upstream repos and examples (WebSearch/WebFetch allowed). Quote the signatures you will rely on. Decide the simplest correct design, the tests, and the risks. Write the note to ${NOTES}/${id}.md (sections: Copy sources (what, where, licence), Verified facts (with sources), Design, Simplicity check, Risks, Tests, Out of scope). If the spec is impossible as written, say so in scope_change with evidence.`,
    { label: `research:${id}`, phase: 'Research', schema: RESEARCH_SCHEMA, ...STAGE(it).research })
}

async function runItem(lane, id) {
  const it = A.items[id]
  const where = `Lane worktree: ${lane.wt} (branch ${lane.branch}; earlier items of this lane may already be committed there).`
  const note = await research(lane, id)
  if (!note) return { id, title: it.title, status: 'failed', error: 'research failed' }
  const start = `Record \`git -C ${lane.wt} rev-parse HEAD\` before you start.`
  const impl = await agent(`${COMMON}\n${where}\n\nIMPLEMENT: ${it.title}\nSpec: ${it.spec}\n\nResearch note (read it fully first if there is one): ${note.note_path}\nPlan:\n${note.plan}\nSimplicity notes: ${note.simplicity}\nScope changes: ${note.scope_change}\nPlanned tests: ${note.tests.join('; ')}\n\nImplement with tests, run the checks, and commit (one or a few focused commits). If a check fails, diagnose and fix; if something pre-existing and unrelated fails, record it in deviations with evidence rather than fixing it. Tick the finished items in TODO.md. Return the structured result.`,
    { label: `impl:${id}`, phase: 'Implement', schema: IMPL_SCHEMA, ...STAGE(it).implement })
  if (!impl) return { id, title: it.title, status: 'failed', error: 'implement failed' }
  if (!impl.commits.length) return { id, title: it.title, status: impl.status, impl, findings: [], refix: null }

  const reviews = await parallel(LENSES.map(l => () => agent(`${COMMON}\n${where}\n\nREVIEW of: ${it.title}\nSpec: ${it.spec}\nImplementer's summary (do not trust it, verify): ${impl.summary}\nCommits: ${impl.commits.join('; ')}\nInspect this item's commits with \`git -C ${lane.wt} show <sha>\` and read the surrounding code. You may run the checks to confirm a suspicion (a throwaway test is fine; revert it with git checkout, commit nothing).\n\n${l.text}\n\nReport only real, concrete findings, each with a failure scenario (for simplicity findings: what the extra code costs and the simpler form). No style nits unless they break AGENTS.md. Empty findings is a valid answer.`,
    { label: `review:${id}:${l.key}`, phase: 'Review', schema: REVIEW_SCHEMA, ...STAGE(it)[l.stage] })))
  const findings = reviews.flatMap((r, i) => r ? r.findings.map(f => ({ ...f, lens: LENSES[i].key })) : [])
  const actionable = findings.filter(f => f.severity !== 'nit')
  let refix = null
  if (actionable.length) {
    refix = await agent(`${COMMON}\n${where}\n\nREFIX: ${it.title}\nSpec: ${it.spec}\nReview findings (JSON):\n${JSON.stringify(findings, null, 1)}\n\nFor each finding: verify it against the code; fix the real ones (nits only if trivial and clearly right); reject wrong ones with a concrete reason. Add a test for every real correctness bug. Simplicity fixes must delete or merge code, not add layers. Re-run the checks and commit.`,
      { label: `refix:${id}`, phase: 'Refix', schema: REFIX_SCHEMA, ...STAGE(it).refix })
  }
  log(`${id}: ${impl.status}, ${actionable.length} actionable findings, refix ${refix ? 'done' : 'skipped'}`)
  return { id, title: it.title, status: impl.status, note: note.note_path, impl, verdicts: reviews.filter(Boolean).map(r => r.verdict), findings, refix }
}

const laneResults = await pipeline(A.lanes, async (lane) => {
  const out = []
  for (const id of lane.items) {
    const r = await runItem(lane, id)
    out.push(r)
    if (r.status === 'blocked' || r.status === 'failed') {
      const skipped = lane.items.slice(lane.items.indexOf(id) + 1)
      if (skipped.length) log(`lane ${lane.key}: ${id} ${r.status}; skipping ${skipped.join(', ')}`)
      skipped.forEach(s => out.push({ id: s, title: A.items[s].title, status: 'skipped' }))
      break
    }
  }
  return { lane: lane.key, branch: lane.branch, items: out }
})

const laneSummary = laneResults.filter(Boolean).map(l => ({
  lane: l.lane,
  branch: l.branch,
  items: l.items.map(i => ({ id: i.id, status: i.status, commits: i.impl ? i.impl.commits : [], refix: i.refix ? i.refix.commits : [] })),
}))
const intWt = `~/Dev/wubble-wt/${A.batch}-int`
const merge = laneSummary.filter(l => l.items.some(i => i.commits.length)).map(l => l.branch)

phase('Integrate')
const integ = merge.length ? await agent(`${COMMON}\n\nINTEGRATE ${A.batch}. Branch ${A.integration_branch} exists in ~/Dev/wubble (at ${BASE}). Create a worktree: \`git -C ~/Dev/wubble worktree add ${intWt} ${A.integration_branch}\` and work there. Merge, in this order, the local branches ${merge.join(', ')} (\`git merge --no-ff\`, no trailers). Resolve conflicts preserving both sides' intent; where two lanes added the same thing, keep one copy.${A.integration_extra ? ' ' + A.integration_extra : ''} Then from ${intWt} run \`cargo fmt --all --check\`, \`cargo clippy --workspace --all-targets -- -D warnings\`, \`cargo test --workspace\`, and any smoke command the lane results mention. Fix integration-only breakage with a commit; if a lane's own change fails, report it with the output instead of rewriting it. Do not push.\n\nLane results for context:\n${JSON.stringify(laneSummary, null, 1)}\n\nReturn: merge commits, result per check (pass/fail counts), anything that failed with evidence.`,
  { label: 'integrate', phase: 'Integrate', model: (A.integration_model || {}).model || 'sonnet', effort: (A.integration_model || {}).effort || 'medium' }) : 'nothing to integrate'

return { lanes: laneResults, integration: integ }
