/**
 * Stable fluids (Stam 1999) on the GPU: semi-Lagrangian advection, vorticity confinement,
 * divergence, Jacobi pressure solve, projection. Carries RGB dye. Coordinates for splats are
 * 0..1 with y up. This is the Wubble backdrop's liquid.
 */
import { Double, VERT_FS, bindTex, formats, freeTarget, program, target, type Formats, type Program, type Target } from './util';

// @source fluid
const ADVECT = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uVel, uSrc; uniform vec2 uTexel; uniform float uDt, uDiss;
void main() {
  vec2 coord = vUv - uDt * texture(uVel, vUv).xy * uTexel;
  o = texture(uSrc, coord) / (1.0 + uDiss * uDt);
}`;

// @source fluid-splat
const SPLAT = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uSrc; uniform float uAspect, uRadius; uniform vec2 uPoint; uniform vec3 uValue; uniform int uRing;
void main() {
  vec2 p = vUv - uPoint; p.x *= uAspect;
  float d2 = dot(p, p);
  vec3 add;
  if (uRing == 1) {
    // outward push: a ripple. Velocity points away from the centre, strongest on a thin ring.
    float d = sqrt(d2);
    float x = d - uRadius * 3.0;
    float ring = exp(-x * x / (uRadius * uRadius * 0.6));
    vec2 dir = d > 1e-5 ? p / d : vec2(0.0);
    dir.x /= uAspect;
    add = vec3(dir * uValue.x * ring, 0.0);
  } else {
    add = uValue * exp(-d2 / uRadius);
  }
  o = vec4(texture(uSrc, vUv).xyz + add, 1.0);
}`;

const CURL = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uVel; uniform vec2 uTexel;
void main() {
  float L = texture(uVel, vUv - vec2(uTexel.x, 0)).y, R = texture(uVel, vUv + vec2(uTexel.x, 0)).y;
  float T = texture(uVel, vUv + vec2(0, uTexel.y)).x, B = texture(uVel, vUv - vec2(0, uTexel.y)).x;
  o = vec4(0.5 * (R - L - T + B), 0, 0, 1);
}`;

// @source fluid-vorticity
const VORTICITY = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uVel, uCurl; uniform vec2 uTexel; uniform float uCurlAmt, uDt;
void main() {
  float L = texture(uCurl, vUv - vec2(uTexel.x, 0)).x, R = texture(uCurl, vUv + vec2(uTexel.x, 0)).x;
  float T = texture(uCurl, vUv + vec2(0, uTexel.y)).x, B = texture(uCurl, vUv - vec2(0, uTexel.y)).x;
  float C = texture(uCurl, vUv).x;
  vec2 f = 0.5 * vec2(abs(T) - abs(B), abs(R) - abs(L));
  f /= length(f) + 1e-4;
  f *= uCurlAmt * C; f.y *= -1.0;
  vec2 v = texture(uVel, vUv).xy + f * uDt;
  o = vec4(clamp(v, -1000.0, 1000.0), 0, 1);
}`;

const DIVERGENCE = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uVel; uniform vec2 uTexel;
void main() {
  float L = texture(uVel, vUv - vec2(uTexel.x, 0)).x, R = texture(uVel, vUv + vec2(uTexel.x, 0)).x;
  float T = texture(uVel, vUv + vec2(0, uTexel.y)).y, B = texture(uVel, vUv - vec2(0, uTexel.y)).y;
  vec2 C = texture(uVel, vUv).xy;
  // free-slip walls: reflect at the edges
  if (vUv.x - uTexel.x < 0.0) L = -C.x;
  if (vUv.x + uTexel.x > 1.0) R = -C.x;
  if (vUv.y + uTexel.y > 1.0) T = -C.y;
  if (vUv.y - uTexel.y < 0.0) B = -C.y;
  o = vec4(0.5 * (R - L + T - B), 0, 0, 1);
}`;

// @source fluid-pressure
const PRESSURE = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uP, uDiv; uniform vec2 uTexel;
void main() {
  float L = texture(uP, vUv - vec2(uTexel.x, 0)).x, R = texture(uP, vUv + vec2(uTexel.x, 0)).x;
  float T = texture(uP, vUv + vec2(0, uTexel.y)).x, B = texture(uP, vUv - vec2(0, uTexel.y)).x;
  o = vec4((L + R + B + T - texture(uDiv, vUv).x) * 0.25, 0, 0, 1);
}`;

const GRADIENT = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uP, uVel; uniform vec2 uTexel;
void main() {
  float L = texture(uP, vUv - vec2(uTexel.x, 0)).x, R = texture(uP, vUv + vec2(uTexel.x, 0)).x;
  float T = texture(uP, vUv + vec2(0, uTexel.y)).x, B = texture(uP, vUv - vec2(0, uTexel.y)).x;
  o = vec4(texture(uVel, vUv).xy - vec2(R - L, T - B), 0, 1);
}`;

const SCALE = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uSrc; uniform float uK;
void main() { o = texture(uSrc, vUv) * uK; }`;

export interface FluidOptions { simRes?: number; dyeRes?: number; pressureIters?: number; curl?: number; velDiss?: number; dyeDiss?: number }

export class Fluid {
  private gl: WebGL2RenderingContext;
  private f: Formats;
  private pr: Record<string, Program>;
  vel!: Double; dye!: Double; private pres!: Double; private div!: Target; private curlT!: Target;
  private aspect = 1;
  opts: Required<FluidOptions>;

  constructor(gl: WebGL2RenderingContext, private draw: () => void, opts: FluidOptions = {}) {
    this.gl = gl;
    this.f = formats(gl).float;
    this.opts = { simRes: 96, dyeRes: 256, pressureIters: 18, curl: 2.5, velDiss: 0.8, dyeDiss: 0.12, ...opts };
    this.pr = {
      advect: program(gl, VERT_FS, ADVECT), splat: program(gl, VERT_FS, SPLAT), curl: program(gl, VERT_FS, CURL),
      vort: program(gl, VERT_FS, VORTICITY), div: program(gl, VERT_FS, DIVERGENCE), pres: program(gl, VERT_FS, PRESSURE),
      grad: program(gl, VERT_FS, GRADIENT), scale: program(gl, VERT_FS, SCALE),
    };
  }

  /** (Re)allocate for a canvas of w×h CSS px. Keeps nothing; cheap at these sizes. */
  resize(w: number, h: number) {
    const gl = this.gl;
    this.aspect = w / Math.max(1, h);
    const dims = (res: number) => {
      const a = this.aspect;
      return a >= 1 ? [Math.round(res * a), res] : [res, Math.round(res / a)];
    };
    const [sw, sh] = dims(this.opts.simRes), [dw, dh] = dims(this.opts.dyeRes);
    this.free();
    const mk = (W: number, H: number) => new Double(target(gl, W, H, this.f), target(gl, W, H, this.f));
    this.vel = mk(sw, sh); this.dye = mk(dw, dh); this.pres = mk(sw, sh);
    this.div = target(gl, sw, sh, this.f, gl.NEAREST); this.curlT = target(gl, sw, sh, this.f, gl.NEAREST);
  }

  private run(pr: Program, out: Target) {
    const gl = this.gl;
    gl.bindFramebuffer(gl.FRAMEBUFFER, out.fbo);
    gl.viewport(0, 0, out.w, out.h);
    this.draw();
  }

  /** Add velocity (px/s-ish, in sim units) and dye at a point (0..1, y up). */
  splat(x: number, y: number, dx: number, dy: number, color: [number, number, number] | null, radius = 0.0025) {
    const gl = this.gl, s = this.pr.splat;
    gl.useProgram(s.p);
    gl.uniform1f(s.u.uAspect, this.aspect);
    gl.uniform2f(s.u.uPoint, x, y);
    gl.uniform1f(s.u.uRadius, radius);
    gl.uniform1i(s.u.uRing, 0);
    bindTex(gl, 0, this.vel.read.tex, s.u.uSrc);
    gl.uniform3f(s.u.uValue, dx, dy, 0);
    this.run(s, this.vel.write); this.vel.swap();
    if (color) {
      bindTex(gl, 0, this.dye.read.tex, s.u.uSrc);
      gl.uniform3f(s.u.uValue, color[0], color[1], color[2]);
      this.run(s, this.dye.write); this.dye.swap();
    }
  }

  /** A ring of outward velocity: the liquid's response to a tap. */
  ripple(x: number, y: number, strength = 900, radius = 0.012) {
    const gl = this.gl, s = this.pr.splat;
    gl.useProgram(s.p);
    gl.uniform1f(s.u.uAspect, this.aspect);
    gl.uniform2f(s.u.uPoint, x, y);
    gl.uniform1f(s.u.uRadius, radius);
    gl.uniform1i(s.u.uRing, 1);
    gl.uniform3f(s.u.uValue, strength, 0, 0);
    bindTex(gl, 0, this.vel.read.tex, s.u.uSrc);
    this.run(s, this.vel.write); this.vel.swap();
  }

// @source fluid-step
  step(dt: number) {
    const gl = this.gl, P = this.pr, o = this.opts;
    const vt: [number, number] = [1 / this.vel.read.w, 1 / this.vel.read.h];
    gl.disable(gl.BLEND);

    gl.useProgram(P.curl.p);
    gl.uniform2f(P.curl.u.uTexel, ...vt);
    bindTex(gl, 0, this.vel.read.tex, P.curl.u.uVel);
    this.run(P.curl, this.curlT);

    gl.useProgram(P.vort.p);
    gl.uniform2f(P.vort.u.uTexel, ...vt);
    gl.uniform1f(P.vort.u.uCurlAmt, o.curl);
    gl.uniform1f(P.vort.u.uDt, dt);
    bindTex(gl, 0, this.vel.read.tex, P.vort.u.uVel);
    bindTex(gl, 1, this.curlT.tex, P.vort.u.uCurl);
    this.run(P.vort, this.vel.write); this.vel.swap();

    gl.useProgram(P.div.p);
    gl.uniform2f(P.div.u.uTexel, ...vt);
    bindTex(gl, 0, this.vel.read.tex, P.div.u.uVel);
    this.run(P.div, this.div);

    // warm start: keep 80% of last frame's pressure
    gl.useProgram(P.scale.p);
    gl.uniform1f(P.scale.u.uK, 0.8);
    bindTex(gl, 0, this.pres.read.tex, P.scale.u.uSrc);
    this.run(P.scale, this.pres.write); this.pres.swap();

    gl.useProgram(P.pres.p);
    gl.uniform2f(P.pres.u.uTexel, ...vt);
    bindTex(gl, 1, this.div.tex, P.pres.u.uDiv);
    for (let i = 0; i < o.pressureIters; i++) {
      bindTex(gl, 0, this.pres.read.tex, P.pres.u.uP);
      this.run(P.pres, this.pres.write); this.pres.swap();
    }

    gl.useProgram(P.grad.p);
    gl.uniform2f(P.grad.u.uTexel, ...vt);
    bindTex(gl, 0, this.pres.read.tex, P.grad.u.uP);
    bindTex(gl, 1, this.vel.read.tex, P.grad.u.uVel);
    this.run(P.grad, this.vel.write); this.vel.swap();

    gl.useProgram(P.advect.p);
    gl.uniform2f(P.advect.u.uTexel, ...vt);
    gl.uniform1f(P.advect.u.uDt, dt);
    gl.uniform1f(P.advect.u.uDiss, o.velDiss);
    bindTex(gl, 0, this.vel.read.tex, P.advect.u.uVel);
    bindTex(gl, 1, this.vel.read.tex, P.advect.u.uSrc);
    this.run(P.advect, this.vel.write); this.vel.swap();

    gl.uniform1f(P.advect.u.uDiss, o.dyeDiss);
    bindTex(gl, 0, this.vel.read.tex, P.advect.u.uVel);
    bindTex(gl, 1, this.dye.read.tex, P.advect.u.uSrc);
    this.run(P.advect, this.dye.write); this.dye.swap();
  }

  free() {
    const gl = this.gl;
    this.vel?.free(gl); this.dye?.free(gl); this.pres?.free(gl);
    freeTarget(gl, this.div); freeTarget(gl, this.curlT);
  }

  dispose() {
    this.free();
    for (const k in this.pr) this.gl.deleteProgram(this.pr[k].p);
  }
}
