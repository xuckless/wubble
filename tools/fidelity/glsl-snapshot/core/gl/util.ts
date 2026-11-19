/** Small WebGL2 helpers shared by the Wubble renderers. */

export const VERT_FS = `#version 300 es
in vec2 aPos;
out vec2 vUv;
void main() { vUv = aPos * 0.5 + 0.5; gl_Position = vec4(aPos, 0.0, 1.0); }`;

/** Quad in pixel space: aPos is 0..1, expanded to uRect (x, y, w, h in CSS px, top-left origin). */
export const VERT_QUAD = `#version 300 es
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

export interface Program { p: WebGLProgram; u: Record<string, WebGLUniformLocation | null> }

export function program(gl: WebGL2RenderingContext, vs: string, fs: string): Program {
  const sh = (type: number, src: string) => {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) + '\n' + src.split('\n').map((l, i) => `${i + 1}: ${l}`).join('\n'));
    return s;
  };
  const p = gl.createProgram()!;
  gl.attachShader(p, sh(gl.VERTEX_SHADER, vs));
  gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
  gl.bindAttribLocation(p, 0, 'aPos');
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? 'link failed');
  const u: Program['u'] = {};
  const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS) as number;
  for (let i = 0; i < n; i++) {
    const info = gl.getActiveUniform(p, i)!;
    const name = info.name.replace(/\[0\]$/, '');
    u[name] = gl.getUniformLocation(p, info.name);
  }
  return { p, u };
}

/** A unit quad (-1..1) bound to attribute 0. Draw with gl.drawArrays(TRIANGLE_STRIP, 0, 4). */
export function quad(gl: WebGL2RenderingContext) {
  const vao = gl.createVertexArray()!;
  gl.bindVertexArray(vao);
  const buf = gl.createBuffer()!;
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  return { vao, buf };
}

export interface Target { tex: WebGLTexture; fbo: WebGLFramebuffer; w: number; h: number }

export interface Formats { internal: number; format: number; type: number }

/** Half-float RGBA if the GPU can render to it, else plain RGBA8 (the liquid then can't hold signed velocity and goes still; a flat backdrop remains). */
export function formats(gl: WebGL2RenderingContext): { float: Formats; byte: Formats; linearFloat: boolean } {
  const canFloat = !!gl.getExtension('EXT_color_buffer_float') || !!gl.getExtension('EXT_color_buffer_half_float');
  const linearFloat = true; // RGBA16F is always filterable in WebGL2
  const byte = { internal: gl.RGBA8, format: gl.RGBA, type: gl.UNSIGNED_BYTE };
  return { float: canFloat ? { internal: gl.RGBA16F, format: gl.RGBA, type: gl.HALF_FLOAT } : byte, byte, linearFloat };
}

export function target(gl: WebGL2RenderingContext, w: number, h: number, f: Formats, filter: number = gl.LINEAR): Target {
  const tex = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.texImage2D(gl.TEXTURE_2D, 0, f.internal, w, h, 0, f.format, f.type, null);
  const fbo = gl.createFramebuffer()!;
  gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
  gl.viewport(0, 0, w, h);
  gl.clearColor(0, 0, 0, 0);
  gl.clear(gl.COLOR_BUFFER_BIT);
  return { tex, fbo, w, h };
}

export function freeTarget(gl: WebGL2RenderingContext, t: Target | null | undefined) {
  if (!t) return;
  gl.deleteTexture(t.tex);
  gl.deleteFramebuffer(t.fbo);
}

export class Double {
  constructor(public read: Target, public write: Target) {}
  swap() { const t = this.read; this.read = this.write; this.write = t; }
  free(gl: WebGL2RenderingContext) { freeTarget(gl, this.read); freeTarget(gl, this.write); }
}

export function bindTex(gl: WebGL2RenderingContext, unit: number, tex: WebGLTexture, loc: WebGLUniformLocation | null) {
  gl.activeTexture(gl.TEXTURE0 + unit);
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.uniform1i(loc, unit);
}

export function hex(c: string): [number, number, number] {
  const n = parseInt(c.replace('#', ''), 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}
