/**
 * The PollActor of the ConversationManager (#31, ADR 0073): one serial poll loop for each
 * accepted ask, on the poll path of Genie One MCP.
 *
 * - G-07a: each poll is `mcp_call genie_poll_response {conversation_id, response_id}` with no
 *   `meta`, so no request carries `_meta.progressToken`.
 * - G-07b, ARC-F1: each poll runs on the ConversationManager poll queue of its conversation, one
 *   at a time. Rust R8 also refuses an overlapping poll with `poll_inflight` (E_POLL_INFLIGHT),
 *   for example a poll of the main agent; the actor then polls again later with the same IDs.
 * - G-07c: the delay follows Table G-POLL and counts from the end of the previous poll.
 * - G-08: each delay is at least 1.5 s x the in-flight asks on the profile.
 * - G-18a, G-18b, ARC-F12: `unknown` (and any status the bridge does not know) is not terminal;
 *   the actor polls again with the same IDs until the give-up.
 * - G-16c, ARC-F4: each poll has a 30 s time cap; a poll timeout polls again with the same IDs.
 * - ARC-F10: the IDs, the profile and the ask time go to a {@link PollJournal} when the loop
 *   starts. Only a terminal status removes them, so after a crash the loop resumes with the
 *   same IDs and nothing is asked again.
 *
 * The actor never calls `genie_ask`: it cannot ask a question again (G-13b, MOG-10b).
 * At the give-up the loop ends with `gave_up`, and `resume` yields `abandoned`. The soft notice,
 * the `abandoned` exchange status and the late re-polls are TODO(#149).
 * The `timeout` and `poll_inflight` codes cross IPC as themselves (#495). The Tauri host routes
 * `mcp_call` to R8 in TODO(#467).
 */
// Adapted from datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_bridge.py @ a31fd2c (MIT).
// From it: after a poll runs out of time, poll again with the same conversation and response
// IDs, because asking again starts a duplicate question (`late_text`, `OUT_OF_TIME`).
import type { MogId, ProfileName } from '../../shared/interfaces';
import type { CommandArgs, CommandReturns } from '../../shared/ipc';

/** ARC-F4: the time cap of one poll. */
export const POLL_TIMEOUT_MS = 30_000;
/** ARC-F4, G-12b: no poll starts after this time from the ask. */
export const GIVE_UP_MS = 10 * 60_000;
/** G-08: the delay floor for each in-flight ask on the profile. */
export const INFLIGHT_FLOOR_MS = 1_500;
/** Table G-POLL: the longest base delay. */
export const MAX_POLL_DELAY_MS = 15_000;
/** Table G-POLL: the jitter on every step (+-10 %). */
export const POLL_JITTER = 0.1;
/** R8 `E_POLL_INFLIGHT` (`mog_core::mcp_transport::E_POLL_INFLIGHT`, ARC-F1). */
export const E_POLL_INFLIGHT = 'poll_inflight';
/** R8 code of a call that ran out of time (`mog_core::http_transport::TIMEOUT`). */
export const E_TIMEOUT = 'timeout';

export type PollStatus = 'in_progress' | 'completed' | 'incomplete' | 'failed' | 'unknown';
export type TerminalPollStatus = 'completed' | 'incomplete' | 'failed';
const TERMINAL: ReadonlySet<string> = new Set<TerminalPollStatus>([
  'completed',
  'incomplete',
  'failed',
]);

export type McpCallArgs = CommandArgs['mcp_call'];
export type McpCallReply = CommandReturns['mcp_call'];
/** One `mcp_call` to the Rust core. */
export type McpCaller = (args: McpCallArgs) => Promise<McpCallReply>;

/**
 * The base delay of Table G-POLL for an ask that is `elapsedMs` old. After 180 s the base is
 * the previous base x 1.5, at most 15 s.
 */
export function basePollDelayMs(elapsedMs: number, previousBaseMs?: number): number {
  if (elapsedMs < 20_000) return 2_000;
  if (elapsedMs < 60_000) return 3_000;
  if (elapsedMs < 180_000) return 5_000;
  return Math.min(Math.max(previousBaseMs ?? 5_000, 5_000) * 1.5, MAX_POLL_DELAY_MS);
}

/**
 * The delay before the next poll (G-07c, G-08): the Table G-POLL base, +-10 % jitter, then at
 * least 1.5 s x the in-flight asks on the profile. `random` gives a number in [0, 1).
 */
export function pollDelayMs(p: {
  elapsedMs: number;
  previousBaseMs?: number | undefined;
  inflightAsks: number;
  random: () => number;
}): { baseMs: number; delayMs: number } {
  const baseMs = basePollDelayMs(p.elapsedMs, p.previousBaseMs);
  const r = Math.min(Math.max(p.random(), 0), 1);
  const jittered = baseMs * (1 + POLL_JITTER * (2 * r - 1));
  const floor = INFLIGHT_FLOOR_MS * Math.max(1, Math.floor(p.inflightAsks));
  return { baseMs, delayMs: Math.max(jittered, floor) };
}

/** What a PollActor needs to poll one accepted ask. This is what the journal keeps (ARC-F10). */
export interface PollTarget {
  opId: string;
  mogId: MogId;
  profile: ProfileName;
  host: string;
  conversationId: string;
  responseId: string;
  /** When the ask was sent, on the clock of {@link PollEnv.now}. */
  askedAt: number;
}

/** ARC-F10: where the PollActor keeps its IDs and profile for crash recovery. */
export interface PollJournal {
  save(target: PollTarget): void;
  get(conversationId: string, responseId: string): PollTarget | undefined;
  remove(conversationId: string, responseId: string): void;
  list(): PollTarget[];
}

/** A journal in brain memory. TODO(#39): RecoveryManager keeps it in the `genie_exchange` row. */
export function createMemoryPollJournal(): PollJournal {
  const rows = new Map<string, PollTarget>();
  const key = (c: string, r: string) => JSON.stringify([c, r]);
  return {
    save: (t) => void rows.set(key(t.conversationId, t.responseId), { ...t }),
    get: (c, r) => {
      const t = rows.get(key(c, r));
      return t && { ...t };
    },
    remove: (c, r) => void rows.delete(key(c, r)),
    list: () => [...rows.values()].map((t) => ({ ...t })),
  };
}

/** G-08: the in-flight asks of each profile, counted over every PollActor of the brain. */
export interface InflightAsks {
  /** Count one ask on `profile` until the returned function runs. */
  add(profile: ProfileName): () => void;
  count(profile: ProfileName): number;
}

export function createInflightAsks(): InflightAsks {
  const open = new Map<ProfileName, Set<object>>();
  return {
    add(profile) {
      const token = {};
      const set = open.get(profile) ?? new Set<object>();
      set.add(token);
      open.set(profile, set);
      return () => {
        set.delete(token);
        if (set.size === 0 && open.get(profile) === set) open.delete(profile);
      };
    },
    count: (profile) => open.get(profile)?.size ?? 0,
  };
}

/** One poll reply that the actor passes on. */
export interface PollUpdate {
  /** 1 for the first poll reply of this loop. */
  n: number;
  status: PollStatus;
  /** The `structuredContent` of the reply. */
  result: Record<string, unknown>;
  /** The time since the ask, when the poll ended. */
  elapsedMs: number;
}

/** How a poll loop ends. */
export type PollOutcome =
  | { kind: 'terminal'; status: TerminalPollStatus; result: Record<string, unknown> }
  /** An `isError` reply. The GL-5 mapper (#32) classifies it; the IDs stay in the journal. */
  | { kind: 'error'; reply: McpCallReply }
  /** The give-up time came (G-12b). The IDs stay in the journal for the late re-polls (#149). */
  | { kind: 'gave_up' }
  | { kind: 'aborted' };

/** The clock, timers and seams of the PollActor. Tests give a sim clock and a double. */
export interface PollEnv {
  call: McpCaller;
  /** Milliseconds; `PollTarget.askedAt` is on this clock. */
  now: () => number;
  /** Resolve after `ms`, or at once when `signal` aborts. */
  sleep: (ms: number, signal?: AbortSignal) => Promise<void>;
  /** A number in [0, 1) for the jitter. */
  random: () => number;
  journal: PollJournal;
  inflight: InflightAsks;
  giveUpMs: number;
  pollTimeoutMs: number;
}

/** Run `task` on the poll queue of the conversation (ConversationManager.enqueuePoll). */
export type PollQueue = <T>(conversationId: string, task: () => Promise<T>) => Promise<T>;

/** The fixed code of a refused IPC call (`{ error: code }`), if `e` is one. */
export function ipcErrorCode(e: unknown): string | undefined {
  if (e && typeof e === 'object' && 'error' in e) {
    const code = (e as { error: unknown }).error;
    if (typeof code === 'string') return code;
  }
  return undefined;
}

function pollStatus(v: unknown): PollStatus {
  // ARC-F12: a status the bridge does not know is `unknown`, so it is not terminal.
  return v === 'in_progress' || v === 'completed' || v === 'incomplete' || v === 'failed'
    ? v
    : 'unknown';
}

function asObject(v: unknown): Record<string, unknown> {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
}

/** The `mcp_call` of one poll: no `meta` (G-07a), the 30 s cap (ARC-F4). */
export function pollCall(t: PollTarget, timeoutMs = POLL_TIMEOUT_MS): McpCallArgs {
  return {
    profile: t.profile,
    server: 'genie_one',
    tool: 'genie_poll_response',
    args: { conversation_id: t.conversationId, response_id: t.responseId },
    opId: t.opId,
    tags: { app: 'bottle', mog: t.mogId, op: 'genie' },
    timeoutMs,
  };
}

/**
 * The PollActor loop. It yields each poll reply and returns how the loop ended. The caller
 * counts the ask in `env.inflight` (ConversationManager.poll does).
 */
export async function* runPollActor(
  target: PollTarget,
  env: PollEnv,
  queue: PollQueue,
  signal?: AbortSignal,
): AsyncGenerator<PollUpdate, PollOutcome, undefined> {
  env.journal.save(target);
  const elapsed = () => Math.max(0, env.now() - target.askedAt);
  let previousBaseMs: number | undefined;
  let n = 0;
  for (;;) {
    if (signal?.aborted) return { kind: 'aborted' };
    // The delay counts from now: the end of the ask or of the previous poll (G-07c).
    const at = elapsed();
    const { baseMs, delayMs } = pollDelayMs({
      elapsedMs: at,
      previousBaseMs,
      inflightAsks: env.inflight.count(target.profile),
      random: env.random,
    });
    previousBaseMs = baseMs;
    // G-08: an ask that starts while this actor sleeps raises the floor too, so check it again
    // on each wake-up and sleep on to the later time.
    const from = env.now();
    let due = from + delayMs;
    for (;;) {
      // ARC-F4, G-12b: no poll starts after the give-up. TODO(#149): late re-polls.
      if (due - target.askedAt > env.giveUpMs) return { kind: 'gave_up' };
      await env.sleep(due - env.now(), signal);
      if (signal?.aborted) return { kind: 'aborted' };
      const floorDue = from + INFLIGHT_FLOOR_MS * Math.max(1, env.inflight.count(target.profile));
      if (floorDue <= env.now()) break;
      due = floorDue;
    }
    let reply: McpCallReply;
    try {
      reply = await queue(target.conversationId, () =>
        env.call(pollCall(target, env.pollTimeoutMs)),
      );
    } catch (e) {
      const code = ipcErrorCode(e);
      // G-16c: a poll timeout polls again with the same IDs. ARC-F1: another poll on this
      // conversation is open (the main agent's), so wait one more delay and poll again.
      // Both codes are in ERROR_CODES (#495), so they cross IPC as themselves.
      if (code === E_TIMEOUT || code === E_POLL_INFLIGHT) continue;
      // Anything else ends the loop. The IDs stay in the journal, so polls can resume (ARC-F5).
      throw e;
    }
    if (reply.isError) return { kind: 'error', reply };
    const result = asObject(reply.structuredContent);
    const status = pollStatus(result.status);
    yield { n: ++n, status, result, elapsedMs: elapsed() };
    if (TERMINAL.has(status)) {
      env.journal.remove(target.conversationId, target.responseId);
      return { kind: 'terminal', status: status as TerminalPollStatus, result };
    }
  }
}
