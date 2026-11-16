/**
 * The expanded card sections (UX-18, Table UX-EXP, ADR 0043). The brain builds the list and the
 * view renders it in the order it gets (ARC-2). The order is fixed: full answer, key numbers,
 * SQL (collapsed), table preview, "Sent to Genie", "Resend without context", Copy as Markdown or
 * CSV, footer, "Where your data goes". A section with no content is left out. The footer and
 * "Where your data goes" always show.
 */

export const SECTION_ORDER = [
  'answer',
  'keyNumbers',
  'sql',
  'tablePreview',
  'sentToGenie',
  'resend',
  'copy',
  'footer',
  'dataFlow',
] as const;

export type SectionId = (typeof SECTION_ORDER)[number];

/**
 * Table preview caps. 50 rows is the `maxRows` of `getQueryResult` (Section 4.7), so the preview
 * shows the whole result the bridge fetches. 20 columns and 200 characters a cell are our limits
 * for a card that is a few hundred pixels wide (ADR 0043). The copy keeps every row and cell.
 */
export const PREVIEW_ROWS = 50;
export const PREVIEW_COLUMNS = 20;
export const PREVIEW_CELL_CHARS = 200;

export interface Table {
  columns: string[];
  rows: string[][];
}

export interface SectionInput {
  answerMd?: string;
  /** Key numbers, verbatim from the answer (AT-09). */
  keyNumbers?: string[];
  sql?: string;
  table?: Table;
  /** The "Sent to Genie" text: what context went with the question. Set only when it was sent. */
  sentToGenie?: string;
  /** True when "Resend without context" applies (a preamble was sent, or POLICY_DENIED). */
  canResend?: boolean;
  sttBadge?: string;
  deepLink?: string;
  /** The "Where your data goes" text. */
  dataFlowText: string;
}

export type ExpandedSection =
  | { id: 'answer'; md: string }
  | { id: 'keyNumbers'; items: string[] }
  | { id: 'sql'; sql: string; collapsed: true }
  | {
      id: 'tablePreview';
      columns: string[];
      rows: string[][];
      moreRows: number;
      moreColumns: number;
    }
  | { id: 'sentToGenie'; text: string }
  | { id: 'resend' }
  | { id: 'copy'; formats: ('markdown' | 'csv')[] }
  | { id: 'footer'; sttBadge?: string; deepLink?: string }
  | { id: 'dataFlow'; text: string };

export function buildSections(i: SectionInput): ExpandedSection[] {
  const out: ExpandedSection[] = [];
  if (i.answerMd) out.push({ id: 'answer', md: i.answerMd });
  if (i.keyNumbers?.length) out.push({ id: 'keyNumbers', items: i.keyNumbers });
  if (i.sql) out.push({ id: 'sql', sql: i.sql, collapsed: true });
  if (i.table) {
    const rows = i.table.rows.slice(0, PREVIEW_ROWS);
    const cell = (c: string) =>
      c.length > PREVIEW_CELL_CHARS ? `${c.slice(0, PREVIEW_CELL_CHARS - 1)}\u2026` : c;
    const cut = (r: string[]) => r.slice(0, PREVIEW_COLUMNS).map(cell);
    out.push({
      id: 'tablePreview',
      columns: cut(i.table.columns),
      rows: rows.map(cut),
      moreRows: i.table.rows.length - rows.length,
      moreColumns: Math.max(0, i.table.columns.length - PREVIEW_COLUMNS),
    });
  }
  if (i.sentToGenie) out.push({ id: 'sentToGenie', text: i.sentToGenie });
  if (i.canResend) out.push({ id: 'resend' });
  if (i.table) out.push({ id: 'copy', formats: ['markdown', 'csv'] });
  out.push({
    id: 'footer',
    ...(i.sttBadge !== undefined ? { sttBadge: i.sttBadge } : {}),
    ...(i.deepLink !== undefined ? { deepLink: i.deepLink } : {}),
  });
  out.push({ id: 'dataFlow', text: i.dataFlowText });
  return out;
}

const mdCell = (s: string) => s.replace(/[\\`|]/g, '\\$&').replace(/\r?\n/g, ' ');

export function tableToMarkdown(t: Table): string {
  const row = (cells: string[]) => `| ${cells.map(mdCell).join(' | ')} |`;
  return [row(t.columns), row(t.columns.map(() => '---')), ...t.rows.map(row)].join('\n');
}

/** A whole cell that is a plain number: the only cell that may start with `-` or `+` unguarded. */
const PURE_NUMBER = /^[-+]?(\d+(\.\d*)?|\.\d+)([eE][-+]?\d+)?$/;

/** A cell that starts with `=`, `+`, `-`, `@`, tab or CR gets a `'` first, so a sheet never runs it. */
const csvCell = (s: string) => {
  const safe = /^[=+\-@\t\r]/.test(s) && !PURE_NUMBER.test(s) ? `'${s}` : s;
  return /[",\r\n]/.test(safe) ? `"${safe.replace(/"/g, '""')}"` : safe;
};

export function tableToCsv(t: Table): string {
  return [t.columns, ...t.rows].map((r) => r.map(csvCell).join(',')).join('\r\n');
}
