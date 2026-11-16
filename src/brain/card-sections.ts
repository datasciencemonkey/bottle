/**
 * The expanded card sections (UX-18, Table UX-EXP, ADR 0032). The brain builds the list and the
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

/** Rows in the table preview. The copy keeps every row. */
export const PREVIEW_ROWS = 10;

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
  | { id: 'tablePreview'; columns: string[]; rows: string[][]; moreRows: number }
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
    out.push({
      id: 'tablePreview',
      columns: i.table.columns,
      rows,
      moreRows: i.table.rows.length - rows.length,
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

const mdCell = (s: string) => s.replace(/\|/g, '\\|').replace(/\r?\n/g, ' ');

export function tableToMarkdown(t: Table): string {
  const row = (cells: string[]) => `| ${cells.map(mdCell).join(' | ')} |`;
  return [row(t.columns), row(t.columns.map(() => '---')), ...t.rows.map(row)].join('\n');
}

/** A cell that starts with a formula character gets a `'` first, so a sheet never runs it. */
const csvCell = (s: string) => {
  const safe = /^([=+@\t\r]|-(?![\d.]))/.test(s) ? `'${s}` : s;
  return /[",\r\n]/.test(safe) ? `"${safe.replace(/"/g, '""')}"` : safe;
};

export function tableToCsv(t: Table): string {
  return [t.columns, ...t.rows].map((r) => r.map(csvCell).join(',')).join('\r\n');
}
