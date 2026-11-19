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
    // Forced-colors drops box-shadow: the outline must stay (transparent), never `none`.
    expect(css).not.toMatch(/outline:\s*none/);
    expect(css).toMatch(/\.hub :focus-visible\s*\{[^}]*outline:\s*2px solid transparent/);
  });

  it('[#561] colours come from the tokens: the hub declares no hex colour of its own', () => {
    const rules = read('hub.css').replace(/\/\*[\s\S]*?\*\//g, '');
    expect(rules).not.toMatch(/#[0-9a-f]{3,8}\b/i);
  });

  // review of #602 round 3: the earlier test ("a pinned sidebar pushes the content column
  // instead of covering it", Hub.test.tsx) only ever checked the `--hub-sidebar-width` CSS
  // variable the panel writes, not the actual rule that positions `.conv` — which is exactly
  // where the round-3 bug was (blocker 1). These read the real, shipped CSS rule instead.
  it('[blocker 1, review #602 round 3] `.conv` cancels only the base padding, like `.rec-area`', () => {
    const css = read('hub.css').replace(/\/\*[\s\S]*?\*\//g, '');
    const conv = /(?:^|\n)\s*\.conv\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    // The bug: `calc((var(--mog-space-6) + var(--hub-sidebar-width, 0px)) * -1)` cancelled the
    // pinned sidebar's own width too, landing `.conv` back under the panel instead of beside it.
    expect(conv).not.toMatch(/--hub-sidebar-width/);
    expect(conv).toMatch(/margin-left:\s*calc\(var\(--mog-space-6\)\s*\*\s*-1\)/);
    const recArea = /(?:^|\n)\s*\.rec-area\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    expect(recArea).not.toMatch(/--hub-sidebar-width/);
  });

  it('[blocker 2, review #602 round 3] the sidebar is a fixed, full-window-height rail', () => {
    const tsx = read('HubSidebar.tsx');
    // `position: fixed` + `inset-y-0` (not `absolute`/`top-0`/`h-full` inside `.hub-body`, which
    // scrolled away with a tall page and started below the header): the panel is pinned to the
    // window itself, independent of any ancestor's scroll position or height.
    expect(tsx).toMatch(/\bfixed inset-y-0 left-0\b/);
    expect(tsx).not.toMatch(/\babsolute top-0 left-0\b/);
    // A pinned rail is flat with a border; only an unpinned overlay keeps the soft shadow.
    expect(tsx).toMatch(/pinned \? '' : 'shadow-lg'/);
  });

  it('[blocker 3, review #602 round 3] `.hub` uses one grid-based header layout in every area', () => {
    const css = read('hub.css');
    const hubRule = /(?:^|\n)\s*\.hub\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    expect(hubRule).toMatch(/display:\s*grid/);
    expect(hubRule).toMatch(/grid-template-columns:\s*auto 1fr/);
    expect(hubRule).toMatch(/height:\s*100vh/);
    expect(hubRule).not.toMatch(/min-height/);
    // `.hub--wide` no longer repeats the grid: one header layout, not two.
    const wideRule = /(?:^|\n)\s*\.hub--wide\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    expect(wideRule).not.toMatch(/display:\s*grid/);
    expect(wideRule).toMatch(/padding-bottom:\s*0/);
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
