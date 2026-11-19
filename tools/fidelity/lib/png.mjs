// PNG in and out without dependencies: 8-bit RGB/RGBA (and grey) non-interlaced images, which is
// what Chromium's and wubble's screenshots are. Images are `{ width, height, data }` with `data`
// RGBA bytes.
import { inflateSync, deflateSync } from 'node:zlib';
import { readFileSync, writeFileSync } from 'node:fs';

const SIG = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

export function decode(buf) {
  if (!buf.subarray(0, 8).equals(SIG)) throw new Error('not a png');
  let i = 8, width = 0, height = 0, depth = 0, type = 0, interlace = 0;
  const idat = [];
  while (i < buf.length) {
    const len = buf.readUInt32BE(i), kind = buf.toString('latin1', i + 4, i + 8), body = buf.subarray(i + 8, i + 8 + len);
    if (kind === 'IHDR') { width = body.readUInt32BE(0); height = body.readUInt32BE(4); depth = body[8]; type = body[9]; interlace = body[12]; }
    else if (kind === 'IDAT') idat.push(body);
    else if (kind === 'IEND') break;
    i += 12 + len;
  }
  const channels = { 0: 1, 2: 3, 4: 2, 6: 4 }[type];
  if (depth !== 8 || !channels || interlace) throw new Error(`unsupported png (depth ${depth}, type ${type}, interlace ${interlace})`);
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * channels, out = Buffer.alloc(width * height * 4);
  let prev = Buffer.alloc(stride);
  for (let y = 0; y < height; y++) {
    const f = raw[y * (stride + 1)], line = Buffer.from(raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)));
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? line[x - channels] : 0, b = prev[x], c = x >= channels ? prev[x - channels] : 0;
      let p = 0;
      if (f === 1) p = a; else if (f === 2) p = b; else if (f === 3) p = (a + b) >> 1;
      else if (f === 4) { const q = a + b - c, pa = Math.abs(q - a), pb = Math.abs(q - b), pc = Math.abs(q - c); p = pa <= pb && pa <= pc ? a : pb <= pc ? b : c; }
      line[x] = (line[x] + p) & 255;
    }
    for (let x = 0; x < width; x++) {
      const s = x * channels, d = (y * width + x) * 4;
      if (channels >= 3) { out[d] = line[s]; out[d + 1] = line[s + 1]; out[d + 2] = line[s + 2]; out[d + 3] = channels === 4 ? line[s + 3] : 255; }
      else { out[d] = out[d + 1] = out[d + 2] = line[s]; out[d + 3] = channels === 2 ? line[s + 1] : 255; }
    }
    prev = line;
  }
  return { width, height, data: out };
}

const CRC = new Int32Array(256).map((_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c; });
const crc32 = b => { let c = -1; for (const x of b) c = CRC[(c ^ x) & 255] ^ (c >>> 8); return (c ^ -1) >>> 0; };
const chunk = (kind, body) => {
  const len = Buffer.alloc(4); len.writeUInt32BE(body.length);
  const kb = Buffer.from(kind, 'latin1'), crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(Buffer.concat([kb, body])));
  return Buffer.concat([len, kb, body, crc]);
};

export function encode({ width, height, data }) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4); ihdr[8] = 8; ihdr[9] = 6;
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) data.copy(raw, y * (width * 4 + 1) + 1, y * width * 4, (y + 1) * width * 4);
  return Buffer.concat([SIG, chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

export const read = path => decode(readFileSync(path));
export const write = (path, img) => writeFileSync(path, encode(img));

/** A blank image. */
export const blank = (width, height, rgba = [0, 0, 0, 255]) => {
  const data = Buffer.alloc(width * height * 4);
  for (let i = 0; i < data.length; i += 4) data.set(rgba, i);
  return { width, height, data };
};

/** The `[x, y, w, h]` part of `img` (clamped to it). */
export function crop(img, [x, y, w, h]) {
  x = Math.max(0, Math.round(x)); y = Math.max(0, Math.round(y));
  w = Math.min(Math.round(w), img.width - x); h = Math.min(Math.round(h), img.height - y);
  const out = blank(w, h);
  for (let r = 0; r < h; r++) img.data.copy(out.data, r * w * 4, ((y + r) * img.width + x) * 4, ((y + r) * img.width + x + w) * 4);
  return out;
}

/** `img` resampled (bilinear) to `w`×`h`. */
export function resize(img, w, h) {
  if (img.width === w && img.height === h) return img;
  const out = blank(w, h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const sx = Math.min(img.width - 1, Math.max(0, (x + 0.5) * img.width / w - 0.5)), sy = Math.min(img.height - 1, Math.max(0, (y + 0.5) * img.height / h - 0.5));
    const x0 = Math.floor(sx), y0 = Math.floor(sy), x1 = Math.min(img.width - 1, x0 + 1), y1 = Math.min(img.height - 1, y0 + 1), fx = sx - x0, fy = sy - y0;
    for (let c = 0; c < 4; c++) {
      const p = (xx, yy) => img.data[(yy * img.width + xx) * 4 + c];
      out.data[(y * w + x) * 4 + c] = Math.round((p(x0, y0) * (1 - fx) + p(x1, y0) * fx) * (1 - fy) + (p(x0, y1) * (1 - fx) + p(x1, y1) * fx) * fy);
    }
  }
  return out;
}

/** Images side by side with `gap` px between, on `bg`. */
export function hstack(images, gap = 8, bg = [24, 26, 33, 255]) {
  const w = images.reduce((s, i) => s + i.width, 0) + gap * (images.length - 1);
  const h = Math.max(...images.map(i => i.height));
  const out = blank(w, h, bg);
  let x0 = 0;
  for (const img of images) {
    for (let y = 0; y < img.height; y++) img.data.copy(out.data, (y * w + x0) * 4, y * img.width * 4, (y + 1) * img.width * 4);
    x0 += img.width + gap;
  }
  return out;
}
