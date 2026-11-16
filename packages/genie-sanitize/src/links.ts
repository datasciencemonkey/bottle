/**
 * G-47: removes markdown links in linear time. A regex cannot match CommonMark links
 * with balanced brackets in the text or balanced parentheses in the destination, so
 * this is a small scanner over precomputed bracket and parenthesis matches.
 *
 * Handled forms (the visible text stays, the destination goes):
 * - inline links and images: [text](dest "title"), ![alt](<dest with spaces>)
 * - full and collapsed reference links: [text][label], [text][]
 * - reference definitions at the start of a line (after any quote or list markers),
 *   with the destination on the same or the next line: [label]: dest "title"
 * - a stray "](dest)" that is left without its opening bracket.
 *
 * It errs on the side of removal: a parenthesised group right after "]" is treated as
 * a destination even when CommonMark would not render it as one.
 */

/** Link text deeper than this is dropped instead of scanned, which also bounds recursion. */
const MAX_DEPTH = 64;

/**
 * Characters that may lead a line before a reference definition: indentation, block
 * quote markers and list markers ("-", "*", "+", "1.", "1)"). Erring towards removal,
 * any mix of them counts.
 */
const LINE_PREFIX = /^[ \t>*+\-0-9.)]$/;

interface Index {
  readonly s: string;
  /** Matching "]" for each "[" (or -1). */
  readonly bracketClose: Int32Array;
  /** Matching ")" for each "(" (or -1). */
  readonly parenClose: Int32Array;
  /** The first unescaped ")" at or after each position (or -1). */
  readonly nextParen: Int32Array;
  /** The first ">" at or after each position (or -1). */
  readonly nextGt: Int32Array;
  /** The first "\n" at or after each position (or the text length). */
  readonly nextNewline: Int32Array;
}

function buildIndex(s: string): Index {
  const n = s.length;
  const escaped = new Uint8Array(n);
  const bracketClose = new Int32Array(n).fill(-1);
  const parenClose = new Int32Array(n).fill(-1);
  const brackets: number[] = [];
  const parens: number[] = [];
  for (let i = 0; i < n; i++) {
    const c = s[i];
    if (c === '\\' && i + 1 < n) {
      escaped[i + 1] = 1;
      i++;
    } else if (c === '[') brackets.push(i);
    else if (c === ']') {
      const open = brackets.pop();
      if (open !== undefined) bracketClose[open] = i;
    } else if (c === '(') parens.push(i);
    else if (c === ')') {
      const open = parens.pop();
      if (open !== undefined) parenClose[open] = i;
    }
  }
  const nextParen = new Int32Array(n + 1);
  const nextGt = new Int32Array(n + 1);
  const nextNewline = new Int32Array(n + 1);
  nextParen[n] = -1;
  nextGt[n] = -1;
  nextNewline[n] = n;
  for (let i = n - 1; i >= 0; i--) {
    const c = s[i];
    nextParen[i] = c === ')' && !escaped[i] ? i : (nextParen[i + 1] ?? -1);
    nextGt[i] = c === '>' ? i : (nextGt[i + 1] ?? -1);
    nextNewline[i] = c === '\n' ? i : (nextNewline[i + 1] ?? n);
  }
  return { s, bracketClose, parenClose, nextParen, nextGt, nextNewline };
}

function at(a: Int32Array, i: number, fallback: number): number {
  return i < a.length ? (a[i] ?? fallback) : fallback;
}

/** End of the "(dest "title")" group that starts at `p`, or -1 when it does not close before `limit`. */
function destinationEnd(ix: Index, p: number, limit: number): number {
  let from = p + 1;
  if (ix.s[from] === '<') {
    const gt = at(ix.nextGt, from, -1);
    if (gt >= 0 && gt < at(ix.nextNewline, from, ix.s.length)) from = gt + 1;
  }
  const balanced = from === p + 1 ? at(ix.parenClose, p, -1) : -1;
  const end = balanced >= 0 ? balanced : at(ix.nextParen, from, -1);
  return end >= 0 && end < limit ? end : -1;
}

function scan(ix: Index, from: number, to: number, depth: number): string {
  const { s } = ix;
  const out: string[] = [];
  let lineBlank = from === 0 || s[from - 1] === '\n';
  let i = from;
  while (i < to) {
    const c = s[i] as string;
    if (c === '\\' && i + 1 < to) {
      out.push(c, s[i + 1] as string);
      i += 2;
      lineBlank = false;
      continue;
    }
    if (c === '[') {
      const j = at(ix.bracketClose, i, -1);
      if (j >= 0 && j < to) {
        const next = j + 1 < to ? s[j + 1] : undefined;
        if (lineBlank && next === ':') {
          // A reference definition: drop the line, and the next one when the destination is there.
          let k = Math.min(at(ix.nextNewline, j + 2, s.length), to);
          if (s.slice(j + 2, k).trim() === '' && k < to) k = Math.min(at(ix.nextNewline, k + 1, s.length), to);
          i = k;
          continue;
        }
        let end = -1;
        if (next === '(') end = destinationEnd(ix, j + 1, to);
        else if (next === '[') {
          const label = at(ix.bracketClose, j + 1, -1);
          end = label >= 0 && label < to ? label : -1;
        }
        if (end >= 0) {
          if (out[out.length - 1] === '!') out.pop(); // an image
          if (depth < MAX_DEPTH) out.push(scan(ix, i + 1, j, depth + 1));
          i = end + 1;
          lineBlank = false;
          continue;
        }
      }
    } else if (c === ']' && i + 1 < to && s[i + 1] === '(') {
      const end = destinationEnd(ix, i + 1, to);
      if (end >= 0) {
        i = end + 1;
        continue;
      }
    }
    if (c === '\n') lineBlank = true;
    else if (lineBlank && LINE_PREFIX.test(c)) {
      // Spaces, quote markers and list markers may come before a definition.
    } else lineBlank = false;
    out.push(c);
    i++;
  }
  return out.join('');
}

/** G-47: replaces markdown links, images and reference links with their text and removes definitions. */
export function stripMarkdownLinks(s: string): string {
  if (!s.includes('[') && !s.includes(']')) return s;
  return scan(buildIndex(s), 0, s.length, 0);
}
