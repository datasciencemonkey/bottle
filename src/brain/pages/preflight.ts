/**
 * The Pages preflight of T14 (F7 step 3, PG-11, PG-12, ARC-F15).
 *
 * Before any Pages API call, the PageService checks that a domain ID is present and that the
 * profile has the `knowledge` or `all-apis` scope. Else it stops and shows the admin checklist
 * (section 10, items 10 and 18). When the domain list is readable, the domain must be in it
 * (ADM-20). The first Pages reply also tells a 403 (no scope) and a Beta that is off (ADM-21).
 */
import type { ProfileName } from '../../shared/interfaces';
import { workspaceKey, type DomainList } from './home-domain';

export type PreflightCode = 'NO_DOMAIN' | 'DOMAIN_NOT_FOUND' | 'NO_SCOPE' | 'PAGES_BETA_OFF';

/** What the card shows: a reason, the section 10 item numbers and their text. */
export interface PreflightFailure {
  code: PreflightCode;
  text: string;
  checklist: number[];
  items: string[];
}

export type PreflightResult =
  { ok: true; domainId: string } | { ok: false; failure: PreflightFailure };

export interface PreflightInput {
  profile: ProfileName;
  host: string;
  /** The draft domain: the review card change, else the MOG home domain. */
  domainId?: string;
  /** The OAuth scopes of the profile, when known. Unknown scopes go on to the call (ARC-F15). */
  scopes?: readonly string[];
  /** The picker result, when Bottle has it. A paste result cannot prove that a domain exists. */
  domains?: DomainList;
  /** The `request_id` of the create call. Only a 409 with this ID counts as created (ARC-F15). */
  requestId?: string;
}

/** Section 10, items 10 and 18 (ADM-20, ADM-21, ADM-15). */
const ITEM_10 =
  '10. Unity Catalog Pages (Beta) is on in the home workspace, and a domain exists for the users. Give users the domain ID.';
const ITEM_18 =
  '18. Any custom OAuth app for Bottle has the ai-gateway, offline_access and knowledge scopes (plus genie for genie_agent and postgres for v1 Lakebase sync), or all-apis.';

const PAGE_SCOPES = ['knowledge', 'all-apis'];

function failure(code: PreflightCode, input: PreflightInput): PreflightFailure {
  const ws = workspaceKey(input.host);
  switch (code) {
    case 'NO_DOMAIN':
      return {
        code,
        text: `No home domain is set for this MOG in ${ws}. Pick a domain or paste its ID. If no domain exists, ask your admin to make one.`,
        checklist: [10],
        items: [ITEM_10],
      };
    case 'DOMAIN_NOT_FOUND':
      return {
        code,
        text: `The domain ${input.domainId ?? ''} does not exist in ${ws}. Pick a different domain, or ask your admin for the domain ID.`,
        checklist: [10],
        items: [ITEM_10],
      };
    case 'NO_SCOPE':
      return {
        code,
        text: `The profile ${input.profile} cannot use Unity Catalog Pages. The profile needs the knowledge scope or all-apis. Ask your admin.`,
        checklist: [18],
        items: [ITEM_18],
      };
    case 'PAGES_BETA_OFF':
      return {
        code,
        text: `Unity Catalog Pages (Beta) is off in ${ws}. Ask your admin to turn it on.`,
        checklist: [10],
        items: [ITEM_10],
      };
  }
}

/** The checks before a Pages call. It makes no call itself. */
export function pagesPreflight(input: PreflightInput): PreflightResult {
  const id = input.domainId?.trim();
  if (!id) return { ok: false, failure: failure('NO_DOMAIN', input) };
  if (input.scopes && !input.scopes.some((s) => PAGE_SCOPES.includes(s)))
    return { ok: false, failure: failure('NO_SCOPE', input) };
  const list = input.domains?.kind === 'list' ? input.domains : undefined;
  if (list && !list.truncated && !list.domains.some((d) => d.id === id))
    return { ok: false, failure: failure('DOMAIN_NOT_FOUND', input) };
  return { ok: true, domainId: id };
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/**
 * The meaning of a Pages API reply (ARC-F15). A 409 counts as created only when it carries the
 * `request_id` that was sent; any other 409 (for example ALREADY_EXISTS for a name) is 'other'.
 * 'other' goes back to the caller, which keeps the reviewed draft with [Retry].
 */
export function classifyPagesResponse(
  res: { status: number; json?: unknown },
  sent: { requestId?: string | undefined } = {},
): 'ok' | 'NO_SCOPE' | 'PAGES_BETA_OFF' | 'DOMAIN_NOT_FOUND' | 'other' {
  const body = isObj(res.json) ? res.json : {};
  const code = typeof body['error_code'] === 'string' ? body['error_code'] : '';
  const message = typeof body['message'] === 'string' ? body['message'] : '';
  if (res.status >= 200 && res.status < 300) return 'ok';
  if (res.status === 409)
    return sent.requestId !== undefined && body['request_id'] === sent.requestId ? 'ok' : 'other';
  if (res.status === 403 || (res.status === 401 && /scope/i.test(message))) return 'NO_SCOPE';
  if (
    (res.status === 404 || res.status === 501) &&
    ['FEATURE_DISABLED', 'ENDPOINT_NOT_FOUND', 'NOT_IMPLEMENTED'].includes(code)
  )
    return 'PAGES_BETA_OFF';
  if (
    (res.status === 404 && code === 'RESOURCE_DOES_NOT_EXIST') ||
    (res.status === 400 && /\bdomain/i.test(message))
  )
    return 'DOMAIN_NOT_FOUND';
  return 'other';
}

/**
 * Run the preflight, then `send` only when it passes (PG-11, PG-12). A reply that shows a missing
 * scope, a Beta that is off or a domain that does not exist becomes the admin checklist. Other
 * replies go back with `created`: false means the caller keeps the draft with [Retry].
 */
export async function withPagesPreflight<R extends { status: number; json?: unknown }>(
  input: PreflightInput,
  send: (domainId: string) => Promise<R>,
): Promise<{ ok: true; created: boolean; res: R } | { ok: false; failure: PreflightFailure }> {
  const pre = pagesPreflight(input);
  if (!pre.ok) return pre;
  const res = await send(pre.domainId);
  const kind = classifyPagesResponse(res, { requestId: input.requestId });
  return kind === 'ok' || kind === 'other'
    ? { ok: true, created: kind === 'ok', res }
    : { ok: false, failure: failure(kind, input) };
}
