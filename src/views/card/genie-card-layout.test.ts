/**
 * #523: the card never clips its text at the right edge, and a wide table scrolls inside the
 * card. The webview has no layout engine in unit tests, so these read the style sheets and check
 * the declarations that keep text and controls inside the card box.
 *
 * Cause of the clip (found with a 340 px card in a browser): `.genie` is `overflow: hidden`, so
 * any child that is wider than the card, such as the footer once it holds a third control or a
 * long unbreakable name, makes the box scrollable, and a focus or a click on a control scrolls
 * the text under the edge. The fix keeps every flex child at `min-width: 0`, lets long names
 * break, wraps the footer, and uses `overflow: clip`, which can never be scrolled.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const css = (f: string) =>
  readFileSync(join(__dirname, f), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
const genie = css('genie-card.css');
const md = css('safe-markdown.css');

/** The declarations of every rule that has `selector` as a whole selector (alone or in a list). */
function rule(sheet: string, selector: string): string {
  const out: string[] = [];
  for (const m of sheet.matchAll(/([^{}]+)\{([^}]*)\}/g)) {
    const list = (m[1] ?? '').split(',').map((x) => x.trim());
    const want = selector.split(',').map((x) => x.trim());
    if (want.every((w) => list.includes(w))) out.push(m[2] ?? '');
  }
  if (out.length === 0) throw new Error(`no rule for ${selector}`);
  return out.join('\n');
}

describe('card text and controls stay inside the card (#523)', () => {
  it('the card box is clipped, not scrollable, so a wide child cannot shift the text', () => {
    const r = rule(genie, '.genie');
    expect(r).toMatch(/overflow:\s*hidden;\s*overflow:\s*clip;/);
    expect(rule(genie, '.genie-answer,\n.genie-failure,\n.genie-message')).toMatch(
      /overflow:\s*clip/,
    );
  });

  it('every flex level of the answer can shrink below its content', () => {
    for (const sel of ['.genie-body', '.genie-md', '.genie-answer-body', '.genie-foot']) {
      expect(rule(genie, sel), sel).toMatch(/min-width:\s*0/);
    }
  });

  it('answer text, list items and inline code break inside the card', () => {
    const body = rule(genie, '.genie-body [data-answer]');
    expect(body).toMatch(/overflow-wrap:\s*anywhere/);
    expect(body).toMatch(/word-break:\s*break-word/);
    const parts = rule(
      genie,
      '.genie-body [data-answer] p,\n.genie-body [data-answer] li,\n.genie-body [data-answer] code',
    );
    expect(parts).toMatch(/overflow-wrap:\s*anywhere/);
    expect(parts).toMatch(/max-width:\s*100%/);
  });

  it('the footer wraps before it can push the card wider, and Open in Databricks has its own line', () => {
    expect(rule(genie, '.genie-foot')).toMatch(/flex-wrap:\s*wrap/);
    expect(rule(genie, '.genie-databricks')).toMatch(/flex:\s*0 0 100%/);
  });
});

describe('tables scroll inside the card (#523)', () => {
  it('a wide table scrolls sideways in its own box and never widens the card', () => {
    const wrap = rule(md, '.md-table-wrap');
    expect(wrap).toMatch(/overflow-x:\s*auto/);
    expect(wrap).toMatch(/max-width:\s*100%/);
    // The edge fade: scroll shadows, with covers that follow the content.
    expect(wrap).toMatch(/local/);
    expect(wrap).toMatch(/scroll;/);
    expect(rule(md, '.md-table-block')).toMatch(/min-width:\s*0/);
  });

  it('cells do not squash: they keep one line, with tabular figures and aligned numbers', () => {
    expect(rule(md, '.md-table')).toMatch(/font-variant-numeric:\s*tabular-nums/);
    expect(rule(md, '.md-table th,\n.md-table td')).toMatch(/white-space:\s*nowrap/);
    expect(rule(md, ".md-table [data-align='right']")).toMatch(/text-align:\s*right/);
  });

  it('the header is muted and rows are separated by a hairline', () => {
    expect(rule(md, '.md-table th')).toMatch(/color:\s*var\(--mog-text-muted\)/);
    expect(rule(md, '.md-table tbody tr + tr td')).toMatch(
      /border-top:\s*1px solid var\(--mog-edge\)/,
    );
  });

  it('a link never relies on colour alone and keeps a visible focus ring', () => {
    expect(rule(md, 'a.md-link:focus-visible')).toMatch(/box-shadow:\s*var\(--mog-focus-ring\)/);
    expect(rule(md, 'a.md-link:focus-visible')).toMatch(/outline:\s*2px solid transparent/);
    expect(rule(md, '.md-link--blocked')).toMatch(/cursor:\s*text/);
  });
});
