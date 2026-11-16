// Accessibility helpers for acceptance tests (#230, AT-20). They read the DOM only.
//
//   accessibleText(el): the text a screen reader reads for `el`: the text of the
//     `aria-labelledby` elements, else `aria-label`, else the text content (accname 1.2 order).
//     Whitespace is collapsed.
//   contrastRatio(fg, bg): the WCAG 2.x contrast ratio of two `#rrggbb` colours (UX-34a).

const squash = (s: string | null | undefined): string => (s ?? '').replace(/\s+/g, ' ').trim();

export function accessibleText(el: Element): string {
  // accname 1.2: aria-labelledby first, then aria-label, then the content.
  const ids = squash(el.getAttribute('aria-labelledby'));
  if (ids) {
    const doc = el.ownerDocument;
    const text = squash(
      ids
        .split(' ')
        .map((id) => doc.getElementById(id)?.textContent ?? '')
        .join(' '),
    );
    if (text) return text;
  }
  const label = squash(el.getAttribute('aria-label'));
  if (label) return label;
  return squash(el.textContent);
}

function luminance(hex: string): number {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex.trim());
  if (!m) throw new Error(`not a #rrggbb colour: ${hex}`);
  const [r, g, b] = m.slice(1).map((h) => {
    const c = parseInt(h, 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrastRatio(fg: string, bg: string): number {
  const [hi, lo] = [luminance(fg), luminance(bg)].sort((a, b) => b - a) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}
