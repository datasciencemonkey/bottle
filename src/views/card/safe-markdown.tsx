/** SEC-25a/SEC-25b: strict allowlist markdown renderer. No HTML, no images, trusted links only. */
import { Fragment, type ReactNode } from 'react';

export interface LinkTrust {
  /** Host of the active profile (one profile = one workspace). */
  profileHost: string;
  /** `AllowedAppUrls` entries; only their hosts are trusted. */
  allowedAppUrls: readonly string[];
}

function hostOf(url: string): string | undefined {
  try {
    const u = new URL(url);
    return u.protocol === 'https:' ? u.host.toLowerCase() : undefined;
  } catch {
    return undefined;
  }
}

/** Returns the normalised URL when it is https, has no userinfo and its host is trusted. */
export function trustedUrl(raw: string, trust: LinkTrust): string | undefined {
  if (!/^https:\/\//i.test(raw) || /[\s<>"]/.test(raw)) return undefined;
  let u: URL;
  try {
    u = new URL(raw);
  } catch {
    return undefined;
  }
  if (u.username || u.password) return undefined;
  const hosts = new Set(
    [trust.profileHost, ...trust.allowedAppUrls.map((a) => hostOf(a) ?? a)].map((h) =>
      h.replace(/^https:\/\//i, '').toLowerCase(),
    ),
  );
  return hosts.has(u.host.toLowerCase()) ? u.href : undefined;
}

const INLINE = /(!?)\[([^\]]*)\]\(([^)\s]*)\)|`([^`]+)`|\*\*([^*]+)\*\*|\*([^*]+)\*/g;

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
  const lines = text.split(/\r?\n/);
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
