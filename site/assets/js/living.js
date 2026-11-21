/* The living bubble on its fluid backdrop: src/wubble/core/living.ts and what it imports (core/gl/scene.ts,
   fluid.ts, glsl.ts, util.ts, core/softbody.ts, minis.ts, physics.ts) from the owner's design repo
   xuckless/the-wallpaper-project @598f814, bundled unchanged with rolldown (see site/README.md).
   Exposes window.wubbleLiving = mountLiving. */
(function() {
	// src/wubble/core/gl/util.ts
	/** Small WebGL2 helpers shared by the Wubble renderers. */
	const VERT_FS = `#version 300 es
in vec2 aPos;
out vec2 vUv;
void main() { vUv = aPos * 0.5 + 0.5; gl_Position = vec4(aPos, 0.0, 1.0); }`;
	/** Quad in pixel space: aPos is 0..1, expanded to uRect (x, y, w, h in CSS px, top-left origin). */
	const VERT_QUAD = `#version 300 es
in vec2 aPos;
uniform vec4 uQuad;
uniform vec2 uView;
out vec2 vPx;
void main() {
  vec2 unit = aPos * 0.5 + 0.5;
  vPx = uQuad.xy + unit * uQuad.zw;
  vec2 clip = vPx / uView * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
}`;
	function program(gl, vs, fs) {
		const sh = (type, src) => {
			const s = gl.createShader(type);
			gl.shaderSource(s, src);
			gl.compileShader(s);
			if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) + "\n" + src.split("\n").map((l, i) => `${i + 1}: ${l}`).join("\n"));
			return s;
		};
		const p = gl.createProgram();
		gl.attachShader(p, sh(gl.VERTEX_SHADER, vs));
		gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
		gl.bindAttribLocation(p, 0, "aPos");
		gl.linkProgram(p);
		if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? "link failed");
		const u = {};
		const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
		for (let i = 0; i < n; i++) {
			const info = gl.getActiveUniform(p, i);
			const name = info.name.replace(/\[0\]$/, "");
			u[name] = gl.getUniformLocation(p, info.name);
		}
		return {
			p,
			u
		};
	}
	/** A unit quad (-1..1) bound to attribute 0. Draw with gl.drawArrays(TRIANGLE_STRIP, 0, 4). */
	function quad(gl) {
		const vao = gl.createVertexArray();
		gl.bindVertexArray(vao);
		const buf = gl.createBuffer();
		gl.bindBuffer(gl.ARRAY_BUFFER, buf);
		gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([
			-1,
			-1,
			1,
			-1,
			-1,
			1,
			1,
			1
		]), gl.STATIC_DRAW);
		gl.enableVertexAttribArray(0);
		gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
		return {
			vao,
			buf
		};
	}
	/** Half-float RGBA if the GPU can render to it, else plain RGBA8 (the liquid then can't hold signed velocity and goes still; a flat backdrop remains). */
	function formats(gl) {
		const canFloat = !!gl.getExtension("EXT_color_buffer_float") || !!gl.getExtension("EXT_color_buffer_half_float");
		const linearFloat = true;
		const byte = {
			internal: gl.RGBA8,
			format: gl.RGBA,
			type: gl.UNSIGNED_BYTE
		};
		return {
			float: canFloat ? {
				internal: gl.RGBA16F,
				format: gl.RGBA,
				type: gl.HALF_FLOAT
			} : byte,
			byte,
			linearFloat
		};
	}
	function target(gl, w, h, f, filter = gl.LINEAR) {
		const tex = gl.createTexture();
		gl.bindTexture(gl.TEXTURE_2D, tex);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
		gl.texImage2D(gl.TEXTURE_2D, 0, f.internal, w, h, 0, f.format, f.type, null);
		const fbo = gl.createFramebuffer();
		gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
		gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
		gl.viewport(0, 0, w, h);
		gl.clearColor(0, 0, 0, 0);
		gl.clear(gl.COLOR_BUFFER_BIT);
		return {
			tex,
			fbo,
			w,
			h
		};
	}
	function freeTarget(gl, t) {
		if (!t) return;
		gl.deleteTexture(t.tex);
		gl.deleteFramebuffer(t.fbo);
	}
	var Double = class {
		read;
		write;
		constructor(read, write) {
			this.read = read;
			this.write = write;
		}
		swap() {
			const t = this.read;
			this.read = this.write;
			this.write = t;
		}
		free(gl) {
			freeTarget(gl, this.read);
			freeTarget(gl, this.write);
		}
	};
	function bindTex(gl, unit, tex, loc) {
		gl.activeTexture(gl.TEXTURE0 + unit);
		gl.bindTexture(gl.TEXTURE_2D, tex);
		gl.uniform1i(loc, unit);
	}
	function hex(c) {
		const n = parseInt(c.replace("#", ""), 16);
		return [
			(n >> 16 & 255) / 255,
			(n >> 8 & 255) / 255,
			(n & 255) / 255
		];
	}
	// src/wubble/core/gl/fluid.ts
	/**
	* Stable fluids (Stam 1999) on the GPU: semi-Lagrangian advection, vorticity confinement,
	* divergence, Jacobi pressure solve, projection. Carries RGB dye. Coordinates for splats are
	* 0..1 with y up. This is the Wubble backdrop's liquid.
	*/
	const ADVECT = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uVel, uSrc; uniform vec2 uTexel; uniform float uDt, uDiss;
void main() {
  vec2 coord = vUv - uDt * texture(uVel, vUv).xy * uTexel;
  o = texture(uSrc, coord) / (1.0 + uDiss * uDt);
}`;
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
	var Fluid = class {
		draw;
		gl;
		f;
		pr;
		vel;
		dye;
		pres;
		div;
		curlT;
		aspect = 1;
		opts;
		constructor(gl, draw, opts = {}) {
			this.draw = draw;
			this.gl = gl;
			this.f = formats(gl).float;
			this.opts = {
				simRes: 96,
				dyeRes: 256,
				pressureIters: 18,
				curl: 2.5,
				velDiss: .8,
				dyeDiss: .12,
				...opts
			};
			this.pr = {
				advect: program(gl, VERT_FS, ADVECT),
				splat: program(gl, VERT_FS, SPLAT),
				curl: program(gl, VERT_FS, CURL),
				vort: program(gl, VERT_FS, VORTICITY),
				div: program(gl, VERT_FS, DIVERGENCE),
				pres: program(gl, VERT_FS, PRESSURE),
				grad: program(gl, VERT_FS, GRADIENT),
				scale: program(gl, VERT_FS, SCALE)
			};
		}
		/** (Re)allocate for a canvas of w×h CSS px. Keeps nothing; cheap at these sizes. */
		resize(w, h) {
			const gl = this.gl;
			this.aspect = w / Math.max(1, h);
			const dims = (res) => {
				const a = this.aspect;
				return a >= 1 ? [Math.round(res * a), res] : [res, Math.round(res / a)];
			};
			const [sw, sh] = dims(this.opts.simRes), [dw, dh] = dims(this.opts.dyeRes);
			this.free();
			const mk = (W, H) => new Double(target(gl, W, H, this.f), target(gl, W, H, this.f));
			this.vel = mk(sw, sh);
			this.dye = mk(dw, dh);
			this.pres = mk(sw, sh);
			this.div = target(gl, sw, sh, this.f, gl.NEAREST);
			this.curlT = target(gl, sw, sh, this.f, gl.NEAREST);
		}
		run(pr, out) {
			const gl = this.gl;
			gl.bindFramebuffer(gl.FRAMEBUFFER, out.fbo);
			gl.viewport(0, 0, out.w, out.h);
			this.draw();
		}
		/** Add velocity (px/s-ish, in sim units) and dye at a point (0..1, y up). */
		splat(x, y, dx, dy, color, radius = .0025) {
			const gl = this.gl, s = this.pr.splat;
			gl.useProgram(s.p);
			gl.uniform1f(s.u.uAspect, this.aspect);
			gl.uniform2f(s.u.uPoint, x, y);
			gl.uniform1f(s.u.uRadius, radius);
			gl.uniform1i(s.u.uRing, 0);
			bindTex(gl, 0, this.vel.read.tex, s.u.uSrc);
			gl.uniform3f(s.u.uValue, dx, dy, 0);
			this.run(s, this.vel.write);
			this.vel.swap();
			if (color) {
				bindTex(gl, 0, this.dye.read.tex, s.u.uSrc);
				gl.uniform3f(s.u.uValue, color[0], color[1], color[2]);
				this.run(s, this.dye.write);
				this.dye.swap();
			}
		}
		/** A ring of outward velocity: the liquid's response to a tap. */
		ripple(x, y, strength = 900, radius = .012) {
			const gl = this.gl, s = this.pr.splat;
			gl.useProgram(s.p);
			gl.uniform1f(s.u.uAspect, this.aspect);
			gl.uniform2f(s.u.uPoint, x, y);
			gl.uniform1f(s.u.uRadius, radius);
			gl.uniform1i(s.u.uRing, 1);
			gl.uniform3f(s.u.uValue, strength, 0, 0);
			bindTex(gl, 0, this.vel.read.tex, s.u.uSrc);
			this.run(s, this.vel.write);
			this.vel.swap();
		}
		step(dt) {
			const gl = this.gl, P = this.pr, o = this.opts;
			const vt = [1 / this.vel.read.w, 1 / this.vel.read.h];
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
			this.run(P.vort, this.vel.write);
			this.vel.swap();
			gl.useProgram(P.div.p);
			gl.uniform2f(P.div.u.uTexel, ...vt);
			bindTex(gl, 0, this.vel.read.tex, P.div.u.uVel);
			this.run(P.div, this.div);
			gl.useProgram(P.scale.p);
			gl.uniform1f(P.scale.u.uK, .8);
			bindTex(gl, 0, this.pres.read.tex, P.scale.u.uSrc);
			this.run(P.scale, this.pres.write);
			this.pres.swap();
			gl.useProgram(P.pres.p);
			gl.uniform2f(P.pres.u.uTexel, ...vt);
			bindTex(gl, 1, this.div.tex, P.pres.u.uDiv);
			for (let i = 0; i < o.pressureIters; i++) {
				bindTex(gl, 0, this.pres.read.tex, P.pres.u.uP);
				this.run(P.pres, this.pres.write);
				this.pres.swap();
			}
			gl.useProgram(P.grad.p);
			gl.uniform2f(P.grad.u.uTexel, ...vt);
			bindTex(gl, 0, this.pres.read.tex, P.grad.u.uP);
			bindTex(gl, 1, this.vel.read.tex, P.grad.u.uVel);
			this.run(P.grad, this.vel.write);
			this.vel.swap();
			gl.useProgram(P.advect.p);
			gl.uniform2f(P.advect.u.uTexel, ...vt);
			gl.uniform1f(P.advect.u.uDt, dt);
			gl.uniform1f(P.advect.u.uDiss, o.velDiss);
			bindTex(gl, 0, this.vel.read.tex, P.advect.u.uVel);
			bindTex(gl, 1, this.vel.read.tex, P.advect.u.uSrc);
			this.run(P.advect, this.vel.write);
			this.vel.swap();
			gl.uniform1f(P.advect.u.uDiss, o.dyeDiss);
			bindTex(gl, 0, this.vel.read.tex, P.advect.u.uVel);
			bindTex(gl, 1, this.dye.read.tex, P.advect.u.uSrc);
			this.run(P.advect, this.dye.write);
			this.dye.swap();
		}
		free() {
			const gl = this.gl;
			this.vel?.free(gl);
			this.dye?.free(gl);
			this.pres?.free(gl);
			freeTarget(gl, this.div);
			freeTarget(gl, this.curlT);
		}
		dispose() {
			this.free();
			for (const k in this.pr) this.gl.deleteProgram(this.pr[k].p);
		}
	};
	// src/wubble/core/gl/glsl.ts
	/** GLSL chunks shared by the Wubble shaders. */
	/**
	* Thin-film interference for a soap film (n = 1.33) in air.
	* Reflectance of a film of thickness `d` nm, at refracted-angle cosine `cosT`, for each RGB channel.
	* The first surface adds a half-wave phase shift, so R ∝ sin²(2π n d cosT / λ).
	* Each channel averages three wavelengths across its band so colours stay smooth, the way
	* real white light does, instead of the hard bands a single wavelength gives.
	* Films under ~30 nm reflect almost nothing: the "black film" just before a bubble pops.
	*/
	const FILM = `
vec3 thinFilm(float d, float cosT) {
  float opd = 2.0 * 1.33 * d * cosT;
  const vec3 l0 = vec3(680.0, 560.0, 470.0);
  const vec3 l1 = vec3(620.0, 530.0, 445.0);
  const vec3 l2 = vec3(590.0, 500.0, 420.0);
  vec3 r = sin(3.14159265 * opd / l0); r *= r;
  vec3 g = sin(3.14159265 * opd / l1); g *= g;
  vec3 b = sin(3.14159265 * opd / l2); b *= b;
  vec3 c = (r + g + b) / 3.0;
  // perceptual weighting: soap film reads brighter in green and blue than a flat average
  return c * vec3(1.0, 0.96, 1.08);
}
float cosRefracted(float cosI) {
  float s = sqrt(max(0.0, 1.0 - cosI * cosI)) / 1.33;
  return sqrt(max(0.0, 1.0 - s * s));
}`;
	const NOISE = `
vec3 hash3(vec3 p) {
  p = vec3(dot(p, vec3(127.1, 311.7, 74.7)), dot(p, vec3(269.5, 183.3, 246.1)), dot(p, vec3(113.5, 271.9, 124.6)));
  return -1.0 + 2.0 * fract(sin(p) * 43758.5453123);
}
float noise3(vec3 p) {
  vec3 i = floor(p), f = fract(p);
  vec3 u = f * f * (3.0 - 2.0 * f);
  return mix(mix(mix(dot(hash3(i + vec3(0,0,0)), f - vec3(0,0,0)), dot(hash3(i + vec3(1,0,0)), f - vec3(1,0,0)), u.x),
                 mix(dot(hash3(i + vec3(0,1,0)), f - vec3(0,1,0)), dot(hash3(i + vec3(1,1,0)), f - vec3(1,1,0)), u.x), u.y),
             mix(mix(dot(hash3(i + vec3(0,0,1)), f - vec3(0,0,1)), dot(hash3(i + vec3(1,0,1)), f - vec3(1,0,1)), u.x),
                 mix(dot(hash3(i + vec3(0,1,1)), f - vec3(0,1,1)), dot(hash3(i + vec3(1,1,1)), f - vec3(1,1,1)), u.x), u.y), u.z);
}
float fbm(vec3 p) {
  float a = 0.5, s = 0.0;
  for (int i = 0; i < 4; i++) { s += a * noise3(p); p = p * 2.03 + vec3(1.7, 9.2, 3.1); a *= 0.5; }
  return s;
}
float hash1(vec2 p) { return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453); }`;
	const SDF = `
float sdRound(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}`;
	/**
	* Wave lens, after Ultra Reactivate's distortion-waves renderer (its waveLens + waveBlurMask + waveBand):
	* a wavefront at signed distance d from each pixel lifts a Gaussian lens around the front, narrower
	* ahead (d > 0) than behind, pushing samples along the front's normal. The push also drives a
	* 5-tap spectral split (chromatic aberration) and a progressive blur that reaches further ahead
	* than behind. A soft colour band leads the front (smootherstep) and trails off behind it.
	*
	* A wave is vec4(x, y, age 0..1, reach px). Energy falls as (1 − age)², the radius eases out.
	* WAVE_PARAMS are tuned per surface by the caller through `uWaveAmp` (px of push at full energy).
	*/
	const WAVE = `
struct WaveHit { vec2 disp; float band; float blur; };
WaveHit waveAt(vec2 p, vec4 wv, float amp, float width) {
  WaveHit h; h.disp = vec2(0.0); h.band = 0.0; h.blur = 0.0;
  float k = clamp(wv.z, 0.0, 1.0);
  float rad = mix(2.0, wv.w, 1.0 - pow(1.0 - k, 2.2));
  vec2 raw = p - wv.xy; float r = max(length(raw), 1e-3);
  vec2 n = raw / r;
  // the front isn't a perfect circle: two slow lobes, like the original's bend
  float a = atan(raw.y, raw.x);
  float bend = 0.62 * sin(2.0 * a + k * 6.0) + 0.38 * sin(3.0 * a - k * 9.0 + 1.7);
  float d = r - rad + bend * width * 0.18 * (1.0 - k);
  float energy = (1.0 - k) * (1.0 - k);
  float u = d / (width * (d > 0.0 ? 0.85 : 1.5));
  float lift = exp(-u * u);
  h.disp = n * energy * amp * lift;
  float bu = d / (d > 0.0 ? width * 1.6 : width * 0.9);
  h.blur = energy * exp(-bu * bu);
  if (d >= 0.0) { float q = clamp(1.0 - d / (width * 1.1), 0.0, 1.0); h.band = q * q * q * (q * (q * 6.0 - 15.0) + 10.0); }
  else h.band = pow(1.0 - clamp(-d / (width * 2.4), 0.0, 1.0), 2.3);
  h.band *= energy;
  return h;
}
const vec3 SPECTRUM[5] = vec3[5](vec3(0.571, 0.0, 0.0), vec3(0.343, 0.206, 0.0), vec3(0.086, 0.588, 0.086), vec3(0.0, 0.206, 0.343), vec3(0.0, 0.0, 0.571));`;
	/**
	* Border beam and voice glow, after Ultra Reactivate's border-beam and voice-glow (CSS there,
	* ported into the glass shader here so they share the glass's geometry and run anywhere a shader runs).
	* `perimeterAt` is the distortion-waves renderer's arc-length parameterisation of a rounded rect.
	*/
	const BEAM = `
vec3 sheen(float u) { // the brand film, cyclic: Sky → Lilac → Blush → Mint → Sky
  const vec3 c0 = vec3(0.486, 0.784, 1.0), c1 = vec3(0.722, 0.651, 1.0), c2 = vec3(1.0, 0.62, 0.847), c3 = vec3(0.541, 0.949, 0.816);
  float x = fract(u) * 4.0; int i = int(x); float m = fract(x); m = m * m * (3.0 - 2.0 * m);
  vec3 a = i == 0 ? c0 : i == 1 ? c1 : i == 2 ? c2 : c3;
  vec3 b = i == 0 ? c1 : i == 1 ? c2 : i == 2 ? c3 : c0;
  return mix(a, b, m);
}
// arc length along the rim of a rounded rect (clockwise from top centre) and the total perimeter
vec2 perimeterAt(vec2 p, vec2 halfSize, float r) {
  float ex = max(halfSize.x - r, 0.0), ey = max(halfSize.y - r, 0.0);
  float arc = 0.5 * 3.14159265 * r, P = 4.0 * (ex + ey + arc);
  float ax = abs(p.x), ay = abs(p.y);
  if (ax > ex && ay > ey) {
    vec2 v = p - vec2(sign(p.x) * ex, sign(p.y) * ey);
    float a, base;
    if (p.x >= 0.0 && p.y < 0.0) { a = atan(v.x, -v.y); base = ex; }
    else if (p.x >= 0.0) { a = atan(v.y, v.x); base = ex + arc + 2.0 * ey; }
    else if (p.y >= 0.0) { a = atan(-v.x, v.y); base = 3.0 * ex + 2.0 * arc + 2.0 * ey; }
    else { a = atan(-v.y, -v.x); base = 3.0 * ex + 3.0 * arc + 4.0 * ey; }
    return vec2(base + r * clamp(a, 0.0, 1.5707963), P);
  }
  bool sideHit = ax > ex ? true : (ay > ey ? false : (halfSize.x - ax) < (halfSize.y - ay));
  if (sideHit) return vec2(p.x >= 0.0 ? ex + arc + (p.y + ey) : 3.0 * ex + 3.0 * arc + 2.0 * ey + (ey - p.y), P);
  return vec2(p.y < 0.0 ? (p.x >= 0.0 ? p.x : P + p.x) : ex + 2.0 * arc + 2.0 * ey + (ex - p.x), P);
}
// voice lobes: (x offset, width, height, band) for a ~350 px reference width, fanning from the bottom centre
const vec4 LOBES[7] = vec4[7](vec4(0.0, 74.0, 46.0, 0.0), vec4(-36.0, 54.0, 40.0, 1.0), vec4(36.0, 54.0, 40.0, 1.0),
  vec4(-72.0, 48.0, 32.0, 2.0), vec4(72.0, 48.0, 32.0, 2.0), vec4(-108.0, 42.0, 26.0, 1.0), vec4(108.0, 42.0, 26.0, 1.0));
`;
	const COMPOSITE = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uDye; uniform vec3 uGround; uniform vec2 uView; uniform float uTime, uMute;
uniform vec4 uRip[8]; uniform int uRipN;
${FILM}
${NOISE}
${WAVE}
void main() {
  vec2 px = vec2(vUv.x, 1.0 - vUv.y) * uView;
  // tap ripples: the distortion-wave lens pushes the liquid outward, splits it spectrally at the front
  vec2 disp = vec2(0.0); float band = 0.0;
  for (int i = 0; i < 8; i++) {
    if (i >= uRipN) break;
    WaveHit h = waveAt(px, uRip[i], 14.0, 34.0);
    disp += h.disp; band = max(band, h.band);
  }
  vec2 uv = (px - disp) / uView; uv.y = 1.0 - uv.y;
  vec2 sp = disp / uView * 0.45; sp.y = -sp.y;
  vec3 dye = max(vec3(texture(uDye, uv + sp).r, texture(uDye, uv).g, texture(uDye, uv - sp).b), 0.0);
  vec3 lit = 1.0 - exp(-dye * 0.9);
  float l = dot(lit, vec3(0.2126, 0.7152, 0.0722));
  lit = max(mix(vec3(l), lit, 0.85), 0.0);
  vec3 col = uGround + lit * uMute;
  col += thinFilm(300.0 + 120.0 * band, 0.9) * band * 0.05;
  // ±0.5/255 noise only to break 8-bit banding on the dark ground
  col += (hash1(px + fract(uTime)) - 0.5) / 255.0;
  o = vec4(col, 1.0);
}`;
	const BLUR = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uTex; uniform vec2 uStep; uniform float uSigma;
void main() {
  float r = ceil(uSigma * 2.5);
  vec4 s = vec4(0.0); float wsum = 0.0;
  for (float i = -12.0; i <= 12.0; i += 1.0) {
    if (abs(i) > r) continue;
    float w = exp(-i * i / (2.0 * uSigma * uSigma));
    s += texture(uTex, vUv + uStep * i) * w; wsum += w;
  }
  o = s / wsum;
}`;
	const BLIT = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uTex;
void main() { o = texture(uTex, vUv); }`;
	const GLASS = `#version 300 es
precision highp float;
in vec2 vPx; out vec4 o;
uniform vec2 uView; uniform vec4 uRect; uniform float uR, uTier, uFilm, uAlpha, uShadow, uTime, uDpr;
uniform vec2 uLight; uniform vec3 uTint;
uniform sampler2D uBg, uB1, uB2, uB3;
uniform vec4 uWave[4]; uniform int uWaveN;
uniform vec4 uBeam; uniform vec4 uVoice; uniform float uVoiceAmt, uMaterial;
${SDF}
${FILM}
${WAVE}
${BEAM}
float sq(float x) { return x * x; }
// Frosted: the glass reads the two deepest blur levels (≈20 and ≈44 CSS px sigma), never the sharp backdrop.
// thin 0 → 40% toward the deepest, regular 1 → 75%, thick 2 → all of it; a wave's blur pushes deeper still.
vec3 bg(vec2 px, float tier) {
  vec2 uv = px / uView; uv.y = 1.0 - uv.y;
  vec3 a = texture(uB2, uv).rgb, b = texture(uB3, uv).rgb;
  return mix(a, b, clamp(0.4 + tier * 0.3, 0.0, 1.0));
}
const vec3 LUMA = vec3(0.2126, 0.7152, 0.0722);
// @source glass-frost
// Coloured frost: what's behind keeps its hue but loses its detail and most of its contrast. The light is
// pulled toward a milky floor (so a dark ground still reads as a pane, not a hole), and the colour that is
// left is lifted (×2.7) so the pane carries the tint of what's behind it.
const vec3 FROST = vec3(0.115, 0.85, 2.7); // milk floor, how much of the original light survives, colour lift
vec3 frost(vec3 b, float tier) {
  float L = dot(b, LUMA);
  vec3 ch = b - L;
  float floorL = FROST.x + 0.012 * tier;             // thicker glass is a touch milkier
  return vec3(floorL + L * FROST.y) + ch * FROST.z;
}
void main() {
  vec2 half_ = uRect.zw * 0.5, c = uRect.xy + half_, p = vPx - c;
  float r = min(uR, min(half_.x, half_.y));
  float d = sdRound(p, half_, r);
  float aa = 0.75 / uDpr;
  // soft drop shadow, grows with tier
  float sBlur = mix(24.0, 60.0, uTier * 0.5), sOff = mix(10.0, 24.0, uTier * 0.5);
  float ds = sdRound(p - vec2(0.0, sOff * 0.5), half_ - 2.0, r);
  float shadow = (1.0 - smoothstep(-sBlur * 0.3, sBlur, ds)) * mix(0.22, 0.38, uTier * 0.5) * uShadow; // soft: frost floats, it isn't a slab
  float inside = 1.0 - smoothstep(-aa, aa, d);

  // ── border beam: a head running round the rim with a smootherstep tail; stroke, inner glow, bloom
  vec3 fx = vec3(0.0); float fxA = 0.0; vec3 bloomC = vec3(0.0); float bloomA = 0.0;
  if (uBeam.z > 0.001) {
    vec2 per = perimeterAt(p, half_, r);
    float s = per.x / per.y;
    float behind = fract(uBeam.x - s);
    float ahead = fract(s - uBeam.x);
    float along = behind < uBeam.y ? pow(1.0 - behind / uBeam.y, 2.2) : 0.0;
    along = max(along, 1.0 - smoothstep(0.0, 0.012, ahead)); // a crisp head
    float stroke = exp(-sq((d + 0.8) / 1.0));
    float inner = d < 0.0 ? exp(d / 10.0) : 0.0;
    float bloom = d > 0.0 ? exp(-d / 8.0) : 0.0;
    vec3 bc = sheen(s * 1.5 + uBeam.w);
    float k = along * uBeam.z;
    fx += bc * (stroke * 1.3 + inner * 0.32) * k; fxA = max(fxA, (stroke * 0.9 + inner * 0.25) * k);
    bloomC += bc * bloom * 0.55 * k; bloomA += bloom * 0.4 * k;
  }
  // ── voice glow: seven soft lobes fanning up from the bottom centre, each following a band
  if (uVoiceAmt > 0.001) {
    float sc = uRect.z / 350.0;
    vec2 base = vec2(0.0, half_.y);
    float levels[3] = float[3](uVoice.x, uVoice.y, uVoice.z);
    float spread = 1.0 + 0.35 * uVoice.y;
    vec3 vc = vec3(0.0); float va = 0.0;
    for (int i = 0; i < 7; i++) {
      vec4 L = LOBES[i];
      float lv = levels[int(L.w)];
      float x = L.x * sc * spread + sin(uVoice.w + float(i)) * 6.0 * sc;
      vec2 e = (p - base - vec2(x, 0.0)) / (vec2(L.y, L.z) * sc * (0.55 + 0.9 * lv));
      float g = exp(-dot(e, e) * 1.4) * (0.25 + lv);
      vc += sheen(float(i) * 0.13 + uVoice.w * 0.05) * g; va += g;
    }
    float rimGlow = exp(-sq((d + 0.8) / 1.2)) * (0.3 + uVoice.x + uVoice.y) * 0.5;
    fx += (vc * 0.6 + sheen(atan(p.y, p.x) / 6.2831853 + uVoice.w * 0.04) * rimGlow) * uVoiceAmt * step(d, 0.5);
    fxA = max(fxA, clamp((va * 0.45 + rimGlow) * uVoiceAmt, 0.0, 1.0) * step(d, 0.5));
    float vb = d > 0.0 ? exp(-d / 10.0) * smoothstep(-half_.y, half_.y, p.y) : 0.0;
    bloomC += vc * vb * 0.4 * uVoiceAmt; bloomA += min(1.0, va) * vb * 0.3 * uVoiceAmt;
  }

  if (inside <= 0.0) {
    float sa = shadow * uMaterial;
    o = vec4(bloomC, clamp(bloomA, 0.0, 1.0)) * uAlpha + vec4(0.0, 0.0, 0.0, sa * uAlpha) * (1.0 - clamp(bloomA, 0.0, 1.0));
    return;
  }
  if (uMaterial < 0.5) {
    // effects only, over live content
    float a0 = inside * uAlpha;
    o = vec4(fx * a0, clamp(fxA, 0.0, 1.0) * a0);
    return;
  }

  // normal from the SDF gradient
  vec2 e = vec2(0.5, 0.0);
  vec2 n = normalize(vec2(sdRound(p + e.xy, half_, r) - sdRound(p - e.xy, half_, r), sdRound(p + e.yx, half_, r) - sdRound(p - e.yx, half_, r)) + 1e-5);
  // frosted glass doesn't lens: the rim only bends the light a little, so the edge stays soft
  float bezel = clamp(min(half_.x, half_.y) * 0.5, 6.0, 22.0);
  float t = clamp(1.0 + d / bezel, 0.0, 1.0);
  vec2 off = -n * t * t * bezel * 0.35;
  // a press sends a distortion wave through the glass: lens push, extra blur, spectral split
  vec2 disp = vec2(0.0); float wBand = 0.0, wBlur = 0.0;
  for (int i = 0; i < 4; i++) {
    if (i >= uWaveN) break;
    WaveHit h = waveAt(vPx, uWave[i], 9.0, 26.0);
    disp += h.disp; wBand = max(wBand, h.band); wBlur = max(wBlur, h.blur);
  }
  float tierB = min(2.0, uTier + wBlur * 1.2);
  // the wave still splits the light spectrally as it passes (only the wave: resting frost has no dispersion)
  vec3 col;
  col.r = bg(vPx + off - disp * 0.85, tierB).r;
  col.g = bg(vPx + off - disp * 1.00, tierB).g;
  col.b = bg(vPx + off - disp * 1.18, tierB).b;
  col = frost(col, uTier);
  // the Space colour, lightly: the colour should mostly come from what's behind
  col = mix(col, uTint * 0.55 + 0.1, 0.07 * step(0.001, dot(uTint, uTint)));
  // the wave's leading band catches a little film colour
  col += (thinFilm(320.0 + 140.0 * wBand, 0.85) * 0.12 + 0.03) * wBand;
  // pointer light: a broad, soft brightening in the frost (no specular band, no hotspot)
  vec2 lp = uLight - c;
  float glow = exp(-dot(p - lp, p - lp) / (2.0 * sq(max(uRect.z, uRect.w) * 0.45)));
  col += glow * 0.03;
  // the edge: a faint, soft light just inside, a little brighter facing the pointer. No hairline.
  float rim = exp(d / 1.6) * step(d, 0.0);
  float facing = max(0.0, dot(n, normalize(lp - p + 1e-4)));
  col += rim * (0.035 + 0.05 * facing * facing);
  // ±0.5/255 grain against 8-bit banding in the smooth frost
  col += (fract(sin(dot(vPx, vec2(12.9898, 78.233))) * 43758.5453) - 0.5) / 255.0;
  // film rim: the brand sheen as a soft 1.5 px ring that drifts slowly round the edge, only when touched.
  // It matches the CSS .wb-film rim (a conic Sky → Lilac → Blush → Mint ring at 80%), not interference
  // colours: high-order film bands read as anodised metal on small controls.
  if (uFilm > 0.001) {
    vec2 per = perimeterAt(p, half_, r);
    float s = per.x / per.y;
    vec3 rc = sheen(s + 0.58 - uTime * 0.06);
    float ring = 1.0 - smoothstep(1.0, 2.0, -d);           // 1.5 px band, antialiased inward
    float glowIn = exp(d / 5.0) * 0.12;                        // a breath of colour just inside
    col = mix(col, rc, ring * 0.8 * uFilm);
    col += rc * glowIn * uFilm;
  }
  col += fx;
  float a = inside * uAlpha;
  o = vec4(col * a, a) + vec4(0.0, 0.0, 0.0, shadow * uAlpha) * (1.0 - a);
}`;
	const BUBBLE = `#version 300 es
precision highp float;
in vec2 vPx; out vec4 o;
uniform vec2 uView, uC, uLight; uniform float uR, uRing[24], uBody, uAlpha, uTime, uSwirl, uSeed, uDpr, uHasW, uOverlay;
uniform sampler2D uBg, uW; uniform mat2 uWInv; uniform vec2 uWOff; uniform float uWScale;
uniform sampler2D uB1, uB2;
uniform vec4 uWave[4]; uniform int uWaveN;
uniform vec2 uFace; // yaw, pitch (radians): the bubble turns its face toward the cursor
${FILM}
// rotate a direction by yaw (about y) then pitch (about x), and back. Screen space: x right, y down, z toward the viewer.
vec3 faceRot(vec3 v, vec2 a) {
  float cy = cos(a.x), sy = sin(a.x); v = vec3(v.x * cy + v.z * sy, v.y, -v.x * sy + v.z * cy);
  float cp = cos(a.y), sp = sin(a.y); return vec3(v.x, v.y * cp + v.z * sp, -v.y * sp + v.z * cp);
}
vec3 faceRotInv(vec3 v, vec2 a) {
  float cp = cos(a.y), sp = sin(a.y); v = vec3(v.x, v.y * cp - v.z * sp, v.y * sp + v.z * cp);
  float cy = cos(a.x), sy = sin(a.x); return vec3(v.x * cy - v.z * sy, v.y, v.x * sy + v.z * cy);
}
${NOISE}
float ringAt(float ang) {
  float f = (ang / 6.2831853 + 0.5) * 24.0;
  int i1 = int(floor(f)); float t = fract(f);
  int i0 = (i1 + 23) % 24, i2 = (i1 + 1) % 24, i3 = (i1 + 2) % 24;
  i1 = i1 % 24;
  float p0 = uRing[i0], p1 = uRing[i1], p2 = uRing[i2], p3 = uRing[i3];
  // Catmull-Rom through the ring samples
  return 0.5 * ((2.0 * p1) + (-p0 + p2) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t);
}
${WAVE}
// the backdrop at a blur level 0..1 (sharp → the blur pyramid), as the original's stacked copies
vec3 bgLevel(vec2 px, float tb) {
  vec2 uv = px / uView; uv.y = 1.0 - uv.y;
  vec3 c = textureLod(uBg, uv, 0.0).rgb;
  if (tb > 0.0) c = mix(c, textureLod(uB1, uv, 0.0).rgb, smoothstep(0.0, 0.5, tb));
  if (tb > 0.5) c = mix(c, textureLod(uB2, uv, 0.0).rgb, smoothstep(0.5, 1.0, tb));
  return c;
}
// 5-tap spectral split along the push, weighted so the taps sum back to white
vec3 spectral(vec2 px, vec2 split, float tb) {
  if (dot(split, split) < 0.0625) return bgLevel(px, tb);
  vec3 o = vec3(0.0);
  for (int i = 0; i < 5; i++) o += bgLevel(px + split * (float(i) / 2.0 - 1.0), tb) * SPECTRUM[i];
  return o;
}
vec3 bgAt(vec2 px) { return bgLevel(px, 0.0); }
void main() {
  vec2 p = vPx - uC;
  float ang = atan(p.y, p.x);
  float R = uR + ringAt(ang);
  float len = length(p);
  float d = len - R;
  float aa = 0.9 / uDpr;
  float inside = 1.0 - smoothstep(-aa, aa, d);
  if (inside <= 0.0) { o = vec4(0.0); return; }
  vec2 q = p / R; float rho = min(length(q), 1.0);
  float nz = sqrt(max(0.0, 1.0 - rho * rho));
  vec3 n = normalize(vec3(q, nz));
  float cosI = nz, cosT = cosRefracted(cosI);

  // film thickness: first-order only (≈200–560 nm), so colours stay soft and pastel, never banded.
  // Swirled slowly by noise and drained by gravity: a little thicker (warmer) toward the bottom.
  float t = uTime * 0.025 + uSeed;
  vec3 sp = vec3(n.xy * 1.1, t);
  float ca = cos(uSwirl + t * 0.5), sa = sin(uSwirl + t * 0.5);
  sp.xy = mat2(ca, -sa, sa, ca) * sp.xy;
  float warp = fbm(sp + vec3(fbm(sp * 1.3 + 4.0), fbm(sp * 1.3 + 9.0), 0.0) * 0.8);
  float drain = 0.5 + 0.5 * q.y;
  float thick = 250.0 + 150.0 * drain + 170.0 * warp;
  vec3 film = thinFilm(thick, cosT);
  film = mix(vec3(dot(film, vec3(0.3333))), film, 0.72) * 0.9 + 0.08; // calm the saturation
  float fres = pow(1.0 - cosI, 3.0);

  // body: a clear lens for little bubbles, smooth dark glass for the logo
  // pokes send a distortion wave through the body: it lenses, splits and blurs what the bubble refracts
  WaveHit wv; wv.disp = vec2(0.0); wv.band = 0.0; wv.blur = 0.0;
  for (int i = 0; i < 4; i++) {
    if (i >= uWaveN) break;
    WaveHit h = waveAt(vPx, uWave[i], R * 0.12, R * 0.22);
    wv.disp += h.disp; wv.band = max(wv.band, h.band); wv.blur = max(wv.blur, h.blur);
  }
  vec2 lensOff = -q * (1.0 - nz) * R * 0.38;
  vec3 refr = spectral(vPx + lensOff - wv.disp, wv.disp * 0.5, wv.blur * 0.8);
  vec3 glassTop = vec3(0.150, 0.165, 0.215), glassBot = vec3(0.035, 0.039, 0.055);
  // the lit side of the glass faces the light (the pointer)
  vec2 ld = (uLight - uC) / (uR * 1.2);
  vec2 ldir = length(ld) > 0.05 ? normalize(ld) : vec2(-0.3, -0.95);
  float lit = dot(q, ldir) * min(1.0, 0.45 + length(ld) * 0.4);
  vec3 bodyCol = mix(glassBot, glassTop, smoothstep(-0.9, 0.9, lit));
  bodyCol += refr * 0.12;
  vec3 col = mix(refr, bodyCol, uBody);
  // film shows only where the surface turns away (Fresnel), never as a wash across the face
  col += film * fres * mix(0.75, 0.55, uBody);
  // light carried through the glass gathers at the lower rim, tinted by the film
  col += film * smoothstep(0.6, 1.0, rho) * max(0.0, q.y) * 0.22 * uBody;
  // a hairline rim and a faint crescent reflection along the top
  float rimLine = exp(-(d + 1.1) * (d + 1.1));
  col = mix(col, film * 1.1 + 0.1, rimLine * 0.55);
  float crescent = smoothstep(0.78, 0.97, rho) * smoothstep(0.1, 0.8, dot(q, ldir));
  col += crescent * 0.16;
  float rimBand = rimLine;
  // the front of the wave catches the light: a quiet film-coloured band
  col += (film * 0.5 + 0.06) * wv.band * 0.35;

  // the W: a glass tube in the brand sheen (Sky → Lilac → Blush → Mint), lit softly
  float wA = 0.0;
  if (uHasW > 0.5) {
    // the W lives on an inner sphere (radius 0.9 R). Un-rotate the visible point to find where it sits on the face.
    const float KR = 0.9;
    vec2 qi = (q - wv.disp * 0.55 / R) / KR; float ri = dot(qi, qi);
    vec3 si = vec3(qi, sqrt(max(0.0, 1.0 - ri)));
    vec3 s0 = faceRotInv(si, uFace);
    float facing = smoothstep(0.05, 0.25, s0.z) * step(ri, 1.0);
    vec2 v = (s0.xy * KR * R) / uWScale + vec2(60.0);
    vec2 vw = uWInv * (v - uWOff) + vec2(60.0, 65.0);
    vec2 wuv = vec2(vw.x / 120.0, 1.0 - vw.y / 120.0);
    if (wuv.x > 0.0 && wuv.x < 1.0 && wuv.y > 0.0 && wuv.y < 1.0) {
      // distance to the W's centre line, in viewBox units; the tube radius is 5 (stroke width 10)
      const float TUBE = 5.0;
      float dist = texture(uW, wuv).a * 16.0;
      float e = 2.0 / 384.0;
      vec2 grad = vec2(texture(uW, wuv + vec2(e, 0.0)).a - texture(uW, wuv - vec2(e, 0.0)).a,
                       -(texture(uW, wuv + vec2(0.0, e)).a - texture(uW, wuv - vec2(0.0, e)).a));
      vec2 gdir = length(grad) > 1e-6 ? normalize(grad) : vec2(0.0);
      float px = 1.0 / uWScale; // one screen pixel in viewBox units
      wA = (1.0 - smoothstep(TUBE - px * 0.6, TUBE + px * 0.6, dist)) * facing;
      float rr = clamp(dist / TUBE, 0.0, 1.0);
      float h = sqrt(max(0.0, 1.0 - rr * rr)); // round tube profile
      vec3 wn = faceRot(normalize(vec3(gdir * rr, h + 0.001)), uFace);
      float g = clamp((vw.x - 28.0) / 64.0 * 0.75 + (vw.y - 46.0) / 40.0 * 0.25 + 0.06 * sin(uTime * 0.25 + uSwirl * 0.4), 0.0, 1.0);
      vec3 sky = vec3(0.486, 0.784, 1.0), lilac = vec3(0.722, 0.651, 1.0), blush = vec3(1.0, 0.62, 0.847), mint = vec3(0.541, 0.949, 0.816);
      vec3 wc = g < 0.36 ? mix(sky, lilac, g / 0.36) : g < 0.68 ? mix(lilac, blush, (g - 0.36) / 0.32) : mix(blush, mint, (g - 0.68) / 0.32);
      // soft key light from the top left, a little darker on the shadow side of the tube
      float shade = 0.74 + 0.30 * max(0.0, dot(wn, normalize(vec3(-0.35, -0.6, 0.72))));
      wc *= shade;
      vec3 L = normalize(vec3((uLight - uC) / (uR * 1.3), 1.2));
      float wspec = pow(max(dot(wn, normalize(L + vec3(0.0, 0.0, 1.0))), 0.0), 36.0);
      wc += wspec * 0.3;
      col = mix(col, wc, wA);
    }
  }

  // specular: one soft reflection of the light (the pointer), and a small sharp one beside it
  vec3 L = normalize(vec3((uLight - uC) / (uR * 0.85), 0.75));
  vec3 H = normalize(L + vec3(0.0, 0.0, 1.0));
  float sp1 = pow(max(dot(n, H), 0.0), 34.0) * 0.42;
  vec3 L2 = normalize(vec3(L.xy * 1.12 + vec2(0.14, -0.05), L.z));
  float sp2 = pow(max(dot(n, normalize(L2 + vec3(0.0, 0.0, 1.0))), 0.0), 900.0) * 0.55;
  col += (sp1 + sp2) * (1.0 - wA * 0.8);
  col = min(col, vec3(1.0));

  float a = inside * uAlpha;
  if (uOverlay > 0.5) {
    // over live DOM there is no backdrop copy: draw only what the film adds (reflection), premultiplied
    vec3 add = film * fres * 0.9 + sp1 + sp2 + crescent * 0.16;
    add = mix(add, film * 1.1 + 0.1, rimBand * 0.55);
    float al = clamp(0.03 + fres * 0.7 + sp1 + sp2 + rimBand * 0.5, 0.0, 1.0) * a;
    o = vec4(add * a, al);
    return;
  }
  // the bubble carries its own refracted copy of the backdrop, so it can cover fully
  o = vec4(col * a, a);
}`;
	const POP = `#version 300 es
precision highp float;
in vec2 vPx; out vec4 o;
uniform vec2 uC; uniform float uR, uK, uAng, uSeed, uDpr;
${FILM}
${NOISE}
void main() {
  vec2 p = vPx - uC; float d = length(p); float ang = atan(p.y, p.x);
  float k = uK;
  vec4 acc = vec4(0.0);
  // the tear: film remaining away from the tear point, snapping back to the far side
  float open = k * 3.4;
  float da = abs(mod(ang - uAng + 3.14159, 6.28318) - 3.14159);
  float keep = smoothstep(open - 0.25, open, da);
  float rr = uR * (1.0 + 0.12 * k);
  float w = max(0.6, 2.6 * (1.0 - k));
  float ring = exp(-(d - rr) * (d - rr) / (w * w)) * keep * (1.0 - smoothstep(0.55, 1.0, k));
  vec3 fc = thinFilm(300.0 + 300.0 * sin(ang * 3.0 + uSeed), 0.8) * 1.4 + 0.1;
  acc += vec4(fc, 1.0) * ring;
  // droplets thrown out along the tear
  for (int i = 0; i < 9; i++) {
    float fi = float(i);
    float a = uAng + (hash1(vec2(fi, uSeed)) - 0.5) * 3.8;
    float sp = 0.6 + hash1(vec2(uSeed, fi)) * 0.9;
    float dist = uR * (1.0 + k * 1.6 * sp);
    vec2 c = vec2(cos(a), sin(a)) * dist + vec2(0.0, k * k * uR * 0.5);
    float s = (1.2 + 1.8 * hash1(vec2(fi * 3.1, uSeed))) * (1.0 - k);
    float dd = length(p - c);
    float m = 1.0 - smoothstep(s - 0.6 / uDpr, s + 0.6 / uDpr, dd);
    acc += vec4(vec3(0.95), 1.0) * m * (1.0 - k);
  }
  acc.a = clamp(acc.a, 0.0, 1.0);
  o = vec4(acc.rgb * acc.a, acc.a);
}`;
	/**
	* The logo W as a distance field: each texel stores the distance (viewBox units, 0–16 → 0–255) to the
	* W's centre line, measured against its quadratic Béziers (each sampled as a 48-segment polyline). The shader turns it into a round tube
	* with an analytic profile and antialiases the edge per pixel, so it stays smooth at any size.
	*/
	function wTexture(gl) {
		const S = 384, MAXD = 16;
		const quads = [
			[
				32,
				48,
				38,
				82,
				46,
				82
			],
			[
				46,
				82,
				53,
				82,
				60,
				60
			],
			[
				60,
				60,
				67,
				82,
				74,
				82
			],
			[
				74,
				82,
				82,
				82,
				88,
				48
			]
		];
		const pts = [];
		for (const [x0, y0, cx, cy, x1, y1] of quads) for (let i = 0; i <= 48; i++) {
			const t = i / 48, u = 1 - t;
			pts.push(u * u * x0 + 2 * u * t * cx + t * t * x1, u * u * y0 + 2 * u * t * cy + t * t * y1);
		}
		const data = new Uint8Array(S * S * 4);
		for (let j = 0; j < S; j++) {
			const y = (1 - (j + .5) / S) * 120;
			for (let i = 0; i < S; i++) {
				const x = (i + .5) / S * 120;
				let best = 1e9;
				for (let k = 0; k + 3 < pts.length; k += 2) {
					const ax = pts[k], ay = pts[k + 1], bx = pts[k + 2], by = pts[k + 3];
					const dx = bx - ax, dy = by - ay, l2 = dx * dx + dy * dy || 1;
					const t = Math.max(0, Math.min(1, ((x - ax) * dx + (y - ay) * dy) / l2));
					const ex = x - ax - dx * t, ey = y - ay - dy * t, d2 = ex * ex + ey * ey;
					if (d2 < best) best = d2;
				}
				data[(j * S + i) * 4 + 3] = Math.min(255, Math.round(Math.sqrt(best) / MAXD * 255));
			}
		}
		const tex = gl.createTexture();
		gl.bindTexture(gl.TEXTURE_2D, tex);
		gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, S, S, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
		gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
		return tex;
	}
	var Scene = class {
		canvas;
		gl;
		q;
		fluid;
		P;
		bg;
		b1;
		b1t;
		b2;
		b2t;
		b3;
		b3t;
		wTex;
		byte = formats;
		w = 0;
		h = 0;
		dpr = 1;
		fluidAspect = 0;
		time = 0;
		glass = [];
		bubbles = [];
		pops = [];
		ripples = [];
		light = {
			x: 0,
			y: 0
		};
		lightTarget = {
			x: 0,
			y: 0
		};
		pointer = {
			x: 0,
			y: 0,
			px: 0,
			py: 0,
			has: false
		};
		palette;
		paletteTarget;
		ground;
		emitters;
		opts;
		constructor(canvas, o = {}) {
			this.canvas = canvas;
			const gl = canvas.getContext("webgl2", {
				alpha: !!o.overlay,
				antialias: false,
				premultipliedAlpha: true,
				powerPreference: "low-power"
			});
			if (!gl) throw new Error("WebGL2 unavailable");
			this.gl = gl;
			this.q = quad(gl);
			const draw = () => gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
			this.fluid = new Fluid(gl, draw, o.fluid);
			this.P = {
				comp: program(gl, VERT_FS, COMPOSITE),
				blur: program(gl, VERT_FS, BLUR),
				blit: program(gl, VERT_FS, BLIT),
				glass: program(gl, VERT_QUAD, GLASS),
				bubble: program(gl, VERT_QUAD, BUBBLE),
				pop: program(gl, VERT_QUAD, POP)
			};
			this.wTex = wTexture(gl);
			const pal = (o.palette ?? [
				"#2F5FC4",
				"#A84886",
				"#237E69"
			]).map(hex);
			this.palette = pal.map((c) => [...c]);
			this.paletteTarget = pal;
			this.ground = hex(o.ground ?? "#08090D");
			this.opts = {
				ambient: o.ambient ?? 1,
				pointerForce: o.pointerForce ?? 1,
				overlay: !!o.overlay,
				intensity: o.intensity ?? .3
			};
			this.emitters = [
				0,
				1,
				2,
				3,
				4
			].map((i) => ({
				phase: i * 1.7,
				speed: .018 + i * .005,
				ax: .32 + .06 * i,
				ay: .28 + .04 * (4 - i),
				fx: 1 + i % 2,
				fy: 2 - i % 2 * 1,
				color: i % 3
			}));
		}
		setPalette(colors) {
			this.paletteTarget = colors.map(hex);
		}
		resize(w, h, dpr = Math.min(devicePixelRatio, 2)) {
			if (w === this.w && h === this.h && dpr === this.dpr) return;
			this.w = w;
			this.h = h;
			this.dpr = dpr;
			this.canvas.width = Math.max(1, Math.round(w * dpr));
			this.canvas.height = Math.max(1, Math.round(h * dpr));
			const gl = this.gl, f = this.byte(gl).float;
			[
				this.bg,
				this.b1,
				this.b1t,
				this.b2,
				this.b2t,
				this.b3,
				this.b3t
			].forEach((t) => freeTarget(gl, t));
			const hw = Math.max(1, Math.round(w / 2)), hh = Math.max(1, Math.round(h / 2));
			this.bg = target(gl, hw, hh, f);
			this.b1 = target(gl, Math.ceil(hw / 2), Math.ceil(hh / 2), f);
			this.b1t = target(gl, this.b1.w, this.b1.h, f);
			this.b2 = target(gl, Math.ceil(hw / 4), Math.ceil(hh / 4), f);
			this.b2t = target(gl, this.b2.w, this.b2.h, f);
			this.b3 = target(gl, Math.ceil(hw / 8), Math.ceil(hh / 8), f);
			this.b3t = target(gl, this.b3.w, this.b3.h, f);
			if (this.opts.overlay) return;
			const aspect = w / Math.max(1, h);
			if (this.fluidAspect && Math.abs(Math.log(aspect / this.fluidAspect)) < .35) return;
			const fresh = !this.fluidAspect;
			this.fluidAspect = aspect;
			this.fluid.resize(w, h);
			if (!fresh) {
				for (let i = 0; i < 6; i++) {
					const c = this.palette[i % 3];
					this.fluid.splat(.15 + Math.random() * .7, .15 + Math.random() * .7, 0, 0, [
						c[0] * .9,
						c[1] * .9,
						c[2] * .9
					], .03);
				}
				return;
			}
			for (let i = 0; i < 9; i++) {
				const c = this.palette[i % 3];
				this.fluid.splat(.15 + Math.random() * .7, .15 + Math.random() * .7, (Math.random() - .5) * 80, (Math.random() - .5) * 80, [
					c[0] * .9,
					c[1] * .9,
					c[2] * .9
				], .02);
			}
		}
		/** Pointer in CSS px. Pushes the liquid along its motion and moves the light. */
		move(x, y) {
			const p = this.pointer;
			if (!p.has) {
				p.px = x;
				p.py = y;
			}
			p.x = x;
			p.y = y;
			p.has = true;
			this.lightTarget = {
				x,
				y
			};
		}
		leave() {
			this.pointer.has = false;
		}
		/** A tap: a wave ring through the backdrop and an outward push in the liquid. */
		ripple(x, y, reach = 260, strength = 1) {
			this.ripples.push({
				x,
				y,
				age: 0,
				life: 1.15,
				reach
			});
			if (this.ripples.length > 8) this.ripples.shift();
			if (this.w && !this.opts.overlay) this.fluid.ripple(x / this.w, 1 - y / this.h, 1100 * strength, .01);
		}
		/** Push dye and velocity in directly (e.g. a window dragged through the liquid). */
		stir(x, y, dx, dy, dye = 0) {
			if (!this.w || this.opts.overlay) return;
			const c = this.palette[Math.floor(Math.random() * 3)];
			this.fluid.splat(x / this.w, 1 - y / this.h, dx, -dy, dye ? [
				c[0] * dye,
				c[1] * dye,
				c[2] * dye
			] : null, .004);
		}
		pop(x, y, r, angle = -Math.PI / 2) {
			this.pops.push({
				x,
				y,
				r,
				age: 0,
				life: .36,
				angle,
				seed: Math.random() * 100
			});
			if (this.w && !this.opts.overlay) this.fluid.ripple(x / this.w, 1 - y / this.h, 500, Math.max(.002, r / this.w * .6));
		}
		frame(dt) {
			const gl = this.gl;
			if (!this.w) return;
			this.time += dt;
			this.lastDt = dt;
			this.ageWaves(dt);
			const t = this.time;
			const k = 1 - Math.exp(-dt * 5);
			this.palette.forEach((c, i) => {
				const g = this.paletteTarget[i % this.paletteTarget.length];
				for (let j = 0; j < 3; j++) c[j] += (g[j] - c[j]) * k;
			});
			const lk = 1 - Math.exp(-dt * 18);
			this.light.x += (this.lightTarget.x - this.light.x) * lk;
			this.light.y += (this.lightTarget.y - this.light.y) * lk;
			gl.bindVertexArray(this.q.vao);
			gl.disable(gl.BLEND);
			if (this.opts.overlay) {
				gl.bindFramebuffer(gl.FRAMEBUFFER, null);
				gl.viewport(0, 0, this.canvas.width, this.canvas.height);
				gl.clearColor(0, 0, 0, 0);
				gl.clear(gl.COLOR_BUFFER_BIT);
				if (this.glass.length) this.drawGlass(t);
				this.drawBubbles(t);
				return;
			}
			if (this.opts.ambient > 0) for (const e of this.emitters) {
				e.phase += dt * e.speed;
				const x = .5 + e.ax * Math.sin(e.phase * e.fx * 2.1), y = .5 + e.ay * Math.sin(e.phase * e.fy * 1.7 + 1.3);
				const dx = e.ax * Math.cos(e.phase * e.fx * 2.1) * e.fx * 2.1, dy = e.ay * Math.cos(e.phase * e.fy * 1.7 + 1.3) * e.fy * 1.7;
				const c = this.palette[e.color], a = .5 * dt * this.opts.ambient;
				this.fluid.splat(x, y, dx * 14 * this.opts.ambient, dy * 14 * this.opts.ambient, [
					c[0] * a,
					c[1] * a,
					c[2] * a
				], .012);
			}
			const p = this.pointer;
			if (p.has && this.opts.pointerForce > 0) {
				const dx = p.x - p.px, dy = p.y - p.py;
				if (dx * dx + dy * dy > .25) this.fluid.splat(p.x / this.w, 1 - p.y / this.h, dx * 4 * this.opts.pointerForce, -dy * 4 * this.opts.pointerForce, null, .0016);
				p.px = p.x;
				p.py = p.y;
			}
			this.fluid.step(dt);
			const C = this.P.comp;
			gl.useProgram(C.p);
			bindTex(gl, 0, this.fluid.dye.read.tex, C.u.uDye);
			gl.uniform3f(C.u.uGround, ...this.ground);
			gl.uniform1f(C.u.uMute, this.opts.intensity);
			gl.uniform2f(C.u.uView, this.w, this.h);
			gl.uniform1f(C.u.uTime, t);
			this.ripples = this.ripples.filter((r) => (r.age += dt) < r.life);
			const rip = /* @__PURE__ */ new Float32Array(32);
			this.ripples.forEach((r, i) => rip.set([
				r.x,
				r.y,
				r.age / r.life,
				r.reach
			], i * 4));
			gl.uniform4fv(C.u.uRip, rip);
			gl.uniform1i(C.u.uRipN, this.ripples.length);
			this.to(this.bg);
			this.draw();
			const blur = (src, tmp, out, sigma) => {
				const B = this.P.blur;
				gl.useProgram(B.p);
				gl.uniform1f(B.u.uSigma, sigma);
				bindTex(gl, 0, src.tex, B.u.uTex);
				gl.uniform2f(B.u.uStep, 1 / src.w, 0);
				this.to(tmp);
				this.draw();
				bindTex(gl, 0, tmp.tex, B.u.uTex);
				gl.uniform2f(B.u.uStep, 0, 1 / tmp.h);
				this.to(out);
				this.draw();
			};
			blur(this.bg, this.b1t, this.b1, 3.5);
			blur(this.b1, this.b2t, this.b2, 4.5);
			blur(this.b2, this.b3t, this.b3, 5);
			gl.bindFramebuffer(gl.FRAMEBUFFER, null);
			gl.viewport(0, 0, this.canvas.width, this.canvas.height);
			gl.useProgram(this.P.blit.p);
			bindTex(gl, 0, this.bg.tex, this.P.blit.u.uTex);
			this.draw();
			gl.enable(gl.BLEND);
			gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
			this.drawGlass(t);
			this.drawBubbles(t);
			gl.disable(gl.BLEND);
		}
		drawGlass(t) {
			const gl = this.gl;
			gl.enable(gl.BLEND);
			gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
			const G = this.P.glass;
			gl.useProgram(G.p);
			gl.uniform2f(G.u.uView, this.w, this.h);
			gl.uniform1f(G.u.uTime, t);
			gl.uniform1f(G.u.uDpr, this.dpr);
			gl.uniform2f(G.u.uLight, this.light.x, this.light.y);
			bindTex(gl, 0, this.bg.tex, G.u.uBg);
			bindTex(gl, 1, this.b1.tex, G.u.uB1);
			bindTex(gl, 2, this.b2.tex, G.u.uB2);
			bindTex(gl, 3, this.b3.tex, G.u.uB3);
			for (const s of this.glass) {
				const a = s.alpha ?? 1;
				if (a <= .001 || s.w < 1 || s.h < 1) continue;
				const pad = 70;
				gl.uniform4f(G.u.uQuad, s.x - pad, s.y - pad, s.w + 140, s.h + 140);
				gl.uniform4f(G.u.uRect, s.x, s.y, s.w, s.h);
				this.waveUniforms(G, s.waves);
				const bm = s.beam, vo = s.voice;
				gl.uniform4f(G.u.uBeam, bm?.head ?? 0, bm?.length ?? .25, bm?.strength ?? 0, bm?.hue ?? 0);
				gl.uniform4f(G.u.uVoice, vo?.low ?? 0, vo?.mid ?? 0, vo?.high ?? 0, vo?.phase ?? 0);
				gl.uniform1f(G.u.uVoiceAmt, vo?.amount ?? 0);
				gl.uniform1f(G.u.uMaterial, s.material ?? 1);
				gl.uniform1f(G.u.uR, s.r);
				gl.uniform1f(G.u.uTier, s.tier);
				gl.uniform1f(G.u.uFilm, s.film);
				gl.uniform1f(G.u.uAlpha, a);
				gl.uniform1f(G.u.uShadow, s.shadow ?? 1);
				gl.uniform3f(G.u.uTint, ...s.tint ?? [
					0,
					0,
					0
				]);
				this.draw();
			}
		}
		drawBubbles(t) {
			const gl = this.gl;
			gl.enable(gl.BLEND);
			gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
			const Bu = this.P.bubble;
			gl.useProgram(Bu.p);
			gl.uniform2f(Bu.u.uView, this.w, this.h);
			gl.uniform1f(Bu.u.uTime, t);
			gl.uniform1f(Bu.u.uDpr, this.dpr);
			gl.uniform2f(Bu.u.uLight, this.light.x, this.light.y);
			bindTex(gl, 0, this.bg.tex, Bu.u.uBg);
			bindTex(gl, 1, this.wTex, Bu.u.uW);
			bindTex(gl, 2, this.b1 ? this.b1.tex : this.wTex, Bu.u.uB1);
			bindTex(gl, 3, this.b2 ? this.b2.tex : this.wTex, Bu.u.uB2);
			const zero = /* @__PURE__ */ new Float32Array(24);
			for (const b of this.bubbles) {
				const a = b.alpha ?? 1;
				if (a <= .001 || b.r < .5) continue;
				const ext = b.r * 1.7 + 4;
				gl.uniform4f(Bu.u.uQuad, b.x - ext, b.y - ext, ext * 2, ext * 2);
				gl.uniform2f(Bu.u.uC, b.x, b.y);
				gl.uniform1f(Bu.u.uR, b.r);
				gl.uniform1fv(Bu.u.uRing, b.ring ?? zero);
				gl.uniform1f(Bu.u.uBody, b.body ?? 0);
				gl.uniform1f(Bu.u.uAlpha, a);
				gl.uniform1f(Bu.u.uSwirl, b.swirl ?? 0);
				gl.uniform1f(Bu.u.uSeed, b.seed ?? 0);
				gl.uniform1f(Bu.u.uHasW, b.w ? 1 : 0);
				gl.uniform2f(Bu.u.uFace, b.face?.[0] ?? 0, b.face?.[1] ?? 0);
				this.waveUniforms(Bu, b.waves);
				gl.uniform1f(Bu.u.uOverlay, this.opts.overlay ? 1 : 0);
				if (b.w) {
					gl.uniformMatrix2fv(Bu.u.uWInv, false, b.w.inv);
					gl.uniform2f(Bu.u.uWOff, ...b.w.off);
					gl.uniform1f(Bu.u.uWScale, b.w.scale);
				}
				this.draw();
			}
			const Po = this.P.pop;
			gl.useProgram(Po.p);
			gl.uniform2f(Po.u.uView, this.w, this.h);
			gl.uniform1f(Po.u.uDpr, this.dpr);
			this.pops = this.pops.filter((o) => (o.age += this.lastDt) < o.life);
			for (const o of this.pops) {
				const ext = o.r * 3 + 10;
				gl.uniform4f(Po.u.uQuad, o.x - ext, o.y - ext, ext * 2, ext * 2);
				gl.uniform2f(Po.u.uC, o.x, o.y);
				gl.uniform1f(Po.u.uR, o.r);
				gl.uniform1f(Po.u.uK, o.age / o.life);
				gl.uniform1f(Po.u.uAng, o.angle);
				gl.uniform1f(Po.u.uSeed, o.seed);
				this.draw();
			}
		}
		lastDt = .016;
		waveBuf = /* @__PURE__ */ new Float32Array(16);
		waveUniforms(P, waves) {
			const n = Math.min(4, waves?.length ?? 0);
			for (let i = 0; i < n; i++) {
				const w = waves[waves.length - n + i];
				this.waveBuf.set([
					w.x,
					w.y,
					w.age / w.life,
					w.reach
				], i * 4);
			}
			this.gl.uniform4fv(P.u.uWave, this.waveBuf);
			this.gl.uniform1i(P.u.uWaveN, n);
		}
		/** Send a distortion wave through a glass surface or a bubble, from (x, y) in CSS px. */
		wave(target, x, y, reach, life = .9) {
			(target.waves ??= []).push({
				x,
				y,
				age: 0,
				life,
				reach
			});
			if (target.waves.length > 4) target.waves.shift();
		}
		ageWaves(dt) {
			for (const o of [...this.glass, ...this.bubbles]) if (o.waves?.length) o.waves = o.waves.filter((w) => (w.age += dt) < w.life);
		}
		to(t) {
			this.gl.bindFramebuffer(this.gl.FRAMEBUFFER, t.fbo);
			this.gl.viewport(0, 0, t.w, t.h);
		}
		draw() {
			this.gl.drawArrays(this.gl.TRIANGLE_STRIP, 0, 4);
		}
		dispose() {
			const gl = this.gl;
			this.fluid.dispose();
			[
				this.bg,
				this.b1,
				this.b1t,
				this.b2,
				this.b2t,
				this.b3,
				this.b3t
			].forEach((t) => freeTarget(gl, t));
			for (const k in this.P) gl.deleteProgram(this.P[k].p);
			gl.deleteTexture(this.wTex);
			gl.deleteBuffer(this.q.buf);
			gl.deleteVertexArray(this.q.vao);
			gl.getExtension("WEBGL_lose_context")?.loseContext();
		}
	};
	// src/wubble/core/minis.ts
	var Minis = class {
		scene;
		list = [];
		max = 36;
		constructor(scene) {
			this.scene = scene;
		}
		/** `burst` throws them outward; otherwise they drift up gently. */
		spawn(x, y, n, burst = false, size = 1) {
			for (let i = 0; i < n; i++) {
				const a = -Math.PI / 2 + (Math.random() - .5) * (burst ? 2.6 : .6);
				const sp = burst ? 140 + Math.random() * 160 : 20 + Math.random() * 30;
				const r = (6 + Math.random() * 14) * size;
				const b = {
					x,
					y,
					r: 0,
					body: 0,
					alpha: 1,
					seed: Math.random() * 50,
					swirl: 0
				};
				this.list.push({
					x,
					y,
					vx: Math.cos(a) * sp,
					vy: Math.sin(a) * sp,
					r,
					age: 0,
					life: 3 + Math.random() * 3.5,
					ph: Math.random() * 6.28,
					born: 0,
					b
				});
				this.scene.bubbles.push(b);
			}
			while (this.list.length > this.max) this.remove(0, false);
		}
		/** Pop whichever mini is under (x, y). Returns true if one popped. */
		hit(x, y) {
			for (let i = this.list.length - 1; i >= 0; i--) {
				const m = this.list[i];
				if ((x - m.x) ** 2 + (y - m.y) ** 2 < (m.r + 8) ** 2) {
					this.remove(i, true, Math.atan2(y - m.y, x - m.x));
					return true;
				}
			}
			return false;
		}
		remove(i, pop, angle = -Math.PI / 2) {
			const m = this.list[i];
			if (pop) this.scene.pop(m.x, m.y, m.r, angle);
			this.list.splice(i, 1);
			const j = this.scene.bubbles.indexOf(m.b);
			if (j >= 0) this.scene.bubbles.splice(j, 1);
		}
		/** `avoid`: circles the minis are pushed off. */
		step(dt, t, avoid = [], top = -40) {
			for (let i = this.list.length - 1; i >= 0; i--) {
				const m = this.list[i];
				m.age += dt;
				m.born = Math.min(1, m.born + dt * 5);
				m.vy -= 38 * dt;
				m.vy *= Math.exp(-dt * .9);
				m.vx *= Math.exp(-dt * 1.4);
				m.x += (m.vx + Math.sin(t * 2 + m.ph) * 14) * dt;
				m.y += m.vy * dt;
				for (const o of avoid) {
					const bx = m.x - o.x, by = m.y - o.y, bd = Math.hypot(bx, by), minD = o.r + m.r + 4;
					if (bd < minD && bd > 0) {
						const push = minD - bd;
						m.x += bx / bd * push;
						m.y += by / bd * push;
						m.vx += bx / bd * push * 6;
					}
				}
				if (m.age > m.life || m.y < top) {
					this.remove(i, m.y > top + 20);
					continue;
				}
				const born = 1 - (1 - m.born) ** 3;
				const b = m.b;
				b.x = m.x;
				b.y = m.y;
				b.r = m.r * born * (1 + Math.sin(m.age * 6 + m.ph) * .04);
				b.swirl = m.age * .8;
			}
		}
		clear() {
			for (let i = this.list.length - 1; i >= 0; i--) this.remove(i, false);
		}
	};
	// src/wubble/core/physics.ts
	/** Loop and spring constants: dt is capped so a stalled tab never explodes a spring; a spring rests under REST. */
	const DT_CAP = .032;
	const spring = (x = 0) => ({
		x,
		v: 0
	});
	/** Rubber band: an overshoot `d` (px) maps onto an asymptote of `k` px. */
	const band = (d, k) => Math.sign(d) * k * (1 - 1 / (Math.abs(d) / k + 1));
	const clamp = (v, min, max) => Math.max(min, Math.min(max, v));
	const reducedMotion = () => typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
	/** One semi-implicit Euler step of a damped spring. Returns true while still moving. */
	function step(s, target, k, c, dt) {
		const a = -k * (s.x - target) - c * s.v;
		s.v += a * dt;
		s.x += s.v * dt;
		if (Math.abs(s.v) < .02 && Math.abs(s.x - target) < .02) {
			s.x = target;
			s.v = 0;
			return false;
		}
		return true;
	}
	/**
	* A rAF loop that only runs while `frame` asks for more. `kick()` wakes it.
	* dt is in seconds and capped at 32 ms. It pauses while the document is hidden.
	*/
	var Loop = class {
		frame;
		raf = 0;
		last = 0;
		constructor(frame) {
			this.frame = frame;
		}
		kick = () => {
			if (this.raf) return;
			this.last = performance.now();
			this.raf = requestAnimationFrame(this.tick);
		};
		tick = (now) => {
			const dt = Math.min(DT_CAP, (now - this.last) / 1e3);
			this.last = now;
			if (document.hidden) {
				this.raf = 0;
				document.addEventListener("visibilitychange", this.kick, { once: true });
				return;
			}
			this.raf = this.frame(dt, now) ? requestAnimationFrame(this.tick) : 0;
		};
		get running() {
			return this.raf !== 0;
		}
		stop() {
			cancelAnimationFrame(this.raf);
			this.raf = 0;
			document.removeEventListener("visibilitychange", this.kick);
		}
	};
	/** Tracks pointer velocity (px/s) over the last ~80 ms, for flings. */
	var Velocity = class {
		pts = [];
		push(x, y, t = performance.now()) {
			this.pts.push({
				t,
				x,
				y
			});
			while (this.pts.length > 2 && t - this.pts[0].t > 80) this.pts.shift();
		}
		get() {
			const p = this.pts;
			if (p.length < 2) return {
				x: 0,
				y: 0
			};
			const a = p[0], b = p[p.length - 1], dt = Math.max(1, b.t - a.t) / 1e3;
			return {
				x: (b.x - a.x) / dt,
				y: (b.y - a.y) / dt
			};
		}
		reset() {
			this.pts = [];
		}
	};
	// src/wubble/core/softbody.ts
	/**
	* The Wubble logo as a soft body: a ring of RING_N radial springs coupled to their neighbours
	* (surface tension) and to the mean (volume), a centre that leans toward the pointer and can be
	* dragged and flung, and a W that squashes and wobbles inside it.
	* Framework-free: call `step(dt)` every frame and hand `state` to the Scene.
	*/
	var SoftBody = class {
		ring = /* @__PURE__ */ new Float32Array(24);
		vel = /* @__PURE__ */ new Float32Array(24);
		acc = /* @__PURE__ */ new Float32Array(24);
		th = Float32Array.from({ length: 24 }, (_, i) => -Math.PI + Math.PI * 2 * i / 24);
		home = {
			x: 0,
			y: 0
		};
		R;
		cx = spring();
		cy = spring();
		wsx = spring(1);
		wsy = spring(1);
		wr = spring();
		wx = spring();
		wy = spring();
		swirl = spring();
		/** where the face looks: yaw and pitch, radians */
		yaw = spring();
		pitch = spring();
		t = 0;
		wobble;
		ptr = null;
		/** voice bands while listening (see core/voice.ts); null when not */
		voice = null;
		held = false;
		dragging = false;
		start = {
			x: 0,
			y: 0
		};
		grab = {
			x: 0,
			y: 0
		};
		v = new Velocity();
		state;
		constructor(o = {}) {
			this.R = o.radius ?? 180;
			this.wobble = o.wobble ?? 1;
			this.state = {
				x: 0,
				y: 0,
				r: this.R,
				ring: this.ring,
				body: 1,
				w: {
					inv: [
						1,
						0,
						0,
						1
					],
					off: [60, 65],
					scale: this.R / 51
				},
				seed: Math.random() * 10
			};
		}
		get center() {
			return {
				x: this.home.x + this.cx.x,
				y: this.home.y + this.cy.x
			};
		}
		/** Is (x, y) on the bubble? (with a little slack) */
		hit(x, y, slack = 16) {
			const c = this.center, dx = x - c.x, dy = y - c.y;
			return dx * dx + dy * dy < (this.R + slack) ** 2;
		}
		/** A tap. Near the middle it squashes the W; on the rim it dents where you touched. */
		poke(x, y, strength = 1) {
			const c = this.center, dx = x - c.x, dy = y - c.y, dist = Math.hypot(dx, dy) || 1;
			if (dist < this.R * .55) {
				this.wsy.v -= 7 * strength;
				this.wsx.v += 6 * strength;
				this.wr.v += Math.sign(dx || 1) * 90 * strength;
				for (let i = 0; i < 24; i++) this.vel[i] += -260 * strength * Math.cos(2 * (this.th[i] - Math.PI / 2));
			} else {
				const phi = Math.atan2(dy, dx);
				for (let i = 0; i < 24; i++) {
					const cs = Math.cos(this.th[i] - phi);
					if (cs > 0) this.vel[i] -= 1100 * strength * cs ** 4;
				}
				this.cx.v -= dx / dist * 160 * strength;
				this.cy.v -= dy / dist * 160 * strength;
				this.wx.v -= dx / dist * 60 * strength;
				this.wy.v -= dy / dist * 60 * strength;
			}
			this.swirl.v += 2.6 * strength;
		}
		down(x, y) {
			this.held = true;
			this.dragging = false;
			this.start = {
				x,
				y
			};
			const c = this.center;
			this.grab = {
				x: x - c.x,
				y: y - c.y
			};
			this.v.reset();
			this.v.push(x, y);
			this.ptr = {
				x,
				y
			};
		}
		move(x, y) {
			this.ptr = {
				x,
				y
			};
			if (this.held) {
				this.v.push(x, y);
				if (!this.dragging && Math.abs(x - this.start.x) + Math.abs(y - this.start.y) > 8) this.dragging = true;
			}
		}
		/** Returns 'poke' if it was a tap, 'fling' if dragged. */
		up() {
			if (!this.held) return null;
			this.held = false;
			if (this.dragging) {
				this.dragging = false;
				const f = this.v.get();
				this.cx.v += clamp(f.x, -2400, 2400) * .35;
				this.cy.v += clamp(f.y, -2400, 2400) * .35;
				this.swirl.v += Math.min(6, Math.hypot(f.x, f.y) / 400);
				const a = Math.atan2(f.y, f.x), amt = Math.min(1, Math.hypot(f.x, f.y) / 1600);
				for (let i = 0; i < 24; i++) this.vel[i] -= 700 * amt * Math.cos(this.th[i] - a);
				return "fling";
			}
			if (this.ptr) this.poke(this.ptr.x, this.ptr.y);
			return "poke";
		}
		leave() {
			if (!this.held) this.ptr = null;
		}
		/** Advance. Returns false once everything is at rest (the ambient wobble never fully rests). */
		step(dt) {
			this.t += dt;
			const t = this.t, wob = this.wobble, ptr = this.ptr, R = this.R;
			if (this.dragging && ptr) {
				step(this.cx, band(ptr.x - this.start.x, 120), 420, 28, dt);
				step(this.cy, band(ptr.y - this.start.y, 120), 420, 28, dt);
			} else {
				let tx = 0, ty = 0;
				if (ptr) {
					tx = clamp((ptr.x - this.home.x) * .035, -22, 22);
					ty = clamp((ptr.y - this.home.y) * .035, -22, 22);
				}
				ty += Math.sin(t * 1.1) * 6 * wob;
				step(this.cx, tx, 260, 10, dt);
				step(this.cy, ty, 260, 10, dt);
			}
			let dragA = 0, dragAmt = 0;
			if (this.dragging && ptr) {
				const c = this.center, ddx = ptr.x - c.x, ddy = ptr.y - c.y;
				dragA = Math.atan2(ddy, ddx);
				dragAmt = Math.min(R * .4, Math.hypot(ddx - this.grab.x * .6, ddy - this.grab.y * .6) * .3 + Math.hypot(this.cx.v, this.cy.v) * .01);
			}
			const sv = Math.hypot(this.cx.v, this.cy.v), sa = Math.atan2(this.cy.v, this.cx.v), sAmt = Math.min(R * .14, sv * .012);
			let mean = 0;
			for (let i = 0; i < 24; i++) mean += this.ring[i];
			mean /= 24;
			for (let i = 0; i < 24; i++) {
				const th = this.th[i];
				let target = R * .022 * wob * (Math.sin(2 * th + t * 1.4) * .6 + Math.sin(3 * th - t * .9) * .4);
				if (dragAmt) {
					const cs = Math.cos(th - dragA);
					target += dragAmt * (cs > 0 ? cs ** 3 : 0) - dragAmt * .18 * Math.sin(th - dragA) ** 2;
				}
				if (sAmt > .2) target += sAmt * Math.cos(2 * (th - sa));
				const vb = this.voice;
				if (vb) target += R * (.05 * vb.low * Math.sin(2 * th + vb.phase) + .035 * vb.mid * Math.sin(3 * th - vb.phase * 1.3 + 1) + .018 * vb.high * Math.sin(5 * th + vb.phase * 2.1 + 2));
				const prev = this.ring[(i + 24 - 1) % 24], next = this.ring[(i + 1) % 24];
				this.acc[i] = -180 * (this.ring[i] - target) - 6 * this.vel[i] + 140 * (prev + next - 2 * this.ring[i]) - 60 * mean;
			}
			for (let i = 0; i < 24; i++) {
				this.vel[i] += this.acc[i] * dt;
				this.ring[i] = clamp(this.ring[i] + this.vel[i] * dt, -R * .45, R * .6);
			}
			step(this.wsx, 1, 420, 11, dt);
			step(this.wsy, 1, 420, 11, dt);
			step(this.wr, 0, 300, 9, dt);
			let wtx = 0, wty = 0;
			if (ptr) {
				wtx = clamp((ptr.x - this.center.x) * .012, -5, 5);
				wty = clamp((ptr.y - this.center.y) * .012, -5, 5);
			}
			if (this.dragging) {
				wtx = -this.cx.x * .04;
				wty = -this.cy.x * .04;
			}
			step(this.wx, wtx, 200, 12, dt);
			step(this.wy, wty + Math.sin(t * 2.2) * .8 * wob, 200, 12, dt);
			let fy = 0, fp = 0;
			if (ptr) {
				const dx = (ptr.x - this.center.x) / (R * 2.2), dy = (ptr.y - this.center.y) / (R * 2.2);
				fy = Math.tanh(dx) * .45;
				fp = Math.tanh(dy) * .45;
			} else {
				fy = Math.sin(t * .23) * .12 * wob;
				fp = Math.sin(t * .17 + 1) * .08 * wob;
			}
			if (this.dragging) {
				fy = -Math.tanh(this.cx.v / 900) * .4;
				fp = -Math.tanh(this.cy.v / 900) * .4;
			}
			step(this.yaw, fy, 170, 17, dt);
			step(this.pitch, fp, 170, 17, dt);
			this.swirl.v *= Math.exp(-dt * 1.4);
			this.swirl.x += this.swirl.v * dt;
			const c = this.center, s = this.state;
			s.x = c.x;
			s.y = c.y;
			s.r = R;
			s.swirl = this.swirl.x;
			s.face = [this.yaw.x, this.pitch.x];
			const rot = this.wr.x * .12 * Math.PI / 180, cs = Math.cos(rot), sn = Math.sin(rot);
			const sx = this.wsx.x, sy = this.wsy.x;
			const i00 = cs / sx, i01 = sn / sx;
			s.w = {
				inv: [
					i00,
					-sn / sy,
					i01,
					cs / sy
				],
				off: [60 + this.wx.x, 65 + this.wy.x],
				scale: R / 51
			};
			return true;
		}
	};
	// src/wubble/core/living.ts
	/**
	* The living bubble on its fluid backdrop, framework-free. Used by the start page and the wallpaper.
	* mountLiving(target, opts) → { dispose, poke, scene, body, setPalette, setActive }
	*/
	function mountLiving(target, o = {}) {
		const canvas = document.createElement("canvas");
		canvas.style.cssText = "position:absolute;inset:0;width:100%;height:100%;display:block;touch-action:none;cursor:pointer";
		canvas.setAttribute("aria-hidden", "true");
		if (getComputedStyle(target).position === "static") target.style.position = "relative";
		target.append(canvas);
		const scene = new Scene(canvas, {
			palette: o.palette,
			ambient: 1
		});
		const body = new SoftBody({ wobble: o.wobble ?? 1 });
		const minis = new Minis(scene);
		scene.bubbles.push(body.state);
		const at = o.at ?? {
			x: .5,
			y: .5
		};
		let spawnT = .6, active = true, idle = 0;
		const layout = () => {
			const w = target.clientWidth, h = target.clientHeight;
			scene.resize(w, h);
			body.R = Math.max(40, Math.min(w, h) * (o.size ?? .2));
			body.home = {
				x: w * at.x,
				y: h * at.y
			};
			if (scene.w) scene.frame(0);
		};
		const ro = new ResizeObserver(layout);
		ro.observe(target);
		layout();
		const local = (e) => {
			const r = canvas.getBoundingClientRect();
			return {
				x: e.clientX - r.left,
				y: e.clientY - r.top
			};
		};
		const onDown = (e) => {
			const p = local(e);
			idle = 0;
			if (minis.hit(p.x, p.y)) {
				o.onPop?.();
				return;
			}
			if (body.hit(p.x, p.y)) {
				canvas.setPointerCapture(e.pointerId);
				body.down(p.x, p.y);
				canvas.style.cursor = "grabbing";
			} else {
				scene.ripple(p.x, p.y);
				minis.spawn(p.x, p.y, 4, true, .8);
			}
		};
		const onMove = (e) => {
			const p = local(e);
			idle = 0;
			body.move(p.x, p.y);
			scene.move(p.x, p.y);
		};
		const onUp = () => {
			const c = body.center, r = body.up();
			canvas.style.cursor = "pointer";
			if (r === "poke" && body.ptr) {
				scene.ripple(body.ptr.x, body.ptr.y, 300);
				scene.wave(body.state, body.ptr.x, body.ptr.y, body.R * 2.3);
				minis.spawn(body.ptr.x, body.ptr.y, 3, true);
				o.onPoke?.();
			}
			if (r === "fling") {
				scene.ripple(c.x, c.y, body.R * 2.4, 1.6);
				scene.wave(body.state, c.x, c.y, body.R * 1.6, 1.1);
			}
		};
		const onLeave = () => {
			body.leave();
			scene.leave();
		};
		canvas.addEventListener("pointerdown", onDown);
		canvas.addEventListener("pointermove", onMove);
		canvas.addEventListener("pointerup", onUp);
		canvas.addEventListener("pointercancel", onUp);
		canvas.addEventListener("pointerleave", onLeave);
		/** A poke from code (keyboard, preview). */
		const poke = (rim = Math.random() > .4) => {
			const c = body.center, a = Math.random() * Math.PI * 2;
			const x = rim ? c.x + Math.cos(a) * body.R : c.x + (Math.random() - .5) * body.R * .4;
			const y = rim ? c.y + Math.sin(a) * body.R : c.y + (Math.random() - .5) * body.R * .4;
			body.poke(x, y);
			scene.ripple(x, y, 300);
			scene.wave(body.state, x, y, body.R * 2.3);
			minis.spawn(x, y, 3, true);
			o.onPoke?.();
		};
		let autoT = 2.2, autoN = 0;
		const loop = new Loop((dt) => {
			if (!active) return false;
			const still = reducedMotion();
			body.wobble = still ? 0 : o.wobble ?? 1;
			body.voice = o.voice?.live ? o.voice.b : null;
			body.step(dt);
			if (o.ambient !== false && !still) {
				spawnT -= dt;
				if (spawnT <= 0) {
					spawnT = .9 + Math.random() * 1.4;
					minis.spawn(scene.w * (.12 + Math.random() * .76), scene.h + 20, 1);
				}
			}
			const c = body.center;
			minis.step(dt, scene.time, [{
				x: c.x,
				y: c.y,
				r: body.R
			}]);
			idle += dt;
			if (o.auto && idle > 2) {
				autoT -= dt;
				if (autoT <= 0) {
					autoN++;
					if (autoN % 3 === 2 && minis.list.length) {
						const m = minis.list[minis.list.length - 1];
						minis.hit(m.x, m.y);
					} else poke(autoN % 2 === 0);
					autoT = 2.4 + Math.random() * 1.2;
				}
				const lt = scene.time * .35;
				scene.move(c.x + Math.cos(lt) * body.R * 1.6, c.y - body.R * .9 + Math.sin(lt * 1.3) * body.R * .5);
			}
			scene.frame(still ? dt * .3 : dt);
			return true;
		});
		loop.kick();
		return {
			scene,
			body,
			minis,
			poke,
			setPalette: (p) => scene.setPalette(p),
			/** pause when the page is covered */
			setActive(v) {
				active = v;
				if (v) loop.kick();
			},
			dispose() {
				loop.stop();
				ro.disconnect();
				canvas.removeEventListener("pointerdown", onDown);
				canvas.removeEventListener("pointermove", onMove);
				canvas.removeEventListener("pointerup", onUp);
				canvas.removeEventListener("pointercancel", onUp);
				canvas.removeEventListener("pointerleave", onLeave);
				scene.dispose();
				canvas.remove();
			}
		};
	}
	// ../../notes/site/entry.ts
	window.wubbleLiving = mountLiving;
})();
