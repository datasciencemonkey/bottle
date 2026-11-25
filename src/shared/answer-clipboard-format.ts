/**
 * The owner's "deliver data that is amenable to directly be pasted into... Docs or email or
 * Excel" request (ADR 0124 amendment): convert a finished Genie answer's Markdown into an HTML
 * fragment and a plain-text form for `clipboard_write_text`. Pure, no React, no IPC: a brain
 * module must not import from a view file (the same rule `answer-clipboard.ts`'s own `NOTE_MS`
 * duplication follows), so this is a second, text-only implementation of the card's allowlist
 * Markdown grammar (`src/views/card/safe-markdown.tsx`, SEC-25a/SEC-25b) — not a shared import.
 * The two differ in output (React nodes vs. strings) and in destination (an in-app card vs.
 * Word, Gmail, Docs and Excel), which is also why headings become real `<h1>`-`<h6>` here: the
 * card downgrades a heading to a bold paragraph for its own compact layout, but Word, Gmail and
 * Docs all render a real heading tag larger and bolder, which is what a pasted answer should
 * look like in them.
 *
 * HTML:
 * - bold, italic, lists and code map to `<strong>`, `<em>`, `<ul>`/`<ol>`/`<li>`, `<code>`/`<pre>`;
 * - a table becomes a real `<table>` with `<th>`/`<td>` and minimal inline borders and padding,
 *   so it reads as a table in Gmail, Outlook, Google Docs and Word, none of which reliably load
 *   external or `<style>` CSS on paste;
 * - every literal character is escaped (`escapeHtml`): an attempted injection (a `<script>` or an
 *   `on*` attribute inside the Genie text) lands as text, never as markup;
 * - a link is written only when it passes the trust rule (`trustedUrl`, the same rule the card
 *   uses): only an `https://` link to the workspace host or an allowed app host. Anything else
 *   (another scheme, an untrusted host) is plain escaped text: its label if it has one, else the
 *   raw URL text;
 * - no `<script>`, no `on*` attribute, no `<img>`: an image's alt text is kept, never a fetch.
 *
 * Plain text:
 * - the Markdown syntax is removed (no `**`, `#`, or table pipes);
 * - a table becomes TAB-separated cells, one row per line (the header row too), so it pastes
 *   into cells in Excel or Google Sheets;
 * - a link becomes "label (url)" (or just the bare URL with no label). Plain text is never
 *   clickable, so every link is kept, trusted or not: the host-trust rule is an HTML-only
 *   concern.
 *
 * Citation markers (ADR 0122): stripped already, before this module ever sees the text
 * (`finishedAnswerText`, `agent-answer-rule.ts`). Nothing here looks for `[n]`.
 */
import { trustedUrl, type LinkTrust } from './link-trust';

export interface AnswerClipboardPayload {
  text: string;
  html: string;
}

// ---------- inline grammar (mirrors `safe-markdown.tsx`'s `INLINE` and `trimUrl`) ----------

// Every group is length-bounded so unmatched openers cost O(n), not O(n^2).
const INLINE =
  /(!?)\[([^\]]{0,500})\]\(([^)\s]{0,2048})\)|`([^`]{1,500})`|\*\*([^*]{1,500})\*\*|\*([^*]{1,500})\*|(https?:\/\/[^\s<>"]{1,2048})/g;

/** Trailing punctuation that belongs to the sentence, not to a bare URL. */
function trimUrl(raw: string): { url: string; rest: string } {
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

export function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function linkHtml(label: string, url: string, trust: LinkTrust): string {
  const shown = label !== '' ? label : url;
  const href = trustedUrl(url, trust);
  return href === undefined
    ? escapeHtml(shown)
    : `<a href="${escapeHtml(href)}">${escapeHtml(shown)}</a>`;
}

function inlineHtml(text: string, trust: LinkTrust): string {
  let out = '';
  let last = 0;
  for (const m of text.matchAll(INLINE)) {
    if (m.index > last) out += escapeHtml(text.slice(last, m.index));
    let end = m.index + m[0].length;
    if (m[4] !== undefined) out += `<code>${escapeHtml(m[4])}</code>`;
    else if (m[5] !== undefined) out += `<strong>${escapeHtml(m[5])}</strong>`;
    else if (m[6] !== undefined) out += `<em>${escapeHtml(m[6])}</em>`;
    else if (m[7] !== undefined) {
      const { url, rest } = trimUrl(m[7]);
      out += linkHtml('', url, trust);
      end -= rest.length;
    } else if (m[1] === '!')
      out += escapeHtml(m[2] ?? ''); // image: alt text only, never a fetch
    else out += linkHtml(m[2] ?? '', m[3] ?? '', trust);
    last = end;
  }
  out += escapeHtml(text.slice(last));
  return out;
}

function inlinePlain(text: string): string {
  let out = '';
  let last = 0;
  for (const m of text.matchAll(INLINE)) {
    if (m.index > last) out += text.slice(last, m.index);
    let end = m.index + m[0].length;
    if (m[4] !== undefined) out += m[4];
    else if (m[5] !== undefined) out += m[5];
    else if (m[6] !== undefined) out += m[6];
    else if (m[7] !== undefined) {
      const { url, rest } = trimUrl(m[7]);
      out += url;
      end -= rest.length;
    } else if (m[1] === '!') out += m[2] ?? '';
    else {
      const label = m[2] ?? '';
      const url = m[3] ?? '';
      out += label !== '' ? `${label} (${url})` : url;
    }
    last = end;
  }
  out += text.slice(last);
  return out;
}

// ---------- block grammar (mirrors `safe-markdown.tsx`'s block loop) ----------

const LIST = /^\s*([-*]|\d+\.)\s+/;
const HEADING = /^(#{1,6})\s+(.*)$/;

type BlockNode =
  | { t: 'heading'; level: number; text: string }
  | { t: 'paragraph'; text: string }
  | { t: 'list'; ordered: boolean; items: string[] }
  | { t: 'table'; head: string[]; rows: string[][] }
  | { t: 'codeBlock'; code: string };

/** Split one `|`-delimited row into cells: an escaped `\|` is a pipe in the cell, a pipe inside
 * an inline code span stays in the cell, and the pipes at the edges are dropped. */
function splitRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|') && !s.endsWith('\\|')) s = s.slice(0, -1);
  const cells: string[] = [];
  let cur = '';
  let inCode = false;
  for (let i = 0; i < s.length; i++) {
    const c = s[i] ?? '';
    if (c === '\\' && s[i + 1] === '|') {
      cur += '|';
      i++;
    } else if (c === '`') {
      inCode = !inCode;
      cur += c;
    } else if (c === '|' && !inCode) {
      cells.push(cur.trim());
      cur = '';
    } else cur += c;
  }
  cells.push(cur.trim());
  return cells;
}

/** Whether `line` is a GFM table separator row (`---`, `:--`, `--:`, `:-:`, one or more cells). */
function isSeparatorRow(line: string): boolean {
  if (!line.includes('-')) return false;
  const cells = splitRow(line);
  return cells.length > 0 && cells.every((c) => /^:?-+:?$/.test(c.trim()));
}

function parseBlocks(md: string): BlockNode[] {
  const lines = md.split(/\r?\n/);
  const blocks: BlockNode[] = [];
  let i = 0;
  const tableAt = (at: number): boolean => {
    const head = lines[at] ?? '';
    if (!head.includes('|')) return false;
    const sep = lines[at + 1] ?? '';
    return isSeparatorRow(sep) && splitRow(head).length === splitRow(sep).length;
  };
  while (i < lines.length) {
    const line = lines[i] ?? '';
    const heading = HEADING.exec(line);
    if (line.startsWith('```')) {
      const code: string[] = [];
      for (i++; i < lines.length && !(lines[i] ?? '').startsWith('```'); i++)
        code.push(lines[i] ?? '');
      i++;
      blocks.push({ t: 'codeBlock', code: code.join('\n') });
    } else if (tableAt(i)) {
      const head = splitRow(line);
      const rows: string[][] = [];
      for (i += 2; i < lines.length && (lines[i] ?? '').includes('|'); i++) {
        rows.push(splitRow(lines[i] ?? '').slice(0, head.length));
      }
      blocks.push({ t: 'table', head, rows });
    } else if (LIST.test(line)) {
      const ordered = /^\s*\d+\./.test(line);
      const items: string[] = [];
      while (i < lines.length && LIST.test(lines[i] ?? '')) {
        items.push((lines[i] ?? '').replace(LIST, ''));
        i++;
      }
      blocks.push({ t: 'list', ordered, items });
    } else if (heading) {
      blocks.push({ t: 'heading', level: heading[1]?.length ?? 1, text: heading[2] ?? '' });
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
        !HEADING.test(lines[i] ?? '') &&
        !(para.length > 0 && tableAt(i))
      ) {
        para.push(lines[i] ?? '');
        i++;
      }
      blocks.push({ t: 'paragraph', text: para.join(' ') });
    }
  }
  return blocks;
}

// ---------- HTML rendering ----------

const CELL_STYLE = 'border:1px solid #ccc;padding:4px 8px;text-align:left;';

function blockHtml(b: BlockNode, trust: LinkTrust): string {
  switch (b.t) {
    case 'heading': {
      const n = Math.min(Math.max(b.level, 1), 6);
      return `<h${n} style="margin:12px 0 6px;font-weight:600;">${inlineHtml(b.text, trust)}</h${n}>`;
    }
    case 'paragraph':
      return `<p style="margin:0 0 8px;">${inlineHtml(b.text, trust)}</p>`;
    case 'list': {
      const tag = b.ordered ? 'ol' : 'ul';
      const items = b.items.map((it) => `<li>${inlineHtml(it, trust)}</li>`).join('');
      return `<${tag} style="margin:0 0 8px;padding-left:20px;">${items}</${tag}>`;
    }
    case 'table': {
      const th = b.head
        .map(
          (h) =>
            `<th scope="col" style="${CELL_STYLE}background:#f5f5f5;">${inlineHtml(h, trust)}</th>`,
        )
        .join('');
      const body = b.rows
        .map(
          (r) =>
            `<tr>${b.head.map((_, c) => `<td style="${CELL_STYLE}">${inlineHtml(r[c] ?? '', trust)}</td>`).join('')}</tr>`,
        )
        .join('');
      return `<table style="border-collapse:collapse;margin:0 0 8px;"><thead><tr>${th}</tr></thead><tbody>${body}</tbody></table>`;
    }
    case 'codeBlock':
      return `<pre style="background:#f5f5f5;padding:8px;border-radius:4px;overflow:auto;"><code>${escapeHtml(b.code)}</code></pre>`;
  }
}

export function markdownToHtml(md: string, trust: LinkTrust): string {
  return parseBlocks(md)
    .map((b) => blockHtml(b, trust))
    .join('');
}

// ---------- plain-text rendering ----------

function blockPlain(b: BlockNode): string {
  switch (b.t) {
    case 'heading':
      return inlinePlain(b.text);
    case 'paragraph':
      return inlinePlain(b.text);
    case 'list':
      return b.items
        .map((it, i) => `${b.ordered ? `${i + 1}.` : '-'} ${inlinePlain(it)}`)
        .join('\n');
    case 'table': {
      const row = (cells: string[]) => cells.map((c) => inlinePlain(c)).join('\t');
      return [row(b.head), ...b.rows.map(row)].join('\n');
    }
    case 'codeBlock':
      return b.code;
  }
}

export function markdownToPlainText(md: string): string {
  return parseBlocks(md).map(blockPlain).join('\n\n');
}

/** Both clipboard forms of one answer, for `clipboard_write_text { text, html }` (ADR 0124
 * amendment). `trust` decides which links of `html` are clickable; `text` keeps every link. */
export function formatAnswerForClipboard(md: string, trust: LinkTrust): AnswerClipboardPayload {
  // `<meta charset="utf-8">` so a Mac app that reads public.html never guesses a legacy
  // encoding for em-dashes, currency signs or accented names (review of ADR 0124 amendment).
  return {
    text: markdownToPlainText(md),
    html: `<meta charset="utf-8">${markdownToHtml(md, trust)}`,
  };
}
