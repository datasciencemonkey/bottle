/**
 * The home domain of each MOG for draft UC Pages (U12, PG-18..PG-20).
 *
 * Each MOG has one home domain for each workspace. A draft goes to the home domain with no
 * prompt (PG-18). The review card can change the domain for one draft only (PG-19). The picker
 * lists domains with `GET /api/2.0/domains` (`knowledge.list_domains`); if that call fails (for
 * example, no MANAGE DISCOVER), the user pastes a domain ID or a Catalog Explorer URL (PG-20).
 * The saved MOG config field is `mog.pages` (Appendix F); the admin `PageDomainId` is the default.
 */
import type { MogId, ProfileName } from '../../shared/interfaces';
import { call } from '../../shared/ipc';

/** `mog.pages` in the MOG config: workspace key (host) to domain ID. It holds no secret. */
export interface MogPagesConfig {
  homeDomains: Record<string, string>;
}

export interface DomainOption {
  id: string;
  name: string;
}

/** The picker content: the domain list, or the paste fallback with the reason as text. */
export type DomainList =
  | { kind: 'list'; domains: readonly DomainOption[] }
  | { kind: 'paste'; reason: 'no_permission' | 'failed' | 'empty'; text: string };

export type DomainChoice = { ok: true; domainId: string } | { ok: false; text: string };

const ID = /^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$/;
const BAD_INPUT: DomainChoice = {
  ok: false,
  text: 'This is not a domain ID. Paste a domain ID or a Catalog Explorer URL of the domain.',
};
const PASTE = 'Paste the domain ID or its Catalog Explorer URL.';
const PAGE_SIZE = 50;
/** A stop for a list that never ends; 20 pages of 50 is more than a picker can show. */
const MAX_PAGES = 20;

/**
 * The domain ID in pasted text: an ID, `domains/<id>`, or a URL with `/domains/<id>` in its
 * path. A name (for example "Finance team") is not an ID (PG-06).
 */
export function parseDomainInput(text: string): string | undefined {
  const t = text.trim();
  if (ID.test(t)) return t;
  let path = t;
  if (/^https?:\/\//i.test(t)) {
    try {
      path = new URL(t).pathname;
    } catch {
      return undefined;
    }
  }
  const m = /(?:^|\/)domains\/([^/?#]+)/.exec(path);
  const id = m?.[1] ? decodeURIComponent(m[1]) : undefined;
  return id && ID.test(id) ? id : undefined;
}

/** One key for one workspace: the host in lower case, with no scheme and no trailing slash. */
export function workspaceKey(host: string): string {
  return host
    .trim()
    .toLowerCase()
    .replace(/^https?:\/\//, '')
    .replace(/\/+$/, '');
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

function option(d: unknown): DomainOption | undefined {
  if (!isObj(d)) return undefined;
  const name = typeof d['name'] === 'string' ? d['name'] : undefined;
  const id = typeof d['domain_id'] === 'string' ? d['domain_id'] : name?.replace(/^domains\//, '');
  if (!id || !ID.test(id)) return undefined;
  const label = typeof d['display_name'] === 'string' ? d['display_name'] : id;
  return { id, name: label };
}

/** List the domains of the profile's workspace for the picker (PG-20). It never throws. */
export async function listDomains(profile: ProfileName, mogId: MogId): Promise<DomainList> {
  const domains: DomainOption[] = [];
  let token: string | undefined;
  for (let n = 0; n < MAX_PAGES; n++) {
    let res: { status: number; json?: unknown };
    try {
      res = await call('dbx_http', {
        profile,
        routeId: 'knowledge.list_domains',
        params: token ? { page_size: PAGE_SIZE, page_token: token } : { page_size: PAGE_SIZE },
        tags: { app: 'bottle', mog: mogId, op: 'page' },
      });
    } catch {
      return { kind: 'paste', reason: 'failed', text: `Bottle cannot list domains. ${PASTE}` };
    }
    if (res.status === 401 || res.status === 403)
      return {
        kind: 'paste',
        reason: 'no_permission',
        text: `You do not have the permission to list domains (MANAGE DISCOVER). ${PASTE}`,
      };
    const body = isObj(res.json) ? res.json : undefined;
    const page = body?.['domains'] ?? [];
    if (res.status !== 200 || !body || !Array.isArray(page))
      return { kind: 'paste', reason: 'failed', text: `Bottle cannot list domains. ${PASTE}` };
    for (const d of page) {
      const o = option(d);
      if (o) domains.push(o);
    }
    const next = body['next_page_token'];
    if (typeof next !== 'string' || next === '') break;
    token = next;
  }
  return domains.length > 0
    ? { kind: 'list', domains }
    : {
        kind: 'paste',
        reason: 'empty',
        text: `Bottle found no domain. ${PASTE} If no domain exists, ask your admin for one.`,
      };
}

/**
 * The home domains of all MOGs (PG-18) and the one-draft changes of the review card (PG-19).
 * MogManager (#26) saves `config(mogId)` as `mog.pages` and gives it back with `load`.
 */
export class PageDomains {
  private readonly home = new Map<MogId, Map<string, string>>();
  private readonly drafts = new Map<string, string>();

  /** `adminDefault` is the MDM key `PageDomainId` (Appendix C), unset by default. */
  constructor(private readonly adminDefault?: string) {}

  load(mogId: MogId, config: MogPagesConfig | undefined): void {
    const m = new Map<string, string>();
    for (const [ws, id] of Object.entries(config?.homeDomains ?? {}))
      if (ID.test(id)) m.set(workspaceKey(ws), id);
    this.home.set(mogId, m);
  }

  config(mogId: MogId): MogPagesConfig {
    return { homeDomains: Object.fromEntries(this.home.get(mogId) ?? []) };
  }

  homeDomain(mogId: MogId, host: string): string | undefined {
    return this.home.get(mogId)?.get(workspaceKey(host)) ?? this.adminDefault;
  }

  /** Set at MOG setup or in settings. A bad input keeps the old home domain. */
  setHome(mogId: MogId, host: string, input: string): DomainChoice {
    const id = parseDomainInput(input);
    if (!id) return BAD_INPUT;
    const m = this.home.get(mogId) ?? new Map<string, string>();
    m.set(workspaceKey(host), id);
    this.home.set(mogId, m);
    return { ok: true, domainId: id };
  }

  /** The domain of one draft: its review card change, else the MOG home domain. */
  draftDomain(reviewId: string, mogId: MogId, host: string): string | undefined {
    return this.drafts.get(reviewId) ?? this.homeDomain(mogId, host);
  }

  /** PG-19: change the domain of this draft only. The home domain stays. */
  changeDraft(reviewId: string, input: string): DomainChoice {
    const id = parseDomainInput(input);
    if (!id) return BAD_INPUT;
    this.drafts.set(reviewId, id);
    return { ok: true, domainId: id };
  }

  /** Call when the review closes (approved, cancelled or expired). */
  closeDraft(reviewId: string): void {
    this.drafts.delete(reviewId);
  }

  /**
   * A `view_emit` click payload from the picker: `{target: 'domain', domainId}` or
   * `{target: 'domain_paste', text}`, with `scope` 'home' or 'draft' (and `reviewId` for a
   * draft). Returns undefined for any other payload.
   */
  onViewInput(mogId: MogId, host: string, payload: unknown): DomainChoice | undefined {
    if (!isObj(payload)) return undefined;
    const t = payload['target'];
    const input =
      t === 'domain' ? payload['domainId'] : t === 'domain_paste' ? payload['text'] : undefined;
    if (typeof input !== 'string') return undefined;
    if (payload['scope'] === 'home') return this.setHome(mogId, host, input);
    const reviewId = payload['reviewId'];
    if (payload['scope'] === 'draft' && typeof reviewId === 'string' && reviewId !== '')
      return this.changeDraft(reviewId, input);
    return undefined;
  }
}
