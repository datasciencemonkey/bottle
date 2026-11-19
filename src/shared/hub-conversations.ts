/**
 * The hub Conversations list (#103, #40): the local history of Ask Genie.
 *
 * The brain reads the saved exchanges from the R10 store (`mem.exchange.recent` through
 * `mem_local_exec`) and sends them to the hub window only. The hub view never runs a store
 * statement (ARC-2): it renders what the brain sends and asks again with `conversations.ready`.
 * This file holds names and shape checks only, no state. It is not a contract file, like
 * `hub-mogs.ts`. TODO(#407): the event names move into `src/shared/ipc.ts` with the hub events.
 */

/** The brain-to-hub event that carries the list. */
export const HUB_CONVERSATIONS_EVENT = 'hub:conversations';

/**
 * The brain-to-hub event that opens Conversations on one exchange (a double-click on the MOG or
 * on the card). The payload is `{ requestId }`, the `opId` of the request.
 */
export const HUB_CONVERSATIONS_OPEN_EVENT = 'hub:conversations.open';

/**
 * The `mogId` of the Conversations `view_emit`, like `CLI_VIEW_ID`. It cannot be a MOG ID,
 * because MOG IDs are UUIDs.
 */
export const CONVERSATIONS_VIEW_ID = 'conversations';

/** The most exchanges the hub lists. The newest stay. */
export const CONVERSATIONS_MAX = 200;

/** One key number, as the card shows it. */
export interface ConversationKeyNumber {
  text: string;
  label?: string;
}

/** One saved Ask Genie exchange. */
export interface ConversationRow {
  /** The request id (the `opId`); unique. */
  requestId: string;
  mogId: string;
  mogName: string;
  question: string;
  /** The answer markdown; empty when Genie gave none. */
  answerMd: string;
  keyNumbers: ConversationKeyNumber[];
  /** Epoch ms. */
  askedAt: number;
  /** The CLI profile name (never a token). */
  profile: string;
  workspaceHost: string;
  status: 'answered' | 'failed';
}

/** What the hub shows. `unavailable` is true when the brain could not read the store. */
export interface HubConversations {
  rows: ConversationRow[];
  unavailable?: true;
}

/** The hub asks for the list: when Conversations opens or shows again. */
export interface ConversationsReady {
  action: 'conversations.ready';
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const isStr = (v: unknown): v is string => typeof v === 'string';

/** Shape check of a hub action for Conversations. Unknown fields are dropped. */
export function asConversationsAction(p: unknown): ConversationsReady | undefined {
  return isObj(p) && p['action'] === 'conversations.ready'
    ? { action: 'conversations.ready' }
    : undefined;
}

function isKeyNumber(v: unknown): v is ConversationKeyNumber {
  return isObj(v) && isStr(v['text']) && (v['label'] === undefined || isStr(v['label']));
}

function isRow(v: unknown): v is ConversationRow {
  return (
    isObj(v) &&
    isStr(v['requestId']) &&
    isStr(v['mogId']) &&
    isStr(v['mogName']) &&
    isStr(v['question']) &&
    isStr(v['answerMd']) &&
    Array.isArray(v['keyNumbers']) &&
    v['keyNumbers'].every(isKeyNumber) &&
    typeof v['askedAt'] === 'number' &&
    Number.isFinite(v['askedAt']) &&
    isStr(v['profile']) &&
    isStr(v['workspaceHost']) &&
    (v['status'] === 'answered' || v['status'] === 'failed')
  );
}

/** Shape check of a `hub:conversations` payload. */
export function asHubConversations(p: unknown): HubConversations | undefined {
  if (!isObj(p) || !Array.isArray(p['rows']) || !p['rows'].every(isRow)) return undefined;
  if (p['unavailable'] !== undefined && p['unavailable'] !== true) return undefined;
  return p as unknown as HubConversations;
}

/** Shape check of a `hub:conversations.open` payload. */
export function asConversationsOpen(p: unknown): { requestId?: string } | undefined {
  if (!isObj(p)) return undefined;
  const r = p['requestId'];
  if (r === undefined) return {};
  return isStr(r) && r.length > 0 && r.length <= 64 ? { requestId: r } : undefined;
}
