/**
 * SEC-25a/SEC-25b: strict allowlist markdown renderer. No HTML, no images, trusted links only.
 *
 * Links (#523): a link is drawn only if the trust rule passes (`src/shared/link-trust.ts`: the
 * workspace host or `AllowedAppUrls`). Its label is the clickable text; an empty label shows a
 * short form of the URL; a small icon and a `title` show that it leaves the app and where. Bare
 * `https://` URLs in the text become links under the same rule. A click never navigates the
 * window: it goes to `onOpenLink`, and the brain and Rust check the URL again before the browser
 * opens. A link to any other host is plain text with its host visible and is never clickable.
 *
 * Tables (#523): GFM tables render as a real `<table>` with alignment, in a scroll box. Rows and
 * columns are capped. A cell shows long floats with thousands separators (`table-numbers.ts`).
 */
import { Fragment, type MouseEvent, type ReactNode } from 'react';
import { ExternalLink } from 'lucide-react';
import {
  displayHost,
  NO_LINKS,
  shortUrl,
  trustedUrl,
  type LinkTrust,
} from '../../shared/link-trust';
import { CITATION_MARKER } from '../../shared/citation-marker';
import { formatColumn } from './table-numbers';
import './safe-markdown.css';

export { NO_LINKS, trustedUrl };
export type { LinkTrust };

/** Answers longer than this are cut before parsing (bounds work on hostile input). */
export const MAX_ANSWER_CHARS = 20_000;

/** The most body rows and columns of one table. More rows are counted, not drawn. */
export const MAX_TABLE_ROWS = 50;
export const MAX_TABLE_COLS = 20;

// Every group is length-bounded so unmatched openers cost O(n), not O(n^2).
export const INLINE =
  /(!?)\[([^\]]{0,500})\]\(([^)\s]{0,2048})\)|`([^`]{1,500})`|\*\*([^*]{1,500})\*\*|\*([^*]{1,500})\*|(https?:\/\/[^\s<>"]{1,2048})/g;

/** Trailing punctuation that belongs to the sentence, not to a bare URL. */
function trimUrl(raw: string): { url: string; rest: string } {
  // Count the brackets once, then walk back from the end: linear, however many `)` end the URL.
  let opens = 0;
  let closes = 0;
  for (let i = 0; i < raw.length; i++) {
    const c = raw.charCodeAt(i);
    if (c === 40) opens++;
    else if (c === 41) closes++;
  }
  let end = raw.length;
  while (end > 0) {
    const c = raw[end - 1] ?? '';
    if (/[.,;:!?'"\]}]/.test(c)) end--;
    else if (c === ')' && closes > opens) {
      closes--;
      end--;
    } else break;
  }
  return { url: raw.slice(0, end), rest: raw.slice(end) };
}

interface Ctx {
  trust: LinkTrust;
  onOpenLink: ((url: string) => void) | undefined;
  locale: string | undefined;
  maxTableRows: number;
  overflowHint: string | undefined;
  renderMarker: ((n: number) => ReactNode | undefined) | undefined;
}

/** The last characters of a link label that stay on one line with the icon. */
const TAIL_CHARS = 6;

function TrustedLink({ href, label, ctx }: { href: string; label: string; ctx: Ctx }) {
  const onClick = (e: MouseEvent<HTMLAnchorElement>) => {
    // The window never navigates. The second click of a double-click opens nothing more.
    e.preventDefault();
    if (e.detail > 1) return;
    ctx.onOpenLink?.(href);
  };
  return (
    <a
      className="md-link"
      href={href}
      rel="noopener noreferrer"
      title={href}
      onClick={onClick}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      <span className="md-link-text">
        {label.slice(0, -TAIL_CHARS)}
        {/* The icon stays with the last characters, so it never wraps alone onto a new line. */}
        <span className="md-link-tail">
          {label.slice(-TAIL_CHARS)}
          <ExternalLink className="md-link-icon" size={12} strokeWidth={2} aria-hidden="true" />
        </span>
      </span>
    </a>
  );
}

/** A link to another host: plain text, its host visible, never clickable. */
function BlockedLink({ label, raw }: { label: string; raw: string }) {
  const host = displayHost(raw);
  if (host === undefined) return <>{label}</>;
  const sameAsUrl = label === '' || label === raw;
  return (
    <span className="md-link md-link--blocked" title={`Not opened: ${host} is not your workspace`}>
      {sameAsUrl ? shortUrl(raw) : label}
      {!sameAsUrl && <span className="md-link-host"> ({host})</span>}
    </span>
  );
}

function linkNode(label: string, raw: string, ctx: Ctx, key: string): ReactNode {
  const href = trustedUrl(raw, ctx.trust);
  if (href === undefined) return <BlockedLink key={key} label={label} raw={raw} />;
  return (
    <TrustedLink
      key={key}
      href={href}
      label={label !== '' && label !== raw ? label : shortUrl(href)}
      ctx={ctx}
    />
  );
}

/** Plain text with the `[n]` markers that the caller knows drawn by `renderMarker`. */
function plain(text: string, ctx: Ctx, key: string): ReactNode[] {
  const draw = ctx.renderMarker;
  if (!draw) return [text];
  const out: ReactNode[] = [];
  let last = 0;
  for (const m of text.matchAll(CITATION_MARKER)) {
    const node = draw(Number(m[1]));
    if (node === undefined) continue;
    if (m.index > last) out.push(text.slice(last, m.index));
    out.push(<Fragment key={`${key}-${m.index}`}>{node}</Fragment>);
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

function inline(text: string, ctx: Ctx, key: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  let i = 0;
  for (const m of text.matchAll(INLINE)) {
    if (m.index > last) out.push(...plain(text.slice(last, m.index), ctx, `${key}-t${i}`));
    const k = `${key}-${i++}`;
    let end = m.index + m[0].length;
    if (m[4] !== undefined) out.push(<code key={k}>{m[4]}</code>);
    else if (m[5] !== undefined) out.push(<strong key={k}>{m[5]}</strong>);
    else if (m[6] !== undefined) out.push(<em key={k}>{m[6]}</em>);
    else if (m[7] !== undefined) {
      const { url, rest } = trimUrl(m[7]);
      out.push(<Fragment key={k}>{linkNode('', url, ctx, `${k}-l`)}</Fragment>);
      end -= rest.length;
    } else if (m[1] === '!')
      out.push(m[2]); // image: alt text only, never a fetch
    else out.push(linkNode(m[2] ?? '', m[3] ?? '', ctx, k));
    last = end;
  }
  if (last < text.length) out.push(...plain(text.slice(last), ctx, `${key}-e`));
  return out;
}

// ---- tables ----

type Align = 'left' | 'right' | 'center' | undefined;

/**
 * Split one table row into cells. `\|` is a pipe in the cell, a pipe inside an inline code span
 * stays in the cell, and the pipes at the edges are dropped. One pass, so it is linear. At most
 * `max + 1` cells are returned (the extra one tells the caller that there are more).
 */
export function splitRow(line: string, max = MAX_TABLE_COLS): string[] {
  let s = line.trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|') && !s.endsWith('\\|')) s = s.slice(0, -1);
  const scan = (honourCode: boolean): { cells: string[]; open: boolean } => {
    const cells: string[] = [];
    let cur = '';
    let inCode = false;
    for (let i = 0; i < s.length; i++) {
      const c = s[i] ?? '';
      if (c === '\\' && s[i + 1] === '|') {
        cur += '|';
        i++;
      } else if (c === '`' && honourCode) {
        inCode = !inCode;
        cur += c;
      } else if (c === '|' && !inCode) {
        cells.push(cur.trim());
        cur = '';
        if (cells.length > max) return { cells, open: false };
      } else cur += c;
    }
    cells.push(cur.trim());
    return { cells, open: inCode };
  };
  let r = scan(true);
  // A lone backtick is no code span: split on every pipe again.
  if (r.open) r = scan(false);
  return r.cells;
}

/** The alignment of a separator cell (`---`, `:--`, `--:`, `:-:`), or false if it is not one. */
function sepAlign(cell: string): Align | false {
  const c = cell.trim();
  if (!/^:?-+:?$/.test(c)) return false;
  const l = c.startsWith(':');
  const r = c.endsWith(':');
  return l && r ? 'center' : r ? 'right' : l ? 'left' : undefined;
}

/** The separator row of a table, as alignments, or undefined if `line` is not one. */
function separator(line: string): Align[] | undefined {
  if (!line.includes('-') || line.length > MAX_ANSWER_CHARS) return undefined;
  const cells = splitRow(line, MAX_TABLE_COLS);
  const out: Align[] = [];
  for (const c of cells) {
    const a = sepAlign(c);
    if (a === false) return undefined;
    out.push(a);
  }
  return out.length > 0 ? out : undefined;
}

function Table({
  head,
  aligns,
  rows,
  more,
  ctx,
  k,
}: {
  head: string[];
  aligns: Align[];
  rows: string[][];
  more: number;
  ctx: Ctx;
  k: string;
}) {
  const cols = head.length;
  // One decision per column (`table-numbers.ts`): a column is numeric only if every cell is.
  const shown = head.map((h, c) =>
    formatColumn(
      h,
      rows.map((r) => r[c] ?? ''),
      ctx.locale,
    ),
  );
  const numericCol = shown.map((col) => {
    const filled = col.filter((n) => n.text.trim() !== '');
    return filled.length > 0 && filled.every((n) => n.numeric);
  });
  const alignOf = (c: number): 'left' | 'right' | 'center' =>
    aligns[c] ?? (numericCol[c] ? 'right' : 'left');
  return (
    <div className="md-table-block">
      <div className="md-table-wrap" role="region" aria-label="Table" tabIndex={0}>
        <table className="md-table">
          <thead>
            <tr>
              {head.map((h, c) => (
                <th key={c} scope="col" data-align={alignOf(c)}>
                  {inline(h, ctx, `${k}-h${c}`)}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((r, ri) => (
              <tr key={ri}>
                {Array.from({ length: cols }, (_, c) => {
                  const raw = r[c] ?? '';
                  const n = shown[c]?.[ri] ?? { text: raw, changed: false, numeric: false };
                  return (
                    <td
                      key={c}
                      data-align={alignOf(c)}
                      {...(n.changed ? { title: raw.trim() } : {})}
                    >
                      {n.numeric ? n.text : inline(raw, ctx, `${k}-${ri}-${c}`)}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {more > 0 && (
        <p className="md-table-more" data-table-more>
          {more.toLocaleString(ctx.locale)} more {more === 1 ? 'row' : 'rows'}
          {ctx.overflowHint ? `. ${ctx.overflowHint}` : ''}
        </p>
      )}
    </div>
  );
}

export interface SafeMarkdownProps {
  text: string;
  trust: LinkTrust;
  /** A click on a trusted link. The caller sends it to the brain; the view never opens a URL. */
  onOpenLink?: (url: string) => void;
  /** The locale of table numbers (default: the user's). */
  locale?: string;
  /** The most body rows of a table (default {@link MAX_TABLE_ROWS}). */
  maxTableRows?: number;
  /** What the "N more rows" line adds, for example "Open in Bottle to see all of them". */
  overflowHint?: string;
  /**
   * ADR 0122: draws one `[n]` of the text. It returns undefined for an `n` the caller does not
   * know; the text `[n]` then stays as it is. The marker is a React node, never raw HTML.
   */
  renderMarker?: (n: number) => ReactNode | undefined;
}

const LIST = /^\s*([-*]|\d+\.)\s+/;

export function SafeMarkdown({
  text,
  trust,
  onOpenLink,
  locale,
  maxTableRows = MAX_TABLE_ROWS,
  overflowHint,
  renderMarker,
}: SafeMarkdownProps) {
  const ctx: Ctx = { trust, onOpenLink, locale, maxTableRows, overflowHint, renderMarker };
  const lines = text.slice(0, MAX_ANSWER_CHARS).split(/\r?\n/);
  const blocks: ReactNode[] = [];
  let n = 0;
  let i = 0;
  const tableAt = (at: number): Align[] | undefined => {
    const head = lines[at] ?? '';
    if (!head.includes('|')) return undefined;
    const aligns = separator(lines[at + 1] ?? '');
    if (!aligns) return undefined;
    const cols = splitRow(head, MAX_TABLE_COLS).length;
    return cols === aligns.length ? aligns : undefined;
  };
  while (i < lines.length) {
    const line = lines[i] ?? '';
    const key = `b${n++}`;
    let aligns: Align[] | undefined;
    if (line.startsWith('```')) {
      const code: string[] = [];
      for (i++; i < lines.length && !(lines[i] ?? '').startsWith('```'); i++)
        code.push(lines[i] ?? '');
      i++;
      blocks.push(
        <pre key={key}>
          <code>{code.join('\n')}</code>
        </pre>,
      );
    } else if ((aligns = tableAt(i))) {
      const head = splitRow(line, MAX_TABLE_COLS).slice(0, MAX_TABLE_COLS);
      const rows: string[][] = [];
      let total = 0;
      for (i += 2; i < lines.length && (lines[i] ?? '').includes('|'); i++) {
        total++;
        if (rows.length < maxTableRows)
          rows.push(splitRow(lines[i] ?? '', MAX_TABLE_COLS).slice(0, head.length));
      }
      blocks.push(
        <Table
          key={key}
          head={head}
          aligns={aligns.slice(0, head.length)}
          rows={rows}
          more={total - rows.length}
          ctx={ctx}
          k={key}
        />,
      );
    } else if (LIST.test(line)) {
      const ordered = /^\s*\d+\./.test(line);
      const items: ReactNode[] = [];
      while (i < lines.length && LIST.test(lines[i] ?? '')) {
        const item = (lines[i] ?? '').replace(LIST, '');
        items.push(<li key={`${key}-${i}`}>{inline(item, ctx, `${key}-${i}`)}</li>);
        i++;
      }
      blocks.push(ordered ? <ol key={key}>{items}</ol> : <ul key={key}>{items}</ul>);
    } else if (/^#{1,6}\s+/.test(line)) {
      blocks.push(
        <p key={key}>
          <strong>{inline(line.replace(/^#{1,6}\s+/, ''), ctx, key)}</strong>
        </p>,
      );
      i++;
    } else if (line.trim() === '') {
      i++;
    } else {
      const para: string[] = [];
      while (
        i < lines.length &&
        (lines[i] ?? '').trim() !== '' &&
        !(lines[i] ?? '').startsWith('```') &&
        !LIST.test(lines[i] ?? '') &&
        !/^#{1,6}\s+/.test(lines[i] ?? '') &&
        !(para.length > 0 && tableAt(i))
      ) {
        para.push(lines[i] ?? '');
        i++;
      }
      blocks.push(<p key={key}>{inline(para.join(' '), ctx, key)}</p>);
    }
  }
  return <>{blocks}</>;
}
