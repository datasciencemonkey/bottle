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

import type { AgentWsChip } from './ipc';

/** The brain-to-hub event that carries the list. */
export const HUB_CONVERSATIONS_EVENT = 'hub:conversations';

/**
 * The brain-to-hub event that opens Conversations on one exchange (a double-click on the MOG or
 * on the card). The payload is `{ requestId }`, the `opId` of the request, or `{ threadId }`,
 * the root UUID of an agent thread (the card chip "Open in Conversations"). It may be a thread
 * that is not in the store yet.
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
  /**
   * #523: the `deep_link` that Genie One returned for this answer (the conversation in the
   * workspace), or '' if it gave none. The view draws "Open in Databricks" only if it passes the
   * link trust rule for `workspaceHost`; the brain checks the click again.
   */
  deepLink?: string;
  status: 'answered' | 'failed';
  /** The Genie `conversation_id` of this exchange; `''` when the row was saved before #584. */
  conversationId: string;
}

/** One answer of one workspace inside an agent turn. */
export interface AgentThreadPart {
  requestId: string;
  workspace: AgentWsChip;
  /** The CLI profile name (never a token). */
  profile: string;
  workspaceHost: string;
  status: 'answered' | 'failed';
  answerMd: string;
  deepLink?: string;
  keyNumbers: ConversationKeyNumber[];
}

/** One turn of an agent thread: the user's question, the combined summary and the parts. */
export interface AgentThreadTurn {
  turnUuid: string;
  question: string;
  /** Epoch ms. */
  askedAt: number;
  status: 'answered' | 'failed' | 'stopped';
  summaryMd: string;
  parts: AgentThreadPart[];
}

/** One agent thread (the MOG agent, slice 2a): its turns, oldest first. */
export interface AgentThread {
  /** The root UUID of the thread. */
  threadId: string;
  mogId: string;
  mogName: string;
  mogColorVariant?: string;
  turns: AgentThreadTurn[];
}

/** What the hub shows. `unavailable` is true when the brain could not read the store. */
export interface HubConversations {
  rows: ConversationRow[];
  /** The agent threads. Absent from a brain that sends none. */
  agentThreads?: AgentThread[];
  unavailable?: true;
}

/** The most keys one `conversations.delete` carries. */
export const CONVERSATIONS_DELETE_MAX = 200;

/**
 * The hub Conversations actions: `conversations.ready` asks for the list (when the area opens or
 * shows again); `conversations.opened` says the hub got an open, so the brain stops holding it.
 * `conversations.delete` deletes 1 to 200 threads. A key is `c:<conversationId>`,
 * `r:<requestId>` or `a:<threadUuid>`. The brain resolves the rows from its own last read and
 * refuses a thread that is in progress.
 */
export type ConversationsAction =
  | { action: 'conversations.ready' }
  | { action: 'conversations.opened' }
  /** `onlyFailed`: Clear failed. The brain re-checks each key and skips a thread that has an
   *  answered or in-progress exchange by the time the delete runs. */
  | { action: 'conversations.delete'; threads: string[]; onlyFailed?: true };

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
 * The thread composer's Send on an agent thread: `question` goes to the MOG `mogId` as a
 * follow-up in thread `threadId`. The question has 1 to 1000 code points.
 */
export interface AgentThreadSendAction {
  action: 'agent.thread_send';
  threadId: string;
  mogId: string;
  question: string;
}

/**
 * `thread.active`: the hub window is focused on this thread, so a ⌥G press is a follow-up in it
 * (genie-followup, item 4) instead of a new question on the home workspace. `thread.inactive`
 * clears it: the hub lost focus, or the thread is no longer shown.
 */
export type ThreadFocusAction =
  | { action: 'thread.active'; conversationId: string; profile: string }
  | { action: 'thread.active_agent'; threadId: string }
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
  /** A follow-up of this thread is in flight (also one the view did not start, or a view that
   * was away): the composer shows its busy state and sends no second one. */
  busy?: boolean | undefined;
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
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const isUuid = (v: unknown): v is string => isStr(v) && UUID.test(v);
const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const isKey = (v: unknown): v is string => {
  if (!isStr(v)) return false;
  const id = v.slice(2);
  if (v.startsWith('a:')) return isUuid(id);
  if (v.startsWith('r:')) return id.length > 0 && id.length <= 64;
  return v.startsWith('c:') && id.length > 0 && id.length <= 256;
};

/** Shape check of a hub action for Conversations. Unknown fields are dropped. */
export function asConversationsAction(p: unknown): ConversationsAction | undefined {
  if (!isObj(p)) return undefined;
  const a = p['action'];
  if (a === 'conversations.ready' || a === 'conversations.opened') return { action: a };
  if (a !== 'conversations.delete') return undefined;
  const t = p['threads'];
  if (!Array.isArray(t) || t.length < 1 || t.length > CONVERSATIONS_DELETE_MAX) return undefined;
  if (!t.every(isKey)) return undefined;
  const out: ConversationsAction = { action: 'conversations.delete', threads: [...t] };
  return p['onlyFailed'] === true ? { ...out, onlyFailed: true } : out;
}

/** Shape check of the thread composer's `agent.thread_send`. */
export function asAgentThreadSendAction(p: unknown): AgentThreadSendAction | undefined {
  if (!isObj(p) || p['action'] !== 'agent.thread_send') return undefined;
  const { threadId, mogId, question } = p;
  if (!isUuid(threadId) || !isUuid(mogId)) return undefined;
  if (!isStr(question) || question.trim() === '' || [...question].length > 1000) return undefined;
  return { action: 'agent.thread_send', threadId, mogId, question };
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

/** Shape check of `thread.active` / `thread.active_agent` / `thread.inactive`. */
export function asThreadFocusAction(p: unknown): ThreadFocusAction | undefined {
  if (!isObj(p)) return undefined;
  if (p['action'] === 'thread.inactive') return { action: 'thread.inactive' };
  if (p['action'] === 'thread.active_agent') {
    return isUuid(p['threadId'])
      ? { action: 'thread.active_agent', threadId: p['threadId'] }
      : undefined;
  }
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
  const { conversationId, state, text, busy } = p;
  if (!isStr(conversationId) || !isStr(text)) return undefined;
  if (state !== 'ready' && state !== 'sign_in') return undefined;
  if (busy !== undefined && typeof busy !== 'boolean') return undefined;
  return { conversationId, state, text, busy };
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
    (v['deepLink'] === undefined || isStr(v['deepLink'])) &&
    (v['status'] === 'answered' || v['status'] === 'failed') &&
    isStr(v['conversationId'])
  );
}

function isChip(v: unknown): v is AgentWsChip {
  return isObj(v) && isStr(v['name']) && isStr(v['colorVariant']);
}

function isAgentPart(v: unknown): v is AgentThreadPart {
  return (
    isObj(v) &&
    isStr(v['requestId']) &&
    isChip(v['workspace']) &&
    isStr(v['profile']) &&
    isStr(v['workspaceHost']) &&
    (v['status'] === 'answered' || v['status'] === 'failed') &&
    isStr(v['answerMd']) &&
    (v['deepLink'] === undefined || isStr(v['deepLink'])) &&
    Array.isArray(v['keyNumbers']) &&
    v['keyNumbers'].every(isKeyNumber)
  );
}

function isAgentTurn(v: unknown): v is AgentThreadTurn {
  return (
    isObj(v) &&
    isStr(v['turnUuid']) &&
    v['turnUuid'] !== '' &&
    isStr(v['question']) &&
    isNum(v['askedAt']) &&
    (v['status'] === 'answered' || v['status'] === 'failed' || v['status'] === 'stopped') &&
    isStr(v['summaryMd']) &&
    Array.isArray(v['parts']) &&
    v['parts'].every(isAgentPart)
  );
}

function isAgentThread(v: unknown): v is AgentThread {
  return (
    isObj(v) &&
    isUuid(v['threadId']) &&
    isStr(v['mogId']) &&
    isStr(v['mogName']) &&
    (v['mogColorVariant'] === undefined || isStr(v['mogColorVariant'])) &&
    Array.isArray(v['turns']) &&
    v['turns'].every(isAgentTurn)
  );
}

/** Shape check of a `hub:conversations` payload. */
export function asHubConversations(p: unknown): HubConversations | undefined {
  if (!isObj(p) || !Array.isArray(p['rows']) || !p['rows'].every(isRow)) return undefined;
  const at = p['agentThreads'];
  if (at !== undefined && !(Array.isArray(at) && at.every(isAgentThread))) return undefined;
  if (p['unavailable'] !== undefined && p['unavailable'] !== true) return undefined;
  return p as unknown as HubConversations;
}

/** Shape check of a `hub:conversations.open` payload: a `requestId` or a `threadId` (UUID). */
export function asConversationsOpen(
  p: unknown,
): { requestId?: string; threadId?: string } | undefined {
  if (!isObj(p)) return undefined;
  const r = p['requestId'];
  const t = p['threadId'];
  if (r !== undefined && !(isStr(r) && r.length > 0 && r.length <= 64)) return undefined;
  if (t !== undefined && !isUuid(t)) return undefined;
  return {
    ...(r !== undefined ? { requestId: r } : {}),
    ...(t !== undefined ? { threadId: t } : {}),
  };
}
