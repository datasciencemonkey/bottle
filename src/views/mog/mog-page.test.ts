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
    // A coloured background anywhere in the page would cover the desktop, except the right-click
    // menu (#ui-4, `.mog-menu`): it is UI chrome the user explicitly opened (Hide/Close/Open in
    // Bottle/Change home workspace…), not part of the ambient lamp, and its text needs a real
    // surface to read; it never shows over the mascot itself, only in the small area the click
    // opened it in.
    const withoutMenu = css.replace(/\.mog-menu[^{]*\{[^}]*\}/g, '');
    expect(withoutMenu).not.toMatch(/background(-color)?:\s*(?!transparent)[#a-z]/);
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
