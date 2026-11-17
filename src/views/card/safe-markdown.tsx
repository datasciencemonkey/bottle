/** SEC-25a/SEC-25b: strict allowlist markdown renderer. No HTML, no images, trusted links only. */
import { Fragment, type ReactNode } from 'react';

export interface LinkTrust {
  /** Host of the active profile (one profile = one workspace). */
  profileHost: string;
  /** `AllowedAppUrls` entries; only their hosts are trusted. */
  allowedAppUrls: readonly string[];
}

/** Answers longer than this are cut before parsing (bounds work on hostile input). */
export const MAX_ANSWER_CHARS = 20_000;

/**
 * Trust model (requirements do not define a match rule for `AllowedAppUrls`, so the stricter reading is used):
 * the profile host matches by origin (https, host and port); each `AllowedAppUrls` entry must be an https URL
 * and matches by origin plus path prefix on a segment boundary. Entries that are not https URLs trust nothing.
 */
export function trustedUrl(raw: string, trust: LinkTrust): string | undefined {
  if (!/^https:\/\//i.test(raw) || /[\s<>"]/.test(raw) || raw.includes('..')) return undefined;
  let u: URL;
  try {
    u = new URL(raw);
  } catch {
    return undefined;
  }
  if (u.username || u.password || u.protocol !== 'https:') return undefined;
  if (u.host === trust.profileHost.toLowerCase()) return u.href;
  for (const entry of trust.allowedAppUrls) {
    let e: URL;
    try {
      e = new URL(entry);
    } catch {
      continue;
    }
    if (e.protocol !== 'https:' || e.origin !== u.origin) continue;
    const base = e.pathname.replace(/\/+$/, '');
    if (u.pathname === base || u.pathname.startsWith(`${base}/`)) return u.href;
  }
  return undefined;
}

// Every group is length-bounded so unmatched openers cost O(n), not O(n^2).
export const INLINE =
  /(!?)\[([^\]]{0,500})\]\(([^)\s]{0,2048})\)|`([^`]{1,500})`|\*\*([^*]{1,500})\*\*|\*([^*]{1,500})\*/g;

function inline(text: string, trust: LinkTrust, key: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  let i = 0;
  for (const m of text.matchAll(INLINE)) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const k = `${key}-${i++}`;
    if (m[4] !== undefined) out.push(<code key={k}>{m[4]}</code>);
    else if (m[5] !== undefined) out.push(<strong key={k}>{m[5]}</strong>);
    else if (m[6] !== undefined) out.push(<em key={k}>{m[6]}</em>);
    else if (m[1] === '!')
      out.push(m[2]); // image: alt text only, never a fetch
    else {
      const href = trustedUrl(m[3] ?? '', trust);
      out.push(
        href === undefined ? (
          m[2]
        ) : (
          <Fragment key={k}>
            <a href={href} rel="noopener noreferrer">
              {href}
            </a>
            {m[2] && m[2] !== href ? ` (${m[2]})` : ''}
          </Fragment>
        ),
      );
    }
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

export function SafeMarkdown({ text, trust }: { text: string; trust: LinkTrust }) {
  const lines = text.slice(0, MAX_ANSWER_CHARS).split(/\r?\n/);
  const blocks: ReactNode[] = [];
  let n = 0;
  let i = 0;
  while (i < lines.length) {
    const line = lines[i] ?? '';
    const key = `b${n++}`;
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
    } else if (/^\s*([-*]|\d+\.)\s+/.test(line)) {
      const ordered = /^\s*\d+\./.test(line);
      const items: ReactNode[] = [];
      while (i < lines.length && /^\s*([-*]|\d+\.)\s+/.test(lines[i] ?? '')) {
        const item = (lines[i] ?? '').replace(/^\s*([-*]|\d+\.)\s+/, '');
        items.push(<li key={`${key}-${i}`}>{inline(item, trust, `${key}-${i}`)}</li>);
        i++;
      }
      blocks.push(ordered ? <ol key={key}>{items}</ol> : <ul key={key}>{items}</ul>);
    } else if (/^#{1,6}\s+/.test(line)) {
      blocks.push(
        <p key={key}>
          <strong>{inline(line.replace(/^#{1,6}\s+/, ''), trust, key)}</strong>
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
        !/^\s*([-*]|\d+\.)\s+/.test(lines[i] ?? '') &&
        !/^#{1,6}\s+/.test(lines[i] ?? '')
      ) {
        para.push(lines[i] ?? '');
        i++;
      }
      blocks.push(<p key={key}>{inline(para.join(' '), trust, key)}</p>);
    }
  }
  return <>{blocks}</>;
}
