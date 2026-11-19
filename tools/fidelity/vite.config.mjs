// The harness's own vite server over the design clone (never edited): the clone's config, plus
//  - a deterministic clock and a seeded Math.random, injected into every page at serve time
//    (`window.__wb.step(ms)` advances rAF, timers, performance.now and Date.now by `ms`; CSS
//    animations are stepped by capture.mjs through the Web Animations API);
//  - a frozen-backdrop switch: `globalThis.__wbFrozen = true` stops the liquid's emitters and
//    its fluid step (core/gl/scene.ts, patched in the transform hook);
//  - the parity pages wubble's gallery loads (`/__parity/manifest.json`, `/__parity/tab/<id>`):
//    the design's Spaces and tabs with its titles and favicons.
// Run by run.sh as `npx vite --config <this> --port 5199` from the clone's directory.
import { resolve } from 'node:path';
import { readFileSync } from 'node:fs';

const root = process.env.WUBBLE_DESIGN_ROOT || process.cwd();
const src = resolve(root, 'src');
const PORT = Number(process.env.FIDELITY_PORT || 5199);

/** The clock and the seeded random, before any page script. */
const CLOCK = `<script>(() => {
  let now = 0;
  const epoch = 1763510400000; // 2025-11-19, a fixed day
  let seed = 0x5eed;
  Math.random = () => { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return (seed >>> 0) / 4294967296; };
  performance.now = () => now;
  const RealDate = Date;
  class FakeDate extends RealDate { constructor(...a) { super(...(a.length ? a : [epoch + now])); } static now() { return epoch + now; } }
  window.Date = FakeDate;
  let id = 0;
  const rafs = new Map();
  const timers = new Map();
  window.requestAnimationFrame = cb => { rafs.set(++id, cb); return id; };
  window.cancelAnimationFrame = i => rafs.delete(i);
  const timer = repeat => (cb, ms = 0, ...a) => { timers.set(++id, { at: now + Math.max(0, ms), ms: Math.max(1, ms), repeat, cb: () => (typeof cb === 'function' ? cb(...a) : 0) }); return id; };
  window.setTimeout = timer(false);
  window.setInterval = timer(true);
  window.clearTimeout = window.clearInterval = i => timers.delete(i);
  const runTimers = () => {
    for (;;) {
      const due = [...timers.entries()].filter(([, t]) => t.at <= now).sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) return;
      const [i, t] = due;
      if (t.repeat) t.at += t.ms; else timers.delete(i);
      t.cb();
    }
  };
  window.__wb = {
    now: () => now,
    /** advance the clock by ms: due timers fire, then one animation frame */
    step(ms) {
      now += ms;
      runTimers();
      const cbs = [...rafs.values()];
      rafs.clear();
      for (const cb of cbs) cb(now);
    },
  };
})();</script>`;

function fidelity() {
  return {
    name: 'wubble-fidelity',
    transformIndexHtml(html) {
      return html.replace('<head>', `<head>${CLOCK}`);
    },
    transform(code, id) {
      if (!id.endsWith('/src/wubble/core/gl/scene.ts')) return null;
      const patches = [
        ['this.fluid.step(dt);', 'this.fluid.step(globalThis.__wbFrozen ? 0 : dt);'],
        ['if (this.opts.ambient > 0) {', 'if (this.opts.ambient > 0 && !globalThis.__wbFrozen) {'],
      ];
      for (const [from, to] of patches) {
        if (!code.includes(from)) throw new Error(`fidelity: core/gl/scene.ts no longer has \`${from}\`: update tools/fidelity/vite.config.mjs`);
        code = code.replace(from, to);
      }
      return code;
    },
    configureServer(server) {
      server.middlewares.use('/__parity', async (req, res) => {
        try {
          const sites = await server.ssrLoadModule('/wubble/sites/sites.ts');
          const model = await server.ssrLoadModule('/wubble/browser/model.ts');
          const tokens = await server.ssrLoadModule('/wubble/core/tokens.ts');
          const icons = readFileSync(resolve(src, 'wubble/browser/icons.tsx'), 'utf8');
          const base = `http://127.0.0.1:${PORT}/__parity/tab`;
          const url = req.url || '/';
          if (url.startsWith('/manifest.json')) {
            // browser/model.ts initial(): the Spaces' tabs; chrome.tsx FAVS: the favourites
            const tabs = model.createStore().get().spaces.map(s => s.tabs.map(t => t.history[t.at]));
            const chrome = readFileSync(resolve(src, 'wubble/browser/chrome.tsx'), 'utf8');
            const favs = JSON.parse(/const FAVS = (\[[^\]]*\])/.exec(chrome)[1].replace(/'/g, '"'));
            const id = loc => (loc.kind === 'site' ? `site-${loc.site}` : `page-${loc.page}`);
            const manifest = {
              spaces: tokens.spaces.map((s, i) => ({
                name: s.name, tint: s.accent, icon: s.id,
                pinned: i === 0 ? favs.map(f => `${base}/site-${f}`) : [],
                tabs: tabs[i].map(l => `${base}/${id(l)}`),
              })),
            };
            res.setHeader('content-type', 'application/json');
            return res.end(JSON.stringify(manifest, null, 2));
          }
          const m = /^\/tab\/(site|page)-([\w-]+)(\.svg)?/.exec(url);
          if (!m) { res.statusCode = 404; return res.end(); }
          const [, kind, name, svg] = m;
          const P = Object.fromEntries([...icons.matchAll(/^\s+(\w+): '([^']+)',$/gm)].map(x => [x[1], x[2]]));
          const site = kind === 'site' ? sites.siteById(name) : null;
          if (svg) {
            res.setHeader('content-type', 'image/svg+xml');
            if (site?.icon.path) return res.end(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="${site.icon.viewBox ?? '0 0 24 24'}"><path d="${site.icon.path}" fill="${site.icon.tint}"/></svg>`);
            if (site?.icon.src) return res.end(readFileSync(resolve(src, 'wubble/sites', `${site.id}-favicon.svg`)));
            if (kind === 'page' && name === 'start') {
              // browser/icons.tsx `Mark`: the start page's tab icon
              const mark = /export function Mark[\s\S]*?(<svg[\s\S]*?<\/svg>);/.exec(icons)[1]
                .replace(/width=\{size\} height=\{size\} /, '').replace(' aria-hidden="true"', ' xmlns="http://www.w3.org/2000/svg"')
                .replace(/stop-color=/g, 'stop-color=').replace(/ \/>/g, '/>');
              return res.end(mark);
            }
            const PAGE_ICON = { start: 'bubble', touch: 'touch', room: 'room', foundations: 'foundations', settings: 'settings' };
            const d = P[kind === 'page' ? PAGE_ICON[name] : 'globe'] ?? P.globe;
            return res.end(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#F2F4FA" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="${d}"/></svg>`);
          }
          const title = site ? site.title : kind === 'page' ? model.PAGE_TITLES[name] : name;
          const body = site ? `<img src="${site.image}" style="width:100%;display:block">`
            : name === 'touch' || name === 'room' ? `<iframe src="/wubble/pages/_harness.html?page=${name}" style="border:0;width:100%;height:100vh"></iframe>` : '';
          res.setHeader('content-type', 'text/html');
          return res.end(`<!doctype html><html><head><meta charset="utf-8"><title>${title}</title><link rel="icon" href="${base}/${kind}-${name}.svg"><style>html,body{margin:0;background:${site?.bg ?? tokens.color.web}}</style></head><body>${body}</body></html>`);
        } catch (e) {
          res.statusCode = 500;
          res.end(String(e?.stack ?? e));
        }
      });
    },
  };
}

export default {
  base: './',
  root: src,
  publicDir: resolve(root, 'public'),
  resolve: { alias: { '@utils': resolve(root, 'utils') } },
  server: { host: '127.0.0.1', port: PORT, strictPort: true, hmr: false },
  plugins: [fidelity()],
  logLevel: 'warn',
};
