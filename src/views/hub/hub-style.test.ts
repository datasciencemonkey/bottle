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
    // review of #602 round 4, blocker 1: a leading `var(--hub-sidebar-width, 0px)` column, so a
    // pinned rail pushes the header row (outside `.hub-body`) over the same way it pushes
    // `.hub-content`, instead of covering "Bottle" and the Labs label.
    expect(hubRule).toMatch(/grid-template-columns:\s*var\(--hub-sidebar-width, 0px\) auto 1fr/);
    expect(hubRule).toMatch(/height:\s*100vh/);
    expect(hubRule).not.toMatch(/min-height/);
    // `.hub--wide` no longer repeats the grid: one header layout, not two.
    const wideRule = /(?:^|\n)\s*\.hub--wide\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    expect(wideRule).not.toMatch(/display:\s*grid/);
    expect(wideRule).toMatch(/padding-bottom:\s*0/);
  });

  // review of #602 round 4, blocker 1: `.hub-title`/`.hub-labs` are direct children of `main`
  // (AT-37), outside `.hub-body` — the box the earlier `--hub-sidebar-width` padding lived on —
  // so a pinned rail used to sit on top of them with nothing telling them to move over.
  it('[blocker 1, review #602 round 4] the header row moves with the sidebar-width column', () => {
    const css = read('hub.css').replace(/\/\*[\s\S]*?\*\//g, '');
    const title = /(?:^|\n)\s*\.hub > \.hub-title\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    const labs = /(?:^|\n)\s*\.hub > \.hub-labs\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    expect(title).toMatch(/grid-column:\s*2/);
    expect(labs).toMatch(/grid-column:\s*3/);
  });

  // review of #602 round 4, blocker 2: the toggle no longer carries an elevated z-index (it
  // used to be `relative z-30`, above the sidebar panel's own z-20), so an open overlay covers
  // it instead of the toggle sitting on top of the panel's first nav item.
  it('[blocker 2, review #602 round 4] the toggle has no elevated z-index', () => {
    const tsx = read('Hub.tsx')
      .replace(/\/\/.*$/gm, '')
      .replace(/\/\*[\s\S]*?\*\//g, '');
    expect(tsx).not.toMatch(/z-30/);
    expect(tsx).toMatch(/data-sidebar-toggle/);
  });

  // MEDIUM, review of #602 round 4: `.hub-content` is the column that scrolls now
  // (`overflow-y: auto`); without this the toggle scrolled out of reach on a tall Settings page.
  it('[MEDIUM, review #602 round 4] the area header stays put while the content scrolls', () => {
    const css = read('hub.css');
    const rule = /(?:^|\n)\s*\.hub-area-header\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    expect(rule).toMatch(/position:\s*sticky/);
    expect(rule).toMatch(/top:\s*0/);
  });

  // MEDIUM, review of #602 round 4: a realistic path must truncate with an ellipsis, not wrap
  // the row (which pushed "Save to" onto two lines).
  it('[MEDIUM, review #602 round 4] a long Save-to path truncates instead of wrapping', () => {
    const css = read('hub.css');
    const place = /(?:^|\n)\s*\.rec-place\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    const path = /(?:^|\n)\s*\.rec-path\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    expect(place).not.toMatch(/flex-wrap:\s*wrap/);
    expect(path).toMatch(/text-overflow:\s*ellipsis/);
    expect(path).toMatch(/white-space:\s*nowrap/);
    const tsx = read('RecordingSettings.tsx');
    expect(tsx).toMatch(/className="rec-path"\s+title={storage\.path}/);
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
