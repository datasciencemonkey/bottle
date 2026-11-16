/**
 * G-47: the question sanitizer. It removes control, zero-width and bidi characters,
 * code fences, HTML and markdown links. It runs to a fixpoint, so that nested markup
 * cannot rebuild itself after one pass.
 */

/** C0 and C1 controls except tab and newline, all Unicode format characters (Cf: zero-width, bidi, tags, soft hyphen). */
// eslint-disable-next-line no-control-regex -- G-47: matching control characters is the purpose of this pattern.
const INVISIBLE =
  /[\u0000-\u0008\u000B-\u001F\u007F-\u009F\p{Cf}\u00AD\u061C\u180E\u200B-\u200F\u202A-\u202E\u2060-\u2064\u2066-\u2069\uFEFF]/gu;

import { stripMarkdownLinks } from './links.ts';

const SCRIPT_OR_STYLE_OPEN = /<(script|style)\b[^<>]*>/gi;
const CLOSE_TAG = { script: /<\/script\b[^<>]*>/gi, style: /<\/style\b[^<>]*>/gi } as const;
const HTML_COMMENT = /<!--[\s\S]*?(?:-->|$)/g;
const HTML_TAG = /<\/?[A-Za-z!?][^<>]*>/g; // also covers autolinks such as <https://...>
// An unclosed tag start that is left after the passes. A whole run of "<" goes, so that
// "<<img" cannot turn into "<img" when only the last "<" is removed.
const STRAY_TAG_OPEN = /(?<!<)<+(?=[A-Za-z!/?])/g; // the lookbehind keeps it linear on long "<" runs
// Every character that can open HTML, a markdown link or a code fence. Used to fail
// closed when the passes do not reach a fixpoint.
const MARKUP_CHARS = /[<>[\]`~]/g;
const FENCE_LINE = /^[ \t]*(?:`{3,}[^`\n]*|~{3,}.*)$/gm; // a fence line with its info string
const FENCE_RUN = /`{3,}|~{3,}/g; // a fence run inside a line

const MAX_PASSES = 32;

/** Removes control, zero-width and bidi characters (G-47). Exported for the secret check. */
export function stripInvisible(raw: string): string {
  return raw.replace(/\r\n?|[\u2028\u2029]/g, '\n').replace(INVISIBLE, '');
}

/**
 * Removes script and style elements with their content in linear time. An unclosed
 * element is removed to the end of the text, as a browser would treat it.
 */
function removeScriptAndStyle(s: string): string {
  let out = '';
  let pos = 0;
  SCRIPT_OR_STYLE_OPEN.lastIndex = 0;
  for (let m = SCRIPT_OR_STYLE_OPEN.exec(s); m; m = SCRIPT_OR_STYLE_OPEN.exec(s)) {
    if (m.index < pos) continue;
    out += s.slice(pos, m.index);
    const close = CLOSE_TAG[(m[1] ?? 'script').toLowerCase() as keyof typeof CLOSE_TAG];
    close.lastIndex = m.index + m[0].length;
    const end = close.exec(s);
    pos = end ? end.index + end[0].length : s.length;
    SCRIPT_OR_STYLE_OPEN.lastIndex = pos;
  }
  return out + s.slice(pos);
}

function markupPass(s: string): string {
  return stripMarkdownLinks(removeScriptAndStyle(s).replace(HTML_COMMENT, '').replace(HTML_TAG, ''))
    .replace(FENCE_LINE, '')
    .replace(FENCE_RUN, '');
}

function normalizeSpace(s: string): string {
  return s
    .replace(/[^\S\n\t]/g, ' ')
    .replace(/ {2,}/g, ' ')
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n[ \t]+/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

/**
 * G-47: returns the question text with no banned characters or markup. Nesting can need
 * one pass per layer, so after MAX_PASSES without a fixpoint it fails closed: every
 * character that could still form a tag, a link or a fence is removed.
 */
export function sanitizeQuestion(raw: string): string {
  let s = stripInvisible(raw);
  let stable = false;
  for (let i = 0; i < MAX_PASSES && !stable; i++) {
    const before = s;
    s = markupPass(s);
    if (s === before) {
      s = s.replace(STRAY_TAG_OPEN, '');
      stable = s === before;
    }
  }
  if (!stable) s = s.replace(MARKUP_CHARS, '');
  return normalizeSpace(s);
}
