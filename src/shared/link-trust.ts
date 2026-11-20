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
 * Trust model. It mirrors `mog_core::link_open::validate_link` exactly, so nothing looks clickable
 * that Rust would refuse (`src/shared/link-cases.json` is checked against both):
 * - ASCII only, https only (any case), at most {@link MAX_URL_CHARS} characters;
 * - no control character, space, `\`, `<`, `>` or `"`;
 * - the authority (up to the first `/`, `?` or `#`) is a bare host of `[A-Za-z0-9.-]`: no userinfo,
 *   no port (not even `:443`), no `%` escape, no IPv6, no leading, trailing or doubled dot;
 * - the host equals the workspace host, or the host of an `AllowedAppUrls` entry. An entry must be
 *   an `https://host` URL and trusts that host (any path); anything else trusts nothing.
 * Returns the normalised URL (lower-case host), or undefined.
 */
export function trustedUrl(raw: string, trust: LinkTrust): string | undefined {
  if (raw.length > MAX_URL_CHARS || !/^https:\/\//i.test(raw) || raw.includes('..'))
    return undefined;
  if (/[^\x21-\x7e]|[<>"\\]/.test(raw)) return undefined;
  const rest = raw.slice(8);
  const authority = rest.slice(0, rest.search(/[/?#]|$/));
  const host = hostOfAuthority(authority);
  if (host === undefined) return undefined;
  const allowed = trust.allowedAppUrls.map(hostOfEntry);
  if (host !== bareHost(trust.profileHost) && !allowed.includes(host)) return undefined;
  return `https://${host}${rest.slice(authority.length)}`;
}

/** The lower-case host of an authority, or undefined if it is not a plain host. */
function hostOfAuthority(authority: string): string | undefined {
  if (!/^[A-Za-z0-9.-]{1,253}$/.test(authority)) return undefined;
  if (/^\.|\.$|\.\./.test(authority)) return undefined;
  return authority.toLowerCase();
}

/** The host of an `AllowedAppUrls` entry (`https://host[/path]`), or undefined. */
function hostOfEntry(entry: string): string | undefined {
  if (!/^https:\/\//i.test(entry)) return undefined;
  const rest = entry.slice(8);
  return hostOfAuthority(rest.slice(0, rest.search(/[/?#]|$/)));
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
