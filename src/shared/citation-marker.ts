/**
 * ADR 0122: a citation marker is `[n]` (n is 1 to 3 digits) that is not the label of a link
 * `[n](url)`. The brain and the view use this one pattern. Use it with `matchAll` (it is global).
 */
export const CITATION_MARKER = /\[(\d{1,3})\](?!\()/g;
