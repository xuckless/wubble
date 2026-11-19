// Where the design clone is, and the commit it is at. Shared by every tool in tools/fidelity.
// The clone is never edited by these tools.
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The wubble repository root (two levels above this file's folder). */
export const REPO = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');

/** The design repository clone: `$WUBBLE_DESIGN`, else the lanes' shared clone. */
export function designRoot() {
  const dir = process.env.WUBBLE_DESIGN || resolve(homedir(), 'Dev/wubble-wt/ref/the-wallpaper-project');
  if (!existsSync(resolve(dir, 'src/wubble'))) {
    throw new Error(`no design clone at ${dir} (set WUBBLE_DESIGN to the-wallpaper-project's clone)`);
  }
  return dir;
}

/** `src/wubble` inside the clone: every `design:` citation is relative to it. */
export const designSrc = (root = designRoot()) => resolve(root, 'src/wubble');

/** The clone's commit (short), and whether `src/wubble` has local changes. */
export function designCommit(root = designRoot()) {
  const git = (...a) => execFileSync('git', ['-C', root, ...a], { encoding: 'utf8' }).trim();
  const commit = git('rev-parse', '--short=7', 'HEAD');
  const dirty = git('status', '--porcelain', '--', 'src/wubble') !== '';
  return { commit, dirty };
}
