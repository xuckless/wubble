// `--measure load`: a page reports its own timings to the loopback server (POST /__t) once it has
// loaded. The same script runs in wubble and in plain Chromium, so both are read the same way:
// Navigation Timing (DOMContentLoaded and load end) and Paint Timing (first paint and first
// contentful paint), all from the navigation's time origin, in milliseconds. W3C specifications;
// no code was copied.
(function () {
  // When the first script of the document ran: what document-start work (content scripts the
  // browser injects into every frame, the document's own parse) cost before the page's code.
  const s0 = performance.now();
  const paint = (name) => {
    const entry = performance.getEntriesByName(name)[0];
    return entry ? entry.startTime : null;
  };
  const round = (v) => (typeof v === 'number' ? Math.round(v * 10) / 10 : null);
  const send = (persisted) => {
    const nav = performance.getEntriesByType('navigation')[0];
    const body = JSON.stringify({
      page: location.pathname,
      fp: round(paint('first-paint')),
      fcp: round(paint('first-contentful-paint')),
      dcl: round(nav.domContentLoadedEventEnd),
      load: round(nav.loadEventEnd),
      build: round(window.__build),
      s0: round(s0),
      persisted,
    });
    fetch('/__t', { method: 'POST', body, keepalive: true }).then(() => {
      // `?chain=N` (plain Chromium, which cannot be driven): load the same page again N more
      // times, under another document URL so the assets come from the HTTP cache.
      // `?next=PATH` (the warm-up page): then go to PATH, so that the page measured is not the
      // first load of a fresh browser, as in wubble's run.
      const next = /[?&]next=([^&]+)/.exec(location.search);
      if (next) location.href = decodeURIComponent(next[1]);
      const left = Number((/[?&]chain=(\d+)/.exec(location.search) || [])[1] || 0);
      if (left > 0) location.href = `${location.pathname}?w=${left}&chain=${left - 1}`;
    });
  };
  // A page restored from the back/forward cache fires no load event again, only `pageshow`.
  addEventListener('pageshow', (e) => {
    if (e.persisted) setTimeout(() => send(true), 50);
  });
  addEventListener('load', () => {
    let tries = 0;
    // Load end is set after the load handlers return, and the first contentful paint may follow
    // the load event: look again every 25 ms, at most 200 times.
    const wait = () => {
      const nav = performance.getEntriesByType('navigation')[0];
      // A page that builds its DOM after load (`/bigdom`) sets `__pending` and `__build`.
      const built = !window.__pending || window.__build !== undefined;
      if ((nav.loadEventEnd > 0 && paint('first-contentful-paint') !== null && built) || ++tries > 200) {
        send();
      } else {
        setTimeout(wait, 25);
      }
    };
    setTimeout(wait, 0);
  });
})();
