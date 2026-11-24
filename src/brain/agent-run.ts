/**
 * Slice 2a: the brain client of the one live Genie Code session (plan Amendment 2, Part B).
 *
 * - `submit` sends one ⌥G question with `agent_turn_submit` and an `opId`. The core needs the
 *   ⌥G latch or an Ask ticket, and it starts a session if none is open.
 * - The `agent:turn` and `agent:session` events (content-free) move one `AgentTurnView` for each
 *   turn. On a terminal call the client pulls `agent_turn_call` (the question and the reply),
 *   saves one history row, and keeps the answer as a part. On `ended` with `answered` it pulls
 *   `agent_turn_result`: the note of the MOG, and the sources that decide which parts fold.
 * - AC-2a.11: a turn that ends `timeout`, `offline` or `failed` before any Genie call falls back
 *   to one direct home call, with a one-line reason.
 * - R-13, R-14: the follow-up window learns of each answer and of each session end.
 * - The content of every turn lives in this object only, in memory. A new session, quit or Wipe
 *   clears it. Nothing here writes a question, an answer or a reply to the log (SEC-16c).
 *
 * The clock and every side effect come in through `AgentClientDeps`, so tests use doubles.
 */
import type { AgentTurn, MogId, ProfileName } from '../shared/interfaces';
import {
  call,
  type AgentCallState,
  type AgentCallView,
  type AgentPartView,
  type AgentSessionEvent,
  type AgentSessionState,
  type AgentTurnAccepted,
  type AgentTurnCall,
  type AgentTurnEvent,
  type AgentTurnResult,
  type AgentTurnView,
  type AgentWorkspace,
  type AgentWsChip,
  type CommandArgs,
} from '../shared/ipc';
import type { HubAgentView } from '../shared/hub-agent';
import {
  callViewState,
  fallbackNotice,
  isTerminalCall,
  partOf,
  REFUSAL_TEXT,
  replyRefs,
  answerOf,
  whyOfOutcome,
  whyOfRefusal,
} from './agent-card';
import type { FollowUpWindow } from './agent-route';
import { keyNumbersOf, type TerminalAskEvent } from './components/ask-events';
import type { GenieEnd } from './genie-history';
import type { GeniePhase } from './genie-direct';
import type { BrainLog } from './log';

/** The mode of a submit. `ask_others` is the click on the R-8 chip. */
export type AgentMode = NonNullable<CommandArgs['agent_turn_submit']['mode']>;

/** The IPC functions of the client, one for each command. */
export interface AgentApi {
  submit(args: CommandArgs['agent_turn_submit']): Promise<AgentTurnAccepted>;
  stop(turnId: string): Promise<unknown>;
  callDetail(turnId: string, callId: string): Promise<AgentTurnCall>;
  result(turnId: string): Promise<AgentTurnResult>;
  sessionState(): Promise<AgentSessionState>;
  endSession(): Promise<unknown>;
}

export function ipcAgentApi(): AgentApi {
  return {
    submit: (args) => call('agent_turn_submit', args),
    stop: (turnId) => call('agent_turn_stop', { turnId }),
    callDetail: (turnId, callId) => call('agent_turn_call', { turnId, callId }),
    result: (turnId) => call('agent_turn_result', { turnId }),
    sessionState: () => call('agent_session_state', {}),
    endSession: () => call('agent_session_end', { reason: 'new_session' }),
  };
}

export interface AgentClientDeps {
  api: AgentApi;
  /** The name and colour of a workspace, from the list that Settings shows. Sync, from a cache. */
  chipOf(profile: ProfileName): AgentWsChip;
  /** The names for the fallback line: the workspace the agent runs on, and home. */
  names(): { agent: string; home: string };
  homeProfile(): Promise<ProfileName | undefined>;
  hostOf(profile: ProfileName): Promise<string | undefined>;
  /** A new UUID, for the request id of one history row. */
  newId(): string;
  /** Wall time in Unix ms, for the start and end of a call row. */
  wallNow(): number;
  /** One history row for each Genie call (`genie-history.ts`). Never rejects. */
  recordCall?(x: GenieEnd): void;
  /** The fallback: one direct home call, with the notice line. */
  direct(turn: AgentTurn, notice: string, turnId?: string): Promise<void>;
  /** The card of `mogId` shows this turn (`agent-card.ts`). */
  showTurn(mogId: MogId, turn: AgentTurnView): void;
  /** The card of a turn that left (a removed turn, a cleared session). */
  dropTurn(mogId: MogId, turnId: string): void;
  /** A one-line pop-up on the card of `mogId` (the queue is full). */
  note?(mogId: MogId, text: string): void;
  /** The MOG mood follows the turn. */
  phase?(mogId: MogId, phase: GeniePhase): void;
  followUp?: Pick<FollowUpWindow, 'answered' | 'reset'>;
  /** The panel changed: the hub gets a new `HubAgentView`. */
  onChange?(): void;
  /** A fixed code line only. */
  log?: Pick<BrainLog, 'event'>;
}

type Panel = Pick<HubAgentView, 'session' | 'live' | 'waiting' | 'earlier' | 'threads'>;

interface TurnRec {
  view: AgentTurnView;
  origin: AgentTurn;
  catalog: Map<string, AgentWorkspace>;
  crossWorkspace: boolean;
  agentProfile: ProfileName | undefined;
  /** The answered parts, by call id, in the order the calls ended. */
  parts: Map<string, AgentPartView>;
  fetched: Set<string>;
  pending: Set<Promise<unknown>>;
  /** The mode of the submit: an `ask_others` turn never asks home again (R-9). */
  mode: AgentMode;
  /** Terminal pulls that failed once: the turn retries them once when it ends. */
  failedPulls: Set<string>;
  /** Set when the turn ended; the order of `earlier`. */
  endedSeq?: number;
  ended: boolean;
  fellBack: boolean;
  /** The root UUID of the thread of this turn (the `request_id` of the first turn of the thread). */
  threadId: string;
  /** The turn row was asked for (once). */
  recorded: boolean;
  /** A Wipe ended the session while the turn ran or after it: the turn writes no row. */
  noRecord: boolean;
  /** The outcome the turn ended with (set when `finish` starts). */
  outcome?: string;
  /** `finish` is over: the turn row is asked for, and a Delete may take the turn. */
  settled: boolean;
}

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);
const codeOf = (e: unknown): string =>
  isObj(e) && typeof e['error'] === 'string' ? e['error'] : '';
const TURN_ID = /^[0-9a-f]{32}$/;
const CALL_ID = /^c-[0-9a-f]{8}$/;
const isCount = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0;

/** The 32 hex digits of a turn id as a UUID, for `genie_exchange.split_group_id`. */
export const uuidOfTurn = (turnId: string): string =>
  `${turnId.slice(0, 8)}-${turnId.slice(8, 12)}-${turnId.slice(12, 16)}-${turnId.slice(16, 20)}-${turnId.slice(20)}`;

const CALL_STATES: readonly string[] = [
  'queued',
  'asking',
  'answered',
  'no_answer',
  'failed',
  'timeout',
  'signed_out',
  'stopped',
];
const REFUSED_CODES: readonly string[] = [
  'workspace_not_available',
  'budget_reached',
  'cross_workspace_off',
  'invalid_args',
];
const OUTCOMES: readonly string[] = [
  'answered',
  'stopped',
  'removed',
  'timeout',
  'offline',
  'signed_out',
  'not_eligible',
  'failed',
  'lockdown',
  'refusals',
  'session_ended',
];

/** A shape check of one `agent:turn` payload. A bad item is dropped. */
export function asTurnEvent(p: unknown): AgentTurnEvent | undefined {
  if (!isObj(p) || typeof p['turnId'] !== 'string' || !TURN_ID.test(p['turnId'])) return undefined;
  const turnId = p['turnId'];
  switch (p['kind']) {
    case 'queued':
      return isCount(p['position'])
        ? { kind: 'queued', turnId, position: p['position'] }
        : undefined;
    case 'started':
    case 'thinking':
    case 'slow':
      return { kind: p['kind'], turnId };
    case 'call': {
      const { callId, workspaceId, state, used, budget } = p;
      if (typeof callId !== 'string' || !CALL_ID.test(callId)) return undefined;
      if (typeof workspaceId !== 'string' || !CALL_STATES.includes(state as string))
        return undefined;
      if (!isCount(used) || !isCount(budget)) return undefined;
      return {
        kind: 'call',
        turnId,
        callId,
        workspaceId,
        state: state as AgentCallState,
        used,
        budget,
      };
    }
    case 'refused': {
      const code = p['code'];
      if (!REFUSED_CODES.includes(code as string) || !isCount(p['used'])) return undefined;
      return { kind: 'refused', turnId, code: code as never, used: p['used'] };
    }
    case 'ended': {
      if (!OUTCOMES.includes(p['outcome'] as string) || !isCount(p['calls'])) return undefined;
      const tokens = p['tokens'];
      if (tokens !== undefined && !isCount(tokens)) return undefined;
      return {
        kind: 'ended',
        turnId,
        outcome: p['outcome'] as never,
        calls: p['calls'],
        ...(tokens !== undefined ? { tokens } : {}),
      };
    }
    default:
      return undefined;
  }
}

export function asSessionEvent(p: unknown): AgentSessionEvent | undefined {
  if (!isObj(p) || typeof p['sessionId'] !== 'string') return undefined;
  if (p['kind'] === 'started') return { kind: 'started', sessionId: p['sessionId'] };
  if (p['kind'] === 'ended' && typeof p['reason'] === 'string')
    return { kind: 'ended', sessionId: p['sessionId'], reason: p['reason'] as never };
  return undefined;
}

/** Session end reasons after which no turn of the session stays (the context is gone). */
const CLEARING: ReadonlySet<string> = new Set([
  'new_session',
  'quit',
  'wipe',
  'tainted',
  'profile_changed',
  'd1_changed',
  'catalog_changed',
  'idle',
]);

/** The most earlier turns the panel keeps in memory. */
export const EARLIER_MAX = 20;
/** The most events that wait for a `submit` to return. */
const EARLY_MAX = 32;

const NO_SESSION: AgentSessionState = {
  state: 'none',
  agentProfile: null,
  turns: 0,
  queued: 0,
  tokens: 0,
};

export interface AgentClient {
  /** Send one question to the session. It resolves when the core accepted the turn (or fell back). */
  submit(
    turn: AgentTurn,
    mode: AgentMode,
    actionTicket?: string,
    /** The thread this turn continues (the hub thread composer). Else see `lastAnswered`. */
    threadId?: string,
  ): Promise<void>;
  onTurnEvent(payload: unknown): void;
  onSessionEvent(payload: unknown): void;
  /** Wipe: forget every finished turn, also those that a crash left on screen. */
  clear(): void;
  /** Stop the live turn, or remove a waiting one. */
  stop(turnId: string): Promise<void>;
  /** The New session button. */
  newSession(): Promise<void>;
  /** Read the session state again (the panel header). */
  refreshSession(): Promise<void>;
  /** The turns that hub Conversations shows now (the genie code status is the caller's). */
  panel(): Panel;
  /** The turn view of `turnId`, if the client holds it. */
  turn(turnId: string): AgentTurnView | undefined;
  /** The thread (root UUID) of `turnId`, if the client holds the turn. */
  threadOf(turnId: string): string | undefined;
  /** The MOG that asked in a thread of the panel, if the client holds one of its turns. */
  threadMog(threadId: string): MogId | undefined;
  /**
   * The turns in memory: their stored `request_id`, thread, whether they are in progress, and
   * whether `panel()` sends them to the hub (`shown`; a turn that is not shown has no memory view,
   * so its stored turn must be listed).
   */
  turns(): {
    turnUuid: string;
    threadId: string;
    mogId: MogId;
    inProgress: boolean;
    shown: boolean;
  }[];
  /** Whether the open session holds a turn of this thread (a reply is a `follow_up`). */
  inSession(threadId: string): boolean;
  /** Delete: forget the finished turns of a thread (an in-progress turn stays). */
  forgetThread(threadId: string): void;
}

export function createAgentClient(deps: AgentClientDeps): AgentClient {
  const recs = new Map<string, TurnRec>();
  /** Events that came before the `submit` reply, by turn id. */
  const early = new Map<string, AgentTurnEvent[]>();
  let session: AgentSessionState = NO_SESSION;
  let seq = 0;
  /** The session ended in a way that leaves its failed turns on screen until the next start. */
  let stale = false;
  /** The thread of the newest answered turn: a ⌥G follow-up continues it (R-13). */
  let lastAnswered: string | undefined;
  /**
   * A brain-side Wipe fence (security L3): it counts the Wipes this brain saw. A write notes the
   * number when it starts and refuses to save when the number moved by the time it is ready.
   */
  let wipeGen = 0;
  const fault = () => void deps.log?.event('agent.turn', { ok: 0 });
  const changed = () => {
    try {
      deps.onChange?.();
    } catch {
      fault();
    }
  };
  const show = (r: TurnRec) => {
    try {
      deps.showTurn(r.origin.mogId, r.view);
    } catch {
      fault();
    }
  };
  const update = (r: TurnRec) => {
    show(r);
    changed();
  };

  function newView(a: AgentTurnAccepted, question: string): TurnRec['view'] {
    const agentWs = a.catalog.find((c) => c.isAgentWorkspace);
    return {
      turnId: a.turnId,
      question,
      state: a.position > 0 ? 'queued' : 'planning',
      ...(a.position > 0 ? { position: a.position } : {}),
      used: 0,
      budget: a.budget,
      agentWorkspace: agentWs
        ? deps.chipOf(agentWs.profile)
        : { name: deps.names().agent, colorVariant: '' },
      calls: [],
      parts: [],
    };
  }

  const chipOfAlias = (r: TurnRec, alias: string): AgentWsChip => {
    const ws = r.catalog.get(alias);
    return ws ? deps.chipOf(ws.profile) : { name: alias, colorVariant: '' };
  };

  const syncParts = (r: TurnRec) => {
    r.view = { ...r.view, parts: [...r.parts.values()] };
  };

  /** Pull the question (and, when it ended, the reply) of one call. */
  async function pullCall(r: TurnRec, callId: string, terminal: boolean): Promise<void> {
    let d: AgentTurnCall;
    try {
      d = await deps.api.callDetail(r.view.turnId, callId);
    } catch {
      fault();
      if (terminal) r.failedPulls.add(callId);
      return;
    }
    // A turn that a Wipe or a new session cleared writes no row and shows nothing.
    if (recs.get(r.view.turnId) !== r) return;
    const row = r.view.calls.find((c) => c.callId === callId);
    if (row && d.question !== '' && row.question !== d.question) {
      r.view = {
        ...r.view,
        calls: r.view.calls.map((c) => (c.callId === callId ? { ...c, question: d.question } : c)),
      };
    }
    if (!terminal || !isTerminalCall(d.state) || r.fetched.has(callId)) {
      update(r);
      return;
    }
    r.fetched.add(callId);
    const ws = r.catalog.get(d.workspaceId);
    const host = await deps.hostOf(d.profile).catch(() => undefined);
    if (recs.get(r.view.turnId) !== r) return;
    const part = partOf(d, {
      chip: deps.chipOf(d.profile),
      host,
      notShownToAgent: !r.crossWorkspace && ws !== undefined && !ws.isAgentWorkspace,
    });
    if (part) r.parts.set(callId, part);
    syncParts(r);
    await record(r, d);
    update(r);
  }

  /** One history row for each Genie call. */
  async function record(r: TurnRec, d: AgentTurnCall): Promise<void> {
    if (!deps.recordCall || r.noRecord) return;
    const gen = wipeGen;
    const home = await deps.homeProfile().catch(() => undefined);
    // A Wipe while we waited: write nothing. (A session end that only forgets the turn from the
    // panel still saves its history.)
    if (r.noRecord || gen !== wipeGen) return;
    const refs = replyRefs(d);
    let end: TerminalAskEvent;
    switch (d.state) {
      case 'answered': {
        const md = answerOf(d);
        end =
          d.withheld !== undefined || md.trim() === ''
            ? { t: 'failed', code: 'GENIE_FAILED', userText: '', retry: 'none' }
            : {
                t: 'answer',
                md,
                incomplete: false,
                keyNumbers: keyNumbersOf(md),
                deepLink: refs.deepLink,
                queryItems: [],
              };
        break;
      }
      case 'timeout':
        end = { t: 'abandoned' };
        break;
      case 'stopped':
        end = { t: 'cancel_requested' };
        break;
      case 'signed_out':
        end = { t: 'failed', code: 'AUTH_EXPIRED', userText: '', retry: 'relogin' };
        break;
      default:
        end = { t: 'failed', code: 'GENIE_FAILED', userText: '', retry: 'user' };
    }
    try {
      deps.recordCall({
        opId: deps.newId(),
        mogId: r.origin.mogId,
        profile: d.profile,
        question: d.question,
        end,
        splitGroupId: uuidOfTurn(r.view.turnId),
        isStale: () => gen !== wipeGen || r.noRecord,
        homeWorkspace: home === undefined || d.profile === home,
        ...(refs.conversationId !== undefined ? { conversationId: refs.conversationId } : {}),
        ...(refs.responseId !== undefined ? { responseId: refs.responseId } : {}),
      });
    } catch {
      fault();
    }
  }

  /**
   * The turn row (AC-C1, AC-C2): one `genie_agent` row that holds the user's question and the
   * combined answer. Written once, when the turn ends, or best effort when the session ends.
   */
  async function recordTurn(r: TurnRec, out: string): Promise<void> {
    if (!deps.recordCall || r.recorded || r.noRecord) return;
    r.recorded = true;
    const gen = wipeGen;
    const home = await deps.homeProfile().catch(() => undefined);
    const profile = home ?? r.agentProfile;
    // A Wipe while we waited: the turn row is not written.
    if (profile === undefined || r.noRecord || gen !== wipeGen) return;
    const md = r.view.summary?.md ?? '';
    const end: TerminalAskEvent =
      out === 'answered'
        ? { t: 'answer', md, incomplete: false, keyNumbers: [], deepLink: '', queryItems: [] }
        : out === 'stopped'
          ? { t: 'cancel_requested' }
          : out === 'timeout'
            ? { t: 'abandoned' }
            : { t: 'failed', code: 'GENIE_FAILED', userText: '', retry: 'none' };
    try {
      deps.recordCall({
        opId: uuidOfTurn(r.view.turnId),
        mogId: r.origin.mogId,
        profile,
        question: r.view.question,
        end,
        backend: 'genie_agent',
        threadId: r.threadId,
        isStale: () => gen !== wipeGen || r.noRecord,
        homeWorkspace: true,
      });
    } catch {
      fault();
    }
  }

  const track = (r: TurnRec, p: Promise<void>) => {
    const t = p.catch(fault).finally(() => r.pending.delete(t));
    r.pending.add(t);
  };

  function onCall(r: TurnRec, e: Extract<AgentTurnEvent, { kind: 'call' }>) {
    const now = deps.wallNow();
    const known = r.view.calls.find((c) => c.callId === e.callId);
    const state = callViewState(e.state);
    let calls: AgentCallView[];
    if (known) {
      // A late event never moves a call back from its end.
      const done = known.endedAt !== undefined;
      calls = r.view.calls.map((c) =>
        c.callId !== e.callId || done
          ? c
          : {
              ...c,
              state,
              ...(isTerminalCall(e.state) ? { endedAt: now } : {}),
              ...(c.startedAt === undefined && e.state === 'asking' ? { startedAt: now } : {}),
            },
      );
    } else {
      calls = [
        ...r.view.calls,
        {
          callId: e.callId,
          workspace: chipOfAlias(r, e.workspaceId),
          question: '',
          state,
          ...(e.state === 'asking' ? { startedAt: now } : {}),
          ...(isTerminalCall(e.state) ? { endedAt: now } : {}),
        },
      ];
    }
    r.view = {
      ...r.view,
      calls,
      // A call that ends after its turn ended (offline, timeout) changes its row only.
      ...(r.ended
        ? {}
        : { used: e.used, budget: e.budget, state: r.view.state === 'slow' ? 'slow' : 'running' }),
    } as AgentTurnView;
    update(r);
    // The question comes with the first sight of the call; the reply when it ends.
    if (!known || isTerminalCall(e.state)) track(r, pullCall(r, e.callId, isTerminalCall(e.state)));
  }

  async function finish(r: TurnRec, e: Extract<AgentTurnEvent, { kind: 'ended' }>) {
    if (r.ended) return;
    r.ended = true;
    r.outcome = e.outcome;
    r.endedSeq = ++seq;
    const names = deps.names();
    const out = e.outcome;
    if (out === 'removed') {
      recs.delete(r.view.turnId);
      try {
        deps.dropTurn(r.origin.mogId, r.view.turnId);
      } catch {
        fault();
      }
      changed();
      void refresh();
      return;
    }
    // A turn that never left the queue ran nothing: it saves no turn row.
    const ran = r.view.state !== 'queued';
    const state: AgentTurnView['state'] =
      out === 'answered'
        ? 'done'
        : out === 'stopped'
          ? 'stopped'
          : out === 'timeout'
            ? 'timed_out'
            : out === 'offline'
              ? 'offline'
              : out === 'signed_out'
                ? 'needs_you'
                : 'failed';
    r.view = {
      ...r.view,
      state,
      ...(e.tokens !== undefined ? { tokens: e.tokens } : {}),
      ...(out === 'signed_out'
        ? {
            notice: `Your sign-in to ${r.view.agentWorkspace.name} expired. Sign in again in Settings.`,
          }
        : {}),
    };
    // AC-2a.11: an agent that never asked a workspace falls back to one direct home call. An
    // `ask_others` turn already asked home (R-9): it shows the reason and asks nothing more.
    const why = e.calls === 0 ? whyOfOutcome(out) : undefined;
    const id = r.view.turnId;
    if (why && r.mode === 'ask_others') {
      r.view = { ...r.view, notice: fallbackNotice(why, names, false) };
      update(r);
    } else if (why && !r.fellBack) {
      r.fellBack = true;
      const notice = fallbackNotice(why, names);
      r.view = { ...r.view, notice };
      update(r);
      await deps.direct(r.origin, notice, id).catch(fault);
      if (recs.get(id) !== r) return;
    } else {
      update(r);
    }
    // The calls that ended just before the turn finish their pulls first.
    await Promise.allSettled([...r.pending]);
    if (recs.get(id) !== r) return;
    // A pull that failed once is tried once more.
    const again = [...r.failedPulls];
    r.failedPulls.clear();
    for (const c of again) await pullCall(r, c, true);
    if (recs.get(id) !== r) return;
    if (out === 'answered') await summarise(r);
    if (recs.get(id) !== r) return;
    syncParts(r);
    update(r);
    // A turn that the session end cut off writes its row when the `agent:session` event arrives:
    // the core sends the turn `ended` first and the session `ended{wipe}` right after, and a Wipe
    // must leave no turn row.
    if (ran && out !== 'session_ended') await recordTurn(r, out);
    // A fallback has its own phases: the direct call shows its own outcome.
    if (!r.fellBack)
      deps.phase?.(
        r.origin.mogId,
        out === 'answered' ? 'answered' : out === 'stopped' ? 'cancelled' : 'failed',
      );
    if (out === 'answered') {
      deps.followUp?.answered();
      lastAnswered = r.threadId;
    }
    void refresh();
  }

  /** The note of the MOG, and which parts the sources cite (AC-2a.12, AC-2a.28). */
  async function summarise(r: TurnRec): Promise<void> {
    let res: AgentTurnResult;
    try {
      res = await deps.api.result(r.view.turnId);
    } catch {
      fault();
      return;
    }
    if (res.usage && r.view.tokens === undefined) {
      r.view = { ...r.view, tokens: res.usage.inputTokens + res.usage.outputTokens };
    }
    if (res.answerKind === 'hidden' || res.answer === undefined || res.answer.trim() === '') {
      // No note: every part shows, none folds.
      return;
    }
    // A source of an earlier turn shows as "From an earlier answer".
    for (const s of res.sources) {
      if (r.parts.has(s.callId) || !CALL_ID.test(s.callId)) continue;
      try {
        const d = await deps.api.callDetail(s.turnId, s.callId);
        // The D1 value of the turn that made the call, not of this turn.
        const src = recs.get(s.turnId) ?? r;
        const ws = src.catalog.get(d.workspaceId);
        const part = partOf(d, {
          chip: deps.chipOf(d.profile),
          fromEarlier: s.turnId !== r.view.turnId,
          notShownToAgent: ws !== undefined && !ws.isAgentWorkspace && !src.crossWorkspace,
        });
        if (part) r.parts.set(s.callId, part);
      } catch {
        fault();
      }
    }
    const cited = new Set(res.sources.map((s) => s.callId));
    for (const [id, p] of r.parts) r.parts.set(id, cited.has(id) ? p : { ...p, folded: true });
    const chips = res.sources
      .map((s) => r.parts.get(s.callId)?.workspace)
      .filter((c): c is AgentWsChip => c !== undefined);
    const workspaces = chips.filter((c, i) => chips.findIndex((x) => x.name === c.name) === i);
    r.view = {
      ...r.view,
      // Only D1 on can combine: with D1 off the label stays "Note from your MOG" (AC-2b.6).
      summary: { md: res.answer, kind: r.crossWorkspace ? res.answerKind : 'note', workspaces },
    };
  }

  function apply(r: TurnRec, e: AgentTurnEvent) {
    // After `ended` only a late call of that turn counts (its open calls keep polling).
    if (r.ended && e.kind !== 'call') return;
    switch (e.kind) {
      case 'queued':
        r.view = { ...r.view, state: 'queued', position: e.position };
        return update(r);
      case 'started': {
        const rest = { ...r.view };
        delete rest.position;
        r.view = { ...rest, state: r.view.state === 'slow' ? 'slow' : 'planning' };
        deps.phase?.(r.origin.mogId, 'working');
        update(r);
        return void refresh();
      }
      case 'thinking':
        if (r.view.state === 'queued') r.view = { ...r.view, state: 'planning' };
        return update(r);
      case 'call':
        return onCall(r, e);
      case 'refused':
        r.view = { ...r.view, used: e.used };
        return update(r);
      case 'slow':
        r.view = { ...r.view, state: 'slow' };
        return update(r);
      case 'ended':
        return void finish(r, e)
          .catch(fault)
          .finally(() => {
            r.settled = true;
            // A Delete that waited for this turn goes now.
            changed();
          });
    }
  }

  async function refresh(): Promise<void> {
    try {
      session = await deps.api.sessionState();
    } catch {
      return;
    }
    changed();
  }

  return {
    async submit(turn, mode, actionTicket, threadId) {
      const opId = deps.newId();
      let a: AgentTurnAccepted;
      try {
        a = await deps.api.submit({
          mogId: turn.mogId,
          opId,
          question: turn.question,
          mode,
          ...(actionTicket !== undefined ? { actionTicket } : {}),
        });
      } catch (e) {
        const code = codeOf(e);
        const why = whyOfRefusal(code);
        if (why && mode === 'ask_others') {
          // Home was asked already (R-9): say why the others were not, and ask nothing more.
          deps.note?.(turn.mogId, fallbackNotice(why, deps.names(), false));
          return;
        }
        if (why) {
          await deps.direct(turn, fallbackNotice(why, deps.names()));
          return;
        }
        const text = REFUSAL_TEXT[code];
        if (text) {
          deps.note?.(turn.mogId, text);
          return;
        }
        throw e;
      }
      const rec: TurnRec = {
        view: newView(a, turn.question),
        origin: turn,
        catalog: new Map(a.catalog.map((c) => [c.workspaceId, c])),
        crossWorkspace: a.crossWorkspace,
        agentProfile: a.catalog.find((c) => c.isAgentWorkspace)?.profile,
        parts: new Map(),
        fetched: new Set(),
        pending: new Set(),
        mode,
        failedPulls: new Set(),
        ended: false,
        fellBack: false,
        // A new ask starts a thread (its root is this turn); a follow-up joins one.
        threadId:
          threadId ?? (mode === 'follow_up' ? lastAnswered : undefined) ?? uuidOfTurn(a.turnId),
        recorded: false,
        noRecord: false,
        settled: false,
      };
      recs.set(a.turnId, rec);
      deps.phase?.(turn.mogId, 'working');
      update(rec);
      void refresh();
      // Events that came before this reply.
      for (const e of early.get(a.turnId) ?? []) apply(rec, e);
      early.delete(a.turnId);
    },
    onTurnEvent(payload) {
      const e = asTurnEvent(payload);
      if (!e) return;
      const r = recs.get(e.turnId);
      if (r) return apply(r, e);
      const list = early.get(e.turnId) ?? [];
      if (list.length < EARLY_MAX) list.push(e);
      early.set(e.turnId, list);
      // The map holds a few turns only.
      if (early.size > 8) early.delete(early.keys().next().value as string);
    },
    onSessionEvent(payload) {
      const e = asSessionEvent(payload);
      if (!e) return;
      if (e.kind === 'started') {
        if (stale) {
          for (const [id, r] of recs) if (r.ended) forget(id, r);
          stale = false;
        }
        void refresh();
        return changed();
      }
      deps.followUp?.reset();
      lastAnswered = undefined;
      // A Wipe leaves nothing on disk: no turn, ended or not, writes a row after it, and a write
      // that is already in flight finds the fence moved.
      if (e.reason === 'wipe') fenceWrites();
      for (const r of recs.values()) {
        if (e.reason === 'wipe') continue;
        // Any other end saves what it can of a turn that ran, also one that the session end cut
        // off (`session_ended`: its row waited for this event).
        if (r.ended ? r.outcome === 'session_ended' : r.view.state !== 'queued')
          void recordTurn(r, 'session_ended').catch(fault);
      }
      if (CLEARING.has(e.reason)) {
        for (const [id, r] of recs) if (r.ended) forget(id, r);
      } else {
        stale = true;
      }
      session = { ...NO_SESSION };
      changed();
      void refresh();
    },
    clear() {
      // Every Wipe scope ends here: no row of a turn in flight is written after it.
      fenceWrites();
      clearAll();
    },
    async stop(turnId) {
      if (!recs.has(turnId)) return;
      await deps.api.stop(turnId).catch(fault);
    },
    async newSession() {
      await deps.api.endSession().catch(fault);
      await refresh();
      // With no session open there is no `agent:session` event: the content goes now (M1).
      if (session.state === 'none') clearAll();
    },
    refreshSession: refresh,
    panel() {
      const { live, waiting, earlier } = shown();
      return {
        session: { ...session, queued: waiting.length },
        ...(live ? { live: live.view } : {}),
        waiting: waiting.map((r) => r.view),
        earlier: earlier.map((r) => r.view),
        threads: [...recs.values()].map((r) => ({ turnId: r.view.turnId, threadId: r.threadId })),
      };
    },
    turn: (turnId) => recs.get(turnId)?.view,
    threadOf: (turnId) => recs.get(turnId)?.threadId,
    threadMog: (threadId) => [...recs.values()].find((r) => r.threadId === threadId)?.origin.mogId,
    turns: () => {
      const { live, waiting, earlier } = shown();
      const sent = new Set([...(live ? [live] : []), ...waiting, ...earlier]);
      return [...recs.values()].map((r) => ({
        turnUuid: uuidOfTurn(r.view.turnId),
        threadId: r.threadId,
        mogId: r.origin.mogId,
        // Until `finish` is over: it saves the rows, and a Delete would miss them.
        inProgress: !r.settled,
        shown: sent.has(r),
      }));
    },
    inSession: (threadId) =>
      session.state !== 'none' && [...recs.values()].some((r) => r.threadId === threadId),
    forgetThread(threadId) {
      for (const [id, r] of recs) if (r.threadId === threadId && r.settled) forget(id, r);
      if (lastAnswered === threadId) lastAnswered = undefined;
      changed();
    },
  };

  /** A Wipe: the writes in flight are stale, and no turn writes a row after it. */
  function fenceWrites() {
    wipeGen += 1;
    for (const r of recs.values()) r.noRecord = true;
  }

  /** The turns `panel()` sends: the live one, the waiting ones and the newest ended ones. */
  function shown() {
    const all = [...recs.values()];
    const live = all.find(
      (r) => r.view.state === 'planning' || r.view.state === 'running' || r.view.state === 'slow',
    );
    const waiting = all.filter((r) => r.view.state === 'queued').slice(0, 3);
    const earlier = all
      .filter((r) => r.ended)
      .sort((a, b) => (b.endedSeq ?? 0) - (a.endedSeq ?? 0))
      .slice(0, EARLIER_MAX);
    return { live, waiting, earlier };
  }

  /** Wipe, or New session with no session open: nothing stays, also what a crash left on screen. */
  function clearAll() {
    for (const [id, r] of recs) if (r.ended) forget(id, r);
    stale = false;
    changed();
  }

  function forget(id: string, r: TurnRec) {
    recs.delete(id);
    try {
      deps.dropTurn(r.origin.mogId, id);
    } catch {
      fault();
    }
  }
}
