/**
 * lamp-alive: the MOG window layout. The lamp art is a 160 px square at the top of a 160 x 168
 * window; the target marker has its own 8 px row under it, so it never covers the lamp. The MOG
 * shows no name on the lamp (the user's choice): the name and colour stay in the hub,
 * Conversations and the card. This reads the numbers from `mog.css` and `src-tauri/src/windows.rs`
 * and checks the boxes against each other. The real-browser measure of the same boxes is in
 * `docs/shots/lamp-alive/layout.json` (made by `tools/lamp_alive_shots.py`).
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const css = readFileSync(resolve(root, 'src/views/mog/mog.css'), 'utf8');
const rs = readFileSync(resolve(root, 'src-tauri/src/windows.rs'), 'utf8');
const view = readFileSync(resolve(root, 'src/views/mog/MogView.tsx'), 'utf8');

const rule = (sel: string): string =>
  new RegExp(`(?:^|\\n)${sel.replace(/[.[\]()]/g, '\\$&')}\\s*\\{([^}]*)\\}`).exec(css)?.[1] ?? '';
const px = (body: string, prop: string): number =>
  Number(new RegExp(`(?:^|[\\s;])${prop}:\\s*(-?[\\d.]+)px`).exec(body)?.[1] ?? Number.NaN);
const constant = (name: string): number =>
  Number(new RegExp(`pub const ${name}: \\w+ = ([\\d.]+);`).exec(rs)?.[1] ?? Number.NaN);

const WIDTH = constant('MOG_WIDTH');
const ART = constant('MOG_ART_HEIGHT');
const HEIGHT = constant('MOG_HEIGHT');
const ROW = Number(/--mog-marker-row:\s*(\d+)px/.exec(css)?.[1]);

describe('lamp-alive: the MOG window layout', () => {
  it('the window is the art square plus the marker row', () => {
    expect(ART).toBe(WIDTH);
    expect(ROW).toBe(HEIGHT - ART);
    expect(HEIGHT).toBe(168);
  });

  it('the target marker lies fully below the art and inside the window, with no overlap', () => {
    const m = rule('.target-marker');
    const bottom = px(m, 'bottom');
    const height = px(m, 'border-top');
    const top = HEIGHT - bottom - height;
    expect(top).toBeGreaterThanOrEqual(ART);
    expect(bottom).toBeGreaterThanOrEqual(0);
    expect(HEIGHT - bottom).toBeLessThanOrEqual(HEIGHT);
    expect(m).toMatch(/left:\s*50%;/);
  });

  it('the MOG shows no name pill: no markup and no style', () => {
    expect(view).not.toMatch(/identity-pill|data-identity-pill/);
    expect(css).not.toMatch(/identity-pill/);
    // The name stays for the tooltip, the accessible name and the target toast.
    expect(view).toMatch(/title=\{tooltip\}/);
    expect(view).toMatch(/Talking to/);
  });

  it('the art, the badges and the lamp use the art square, not the whole window', () => {
    expect(css).toMatch(/--mog-art:\s*min\(100vw, calc\(100vh - var\(--mog-marker-row\)\)\);/);
    expect(rule('.mascot')).toMatch(/height:\s*var\(--mog-art\);/);
    expect(rule('.lamp')).toMatch(/width:\s*var\(--mog-art\);/);
    expect(rule('.badges')).toMatch(/bottom:\s*var\(--mog-marker-row\);/);
  });

  it('the card is placed against the art edge of the same window width', () => {
    expect(constant('MOG_ART_LEFT_INSET')).toBe(18);
    expect(constant('MOG_ART_RIGHT_INSET')).toBe(13);
    // The lamp art spans x 44..367 of 400 (`lamp-front.png` and `lamp-back.png`).
    expect(Math.round(44 * (WIDTH / 400))).toBe(18);
    expect(Math.round(WIDTH - 367 * (WIDTH / 400))).toBe(13);
  });
});
