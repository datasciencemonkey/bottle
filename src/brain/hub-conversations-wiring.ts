/**
 * The brain side of the hub Conversations list (#103, #40).
 *
 * - The brain reads the newest exchanges of each defined MOG from the R10 store
 *   (`mem.exchange.recent` through `mem_local_exec`, MEM-01: one MOG per read), merges them
 *   newest first and sends the list to the hub window only (ARC-5). The hub view never runs a
 *   store statement.
 * - The hub sends `conversations.ready` when Conversations opens or shows again, and again with a
 *   backoff while it has no list. The brain sends the list again after each saved exchange
 *   (`refresh`), and by itself once it listens and once the store unlocks (`push`), so a request
 *   lost at launch never leaves the hub with no list.
 * - `openConversations(requestId)` opens Conversations on one exchange (a double-click on the
 *   MOG or the card). The brain holds the selection until the hub acks it
 *   (`conversations.opened`) or asks for the list, so a hub that opens late still selects it, once.
 * - A failed read sends an empty list marked `unavailable`, with no error text.
 * - Rows of a deleted MOG stay in the store until Wipe; the list shows defined MOGs only.
 * - Agent threads (ADR 0121): a turn row (`backend_kind = 'genie_agent'`, `split_group_id` = the
 *   thread root) holds the user's question and the combined answer. The call rows and the
 *   fallback row of the turn carry `split_group_id` = the turn UUID and are its parts. They never
 *   go into the plain `rows`. A group of parts with no turn row (a 0.1.5 build) is a thread with
 *   "Question not saved" (an empty question). A stored turn is always sent: the view swaps it for
 *   the in-memory turn with the same id (`hub:agent` has the newest turns only). A group of parts
 *   of a turn that is in memory and has no turn row yet is not sent, but its rows stay deletable.
 * - `conversations.delete` reads the store again, defers a thread in progress until it settles,
 *   deletes the rows locally (one `mem.exchange.delete` for each MOG: a tombstone that keeps the
 *   Genie conversation id for Wipe and Forget, ADR 0121; never a memory item), tells the agent
 *   client to forget the turns whose rows were deleted, then sends the list.
 */
import { emitTo } from '@tauri-apps/api/event';
import {
  asConversationsAction,
  CONVERSATIONS_DELETE_MAX,
  CONVERSATIONS_MAX,
  CONVERSATIONS_VIEW_ID,
  HUB_CONVERSATIONS_EVENT,
  HUB_CONVERSATIONS_OPEN_EVENT,
  type AgentThread,
  type AgentThreadPart,
  type AgentThreadTurn,
  type ConversationKeyNumber,
  type ConversationRow,
  type HubConversations,
} from '../shared/hub-conversations';
import type { AgentWsChip } from '../shared/ipc';
import { onViewInput, type BrainBus } from './bus';
import type { LocalMemoryStore } from './components/memory-store';

/** One defined MOG: its id, name and accent colour. */
export interface ConversationMog {
  mogId: string;
  name: string;
  /** #ui-4: `mog.ui.variant`. */
  colorVariant?: string;
}

/** The turns the agent client holds in memory (`AgentClient.turns`). */
export interface MemoryTurn {
  /** The stored `request_id` of the turn row. */
  turnUuid: string;
  threadId: string;
  /** The turn is live, queued or not yet settled (its rows may not be saved): a Delete waits. */
  inProgress: boolean;
  /** The agent client sends this turn to the hub (`hub:agent`). Default: yes. */
  shown?: boolean;
}

export interface ConversationsReadDeps {
  store: LocalMemoryStore;
  mogs(): Promise<ConversationMog[]>;
  /** The name and colour of a workspace chip. Default: the profile name and no colour. */
  chipOf?(profile: string): AgentWsChip;
  /** The turns in memory: the view shows the one that `hub:agent` sends instead of its stored turn. */
  memoryTurns?(): MemoryTurn[];
}

export interface ConversationsWireDeps extends ConversationsReadDeps {
  send(list: HubConversations): Promise<unknown>;
  sendOpen(open: { requestId?: string; threadId?: string }): Promise<unknown>;
  /** Delete: the agent client forgets the finished turns of this thread. */
  forgetThread?(threadId: string): void;
  /** Delete: a deleted Genie thread (`c:`) is no follow-up target any more (⌥G, the composer). */
  forgetConversation?(conversationId: string): void;
  /** Whether a hub window exists. Default: yes (the hub is built at start and only hidden). */
  hubPresent?(): Promise<boolean>;
  /** The owner's "little button on the Conversations copy" request: `clipboard_write_text`
   * with `text` only (no `html`), for the row the brain resolved from its own last list read. */
  copyMarkdown?(text: string): Promise<unknown>;
  /** The owner's "Copy" button request (ADR 0124 amendment): formatted (html plus text), for
   * the row's answer and its workspace host, both resolved from the brain's own last list read. */
  copyFormatted?(md: string, host: string): void;
}

export interface HubConversationsWiring {
  /** Send the list to the hub again. Never rejects. */
  refresh(): Promise<void>;
  /**
   * Send the list to the hub with no request from it, when a hub window exists. The brain calls
   * it once its listener is wired and again when the local store unlocks, because the hub's own
   * `conversations.ready` is lost while the brain is not yet listening or the store is locked.
   * Never rejects.
   */
  push(): Promise<void>;
  /**
   * #523: the profile and host of a listed exchange, from the last list the brain sent (never
   * from a view). The link opener trusts this, not the click.
   */
  workspaceOf(requestId: string): { profile: string; host: string } | undefined;
  /** Open Conversations, with this exchange selected when given. Never rejects. */
  openConversations(requestId?: string): Promise<void>;
  /** Open Conversations on an agent thread (it may not be in the store yet). Never rejects. */
  openThread(threadId: string): Promise<void>;
  /** The MOG of an agent thread in the last list, or undefined. */
  threadMog(threadId: string): string | undefined;
  /**
   * Delete threads by key (`c:`, `r:`, `a:`), resolved from a fresh read of the store. With
   * `onlyFailed` (Clear failed) a key whose thread has an answered exchange now stays. A thread in
   * progress waits: `retryDeferred` deletes it once it settled. Sends the list again. Never rejects.
   */
  remove(keys: readonly string[], opts?: { onlyFailed?: boolean }): Promise<void>;
  /** Delete the threads that waited for a turn to settle, when it did. Never rejects. */
  retryDeferred(): Promise<void>;
  /**
   * The MOG and the profile that own a thread's `conversationId` (genie-followup, #145): a
   * follow-up keeps the MOG and the workspace of the first question, read from the stored
   * exchanges — never from the hub view, which only names the conversation it wants, not the
   * workspace it may use (ARC-2). `undefined` for a blank id or one no stored exchange carries.
   * Never rejects.
   */
  findThread(
    conversationId: string,
  ): Promise<{ mogId: string; profile: string; host: string } | undefined>;
  dispose(): void;
}

/** Send the list to the hub window only. */
export function sendHubConversations(list: HubConversations): Promise<void> {
  return emitTo('hub', HUB_CONVERSATIONS_EVENT, list);
}

/** Send an open request to the hub window only. */
export function sendHubConversationsOpen(open: {
  requestId?: string;
  threadId?: string;
}): Promise<void> {
  return emitTo('hub', HUB_CONVERSATIONS_OPEN_EVENT, open);
}

const str = (v: unknown): string => (typeof v === 'string' ? v : '');

function keyNumbersOf(raw: unknown): ConversationKeyNumber[] {
  let v: unknown;
  try {
    v = typeof raw === 'string' ? JSON.parse(raw) : raw;
  } catch {
    return [];
  }
  if (!Array.isArray(v)) return [];
  return v.flatMap((n: unknown) => {
    if (typeof n !== 'object' || n === null) return [];
    const { text, label } = n as Record<string, unknown>;
    if (typeof text !== 'string' || text === '') return [];
    return [typeof label === 'string' && label !== '' ? { text, label } : { text }];
  });
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** One stored row of one MOG, with the columns the list needs. */
interface Stored {
  o: Record<string, unknown>;
  mog: ConversationMog;
  requestId: string;
  askedAt: number;
  split: string;
  agent: boolean;
}

function storedOf(r: unknown, mog: ConversationMog): Stored | undefined {
  if (typeof r !== 'object' || r === null) return undefined;
  const o = r as Record<string, unknown>;
  const requestId = str(o['requestId']);
  const askedAt = o['askedAt'];
  // The brain saves a row only when a request ends, so a pending row is not a conversation yet.
  if (requestId === '' || typeof askedAt !== 'number' || o['status'] === 'pending') {
    return undefined;
  }
  return {
    o,
    mog,
    requestId,
    askedAt,
    split: str(o['splitGroupId']),
    agent: o['backendKind'] === 'genie_agent',
  };
}

function rowOf(x: Stored): ConversationRow {
  const { o, mog } = x;
  return {
    requestId: x.requestId,
    mogId: mog.mogId,
    mogName: mog.name,
    ...(mog.colorVariant !== undefined ? { mogColorVariant: mog.colorVariant } : {}),
    question: str(o['questionUser']),
    answerMd: str(o['finalAnswerMd']),
    keyNumbers: keyNumbersOf(o['keyNumbers']),
    askedAt: x.askedAt,
    profile: str(o['profileName']),
    workspaceHost: str(o['workspaceHost']),
    deepLink: str(o['deepLink']),
    status: o['status'] === 'completed' ? 'answered' : 'failed',
    conversationId: str(o['conversationId']),
  };
}

const NO_CHIP = (profile: string): AgentWsChip => ({ name: profile, colorVariant: '' });

/** The `citations` column of a turn row (ADR 0122): `[{n, profile}]`, one entry for each n. */
function storedCitationsOf(raw: unknown): { n: number; profile: string; requestId?: string }[] {
  let v: unknown;
  try {
    v = typeof raw === 'string' ? JSON.parse(raw) : raw;
  } catch {
    return [];
  }
  if (!Array.isArray(v)) return [];
  const seen = new Set<number>();
  return v.flatMap((c: unknown) => {
    if (typeof c !== 'object' || c === null) return [];
    const { n, profile, requestId } = c as Record<string, unknown>;
    if (typeof n !== 'number' || !Number.isInteger(n) || n < 1 || seen.has(n)) return [];
    if (typeof profile !== 'string' || profile === '') return [];
    seen.add(n);
    return [{ n, profile, ...(typeof requestId === 'string' ? { requestId } : {}) }];
  });
}

/**
 * The answer text of a turn row without the sources footer that a 0.1.8 model wrote itself: the
 * last lines "[n] Source: …, call-<hex>" (with a "Sources" heading, a rule and blank lines above
 * them). Bottle draws the sources itself. Any other text stays as it is.
 */
export function withoutOldFooter(md: string): string {
  const lines = md.split('\n');
  const old =
    /^\s*(?:[-*+]\s+)?\[\d{1,3}\]\s*[*_]{0,2}sources?[*_]{0,2}\s*:.*\bcall-[0-9a-f]{6,}\b/i;
  const filler = /^\s*(?:[*_#\s]*sources?[*_]{0,2}\s*:?\s*[*_]{0,2}|-{3,}|\*{3,}|_{3,})?\s*$/i;
  // From the end: the topmost old footer line under which there are only such lines or blanks.
  let cut = -1;
  for (let i = lines.length - 1; i >= 0; i -= 1) {
    const l = lines[i] ?? '';
    if (old.test(l)) cut = i;
    else if (l.trim() !== '') break;
  }
  if (cut < 0) return md;
  while (cut > 0 && filler.test(lines[cut - 1] ?? '')) cut -= 1;
  return lines.slice(0, cut).join('\n').trimEnd();
}

function partOf(x: Stored, chipOf: (profile: string) => AgentWsChip): AgentThreadPart {
  const r = rowOf(x);
  return {
    requestId: r.requestId,
    workspace: chipOf(r.profile),
    profile: r.profile,
    workspaceHost: r.workspaceHost,
    status: r.status,
    answerMd: r.answerMd,
    ...(r.deepLink ? { deepLink: r.deepLink } : {}),
    keyNumbers: r.keyNumbers,
  };
}

/** A row of the store: which MOG holds it. `delete` needs this for each MOG. */
interface KeyRows {
  rows: { mogId: string; requestId: string }[];
  /**
   * No answered exchange in it: a turn row or a part with status `completed` makes it answered.
   * The view's `isFailedThread` (AgentThread.tsx) uses the same rule on the stored turns; a test
   * keeps the two equal.
   */
  failed: boolean;
}

/** What one read gives: the list, and the rows behind each key (for Delete). */
export interface ConversationsRead {
  list: HubConversations;
  keys: Map<string, KeyRows>;
  /** The MOG of each agent thread. */
  threadMogs: Map<string, string>;
}

/** Read the merged list and the key index. */
export async function readConversationsIndexed(
  deps: ConversationsReadDeps,
): Promise<ConversationsRead> {
  try {
    const mogs = await deps.mogs();
    const per = await Promise.all(
      mogs.map(async (m) => {
        const { rows } = await deps.store.exec('mem.exchange.recent', {
          mogId: m.mogId,
          limit: CONVERSATIONS_MAX,
        });
        return rows.flatMap((r) => storedOf(r, m) ?? []);
      }),
    );
    const all = per.flat();
    const keys = new Map<string, KeyRows>();
    const plain = all
      .filter((x) => !x.agent && x.split === '')
      .map((x) => ({ x, row: rowOf(x) }))
      .sort((a, b) => b.row.askedAt - a.row.askedAt || (a.row.requestId < b.row.requestId ? 1 : -1))
      .slice(0, CONVERSATIONS_MAX);
    for (const { row } of plain) {
      const key = row.conversationId !== '' ? `c:${row.conversationId}` : `r:${row.requestId}`;
      const k = keys.get(key) ?? { rows: [], failed: true };
      k.rows.push({ mogId: row.mogId, requestId: row.requestId });
      if (row.status === 'answered') k.failed = false;
      keys.set(key, k);
    }
    const { threads, threadMogs } = agentThreadsOf(all, deps, keys);
    return {
      list: {
        rows: plain.map((p) => p.row),
        ...(threads.length > 0 ? { agentThreads: threads } : {}),
      },
      keys,
      threadMogs,
    };
  } catch {
    return { list: { rows: [], unavailable: true }, keys: new Map(), threadMogs: new Map() };
  }
}

/** Rebuild the agent threads from the turn rows and the part rows (ADR 0121). */
function agentThreadsOf(
  all: readonly Stored[],
  deps: ConversationsReadDeps,
  keys: Map<string, KeyRows>,
): { threads: AgentThread[]; threadMogs: Map<string, string> } {
  const chipOf = deps.chipOf ?? NO_CHIP;
  const memory = deps.memoryTurns?.() ?? [];
  // Every turn the client holds, shown or not: its parts are never a thread of their own (they
  // join the thread of the turn, and a Delete of that thread takes them).
  const inMemory = new Set(memory.map((t) => t.turnUuid));
  const turnRows = all.filter((x) => x.agent && UUID_RE.test(x.split));
  const parts = new Map<string, Stored[]>();
  for (const x of all) {
    if (x.agent || x.split === '') continue;
    parts.set(x.split, [...(parts.get(x.split) ?? []), x]);
  }
  const byAsked = (a: Stored, b: Stored) =>
    a.askedAt - b.askedAt || (a.requestId < b.requestId ? -1 : 1);
  const turnOf = (x: Stored, ps: Stored[]): AgentThreadTurn => {
    const sorted = [...ps].sort(byAsked);
    const status = x.o['status'];
    const answeredPart = sorted.some((p) => p.o['status'] === 'completed');
    const citations = storedCitationsOf(x.o['citations']).map(({ n, profile, requestId }) => {
      // The marker reaches the part row saved with it (the part the live chip reaches), else the
      // last answer of its workspace in this turn.
      const answered = sorted.filter(
        (p) => p.o['status'] === 'completed' && str(p.o['profileName']) === profile,
      );
      const part = answered.find((p) => p.requestId === requestId) ?? answered.at(-1);
      return { n, workspace: chipOf(profile), ...(part ? { requestId: part.requestId } : {}) };
    });
    return {
      turnUuid: x.requestId,
      question: str(x.o['questionUser']),
      askedAt: x.askedAt,
      // A fallback turn is "failed" on its own row but the home part answered: it shows answered.
      status:
        status === 'completed' || (status !== 'cancelled' && answeredPart)
          ? 'answered'
          : status === 'cancelled'
            ? 'stopped'
            : 'failed',
      summaryMd: withoutOldFooter(str(x.o['finalAnswerMd'])),
      parts: sorted.map((p) => partOf(p, chipOf)),
      ...(citations.length > 0 ? { citations } : {}),
    };
  };
  type Draft = { mog: ConversationMog; turns: AgentThreadTurn[]; rows: Stored[] };
  const threads = new Map<string, Draft>();
  const used = new Set<string>();
  for (const x of [...turnRows].sort(byAsked)) {
    const ps = parts.get(x.requestId) ?? [];
    used.add(x.requestId);
    const d = threads.get(x.split) ?? { mog: x.mog, turns: [], rows: [] };
    // The stored turn is always sent: the view swaps it for the in-memory turn of the same id.
    d.turns.push(turnOf(x, ps));
    d.rows.push(x, ...ps);
    threads.set(x.split, d);
  }
  // A group of parts with no turn row: a 0.1.5 build, or a turn that has not written its row yet.
  for (const [group, ps] of parts) {
    if (used.has(group) || !UUID_RE.test(group) || inMemory.has(group)) continue;
    const first = [...ps].sort(byAsked)[0];
    if (!first) continue;
    const turn = turnOf({ ...first, requestId: group, o: { status: 'failed' } }, ps);
    threads.set(group, { mog: first.mog, turns: [turn], rows: ps });
  }
  // The parts of a turn that is in memory and has no turn row (it never wrote one, or has not yet):
  // they are not sent, but a Delete of their thread must take them.
  const extra = new Map<string, Stored[]>();
  for (const t of memory) {
    if (used.has(t.turnUuid)) continue;
    const ps = parts.get(t.turnUuid);
    if (ps) extra.set(t.threadId, [...(extra.get(t.threadId) ?? []), ...ps]);
  }
  const out: AgentThread[] = [];
  const threadMogs = new Map<string, string>();
  const keyRows = (d: Draft | undefined, more: Stored[]): KeyRows => {
    const rows = [...(d?.rows ?? []), ...more];
    return {
      rows: rows.map((x) => ({ mogId: x.mog.mogId, requestId: x.requestId })),
      failed: !rows.some((x) => x.o['status'] === 'completed'),
    };
  };
  for (const [id, more] of extra)
    if (!threads.has(id)) keys.set(`a:${id}`, keyRows(undefined, more));
  for (const [id, d] of threads) {
    keys.set(`a:${id}`, keyRows(d, extra.get(id) ?? []));
    threadMogs.set(id, d.mog.mogId);
    out.push({
      threadId: id,
      mogId: d.mog.mogId,
      mogName: d.mog.name,
      ...(d.mog.colorVariant !== undefined ? { mogColorVariant: d.mog.colorVariant } : {}),
      turns: d.turns,
    });
  }
  const last = (t: AgentThread) => t.turns[t.turns.length - 1]?.askedAt ?? 0;
  return { threads: out.sort((a, b) => last(b) - last(a)), threadMogs };
}

/** Read the merged list: the newest `CONVERSATIONS_MAX` exchanges of all MOGs. */
export async function readConversations(deps: ConversationsReadDeps): Promise<HubConversations> {
  return (await readConversationsIndexed(deps)).list;
}

/** Wire the hub Conversations list. */
export function wireHubConversations(
  bus: BrainBus,
  deps: ConversationsWireDeps,
): HubConversationsWiring {
  let held: { requestId?: string; threadId?: string } | undefined;
  /** The rows behind each key, from the last read (Delete resolves from here, never from a view). */
  let last: ConversationsRead | undefined;
  /**
   * The owner of each conversation, from the last list read (#145). A conversation never changes
   * its MOG or its workspace, so a send takes it from here with no store round trip. Every
   * publish replaces it, so a Forget or a Wipe (which refresh the list) drops a gone thread.
   */
  let owners = new Map<string, { mogId: string; profile: string; host: string }>();
  let known = new Map<string, { profile: string; host: string }>();
  const remember = (rows: readonly ConversationRow[]) => {
    owners = new Map(
      rows
        .filter((r) => r.conversationId !== '')
        .map((r) => [
          r.conversationId,
          { mogId: r.mogId, profile: r.profile, host: r.workspaceHost },
        ]),
    );
  };
  const publish = async () => {
    try {
      const read = await readConversationsIndexed(deps);
      const list = read.list;
      if (!list.unavailable) {
        remember(list.rows);
        last = read;
      }
      known = new Map(
        list.rows.map((r) => [r.requestId, { profile: r.profile, host: r.workspaceHost }]),
      );
      // The saved parts of an agent thread are not plain rows, and their links must open too.
      for (const t of list.agentThreads ?? [])
        for (const turn of t.turns)
          for (const part of turn.parts)
            known.set(part.requestId, { profile: part.profile, host: part.workspaceHost });
      await deps.send(list);
    } catch {
      // No hub is open. The hub asks again with conversations.ready when it opens.
    }
  };
  /** Threads in progress when a Delete came: `retryDeferred` deletes them once they settled. */
  const deferred = new Map<string, boolean>();
  const runningThreads = () =>
    new Set((deps.memoryTurns?.() ?? []).filter((t) => t.inProgress).map((t) => t.threadId));
  const remove = async (
    keys: readonly string[],
    opts: { onlyFailed?: boolean } = {},
  ): Promise<void> => {
    try {
      if (keys.length === 0 || keys.length > CONVERSATIONS_DELETE_MAX) return;
      // A fresh read, never the last one: a row saved since then would come back.
      const read = await readConversationsIndexed(deps);
      if (read.list.unavailable) return;
      last = read;
      const running = runningThreads();
      const byMog = new Map<string, Set<string>>();
      /** The agent clients' threads to forget, with the MOGs whose rows must go first. */
      const forget = new Map<string, Set<string>>();
      /** The Genie threads (`c:`) to drop as a follow-up target, with their MOGs. */
      const forgetConv = new Map<string, Set<string>>();
      let deferredNow = 0;
      for (const key of new Set(keys)) {
        // A thread in progress is not deleted yet: its rows are not all saved. It goes once the
        // turn settled (the view stopped showing it as in progress a moment before the brain).
        if (key.startsWith('a:') && running.has(key.slice(2))) {
          deferred.set(key, opts.onlyFailed === true);
          deferredNow += 1;
          continue;
        }
        const k = read.keys.get(key);
        if (k && opts.onlyFailed && !k.failed) continue;
        // An answered thread in memory only (no stored row) has no key: Clear failed skips it.
        if (!k && opts.onlyFailed) continue;
        if (key.startsWith('a:')) forget.set(key.slice(2), new Set(k?.rows.map((r) => r.mogId)));
        if (key.startsWith('c:'))
          forgetConv.set(key.slice(2), new Set(k?.rows.map((r) => r.mogId)));
        for (const r of k?.rows ?? []) {
          const set = byMog.get(r.mogId) ?? new Set<string>();
          set.add(r.requestId);
          byMog.set(r.mogId, set);
        }
      }
      // Every thread waits: nothing changed, and a list now would show the thread again.
      if (deferredNow === new Set(keys).size) return;
      const failedMogs = new Set<string>();
      const now = Date.now();
      for (const [mogId, ids] of byMog) {
        const all = [...ids];
        for (let i = 0; i < all.length; i += CONVERSATIONS_DELETE_MAX) {
          try {
            await deps.store.exec('mem.exchange.delete', {
              mogId,
              requestIds: all.slice(i, i + CONVERSATIONS_DELETE_MAX),
              now,
            });
          } catch {
            failedMogs.add(mogId);
          }
        }
      }
      // The client forgets a thread only when every delete of it worked: else the stored copy
      // would come back with fewer details.
      for (const [id, mogIds] of forget) {
        if (![...mogIds].some((m) => failedMogs.has(m))) deps.forgetThread?.(id);
      }
      for (const [id, mogIds] of forgetConv) {
        if (![...mogIds].some((m) => failedMogs.has(m))) deps.forgetConversation?.(id);
      }
    } catch {
      // A failed delete shows again on the next list.
    }
    await publish();
  };
  const retryDeferred = async (): Promise<void> => {
    if (deferred.size === 0) return;
    try {
      const running = runningThreads();
      const ready = [...deferred].filter(([key]) => !running.has(key.slice(2)));
      if (ready.length === 0) return;
      for (const [key] of ready) deferred.delete(key);
      for (const only of [false, true]) {
        const keys = ready.filter(([, f]) => f === only).map(([key]) => key);
        if (keys.length > 0) await remove(keys, { onlyFailed: only });
      }
    } catch {
      // The thread shows again; the user deletes it again.
    }
  };
  const off = onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.mogId !== CONVERSATIONS_VIEW_ID || input.kind !== 'click')
      return;
    const action = asConversationsAction(input.payload);
    if (!action) return;
    if (action.action === 'conversations.opened') {
      held = undefined;
      return;
    }
    if (action.action === 'conversations.delete') {
      // Clear failed sends `onlyFailed` with the same keys: the brain checks each again.
      void remove(action.threads, { onlyFailed: action.onlyFailed === true });
      return;
    }
    if (action.action === 'conversations.copy_markdown') {
      // Resolved from the brain's own last list read, never from the view: the click only
      // names the row by id.
      const row = last?.list.rows.find((r) => r.requestId === action.requestId);
      if (row && row.answerMd !== '') void deps.copyMarkdown?.(row.answerMd);
      return;
    }
    if (action.action === 'conversations.copy') {
      const row = last?.list.rows.find((r) => r.requestId === action.requestId);
      if (row && row.answerMd !== '') deps.copyFormatted?.(row.answerMd, row.workspaceHost);
      return;
    }
    void (async () => {
      await publish();
      const open = held;
      held = undefined;
      if (open) await deps.sendOpen(open).catch(() => undefined);
    })();
  });
  return {
    refresh: publish,
    async push() {
      try {
        if (deps.hubPresent && !(await deps.hubPresent())) return;
      } catch {
        return;
      }
      await publish();
    },
    workspaceOf: (requestId) => known.get(requestId),
    async openThread(threadId) {
      held = { threadId };
      try {
        await deps.sendOpen(held);
      } catch {
        // No hub is open yet: the held selection goes when the hub asks.
      }
    },
    threadMog: (threadId) => last?.threadMogs.get(threadId),
    remove,
    retryDeferred,
    async openConversations(requestId) {
      const open = requestId ? { requestId } : {};
      held = open;
      try {
        await deps.sendOpen(open);
      } catch {
        // No hub is open yet: the held selection goes when the hub asks.
      }
    },
    async findThread(conversationId) {
      if (conversationId === '') return undefined;
      const cached = owners.get(conversationId);
      if (cached) return cached;
      try {
        const { rows } = await readConversations(deps);
        const row = rows.find((r) => r.conversationId === conversationId);
        return row
          ? { mogId: row.mogId, profile: row.profile, host: row.workspaceHost }
          : undefined;
      } catch {
        return undefined;
      }
    },
    dispose: off,
  };
}
