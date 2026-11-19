/**
 * lamp-alive: the MOG window layout. The lamp art is a 160 px square at the top of a 160 x 192
 * window; the name pill and the target marker have their own 32 px row under it, so the name never
 * covers the lamp. This reads the numbers from `mog.css` and `src-tauri/src/windows.rs` and checks
 * the boxes against each other. The real-browser measure of the same boxes is in
 * `docs/shots/lamp-alive/layout.json` (made by the shot script of the PR).
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const css = readFileSync(resolve(root, 'src/views/mog/mog.css'), 'utf8');
const rs = readFileSync(resolve(root, 'src-tauri/src/windows.rs'), 'utf8');

const rule = (sel: string): string =>
  new RegExp(`(?:^|\\n)${sel.replace(/[.[\]()]/g, '\\$&')}\\s*\\{([^}]*)\\}`).exec(css)?.[1] ?? '';
const px = (body: string, prop: string): number =>
  Number(new RegExp(`(?:^|[\\s;])${prop}:\\s*(-?[\\d.]+)px`).exec(body)?.[1] ?? Number.NaN);
const constant = (name: string): number =>
  Number(new RegExp(`pub const ${name}: \\w+ = ([\\d.]+);`).exec(rs)?.[1] ?? Number.NaN);

const WIDTH = constant('MOG_WIDTH');
const ART = constant('MOG_ART_HEIGHT');
const HEIGHT = constant('MOG_HEIGHT');
const ROW = Number(/--mog-name-row:\s*(\d+)px/.exec(css)?.[1]);

// The pill: font 11 px (`--mog-text-xs`), line height 1.3, 1 px padding, 1.5 px border.
const PILL_H = 11 * 1.3 + 2 * 1 + 2 * 1.5;

describe('lamp-alive: the MOG window layout', () => {
  it('the window is the art square plus the name row', () => {
    expect(ART).toBe(WIDTH);
    expect(ROW).toBe(HEIGHT - ART);
    expect(HEIGHT).toBe(192);
  });

  it('the name pill lies fully below the art and inside the window, with no overlap', () => {
    const pill = rule('.identity-pill');
    const bottom = px(pill, 'bottom');
    const top = HEIGHT - bottom - PILL_H;
    expect(top).toBeGreaterThanOrEqual(ART);
    expect(bottom).toBeGreaterThanOrEqual(0);
    expect(pill).toMatch(/text-overflow:\s*ellipsis;/);
    expect(pill).toMatch(/white-space:\s*nowrap;/);
    expect(pill).toMatch(/left:\s*50%;/);
    expect(pill).toMatch(/translateX\(-50%\)/);
  });

  it('the target marker is clear of the art above it and of the pill below it', () => {
    const m = rule('.target-marker');
    const bottom = px(m, 'bottom');
    const height = px(m, 'border-top');
    const markerTop = HEIGHT - bottom - height;
    const markerBottom = HEIGHT - bottom;
    const pillTop = HEIGHT - px(rule('.identity-pill'), 'bottom') - PILL_H;
    expect(markerTop).toBeGreaterThanOrEqual(ART);
    expect(markerBottom).toBeLessThanOrEqual(pillTop);
  });

  it('the art, the badges and the lamp use the art square, not the whole window', () => {
    expect(css).toMatch(/--mog-art:\s*min\(100vw, calc\(100vh - var\(--mog-name-row\)\)\);/);
    expect(rule('.mascot')).toMatch(/height:\s*var\(--mog-art\);/);
    expect(rule('.lamp')).toMatch(/width:\s*var\(--mog-art\);/);
    expect(rule('.badges')).toMatch(/bottom:\s*var\(--mog-name-row\);/);
  });

  it('the card is placed against the art edge of the same window width', () => {
    expect(constant('MOG_ART_LEFT_INSET')).toBe(18);
    expect(constant('MOG_ART_RIGHT_INSET')).toBe(13);
    // The lamp art spans x 44..367 of 400 (`lamp-front.png` and `lamp-back.png`).
    expect(Math.round(44 * (WIDTH / 400))).toBe(18);
    expect(Math.round(WIDTH - 367 * (WIDTH / 400))).toBe(13);
  });
});
