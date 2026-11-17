/**
 * The distilled `answer_summary` of a Genie exchange (#38, MEM-25a, U8, D60; ADR 0061).
 *
 * The MVP distiller is deterministic and local: it turns the answer Markdown into plain text and
 * keeps the first 600 characters or less, cut at a word. It computes nothing and rounds nothing,
 * so every number in the summary is copied from the answer (UX-14a). The full answer stays in
 * `final_answer_md` on the device.
 */

/** The most characters of `answer_summary` (D60, the `genie_exchange` CHECK). */
export const SUMMARY_MAX = 600;

/** Markdown to one line of plain text. */
function plain(md: string): string {
  return (
    md
      // Fenced code blocks (SQL and others) are not summary text.
      .replace(/```[\s\S]*?(```|$)/g, ' ')
      // Table separator rows, then the cell pipes.
      .replace(/^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/gm, ' ')
      .replace(/\|/g, ' ')
      // Images and links keep their text.
      .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
      .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
      // Headings, quotes and list marks at the start of a line.
      .replace(/^\s{0,3}(#{1,6}\s+|>\s?|[-*+]\s+|\d+[.)]\s+)/gm, '')
      // Emphasis and inline code marks.
      .replace(/(\*\*|__|\*|`|~~)/g, '')
      .replace(/\s+/g, ' ')
      .trim()
  );
}

/** The summary of an answer, or `null` when there is no answer text. */
export function distilSummary(answerMd: string | undefined): string | null {
  if (answerMd === undefined) return null;
  const text = plain(answerMd);
  if (!text) return null;
  if (text.length <= SUMMARY_MAX) return text;
  const room = text.slice(0, SUMMARY_MAX - 1);
  const cut = room.lastIndexOf(' ');
  return `${(cut > SUMMARY_MAX / 2 ? room.slice(0, cut) : room).trimEnd()}…`;
}
