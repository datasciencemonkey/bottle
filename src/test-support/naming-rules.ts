// Naming rules for UI text and package metadata (#71, MB-10a, MB-10b, MB-10c, AT-37, ADR 0030).
//
//   bannedNameHits(text): each former product name in `text` (MB-10a). The pattern is built from
//     parts, so this file does not hold the name it bans.
//   mogNounHits(text): each spelling of the noun other than "MOG" / "MOGs" (MB-10c). Identifiers
//     and paths such as `mog-core`, `mog-<id>`, `mog.html` or `mog:view_state` are not nouns.
//   markdownProse(md): the prose of a Markdown file without code fences and code spans.
//   htmlTitle(html): the `<title>` text of an HTML page.
//   rustWindowTitles(src): the `title: "..."` strings of the Rust window specs.
//   visibleTexts(el): each text node and each a11y or tooltip attribute of a rendered view, one
//     string each, so text in neighbouring elements never runs together.

export const PRODUCT_NAME = 'Bottle';
export const EXPERIMENTAL_LABEL = 'experimental';

// A separator is a space, `_`, `-` or an HTML space entity such as `&nbsp;` or `&#160;`.
const SEP = '(?:[\\s_-]|&nbsp;|&#160;|&#x0*a0;|&#32;)*';
const BANNED = new RegExp(['mo', 'ther', SEP, 'bri', 'cks'].join(''), 'gi');
const NOUN = /(?<![\w./:<>-])mogs?(?![\w/:<>-]|\.\w)/gi;

export function bannedNameHits(text: string): string[] {
  return text.match(BANNED) ?? [];
}

export function mogNounHits(text: string): string[] {
  return (text.match(NOUN) ?? []).filter((w) => w !== 'MOG' && w !== 'MOGs');
}

export function markdownProse(md: string): string {
  return md.replace(/^(```|~~~)[\s\S]*?^\1/gm, '').replace(/`[^`\n]*`/g, '');
}

export function htmlTitle(html: string): string | undefined {
  return /<title>([\s\S]*?)<\/title>/i.exec(html)?.[1]?.trim();
}

export function rustWindowTitles(src: string): string[] {
  return [...src.matchAll(/\btitle:\s*"([^"]*)"/g)].map((m) => m[1] ?? '');
}

const ATTRS = ['aria-label', 'title', 'alt', 'placeholder'];

export function visibleTexts(el: Element): string[] {
  const copy = el.cloneNode(true) as Element;
  // Third-party licence text is verbatim (component names, file paths); MB-10a still scans it.
  copy.querySelectorAll('[data-notices]').forEach((n) => n.remove());
  const doc = copy.ownerDocument;
  const walker = doc.createTreeWalker(copy, 4 /* NodeFilter.SHOW_TEXT */);
  const out: string[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) out.push(n.textContent ?? '');
  for (const n of [copy, ...copy.querySelectorAll('*')]) {
    for (const a of ATTRS) out.push(n.getAttribute(a) ?? '');
  }
  return out.filter((t) => t.trim() !== '');
}
