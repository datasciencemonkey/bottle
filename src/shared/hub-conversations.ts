/**
 * The hub Conversations list (#103, #40): the local history of Ask Genie.
 *
 * The brain reads the saved exchanges from the R10 store (`mem.exchange.recent` through
 * `mem_local_exec`) and sends them to the hub window only. The hub view never runs a store
 * statement (ARC-2): it renders what the brain sends and asks again with `conversations.ready`.
 * This file holds names and shape checks only, no state. It is not a contract file, like
 * `hub-mogs.ts`. TODO(#407): the event names move into `src/shared/ipc.ts` with the hub events.
 *
 * A thread (genie-followup, #145, #103) is every exchange that shares one Genie
 * `conversationId`: the reply composer at the bottom of the thread detail (`ThreadComposer.tsx`)
 * sends a follow-up question with that same id, on the same profile, so Genie keeps the context.
 * A row saved before #584 has no `conversationId` and is its own one-exchange thread.
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
  /**
   * #ui-4: `mog.ui.variant`, so the row shows the same colour as the MOG's lamp and card
   * (`mog-palette.ts`). Undefined for a MOG the brain could not resolve a colour for; the view
   * falls back by MOG ID.
   */
  mogColorVariant?: string;
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
  /** The Genie `conversation_id` of this exchange; `''` when the row was saved before #584. */
  conversationId: string;
}

/** What the hub shows. `unavailable` is true when the brain could not read the store. */
export interface HubConversations {
  rows: ConversationRow[];
  unavailable?: true;
}

/**
 * The hub Conversations actions: `conversations.ready` asks for the list (when the area opens or
 * shows again); `conversations.opened` says the hub got an open, so the brain stops holding it.
 */
export type ConversationsAction =
  { action: 'conversations.ready' } | { action: 'conversations.opened' };

/** One thread: every exchange that shares one Genie `conversationId`, oldest first. */
export interface ConversationThread {
  conversationId: string;
  profile: string;
  rows: ConversationRow[];
}

/**
 * The reply composer's argument (genie-followup, `ThreadComposer.tsx`): the thread's live
 * conversation and workspace. `exchangeId` is the thread's latest exchange.
 */
export interface ThreadComposerThread {
  conversationId: string;
  exchangeId: string;
  profile: string;
}

/**
 * The exchanges of `row`'s thread, oldest first: every row of `all` that shares its
 * `conversationId`. A row with no `conversationId` (`''`) is its own thread of one, so rows
 * saved before #584 never merge with an unrelated one just because both are blank.
 */
export function threadOf(all: readonly ConversationRow[], row: ConversationRow): ConversationRow[] {
  const mine =
    row.conversationId === '' ? [row] : all.filter((r) => r.conversationId === row.conversationId);
  return [...mine].sort((a, b) => a.askedAt - b.askedAt || (a.requestId < b.requestId ? -1 : 1));
}

/**
 * The composer's Send sends `question` to `conversationId` on `profile` (genie-followup). The
 * hub asks on the same workspace the thread is already in; Bottle never picks a different one.
 */
export interface ThreadSendAction {
  action: 'thread.send';
  conversationId: string;
  profile: string;
  question: string;
}

/**
 * `thread.active`: the hub window is focused on this thread, so a ⌥G press is a follow-up in it
 * (genie-followup, item 4) instead of a new question on the home workspace. `thread.inactive`
 * clears it: the hub lost focus, or the thread is no longer shown.
 */
export type ThreadFocusAction =
  | { action: 'thread.active'; conversationId: string; profile: string }
  | { action: 'thread.inactive' };

/** The brain-to-hub event with the live status of a sent follow-up (ASKING_GENIE, a stage, or
 * the terminal outcome): the composer shows it while the answer is not in the list yet. */
export const HUB_THREAD_STATUS_EVENT = 'hub:thread.status';

/** The brain-to-hub event with whether a thread's workspace can take a follow-up now. */
export const HUB_THREAD_ACCESS_EVENT = 'hub:thread.access';

/** `ready`: the composer is open. `sign_in`: the workspace has no valid sign-in; `text` says why. */
export interface ThreadAccess {
  conversationId: string;
  state: 'ready' | 'sign_in';
  text: string;
}

/** The composer asks the brain for a thread's `ThreadAccess` (on mount, on focus, on a retry). */
export interface ThreadCheckAction {
  action: 'thread.check';
  conversationId: string;
}

export type ThreadPhase = 'working' | 'answered' | 'failed' | 'cancelled';

/** The composer shows `text` under the input until a later status replaces or clears it. */
export interface ThreadStatus {
  conversationId: string;
  requestId: string;
  phase: ThreadPhase;
  text: string;
  /** On a `failed` status: the question that was not answered, so Try again can send it again. */
  question?: string | undefined;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const isStr = (v: unknown): v is string => typeof v === 'string';

/** Shape check of a hub action for Conversations. Unknown fields are dropped. */
export function asConversationsAction(p: unknown): ConversationsAction | undefined {
  if (!isObj(p)) return undefined;
  const a = p['action'];
  return a === 'conversations.ready' || a === 'conversations.opened' ? { action: a } : undefined;
}

/** Shape check of the composer's `thread.send`. A blank id, profile or question is refused. */
export function asThreadSendAction(p: unknown): ThreadSendAction | undefined {
  if (!isObj(p) || p['action'] !== 'thread.send') return undefined;
  const { conversationId, profile, question } = p;
  if (!isStr(conversationId) || conversationId === '') return undefined;
  if (!isStr(profile) || profile === '') return undefined;
  if (!isStr(question) || question.trim() === '') return undefined;
  return { action: 'thread.send', conversationId, profile, question };
}

/** Shape check of `thread.active` / `thread.inactive`. */
export function asThreadFocusAction(p: unknown): ThreadFocusAction | undefined {
  if (!isObj(p)) return undefined;
  if (p['action'] === 'thread.inactive') return { action: 'thread.inactive' };
  if (p['action'] !== 'thread.active') return undefined;
  const { conversationId, profile } = p;
  if (!isStr(conversationId) || conversationId === '') return undefined;
  if (!isStr(profile) || profile === '') return undefined;
  return { action: 'thread.active', conversationId, profile };
}

const PHASES: readonly ThreadPhase[] = ['working', 'answered', 'failed', 'cancelled'];

/** Shape check of a `hub:thread.status` payload. */
export function asThreadStatus(p: unknown): ThreadStatus | undefined {
  if (!isObj(p)) return undefined;
  const { conversationId, requestId, phase, text, question } = p;
  if (!isStr(conversationId) || !isStr(requestId) || !isStr(text)) return undefined;
  if (!PHASES.includes(phase as ThreadPhase)) return undefined;
  if (question !== undefined && !isStr(question)) return undefined;
  return { conversationId, requestId, phase: phase as ThreadPhase, text, question };
}

/** Shape check of the composer's `thread.check`. */
export function asThreadCheckAction(p: unknown): ThreadCheckAction | undefined {
  if (!isObj(p) || p['action'] !== 'thread.check') return undefined;
  const { conversationId } = p;
  if (!isStr(conversationId) || conversationId === '') return undefined;
  return { action: 'thread.check', conversationId };
}

/** Shape check of a `hub:thread.access` payload. */
export function asThreadAccess(p: unknown): ThreadAccess | undefined {
  if (!isObj(p)) return undefined;
  const { conversationId, state, text } = p;
  if (!isStr(conversationId) || !isStr(text)) return undefined;
  if (state !== 'ready' && state !== 'sign_in') return undefined;
  return { conversationId, state, text };
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
    (v['mogColorVariant'] === undefined || isStr(v['mogColorVariant'])) &&
    isStr(v['question']) &&
    isStr(v['answerMd']) &&
    Array.isArray(v['keyNumbers']) &&
    v['keyNumbers'].every(isKeyNumber) &&
    typeof v['askedAt'] === 'number' &&
    Number.isFinite(v['askedAt']) &&
    isStr(v['profile']) &&
    isStr(v['workspaceHost']) &&
    (v['status'] === 'answered' || v['status'] === 'failed') &&
    isStr(v['conversationId'])
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
