/**
 * UX-09b / UX-03 (ADR 0027): the PNG fallback draws on the transparent MOG panel, so each mood
 * PNG must be transparent outside the mascot, and the "?" cue PNG must hold only the bubble, so
 * it shows over any mood without hiding it. `tools/render_mog_fallback.py` makes these files.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { inflateSync } from 'node:zlib';

const dir = resolve(import.meta.dirname, '../mog-rive/fallback');
const MOODS = ['idle', 'listening', 'processing', 'answering', 'success', 'error', 'sleeping'];

interface Rgba {
  width: number;
  height: number;
  alpha(x: number, y: number): number;
}

/** Decode an 8-bit RGBA, non-interlaced PNG (the format the render script writes). */
function decodePng(file: string): Rgba {
  const buf = readFileSync(file);
  expect(buf.subarray(0, 8).toString('hex')).toBe('89504e470d0a1a0a');
  let pos = 8;
  let width = 0;
  let height = 0;
  const idat: Buffer[] = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString('latin1', pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === 'IHDR') {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      expect([data[8], data[9], data[12]], `${file}: 8-bit RGBA, no interlace`).toEqual([8, 6, 0]);
    } else if (type === 'IDAT') idat.push(data);
    pos += 12 + len;
  }
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * 4;
  const px = new Uint8Array(height * stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)] ?? 0;
    for (let i = 0; i < stride; i++) {
      const x = raw[y * (stride + 1) + 1 + i] ?? 0;
      const a = i >= 4 ? (px[y * stride + i - 4] ?? 0) : 0;
      const b = y > 0 ? (px[(y - 1) * stride + i] ?? 0) : 0;
      const c = i >= 4 && y > 0 ? (px[(y - 1) * stride + i - 4] ?? 0) : 0;
      let pred = 0;
      if (filter === 1) pred = a;
      else if (filter === 2) pred = b;
      else if (filter === 3) pred = (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a);
        const pb = Math.abs(p - b);
        const pc = Math.abs(p - c);
        pred = pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      px[y * stride + i] = (x + pred) & 0xff;
    }
  }
  return { width, height, alpha: (x, y) => px[y * stride + x * 4 + 3] ?? 0 };
}

function transparentShare(img: Rgba): number {
  let n = 0;
  for (let y = 0; y < img.height; y++)
    for (let x = 0; x < img.width; x++) if (img.alpha(x, y) === 0) n++;
  return n / (img.width * img.height);
}

describe('the transparent PNG fallback (ADR 0027)', () => {
  it('[UX-09b] [AT-21] each of the 7 mood PNGs is transparent outside the mascot', () => {
    for (const m of MOODS) {
      const img = decodePng(resolve(dir, `mood-${m}.png`));
      expect([img.width, img.height], m).toEqual([400, 400]);
      for (const [x, y] of [
        [0, 0],
        [399, 0],
        [0, 399],
        [399, 399],
      ] as const)
        expect(img.alpha(x, y), `${m} corner`).toBe(0);
      expect(transparentShare(img), m).toBeGreaterThan(0.4);
      // The lamp body is in every mood and is opaque.
      expect(img.alpha(200, 310), `${m} lamp`).toBe(255);
    }
  });

  it('[UX-03] [UX-09b] the "?" cue PNG holds only the bubble, so it never hides the mood', () => {
    const cue = decodePng(resolve(dir, 'attention.png'));
    expect([cue.width, cue.height]).toEqual([400, 400]);
    expect(transparentShare(cue)).toBeGreaterThan(0.9);
    // The mascot head, body and lamp stay visible through the cue.
    for (const [x, y] of [
      [200, 200],
      [200, 250],
      [200, 310],
      [120, 150],
    ] as const)
      expect(cue.alpha(x, y), `(${x}, ${y})`).toBe(0);
    // The bubble itself is opaque (top right of the art).
    expect(cue.alpha(330, 117)).toBe(255);
  });
});
