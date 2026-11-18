/**
 * The hub and MOG chrome styling (#561). The views load the shared tokens (#559) and their own
 * stylesheet as files, because the CSP blocks inline style elements. These checks keep the
 * calm, accessible base from drifting back to browser defaults.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const dir = import.meta.dirname;
const read = (rel: string) => readFileSync(resolve(dir, rel), 'utf8');

describe('hub and MOG chrome styling (#561)', () => {
  it('[MB-10b] the hub entry loads the shared tokens, then the hub stylesheet, as files', () => {
    const main = read('main.tsx');
    expect(main).toMatch(/^import '\.\.\/tokens\.css';\nimport '\.\/hub\.css';$/m);
    expect(read('../../../hub.html')).not.toMatch(/<style|style=/);
  });

  it('[MOG-01a] the MOG entry loads the shared tokens before its own stylesheet', () => {
    expect(read('../mog/main.tsx')).toMatch(
      /^import '\.\.\/tokens\.css';\nimport '\.\/mog\.css';$/m,
    );
  });

  it('[#561] the hub uses the system font, a visible focus ring and a reduced-motion rule', () => {
    const css = read('hub.css');
    expect(css).toMatch(/\.hub\s*\{[^}]*font:[^;]*var\(--mog-font\)/);
    expect(css).toMatch(/\.hub :focus-visible\s*\{[^}]*box-shadow:\s*var\(--mog-focus-ring\)/);
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
  });

  it('[#561] colours come from the tokens: the hub declares no hex colour of its own', () => {
    const rules = read('hub.css').replace(/\/\*[\s\S]*?\*\//g, '');
    expect(rules).not.toMatch(/#[0-9a-f]{3,8}\b/i);
  });

  it('[UX-39a] the REC and Offline overlays sit inside the MOG window', () => {
    const css = read('../mog/mog.css');
    const rule = /\[data-overlay='recording'\],\s*\[data-overlay='offline'\]\s*\{([^}]*)\}/.exec(
      css,
    );
    expect(rule, 'one rule for both overlays').not.toBeNull();
    expect(rule?.[1]).toMatch(/position:\s*absolute;/);
    expect(rule?.[1]).toMatch(/text-shadow:/);
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
  });
});
