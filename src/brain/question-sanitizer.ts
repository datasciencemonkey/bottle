/**
 * G-47 (#573, ADR 0097): the question sanitizer of an Ask Genie press. It runs in the brain
 * before the G-30a cap (`capQuestion`), so the text that goes in `genie_ask` has none of:
 *
 * - control characters (C0, DEL, C1). A line break or a tab becomes a space; the rest go.
 * - zero-width and bidi characters: every Unicode format character (Cf), which also holds the
 *   tag characters, plus the variation selectors and the other invisible fillers.
 * - code fences (``` or ~~~ runs, with their info string). The words inside stay.
 * - HTML tags and comments, also a tag with a quoted `>` or with no end. `3 < 5` stays.
 * - markdown links and images. `[text](url)`, `![alt](url)` and `[text][ref]` give their text;
 *   a `[ref]: url` definition goes.
 *
 * The invisible characters go first, so they cannot split a tag or a fence. The steps run again
 * until the text stops changing, so one tag inside another goes too.
 */

/**
 * Every format character (Cf: zero-width, bidi, tag characters) and every default-ignorable
 * code point (variation selectors, the grapheme joiner, the Hangul fillers, U+2060-U+206F).
 */
const INVISIBLE = /[\p{Cf}\p{Default_Ignorable_Code_Point}]/gu;
// eslint-disable-next-line no-control-regex -- G-47: these are the characters to remove.
const CONTROL = /[\u0000-\u0008\u000e-\u001f\u007f-\u009f]/g;
const LINE_BREAK = /[\t\n\v\f\r]/g;
const FENCE = /(`{3,}|~{3,})[^\s`~]*/g;
const HTML_COMMENT = /<!--[\s\S]*?(-->|$)/g;
/** A tag, also with a `>` in a quoted attribute; a tag with no end at the end of the text. */
const HTML_TAG = /<\/?[a-zA-Z](?:[^<>"']|"[^"]*"|'[^']*')*>/g;
const HTML_TAG_OPEN = /<\/?[a-zA-Z][^<>]*$/;
const LINK_DEFINITION = /(^|\s)\[[^\]]*\]:\s*\S+/g;
/** A link or image; its URL can hold one level of brackets. */
const LINK = /!?\[([^\]]*)\]\((?:[^()]|\([^()]*\))*\)/g;
const REF_LINK = /!?\[([^\]]*)\]\[[^\]]*\]/g;
/** A bound on the passes: each pass removes text, so a real question settles in two or three. */
const MAX_PASSES = 8;

function pass(s: string): string {
  return s
    .replace(INVISIBLE, '')
    .replace(LINK_DEFINITION, '$1')
    .replace(CONTROL, '')
    .replace(LINE_BREAK, ' ')
    .replace(FENCE, ' ')
    .replace(HTML_COMMENT, '')
    .replace(HTML_TAG, '')
    .replace(HTML_TAG_OPEN, '')
    .replace(LINK, '$1')
    .replace(REF_LINK, '$1');
}

/** G-47: the sanitized question, with single spaces and no edge spaces. Can be empty. */
export function sanitizeQuestion(q: string): string {
  let s = q;
  for (let i = 0; i < MAX_PASSES; i++) {
    const next = pass(s);
    if (next === s) break;
    s = next;
  }
  return s.replace(/\s+/g, ' ').trim();
}
