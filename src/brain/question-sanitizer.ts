/**
 * G-47 (#573, ADR 0097): the question sanitizer of an Ask Genie press. It runs in the brain
 * before the G-30a cap (`capQuestion`), so the text that goes in `genie_ask` has none of:
 *
 * - control characters (C0, DEL, C1). A line break or a tab becomes a space; the rest go.
 * - zero-width characters (ZWSP, ZWNJ, ZWJ, word joiner, BOM, soft hyphen and the like).
 * - bidi characters (LRM, RLM, ALM, the embeddings, overrides and isolates).
 * - code fences (``` or ~~~ runs, with their info string). The words inside stay.
 * - HTML tags and comments. A plain `3 < 5` is not a tag and stays.
 * - markdown links and images. `[text](url)`, `![alt](url)` and `[text][ref]` give their text;
 *   a `[ref]: url` definition goes.
 *
 * The invisible characters go first, so they cannot split a tag or a fence. The steps run again
 * until the text stops changing, so one tag inside another goes too.
 */

const INVISIBLE = /[\u00ad\u061c\u180e\u200b-\u200f\u202a-\u202e\u2060-\u206f\ufeff]/g;
// eslint-disable-next-line no-control-regex -- G-47: these are the characters to remove.
const CONTROL = /[\u0000-\u0008\u000e-\u001f\u007f-\u009f]/g;
const LINE_BREAK = /[\t\n\v\f\r]/g;
const FENCE = /(`{3,}|~{3,})[^\s`~]*/g;
const HTML_COMMENT = /<!--[\s\S]*?(-->|$)/g;
const HTML_TAG = /<\/?[a-zA-Z][^<>]*>/g;
const LINK_DEFINITION = /(^|\s)\[[^\]]*\]:\s*\S+/g;
const LINK = /!?\[([^\]]*)\]\([^)]*\)/g;
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
