// A dependency-free Chrome DevTools Protocol client over Node's built-in WebSocket, driving a
// headless Chromium (grown from notes/fidelity/harness/cdp.mjs).
import { spawn } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const sleep = ms => new Promise(r => setTimeout(r, ms));

export const CHROMIUM = process.env.FIDELITY_CHROMIUM || '/usr/bin/chromium-browser';

/** Starts headless Chromium (WebGL on SwiftShader) and connects to its first page. */
export async function launch({ port = 9333, width = 1360, height = 880 } = {}) {
  const profile = mkdtempSync(join(tmpdir(), 'wubble-fidelity-'));
  const proc = spawn(CHROMIUM, [
    '--headless=new', '--no-sandbox', '--use-angle=swiftshader', '--enable-unsafe-swiftshader',
    '--hide-scrollbars', '--font-render-hinting=none', '--disable-lcd-text', '--force-color-profile=srgb',
    `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, `--window-size=${width},${height}`, 'about:blank',
  ], { stdio: 'ignore' });
  let list;
  for (let i = 0; i < 100 && !list; i++) {
    try { list = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); } catch { await sleep(100); }
  }
  if (!list) { proc.kill(); throw new Error(`chromium did not start (${CHROMIUM})`); }
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
  const send = (method, params = {}) => new Promise((resolve, reject) => {
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
    send, evaluate, on: f => listeners.add(f),
    close() { ws.close(); proc.kill(); },
  };
}
