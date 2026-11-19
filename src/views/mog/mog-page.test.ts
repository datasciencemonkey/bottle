import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const dir = import.meta.dirname;
const read = (name: string) => readFileSync(resolve(dir, name), 'utf8');

describe('MOG page (V2)', () => {
  it('[MOG-01a] the page, body and root paint no background, so the desktop shows through', () => {
    const css = read('mog.css');
    const rule = /html,\s*body,\s*#root\s*\{([^}]*)\}/.exec(css);
    expect(rule, 'one rule for html, body and #root').not.toBeNull();
    expect(rule?.[1]).toMatch(/background:\s*transparent;/);
    expect(rule?.[1]).toMatch(/margin:\s*0;/);
    expect(rule?.[1]).toMatch(/overflow:\s*hidden;/);
    // A coloured background anywhere in this page would cover the desktop. The one past
    // exception, the right-click menu's own solid surface (`.mog-menu`), is gone (#ui-4 review
    // #603, findings 1 and 4): that menu is now a native Tauri menu built from `MogView.tsx`, so
    // it is never part of this page's own CSS at all.
    expect(css).not.toMatch(/background(-color)?:\s*(?!transparent)[#a-z]/);
  });

  it('[MOG-01a] the MOG entry loads the style as a file (the CSP blocks inline styles)', () => {
    expect(read('main.tsx')).toMatch(/^import '\.\/mog\.css';$/m);
    const html = readFileSync(resolve(dir, '../../../mog.html'), 'utf8');
    expect(html).not.toMatch(/<style|style=/);
  });

  it('[MOG-07] [UX-39a] the badges sit inside the window, over the mascot, with no background', () => {
    const css = read('mog.css');
    const rule = /\.badges\s*\{([^}]*)\}/.exec(css);
    expect(rule, 'one rule for .badges').not.toBeNull();
    expect(rule?.[1]).toMatch(/position:\s*absolute;/);
    expect(rule?.[1]).toMatch(/bottom:\s*0;/);
    expect(rule?.[1]).toMatch(/z-index:\s*1;/);
    // Text on any desktop: a shadow, not a fill (MOG-01a keeps the page transparent).
    expect(css).toMatch(/\.badges [^{]*\{[^}]*text-shadow:/);
    expect(read('MogView.tsx')).toMatch(/className="badges"/);
  });
});
