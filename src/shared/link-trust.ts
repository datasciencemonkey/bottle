/**
 * The link trust rule (SEC-25b, #523). One rule for the views that decide what to draw as a link
 * and for the brain that checks a click again before it asks Rust to open the URL. Rust checks a
 * third time (`mog_core::link_open`). A view is never trusted: its click is only a request.
 * Pure, no React, no IPC.
 */

export interface LinkTrust {
  /** Host of the active profile (one profile = one workspace): `host` or `https://host`. */
  profileHost: string;
  /** `AllowedAppUrls` entries; only https entries trust anything. */
  allowedAppUrls: readonly string[];
}

/** No link is trusted until the caller knows the profile host (TODO(#40)). */
export const NO_LINKS: LinkTrust = { profileHost: '', allowedAppUrls: [] };

/** The longest URL a link or a click may carry. Rust uses the same bound. */
export const MAX_URL_CHARS = 2048;

/** The lower-case bare host of `host` or `https://host[/]`, or '' if it is not one. */
export function bareHost(profileHost: string): string {
  const t = profileHost.trim().replace(/\/+$/, '');
  const h = t.replace(/^https:\/\//i, '').toLowerCase();
  return /^[a-z0-9.-]{1,253}$/.test(h) ? h : '';
}

/**
 * Trust model (requirements do not define a match rule for `AllowedAppUrls`, so the stricter reading is used):
 * the profile host matches by host (https only, default port, no credentials); each `AllowedAppUrls`
 * entry must be an https URL and matches by origin plus path prefix on a segment boundary. Entries
 * that are not https URLs trust nothing. Returns the normalised URL, or undefined.
 */
export function trustedUrl(raw: string, trust: LinkTrust): string | undefined {
  if (raw.length > MAX_URL_CHARS) return undefined;
  if (!/^https:\/\//i.test(raw) || /[\s<>"\\]/.test(raw) || raw.includes('..')) return undefined;
  let u: URL;
  try {
    u = new URL(raw);
  } catch {
    return undefined;
  }
  if (u.username || u.password || u.protocol !== 'https:') return undefined;
  const profile = bareHost(trust.profileHost);
  if (profile !== '' && u.host === profile) return u.href;
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

/** The host of an http(s) URL for display, or undefined for anything else. */
export function displayHost(raw: string): string | undefined {
  if (!/^https?:\/\//i.test(raw) || raw.length > MAX_URL_CHARS) return undefined;
  try {
    return new URL(raw).host || undefined;
  } catch {
    return undefined;
  }
}

/** The longest path a short URL shows. */
const SHORT_PATH_CHARS = 24;

/** A short form of a URL: the host plus a short path, no scheme, query or fragment. */
export function shortUrl(raw: string): string {
  try {
    const u = new URL(raw);
    const path = u.pathname === '/' ? '' : u.pathname.replace(/\/+$/, '');
    const cut =
      path.length > SHORT_PATH_CHARS ? `${path.slice(0, SHORT_PATH_CHARS - 1)}\u2026` : path;
    return `${u.host}${cut}`;
  } catch {
    return raw.length > 40 ? `${raw.slice(0, 39)}\u2026` : raw;
  }
}

/**
 * The "Open in Databricks" URL of one Genie answer. It is the `deep_link` that Genie One MCP
 * returned (`genie_poll_response`), kept only if it passes the trust rule for the workspace.
 * Bottle never builds a URL from ids: the route (`/one/chat/threads/<cid>?o=<wsid>`) is not
 * documented as a stable contract (docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp).
 */
export function openInDatabricksUrl(
  deepLink: string | undefined,
  workspaceHost: string | undefined,
): string | undefined {
  if (!deepLink || !workspaceHost) return undefined;
  return trustedUrl(deepLink, { profileHost: workspaceHost, allowedAppUrls: [] });
}

/** The `view_emit` `click` target of a link click (card and hub). The brain checks it again. */
export const LINK_OPEN = 'link_open';

/** A link click, as a view sends it. `requestId` names the exchange (hub). */
export interface LinkOpenClick {
  target: typeof LINK_OPEN;
  url: string;
  requestId?: string;
}

/** Shape check of a link click payload. Returns undefined for anything else. */
export function asLinkOpenClick(p: unknown): LinkOpenClick | undefined {
  if (typeof p !== 'object' || p === null) return undefined;
  const o = p as Record<string, unknown>;
  if (o['target'] !== LINK_OPEN || typeof o['url'] !== 'string') return undefined;
  if (o['url'].length === 0 || o['url'].length > MAX_URL_CHARS) return undefined;
  const r = o['requestId'];
  if (r !== undefined && !(typeof r === 'string' && /^[A-Za-z0-9-]{1,64}$/.test(r)))
    return undefined;
  return {
    target: LINK_OPEN,
    url: o['url'],
    ...(r !== undefined ? { requestId: r as string } : {}),
  };
}
