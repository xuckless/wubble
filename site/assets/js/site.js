/* wubble.cc: downloads, copy, the download button's stretch, section entrances. No network requests. */
(function () {
  'use strict';

  // ── The release: the one place a new version is written (site/README.md) ──
  var VERSION = '0.1.0-preview.7';
  var BASE = 'https://github.com/xuckless/wubble/releases/download/v' + VERSION + '/wubble-' + VERSION;
  var ASSETS = {
    linux: { url: BASE + '-linux-x86_64.tar.xz', file: 'wubble-' + VERSION + '-linux-x86_64.tar.xz', name: 'Linux', label: 'Download for Linux' },
    mac: { url: BASE + '-macos-aarch64.dmg', file: 'wubble-' + VERSION + '-macos-aarch64.dmg', name: 'macOS', label: 'Download for macOS' },
  };
  var INSTALL = 'curl -fsSL https://raw.githubusercontent.com/xuckless/wubble/main/scripts/release/install.sh | sh';

  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };

  // ── Which computer is this? Linux x86_64 and macOS get their file first; everyone else sees both. ──
  function detect() {
    var ua = navigator.userAgent || '';
    var p = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
    if (/android|iphone|ipad|ipod|cros/i.test(ua)) return 'other';
    if (/mac/i.test(p) || /Macintosh/.test(ua)) return navigator.maxTouchPoints > 1 ? 'other' : 'mac'; // an iPad asks as a Mac
    if (/win/i.test(p) || /Windows/.test(ua)) return 'windows';
    if (/linux/i.test(p) || /Linux|X11/.test(ua)) return 'linux';
    return 'other';
  }
  var os = detect();

  $$('[data-version]').forEach(function (el) { el.textContent = VERSION; });
  $$('[data-asset]').forEach(function (a) {
    var as = ASSETS[a.getAttribute('data-asset')];
    a.href = as.url;
    a.setAttribute('aria-label', 'Download ' + as.file);
  });

  var dl = $('#hero-dl'), label = $('#hero-dl-label'), forEl = $('#hero-for');
  if (os === 'linux' || os === 'mac') {
    var a = ASSETS[os];
    dl.href = a.url; label.textContent = a.label;
    dl.setAttribute('aria-label', a.label + ', ' + a.file);
    forEl.textContent = os === 'mac' ? 'macOS, Apple silicon' : 'Linux, x86_64';
    var mine = $(os === 'mac' ? '#os-mac' : '#os-linux');
    mine.classList.add('mine');
    $('#platforms').prepend(mine);
    $$('.dl', mine).forEach(function (b) { b.classList.remove('ghost'); });
  } else {
    // both are listed below; the hero sends people there
    dl.href = '#install'; label.textContent = 'Downloads';
    $('#hero-cmd').hidden = true;
    $('#hero-other').hidden = true; $('#hero-other-sep').hidden = true;
    forEl.textContent = 'Linux and macOS';
    $('#windows').hidden = false;
    if (os === 'windows') forEl.textContent = 'Linux and macOS, not Windows yet';
  }

  // ── Copy the install command: the Copy ↔ Copied swap is the design's Overlap (240 in, 190 out) ──
  var live = $('#live');
  function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) return navigator.clipboard.writeText(text).catch(function () { return legacyCopy(text); });
    return legacyCopy(text);
  }
  function legacyCopy(text) {
    return new Promise(function (ok, no) {
      var t = document.createElement('textarea');
      t.value = text; t.setAttribute('readonly', ''); t.style.cssText = 'position:fixed;opacity:0;top:0;left:0';
      document.body.appendChild(t); t.select();
      try { document.execCommand('copy') ? ok() : no(); } catch (e) { no(e); }
      t.remove();
    });
  }
  $$('[data-copy]').forEach(function (b) {
    var timer = 0;
    b.addEventListener('click', function () {
      copyText(INSTALL).then(function () {
        b.classList.add('done');
        live.textContent = 'Install command copied';
        clearTimeout(timer);
        timer = setTimeout(function () { b.classList.remove('done'); live.textContent = ''; }, 1600);
      }, function () {
        var code = b.parentNode.querySelector('code');
        var r = document.createRange(); r.selectNodeContents(code);
        var s = getSelection(); s.removeAllRanges(); s.addRange(r);
        live.textContent = 'Select the command and copy it';
      });
    });
  });

  // ── Download button: press and it stretches outward, let go and it rubber-bands back.
  //    Port of pages/touch.tsx (touch-download) with core/physics.ts (spring step, band, Loop). ──
  var reduced = window.matchMedia && matchMedia('(prefers-reduced-motion: reduce)');
  var DL = { held: 0.12, hold: [700, 40], pull: 26, release: [520, 16] };
  var DT_CAP = 0.032, REST = 0.02;
  function band(d, k) { return Math.sign(d) * k * (1 - 1 / (Math.abs(d) / k + 1)); }
  function step(s, target, k, c, dt) {
    var acc = -k * (s.x - target) - c * s.v;
    s.v += acc * dt; s.x += s.v * dt;
    if (Math.abs(s.v) < REST && Math.abs(s.x - target) < REST) { s.x = target; s.v = 0; return false; }
    return true;
  }
  $$('.stretch').forEach(function (btn) {
    var st = { x: 1, v: 0 }, held = false, px = 0, cx = 0, w = 0, raf = 0, last = 0;
    function paint() {
      var k = Math.max(0.5, st.x);
      btn.style.transform = k === 1 ? '' : 'scale(' + k.toFixed(4) + ',' + (1 / Math.sqrt(k)).toFixed(4) + ')';
    }
    function tick(now) {
      var dt = Math.min(DT_CAP, (now - last) / 1000); last = now;
      var target = 1;
      if (held && w) {
        var out = Math.max(0, Math.abs(px - cx) - w / 2);
        target = 1 + DL.held + (2 * band(out, DL.pull)) / w;
      }
      var kc = held ? DL.hold : DL.release, moving;
      if (reduced && reduced.matches) { st.x = 1; st.v = 0; moving = false; }
      else moving = step(st, target, kc[0], kc[1], dt);
      paint();
      raf = (moving || held) ? requestAnimationFrame(tick) : 0;
    }
    function kick() { if (!raf) { last = performance.now(); raf = requestAnimationFrame(tick); } }
    btn.addEventListener('pointerdown', function (e) {
      if (e.button !== 0) return;
      var b = btn.getBoundingClientRect();
      w = btn.offsetWidth; cx = b.left + b.width / 2; px = e.clientX; held = true; kick();
    });
    window.addEventListener('pointermove', function (e) { if (held) px = e.clientX; }, { passive: true });
    ['pointerup', 'pointercancel', 'blur'].forEach(function (t) {
      window.addEventListener(t, function () { if (held) { held = false; kick(); } });
    });
    btn.addEventListener('keydown', function (e) {
      if (e.key === 'Enter') { st.v += 2.4; kick(); }
    });
  });

  // ── Section entrances: Overlap in, 30 ms apart, the first time a section is seen (pages/found-ui.tsx) ──
  var secs = $$('.sec');
  secs.forEach(function (sec) { $$('.r', sec).forEach(function (el, i) { el.style.setProperty('--i', String(Math.min(i, 12))); }); });
  if (!('IntersectionObserver' in window)) { secs.forEach(function (s) { s.classList.add('on'); }); return; }
  var io = new IntersectionObserver(function (es) {
    es.forEach(function (e) { if (e.isIntersecting) { e.target.classList.add('on'); io.unobserve(e.target); } });
  }, { rootMargin: '0px 0px -12% 0px' });
  secs.forEach(function (s) { io.observe(s); });
  // a jump straight to #install must not leave the sections above it hidden for a printout or a find
  window.addEventListener('beforeprint', function () { secs.forEach(function (s) { s.classList.add('on'); }); });
})();
