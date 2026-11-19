// A dependency-free Chrome DevTools Protocol client over Node's built-in WebSocket, driving a
// headless Chromium (grown from notes/fidelity/harness/cdp.mjs).
//
// Each browser gets its own profile directory and an ephemeral debugging port: Chromium is started
// with `--remote-debugging-port=0` and writes the port it chose to `<profile>/DevToolsActivePort`
// (line 1 the port, line 2 the browser target's path; Chromium's devtools_http_handler.cc, the
// contract puppeteer's launcher also reads). A stale or concurrent run can therefore never attach to
// another run's browser. The browser runs in its own process group; `close()` kills the group and
// removes the profile, and so does the process's exit.
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const sleep = ms => new Promise(r => setTimeout(r, ms));

export const CHROMIUM = process.env.FIDELITY_CHROMIUM || '/usr/bin/chromium-browser';

const COMMON = ['--headless=new', '--no-sandbox', '--hide-scrollbars', '--font-render-hinting=none', '--disable-lcd-text', '--force-color-profile=srgb'];
/**
 * WebGL back ends. `swiftshader` is the reference (CPU, deterministic). `gpu` is ANGLE on Vulkan:
 * deterministic and identical to SwiftShader on pages that draw no WebGL (the touch kit, the
 * Foundations page), wrong on the liquid (glass blown out white on NVIDIA, ~/Dev/wubble-wt/notes/
 * ui2a/h1.md), so only specimen pages ask for it.
 */
export const GL_FLAGS = {
  swiftshader: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'],
  gpu: ['--use-angle=vulkan', '--enable-features=Vulkan', '--ignore-gpu-blocklist', '--enable-gpu', '--disable-vulkan-surface'],
};

/** The port of a `DevToolsActivePort` file's text, or null. */
export function parseActivePort(text) {
  const port = Number(String(text).split('\n')[0].trim());
  return Number.isInteger(port) && port > 0 && port < 65536 ? port : null;
}

let versionCache;
/** `chromium --version`, part of the web cache's harness hash. */
export function chromiumVersion() {
  versionCache ??= execFileSync(CHROMIUM, ['--version'], { encoding: 'utf8' }).trim();
  return versionCache;
}

const live = new Set();
process.on('exit', () => { for (const b of live) b.kill(); });
for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, () => process.exit(130));

/** Starts headless Chromium with WebGL on `gl` and connects to its first page. */
export async function launch({ gl = 'swiftshader', width = 1360, height = 880 } = {}) {
  if (!GL_FLAGS[gl]) throw new Error(`unknown gl ${gl}`);
  const profile = mkdtempSync(join(tmpdir(), 'wubble-fidelity-'));
  const proc = spawn(CHROMIUM, [
    ...COMMON, ...GL_FLAGS[gl], '--remote-debugging-port=0', `--user-data-dir=${profile}`, `--window-size=${width},${height}`, 'about:blank',
  ], { stdio: 'ignore', detached: true });
  const handle = {
    kill() {
      try { process.kill(-proc.pid, 'SIGKILL'); } catch { try { proc.kill('SIGKILL'); } catch {} }
      rmSync(profile, { recursive: true, force: true });
      live.delete(handle);
    },
  };
  live.add(handle);
  let list;
  for (let i = 0; i < 150 && !list; i++) {
    await sleep(100);
    let port = null;
    try { port = parseActivePort(readFileSync(join(profile, 'DevToolsActivePort'), 'utf8')); } catch {}
    if (!port) continue;
    try { list = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); } catch {}
  }
  if (!list) { handle.kill(); throw new Error(`chromium did not start (${CHROMIUM})`); }
  const page = list.find(t => t.type === 'page');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  let id = 0;
  const pending = new Map(), listeners = new Set();
  ws.onmessage = e => {
    const m = JSON.parse(e.data);
    if (m.id && pending.has(m.id)) {
      const { resolve, reject, method } = pending.get(m.id);
      pending.delete(m.id);
      if (m.error) reject(new Error(`${method}: ${m.error.message}`)); else resolve(m.result);
    } else if (m.method) for (const l of listeners) l(m);
  };
  ws.onclose = () => { for (const { reject, method } of pending.values()) reject(new Error(`${method}: browser closed`)); pending.clear(); };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    if (ws.readyState !== WebSocket.OPEN) { reject(new Error(`${method}: browser closed`)); return; }
    const i = ++id;
    pending.set(i, { resolve, reject, method });
    ws.send(JSON.stringify({ id: i, method, params }));
  });
  const evaluate = async expression => {
    const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(`page: ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`);
    return r.result.value;
  };
  return {
    gl, send, evaluate, on: f => listeners.add(f),
    /** The WebGL renderer string (`SwiftShader Device ...`, `NVIDIA ...`), or null without WebGL. */
    renderer: () => evaluate(`(() => { const c = document.createElement('canvas'); const g = c.getContext('webgl2') || c.getContext('webgl'); if (!g) return null; const x = g.getExtension('WEBGL_debug_renderer_info'); return x ? g.getParameter(x.UNMASKED_RENDERER_WEBGL) : g.getParameter(g.RENDERER); })()`),
    close() { try { ws.close(); } catch {} handle.kill(); },
  };
}

/**
 * The `gl` a request for `gl` really gets on this machine: `gpu` falls back to `swiftshader` when
 * the GPU browser's WebGL is SwiftShader anyway (no GPU, CI) or missing. Probed once per process.
 */
const effective = new Map();
export function effectiveGl(gl) {
  if (gl !== 'gpu') return Promise.resolve(gl);
  if (!effective.has(gl)) {
    effective.set(gl, (async () => {
      const b = await launch({ gl });
      try {
        const r = await b.renderer();
        if (!r || /swiftshader/i.test(r)) { console.error(`capture: gpu WebGL is "${r}": gpu scenes use swiftshader`); return 'swiftshader'; }
        return 'gpu';
      } finally { b.close(); }
    })());
  }
  return effective.get(gl);
}
