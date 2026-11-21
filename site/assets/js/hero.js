/* The hero's living bubble: the design's start page (pages/start.tsx) on this page.
   It runs only with WebGL2, never under prefers-reduced-motion, and stops while the hero is off-screen
   or the tab is hidden (core/physics.ts Loop). Otherwise the still liquid and the SVG mark stay. */
(function () {
  'use strict';
  var stage = document.getElementById('stage'), mark = document.getElementById('mark');
  if (!stage || !mark) return;
  var rm = window.matchMedia && matchMedia('(prefers-reduced-motion: reduce)');
  if (rm && rm.matches) return;
  try { if (!document.createElement('canvas').getContext('webgl2')) return; } catch (e) { return; }

  var s = document.createElement('script');
  s.src = 'assets/js/living.js';
  s.onload = start;
  document.head.appendChild(s);

  function start() {
    if (typeof window.wubbleLiving !== 'function') return;
    // the bubble sits where the mark is laid out: `at` and `size` are read by the design's layout()
    var opts = { at: { x: 0.5, y: 0.3 }, size: 0.1, ambient: true, palette: ['#2F5FC4', '#7563D8', '#237E69'] };
    var live;
    function place() {
      var sr = stage.getBoundingClientRect(), mr = mark.getBoundingClientRect();
      if (!sr.width || !sr.height) return;
      opts.at.x = (mr.left + mr.width / 2 - sr.left) / sr.width;
      opts.at.y = (mr.top + mr.height / 2 - sr.top) / sr.height;
      opts.size = (mr.width / 2) / Math.min(sr.width, sr.height);
      if (live) {
        live.body.home = { x: sr.width * opts.at.x, y: sr.height * opts.at.y };
        live.body.R = Math.max(40, mr.width / 2);
      }
    }
    place();
    try { live = window.wubbleLiving(stage, opts); } catch (e) { return; } // no WebGL2 after all: the still stays
    place();
    var canvas = stage.querySelector('canvas');
    // the page must still scroll when a finger lands on the liquid
    if (canvas) canvas.style.touchAction = 'pan-y';
    if ('ResizeObserver' in window) new ResizeObserver(place).observe(mark);
    window.addEventListener('resize', place);

    var on = true;
    if ('IntersectionObserver' in window) {
      new IntersectionObserver(function (es) {
        on = es[es.length - 1].isIntersecting;
        live.setActive(on);
      }).observe(stage);
    }
    if (rm && rm.addEventListener) rm.addEventListener('change', function (e) { if (e.matches) { live.setActive(false); } });
    // the first frames settle the soft body and seed the liquid; fade in after them
    setTimeout(function () { stage.classList.add('live'); }, 600);
  }
})();
