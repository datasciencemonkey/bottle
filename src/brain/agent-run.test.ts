/**
 * Slice 2a: the brain client of the live orchestrator session. IPC doubles and a simulated clock.
 * All values are plainly fake. Covers AC-2a.2, 2a.9 (UI half), 2a.11, 2a.12, 2a.18, 2a.19 to
 * 2a.21 (UI level), 2a.28 (UI half) and the session events.
 */
import type { AgentTurn } from '../shared/interfaces';
import type {
  AgentCallState,
  AgentSessionState,
  AgentTurnAccepted,
  AgentTurnCall,
  AgentTurnEvent,
  AgentTurnResult,
  AgentTurnView,
} from '../shared/ipc';
import { createFollowUpWindow } from './agent-route';
import { createAgentClient, uuidOfTurn, type AgentApi, type AgentClient } from './agent-run';
import { createSimClock } from './clock';
import type { PartFence } from './genie-direct';
import { createGenieHistory, type ExchangeMemory, type GenieEnd } from './genie-history';
import { createBrainLog } from './log';
import { routerDecision } from '../test-support/model-invoke';

const MOG = '00000000-0000-4000-8000-00000000000a';
const T1 = 'a'.repeat(32);
const T2 = 'b'.repeat(32);
const T3 = 'c'.repeat(32);
const C1 = 'c-00000001';
const C2 = 'c-00000002';
const SALES = {
  workspaceId: 'ws-a',
  profile: 'fake-sales',
  name: 'Sales',
  isHome: true,
  isAgentWorkspace: true,
};
const FIN = {
  workspaceId: 'ws-b',
  profile: 'fake-fin',
  name: 'Finance',
  isHome: false,
  isAgentWorkspace: false,
};
const Q = 'Compare churn in sales and finance';

const turn = (question = Q): AgentTurn => ({
  mogId: MOG,
  question,
  decision: routerDecision({ genie_question: question }),
  recall: [],
  target: 'fake-sales',
});

function accepted(turnId: string, over: Partial<AgentTurnAccepted> = {}): AgentTurnAccepted {
  return {
    turnId,
    sessionId: 's'.repeat(0) + 'd'.repeat(32),
    position: 0,
    newSession: true,
    budget: 10,
    crossWorkspace: false,
    catalog: [SALES, FIN],
    ...over,
  };
}

const reply = (md: string, extra: Record<string, unknown> = {}) => ({
  structuredContent: {
    status: 'completed',
    final_answer: md,
    deep_link: 'https://fake.test/l',
    ...extra,
  },
  text: md,
  isError: false,
  httpStatus: 200,
});

function detail(
  turnId: string,
  callId: string,
  w: typeof SALES,
  state: AgentCallState,
  md = 'Churn was **4.2%** here.',
): AgentTurnCall {
  return {
    callId,
    turnId,
    workspaceId: w.workspaceId,
    profile: w.profile,
    question: `Churn in ${w.name}?`,
    state,
    ...(state === 'answered' ? { reply: reply(md) } : {}),
  };
}

const sessionOf = (over: Partial<AgentSessionState> = {}): AgentSessionState => ({
  state: 'busy',
  sessionId: 'd'.repeat(32),
  startedAt: 1,
  agentProfile: 'fake-sales',
  turns: 1,
  queued: 0,
  tokens: 0,
  ...over,
});

/** A promise that a test opens by hand. */
function gate() {
  let open!: () => void;
  const p = new Promise<void>((res) => (open = res));
  return { p, open };
}

/**
 * `real`: the history is the real `createGenieHistory` over a fake store (it dedupes op ids and
 * asks `isStale()` after its lookups), and `rows` holds the questions that were written.
 */
function rig(opts: { real?: boolean } = {}) {
  const clock = createSimClock(1_000);
  const calls: string[] = [];
  const shown: AgentTurnView[] = [];
  const dropped: string[] = [];
  const notes: string[] = [];
  const directs: { turn: AgentTurn; notice: string; isStale?: PartFence }[] = [];
  const history: GenieEnd[] = [];
  const phases: string[] = [];
  const logLines: string[] = [];
  let changes = 0;
  const script = {
    submit: [] as (AgentTurnAccepted | { error: string })[],
    details: new Map<string, AgentTurnCall>(),
    result: undefined as AgentTurnResult | undefined,
    session: sessionOf(),
    /** Holds `homeProfile` (the await inside `recordTurn`) until it opens. */
    homeGate: undefined as Promise<void> | undefined,
    /** Holds the lookups of the real writer (the await before its first `isStale()`). */
    writeGate: undefined as Promise<void> | undefined,
  };
  const rows: string[] = [];
  const writer = createGenieHistory({
    memory: {
      registerMog: async () => undefined,
      recordExchange: async (x: { questionUser: string }) => void rows.push(x.questionUser),
    } as unknown as ExchangeMemory,
    mogName: async () => {
      await script.writeGate;
      return 'Sales';
    },
    hostOf: async () => undefined,
    log: { event: () => true },
  });
  const api: AgentApi = {
    submit: async (a) => {
      calls.push(
        `submit:${a.mode ?? 'ask'}:${a.opId.length > 0 ? 'op' : ''}${a.actionTicket ? ':ticket' : ''}`,
      );
      const next = script.submit.shift();
      if (!next) throw { error: 'agent_failed' };
      if ('error' in next) throw next;
      return next;
    },
    stop: async (id) => void calls.push(`stop:${id}`),
    callDetail: async (t, c) => {
      calls.push(`call:${t.slice(0, 1)}:${c}`);
      const d = script.details.get(`${t}:${c}`);
      if (!d) throw { error: 'agent_turn_unknown' };
      return d;
    },
    result: async (t) => {
      calls.push(`result:${t.slice(0, 1)}`);
      if (!script.result) throw { error: 'agent_turn_unknown' };
      return script.result;
    },
    sessionState: async () => script.session,
    endSession: async () => void calls.push('end'),
  };
  let n = 0;
  const followUp = createFollowUpWindow(clock);
  const client: AgentClient = createAgentClient({
    api,
    chipOf: (p) => ({
      name: p === 'fake-sales' ? 'Sales' : 'Finance',
      colorVariant: p === 'fake-sales' ? 'sky' : 'rose',
    }),
    names: () => ({ agent: 'Sales', home: 'Sales' }),
    homeProfile: async () => {
      await script.homeGate;
      return 'fake-sales';
    },
    hostOf: async (p) => `https://${p}.fake.test`,
    newId: () => `00000000-0000-4000-8000-${String(++n).padStart(12, '0')}`,
    wallNow: () => clock.now(),
    recordCall: opts.real ? (x) => void writer(x) : (x) => void history.push(x),
    direct: async (t, notice, _id, isStale) =>
      void directs.push({ turn: t, notice, ...(isStale ? { isStale } : {}) }),
    showTurn: (_m, v) => void shown.push(v),
    dropTurn: (_m, id) => void dropped.push(id),
    note: (_m, t) => void notes.push(t),
    phase: (_m, p) => void phases.push(p),
    followUp,
    onChange: () => void changes++,
    log: createBrainLog({ sink: (l) => logLines.push(l) }),
  });
  const ev = (e: AgentTurnEvent) => client.onTurnEvent(e);
  /** Let the pulls that the events started finish. */
  const settle = async () => {
    for (let i = 0; i < 20; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  };
  return {
    clock,
    calls,
    shown,
    dropped,
    notes,
    directs,
    history,
    rows,
    phases,
    logLines,
    script,
    client,
    ev,
    settle,
    followUp,
    changes: () => changes,
  };
}

const callEv = (
  turnId: string,
  callId: string,
  w: typeof SALES,
  state: AgentCallState,
  used = 1,
): AgentTurnEvent => ({
  kind: 'call',
  turnId,
  callId,
  workspaceId: w.workspaceId,
  state,
  used,
  budget: 10,
});

describe('submit', () => {
  it('[AC-2a.2] sends one agent_turn_submit with an opId, and shows the planning state', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    expect(r.calls[0]).toBe('submit:ask:op');
    expect(r.client.panel().live).toMatchObject({
      turnId: T1,
      state: 'planning',
      question: Q,
      budget: 10,
    });
    expect(r.client.panel().live?.agentWorkspace.name).toBe('Sales');
    expect(r.shown.at(-1)?.turnId).toBe(T1);
    expect(r.phases).toContain('working');
  });

  it('[R-9] passes the mode and the host-minted Ask ticket of the chip click', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Plain question'), 'ask', 'ticket-1');
    expect(r.calls[0]).toBe('submit:ask:op:ticket');
  });

  it('[AC-2a.2] two calls show as two call rows with "n of N"', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'started', turnId: T1 });
    r.script.details.set(`${T1}:${C1}`, detail(T1, C1, SALES, 'asking'));
    r.script.details.set(`${T1}:${C2}`, detail(T1, C2, FIN, 'asking'));
    r.ev(callEv(T1, C1, SALES, 'asking', 1));
    r.ev(callEv(T1, C2, FIN, 'asking', 2));
    await r.settle();
    const live = r.client.panel().live;
    expect(live?.state).toBe('running');
    expect(live?.used).toBe(2);
    expect(live?.calls.map((c) => [c.workspace.name, c.state, c.question])).toEqual([
      ['Sales', 'asking', 'Churn in Sales?'],
      ['Finance', 'asking', 'Churn in Finance?'],
    ]);
    expect(live?.calls[0]?.startedAt).toBe(1_000);
  });
});

describe('a finished call and a finished turn', () => {
  async function answered(r: ReturnType<typeof rig>) {
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'started', turnId: T1 });
    r.script.details.set(
      `${T1}:${C1}`,
      detail(T1, C1, SALES, 'answered', 'Churn in sales: **4.2%**.'),
    );
    r.script.details.set(
      `${T1}:${C2}`,
      detail(T1, C2, FIN, 'answered', 'Churn in finance: **6%**.'),
    );
    r.ev(callEv(T1, C1, SALES, 'asking', 1));
    r.ev(callEv(T1, C2, FIN, 'asking', 2));
    r.clock.advance(12_000);
    r.ev(callEv(T1, C1, SALES, 'answered', 2));
    r.ev(callEv(T1, C2, FIN, 'answered', 2));
    await r.settle();
  }

  it('[AC-2a.18] saves one history row for each Genie call, with the turn as split group', async () => {
    const r = rig();
    await answered(r);
    expect(r.history).toHaveLength(2);
    expect(r.history.map((h) => [h.profile, h.question, h.homeWorkspace, h.end.t])).toEqual([
      ['fake-sales', 'Churn in Sales?', true, 'answer'],
      ['fake-fin', 'Churn in Finance?', false, 'answer'],
    ]);
    for (const h of r.history) {
      expect(h.splitGroupId).toBe(uuidOfTurn(T1));
      expect(h.mogId).toBe(MOG);
    }
    expect(new Set(r.history.map((h) => h.opId)).size).toBe(2);
    // A repeat of the same terminal event writes nothing more.
    r.ev(callEv(T1, C1, SALES, 'answered', 2));
    await r.settle();
    expect(r.history).toHaveLength(2);
  });

  it('keeps each ready answer as a part with its chip, footer link and trusted host', async () => {
    const r = rig();
    await answered(r);
    const live = r.client.panel().live;
    expect(live?.parts.map((p) => p.workspace.name)).toEqual(['Sales', 'Finance']);
    const ids = live?.parts[0]?.sections.map((s) => s.id);
    expect(ids).toEqual(['answer', 'keyNumbers', 'footer']);
    const footer = live?.parts[0]?.sections.find((s) => s.id === 'footer');
    expect(footer).toMatchObject({
      deepLink: 'https://fake.test/l',
      workspaceHost: 'https://fake-sales.fake.test',
    });
    // D1 is off: the Finance text did not go to the agent.
    expect(live?.parts[1]?.notShownToAgent).toBe(true);
    expect(live?.parts[0]?.notShownToAgent).toBeUndefined();
    expect(live?.calls.every((c) => c.state === 'ready' && c.endedAt === 13_000)).toBe(true);
  });

  it('[AC-2b.6] a combined answer only counts with D1 on; with D1 off the kind is "note"', async () => {
    for (const on of [false, true]) {
      const r = rig();
      r.script.submit.push(accepted(T1, { crossWorkspace: on }));
      await r.client.submit(turn(), 'ask');
      r.script.details.set(`${T1}:${C1}`, detail(T1, C1, SALES, 'answered', 'Sales.'));
      r.script.result = {
        answer: 'Both fall.',
        answerKind: 'combined',
        sources: [{ workspaceId: 'ws-a', callId: C1, turnId: T1 }],
      };
      r.ev(callEv(T1, C1, SALES, 'answered'));
      r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 1 });
      await r.settle();
      expect(r.client.panel().earlier[0]?.summary?.kind).toBe(on ? 'combined' : 'note');
    }
  });

  it('[AC-2b.5] an earlier source takes the D1 value of its own turn; a forgotten turn fails closed', async () => {
    const r = rig();
    // T1 ran with D1 off and ended; it stays known.
    await answered(r);
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 2 });
    await r.settle();
    // T2 runs with D1 on and cites the Finance call of T1 (D1 off), and one of a forgotten turn.
    r.script.submit.push(accepted(T2, { crossWorkspace: true }));
    await r.client.submit(turn(), 'ask');
    const GONE = 'e'.repeat(32);
    r.script.details.set(
      `${GONE}:c-0000000e`,
      detail(GONE, 'c-0000000e', FIN, 'answered', 'Old finance.'),
    );
    r.script.details.set(`${T2}:c-00000003`, detail(T2, 'c-00000003', FIN, 'answered', 'New.'));
    r.script.result = {
      answer: 'Finance is higher.',
      answerKind: 'combined',
      sources: [
        { workspaceId: 'ws-b', callId: C2, turnId: T1 },
        { workspaceId: 'ws-b', callId: 'c-0000000e', turnId: GONE },
        { workspaceId: 'ws-b', callId: 'c-00000003', turnId: T2 },
      ],
    };
    r.ev(callEv(T2, 'c-00000003', FIN, 'answered', 1));
    r.ev({ kind: 'ended', turnId: T2, outcome: 'answered', calls: 1 });
    await r.settle();
    const done = r.client.panel().earlier.find((t) => t.turnId === T2);
    expect(done?.summary?.kind).toBe('combined');
    const byCall = Object.fromEntries((done?.parts ?? []).map((p) => [p.callId, p]));
    // (a) the earlier turn is present and had D1 off: the agent never saw it.
    expect(byCall[C2]?.notShownToAgent).toBe(true);
    // (b) the earlier turn is forgotten: fail closed.
    expect(byCall['c-0000000e']?.notShownToAgent).toBe(true);
    // The current turn has D1 on: the agent saw it.
    expect(byCall['c-00000003']?.notShownToAgent).toBeUndefined();
  });

  it('[AC-2a.12] [AC-2a.28] the note and the sources decide which part folds and which is from earlier', async () => {
    const r = rig();
    await answered(r);
    // The summary cites the Sales part of this turn, and a call of an earlier turn.
    r.script.details.set(
      `${'e'.repeat(32)}:c-0000000e`,
      detail('e'.repeat(32), 'c-0000000e', FIN, 'answered', 'Old finance.'),
    );
    r.script.result = {
      answer: 'Finance churn is higher than sales.',
      answerKind: 'note',
      sources: [
        { workspaceId: 'ws-a', callId: C1, turnId: T1 },
        { workspaceId: 'ws-b', callId: 'c-0000000e', turnId: 'e'.repeat(32) },
      ],
      usage: {
        inputTokens: 30_000,
        cachedInputTokens: 0,
        outputTokens: 11_000,
        reasoningTokens: 0,
      },
    };
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 2, tokens: 41_000 });
    await r.settle();
    expect(r.calls).toContain('result:a');
    const done = r.client.panel().earlier[0];
    expect(done).toMatchObject({ state: 'done', tokens: 41_000 });
    expect(done?.summary).toMatchObject({
      md: 'Finance churn is higher than sales.',
      kind: 'note',
    });
    const byCall = Object.fromEntries((done?.parts ?? []).map((p) => [p.callId, p]));
    expect(byCall[C1]?.folded).toBeUndefined();
    expect(byCall[C2]?.folded).toBe(true);
    expect(byCall['c-0000000e']).toMatchObject({ fromEarlier: true });
    expect(r.client.panel().live).toBeUndefined();
    expect(r.phases.at(-1)).toBe('answered');
  });

  it('[AC-2a.12] the summary carries the citations of its valid, shown markers only', async () => {
    const r = rig();
    await answered(r);
    r.script.result = {
      answer: 'Sales [1] and finance [2], not [3] or [x].',
      answerKind: 'note',
      sources: [
        { workspaceId: 'ws-a', callId: C1, turnId: T1 },
        { workspaceId: 'ws-b', callId: C2, turnId: T1 },
      ],
    };
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 2 });
    await r.settle();
    const s = r.client.panel().earlier[0]?.summary as { md: string; citations?: unknown[] };
    expect(s.md).toBe('Sales [1] and finance [2], not [3] or [x].');
    // D1 is off: the Finance part is withheld from the agent, so [2] gets no link.
    expect(s.citations).toMatchObject([{ n: 1, callId: C1, workspace: { name: SALES.name } }]);
    expect(s.citations).toHaveLength(1);
  });

  it('[AC-2a.12] a hidden summary shows no note and folds nothing', async () => {
    const r = rig();
    await answered(r);
    r.script.result = { answerKind: 'hidden', sources: [] };
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 2 });
    await r.settle();
    const done = r.client.panel().earlier[0];
    expect(done?.summary).toBeUndefined();
    expect(done?.parts.every((p) => p.folded === undefined)).toBe(true);
  });

  it('[R-13] an answer opens the follow-up window; a session end closes it', async () => {
    const r = rig();
    await answered(r);
    r.script.result = { answerKind: 'hidden', sources: [] };
    expect(r.followUp.isOpen()).toBe(false);
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 2 });
    await r.settle();
    expect(r.followUp.isOpen()).toBe(true);
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'idle' });
    expect(r.followUp.isOpen()).toBe(false);
  });

  it('[AC-2a.9] Stop calls agent_turn_stop; the ready part stays and the turn shows Stopped', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.script.details.set(`${T1}:${C1}`, detail(T1, C1, SALES, 'answered'));
    r.ev(callEv(T1, C1, SALES, 'answered', 1));
    r.ev(callEv(T1, C2, FIN, 'asking', 2));
    r.script.details.set(`${T1}:${C2}`, detail(T1, C2, FIN, 'asking'));
    await r.settle();
    await r.client.stop(T1);
    expect(r.calls).toContain(`stop:${T1}`);
    r.script.details.set(`${T1}:${C2}`, detail(T1, C2, FIN, 'stopped'));
    r.ev(callEv(T1, C2, FIN, 'stopped', 2));
    r.ev({ kind: 'ended', turnId: T1, outcome: 'stopped', calls: 2 });
    await r.settle();
    const t = r.client.panel().earlier[0];
    expect(t?.state).toBe('stopped');
    expect(t?.parts).toHaveLength(1);
    expect(t?.calls.map((c) => c.state)).toEqual(['ready', 'stopped']);
    expect(r.directs).toHaveLength(0);
    expect(r.phases.at(-1)).toBe('cancelled');
  });

  it('shows "slow" at the slow event and keeps the calls', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'slow', turnId: T1 });
    expect(r.client.panel().live?.state).toBe('slow');
    r.script.details.set(`${T1}:${C1}`, detail(T1, C1, SALES, 'asking'));
    r.ev(callEv(T1, C1, SALES, 'asking'));
    expect(r.client.panel().live?.state).toBe('slow');
  });
});

describe('the queue (AC-2a.19 to AC-2a.21, UI level)', () => {
  it('[AC-2a.19] a second turn waits at position 1 and becomes live at `started`', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1), accepted(T2, { position: 1, newSession: false }));
    await r.client.submit(turn('First?'), 'ask');
    await r.client.submit(turn('Second?'), 'follow_up');
    let p = r.client.panel();
    expect(p.live?.turnId).toBe(T1);
    expect(p.waiting.map((t) => [t.turnId, t.state, t.position])).toEqual([[T2, 'queued', 1]]);
    expect(p.session.queued).toBe(1);
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 0 });
    r.script.result = { answerKind: 'hidden', sources: [] };
    r.ev({ kind: 'started', turnId: T2 });
    await r.settle();
    p = r.client.panel();
    expect(p.live).toMatchObject({ turnId: T2, state: 'planning' });
    expect(p.live?.position).toBeUndefined();
    expect(p.waiting).toEqual([]);
    expect(p.earlier.map((t) => t.turnId)).toEqual([T1]);
  });

  it('[AC-2a.19] events that come before the submit reply are not lost', async () => {
    const r = rig();
    r.ev({ kind: 'started', turnId: T1 });
    r.ev({ kind: 'slow', turnId: T1 });
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    expect(r.client.panel().live?.state).toBe('slow');
  });

  it('[AC-2a.20] a 4th waiting turn is refused: one note, no fallback, no turn', async () => {
    const r = rig();
    r.script.submit.push({ error: 'agent_queue_full' });
    await r.client.submit(turn(), 'ask');
    expect(r.notes).toEqual(['Your agent has 3 questions waiting. Try again when one is done.']);
    expect(r.directs).toEqual([]);
    expect(r.client.panel().live).toBeUndefined();
    expect(r.client.panel().waiting).toEqual([]);
  });

  it('[AC-2a.20] a rate limit shows its own note', async () => {
    const r = rig();
    r.script.submit.push({ error: 'agent_rate_limited' });
    await r.client.submit(turn(), 'ask');
    expect(r.notes[0]).toMatch(/too many questions/i);
  });

  it('[AC-2a.21] stopping a waiting turn calls agent_turn_stop and `removed` drops it', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1), accepted(T2, { position: 1, newSession: false }));
    await r.client.submit(turn('First?'), 'ask');
    await r.client.submit(turn('Second?'), 'ask');
    await r.client.stop(T2);
    expect(r.calls).toContain(`stop:${T2}`);
    r.ev({ kind: 'ended', turnId: T2, outcome: 'removed', calls: 0 });
    await r.settle();
    expect(r.client.panel().waiting).toEqual([]);
    expect(r.client.panel().earlier).toEqual([]);
    expect(r.dropped).toEqual([T2]);
    expect(r.directs).toEqual([]);
  });

  it('a stop for a turn that the client does not hold sends nothing', async () => {
    const r = rig();
    await r.client.stop(T3);
    expect(r.calls).toEqual([]);
  });
});

describe('the fallback to the direct home path (AC-2a.11)', () => {
  it.each([
    ['timeout', 'The agent timed out. Asked Sales only.', 'timed_out'],
    ['offline', 'Offline. Asked Sales only.', 'offline'],
    ['failed', 'The agent could not run. Asked Sales only.', 'failed'],
  ] as const)(
    'a turn that ends `%s` before any call asks home once',
    async (outcome, notice, state) => {
      const r = rig();
      r.script.submit.push(accepted(T1));
      await r.client.submit(turn('Compare a and b'), 'ask');
      r.ev({ kind: 'ended', turnId: T1, outcome, calls: 0 });
      await r.settle();
      expect(r.directs).toHaveLength(1);
      expect(r.directs[0]?.notice).toBe(notice);
      expect(r.directs[0]?.turn.question).toBe('Compare a and b');
      expect(r.client.panel().earlier[0]).toMatchObject({ state, notice });
    },
  );

  it('[Wipe] the fallback row is fenced: a clear() after it began makes it stale', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Compare a and b'), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'timeout', calls: 0 });
    await r.settle();
    const isStale = r.directs[0]?.isStale;
    expect(isStale?.(() => undefined)).toBe(false);
    r.client.clear();
    expect(isStale?.(() => undefined)).toBe(true);
  });

  /** A fallback that starts under a client fence: the writer of `genie-direct` asks the fence. */
  async function fallbackUnderFence(r: ReturnType<typeof rig>) {
    const lift = r.client.fence();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Compare a and b'), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'timeout', calls: 0 });
    await r.settle();
    const written: string[] = [];
    const write = (fence: PartFence): void => {
      if (!fence(write)) written.push('row');
    };
    const fence = r.directs[0]?.isStale;
    if (fence) write(fence);
    return { lift, written, write };
  }

  it('[Wipe] a fallback that starts under a client fence is held, and a lift writes one row', async () => {
    const r = rig();
    const { lift, written } = await fallbackUnderFence(r);
    expect(written).toEqual([]);
    lift();
    expect(written).toEqual(['row']);
  });

  it('[Wipe] a fallback that starts under a client fence writes no row after a good Wipe', async () => {
    const r = rig();
    const { lift, written } = await fallbackUnderFence(r);
    r.client.clear();
    lift();
    expect(written).toEqual([]);
  });

  it('a turn that already made a call does not fall back', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'timeout', calls: 1 });
    await r.settle();
    expect(r.directs).toEqual([]);
  });

  it('stopped, removed and answered turns never fall back', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.script.result = { answerKind: 'hidden', sources: [] };
    r.ev({ kind: 'ended', turnId: T1, outcome: 'stopped', calls: 0 });
    await r.settle();
    expect(r.directs).toEqual([]);
  });

  it('a second ended event does not ask twice', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'timeout', calls: 0 });
    r.ev({ kind: 'ended', turnId: T1, outcome: 'timeout', calls: 0 });
    await r.settle();
    expect(r.directs).toHaveLength(1);
  });

  it.each([
    ['agent_profile_unsupported', 'No home workspace is set.'],
    ['agent_not_eligible', 'The orchestrator model is not available in Sales. Asked Sales only.'],
    ['agent_signed_out', 'Your sign-in to Sales expired. Asked Sales only.'],
    ['agent_session_unavailable', 'The agent could not run. Asked Sales only.'],
  ])('a refused submit (%s) asks home with its notice', async (code, notice) => {
    const r = rig();
    r.script.submit.push({ error: code });
    await r.client.submit(turn(), 'ask');
    expect(r.directs).toEqual([{ turn: turn(), notice }]);
    expect(r.client.panel().live).toBeUndefined();
  });

  it('a refusal with no fallback reaches the caller (the ask gate shows "Not sent.")', async () => {
    const r = rig();
    r.script.submit.push({ error: 'ask_not_approved' });
    await expect(r.client.submit(turn(), 'ask')).rejects.toEqual({ error: 'ask_not_approved' });
    expect(r.directs).toEqual([]);
  });
});

describe('needs you, and the session events', () => {
  it('a signed-out end keeps the turn as `needs_you` with its question, and falls back to direct (L3)', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'signed_out', calls: 0 });
    await r.settle();
    expect(r.directs).toHaveLength(1);
    expect(r.client.panel().earlier[0]).toMatchObject({ state: 'needs_you', question: Q });
    expect(r.client.panel().earlier[0]?.notice).toMatch(/sign-in to Sales expired/);
  });

  it('[AC-2a.25] New session calls agent_session_end; its `ended` clears the earlier turns', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'failed', calls: 1 });
    await r.settle();
    expect(r.client.panel().earlier).toHaveLength(1);
    await r.client.newSession();
    expect(r.calls).toContain('end');
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'new_session' });
    expect(r.client.panel().earlier).toEqual([]);
    expect(r.dropped).toEqual([T1]);
    expect(r.client.panel().session.state).toBe('none');
  });

  it.each(['quit', 'wipe'] as const)(
    'a `%s` end clears the content of the panel',
    async (reason) => {
      const r = rig();
      r.script.submit.push(accepted(T1));
      await r.client.submit(turn(), 'ask');
      r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 1 });
      await r.settle();
      r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason });
      expect(r.client.panel().earlier).toEqual([]);
    },
  );

  it('a session end with an unlisted reason keeps the failed turn on screen until the next session starts', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'failed', calls: 1 });
    await r.settle();
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'unlisted' as never });
    expect(r.client.panel().earlier).toHaveLength(1);
    r.client.onSessionEvent({ kind: 'started', sessionId: 'f'.repeat(32) });
    expect(r.client.panel().earlier).toEqual([]);
  });

  it('the panel header takes the session state from agent_session_state', async () => {
    const r = rig();
    r.script.session = sessionOf({ turns: 3, tokens: 124_000 });
    await r.client.refreshSession();
    expect(r.client.panel().session).toMatchObject({ turns: 3, tokens: 124_000 });
  });

  it('keeps at most 20 earlier turns', async () => {
    const r = rig();
    for (let i = 0; i < 22; i++) {
      const id = i.toString(16).padStart(32, '0');
      r.script.submit.push(accepted(id, { newSession: i === 0 }));
      await r.client.submit(turn(`Q${i}?`), 'ask');
      r.ev({ kind: 'ended', turnId: id, outcome: 'failed', calls: 1 });
    }
    await r.settle();
    const earlier = r.client.panel().earlier;
    expect(earlier).toHaveLength(20);
    expect(earlier[0]?.question).toBe('Q21?');
  });
});

describe('bad input and the log', () => {
  it('drops an event with a bad shape', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.client.onTurnEvent({
      kind: 'call',
      turnId: T1,
      callId: 'x',
      workspaceId: 'ws-a',
      state: 'asking',
      used: 1,
      budget: 10,
    });
    r.client.onTurnEvent({ kind: 'ended', turnId: T1, outcome: 'weird', calls: 0 });
    r.client.onTurnEvent({ kind: 'started', turnId: 'short' });
    r.client.onTurnEvent(null);
    r.client.onSessionEvent({ kind: 'ended' });
    expect(r.client.panel().live?.calls).toEqual([]);
    expect(r.client.panel().live?.state).toBe('planning');
  });

  it('[AC-2a.16] a failed pull logs a fixed line and no question or answer text', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Secret fake question about churn?'), 'ask');
    r.ev(callEv(T1, C1, SALES, 'answered')); // no detail: the pull fails
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 1 });
    await r.settle();
    expect(r.logLines.length).toBeGreaterThan(0);
    for (const l of r.logLines) {
      expect(l).not.toMatch(/churn|secret|fake question|Sales|Finance/i);
      expect(l).toMatch(/agent\.turn/);
    }
  });
});

describe('review fixes (H1 to H3, M1, M2, L2, L6)', () => {
  const submitTwo = async (r: ReturnType<typeof rig>) => {
    r.script.submit.push(accepted(T1), accepted(T2, { position: 1, newSession: false }));
    await r.client.submit(turn('First?'), 'ask');
    await r.client.submit(turn('Second?'), 'ask');
  };

  it.each(['new_session', 'wipe', 'quit'] as const)(
    '[H1] a %s ends live and waiting turns with session_ended: no direct call, no call row',
    async (reason) => {
      const r = rig();
      await submitTwo(r);
      r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason });
      r.ev({ kind: 'ended', turnId: T1, outcome: 'session_ended', calls: 0 });
      r.ev({ kind: 'ended', turnId: T2, outcome: 'session_ended', calls: 0 });
      await r.settle();
      expect(r.directs).toEqual([]);
      // Best effort: the turn that ran saves a failed turn row, once. The waiting turn ran
      // nothing. A Wipe leaves no row at all.
      expect(r.history.filter((h) => h.backend === undefined)).toEqual([]);
      const turns = r.history.filter((h) => h.backend === 'genie_agent');
      expect(turns.map((h) => [h.question, h.end.t])).toEqual(
        reason === 'wipe' ? [] : [['First?', 'failed']],
      );
    },
  );

  it.each(['refusals', 'session_ended'] as const)(
    '[H1] [L4] a %s end before any call asks home never',
    async (outcome) => {
      const r = rig();
      r.script.submit.push(accepted(T1));
      await r.client.submit(turn(), 'ask');
      r.ev({ kind: 'ended', turnId: T1, outcome, calls: 0 });
      await r.settle();
      expect(r.directs).toEqual([]);
    },
  );

  it('[L3] a signed_out end before any call falls back to direct with the sign-in notice', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Plain?'), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'signed_out', calls: 0 });
    await r.settle();
    expect(r.directs).toHaveLength(1);
    expect(r.client.panel().earlier[0]?.notice).toBe('Your sign-in to Sales expired. Asked Sales only.');
  });

  it('[H3] a call that ends after its turn ended updates its row only, never the state', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.script.details.set(`${T1}:${C1}`, detail(T1, C1, SALES, 'asking'));
    r.ev(callEv(T1, C1, SALES, 'asking'));
    r.ev({ kind: 'ended', turnId: T1, outcome: 'offline', calls: 1 });
    await r.settle();
    r.script.details.set(`${T1}:${C1}`, detail(T1, C1, SALES, 'answered'));
    r.ev(callEv(T1, C1, SALES, 'answered', 1));
    r.ev({ kind: 'slow', turnId: T1 });
    r.ev({ kind: 'started', turnId: T1 });
    r.ev({ kind: 'thinking', turnId: T1 });
    r.ev({ kind: 'queued', turnId: T1, position: 2 });
    r.ev({ kind: 'refused', turnId: T1, code: 'budget_reached', used: 9 });
    await r.settle();
    const p = r.client.panel();
    expect(p.live).toBeUndefined();
    expect(p.earlier[0]).toMatchObject({ state: 'offline', used: 1 });
    expect(p.earlier[0]?.calls[0]?.state).toBe('ready');
    expect(p.earlier[0]?.parts).toHaveLength(1);
  });

  it('[M1] clear() forgets what an unlisted session end left on screen', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'failed', calls: 1 });
    await r.settle();
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'unlisted' as never });
    expect(r.client.panel().earlier).toHaveLength(1);
    r.client.clear();
    expect(r.client.panel().earlier).toEqual([]);
    expect(r.dropped).toEqual([T1]);
  });

  it('[M2] a turn that a Wipe cleared while it finished is not put back', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.script.details.set(`${T1}:${C1}`, detail(T1, C1, SALES, 'answered'));
    r.script.result = {
      answerKind: 'note',
      answer: 'Note.',
      sources: [{ workspaceId: 'ws-a', callId: C1, turnId: T1 }],
    };
    r.ev(callEv(T1, C1, SALES, 'answered'));
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 1 });
    // The Wipe comes while the pulls run.
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'wipe' });
    const shownBefore = r.shown.length;
    const phasesBefore = r.phases.length;
    await r.settle();
    expect(r.client.panel().earlier).toEqual([]);
    expect(r.followUp.isOpen()).toBe(false);
    expect(r.phases.length).toBe(phasesBefore);
    expect(r.shown.length).toBe(shownBefore);
    expect(r.history).toEqual([]);
  });

  it('[L2] a failed pull is tried once more when the turn ends', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev(callEv(T1, C1, SALES, 'answered')); // no detail yet: the pull fails
    await r.settle();
    expect(r.history).toEqual([]);
    r.script.details.set(`${T1}:${C1}`, detail(T1, C1, SALES, 'answered'));
    r.script.result = { answerKind: 'hidden', sources: [] };
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 1 });
    await r.settle();
    expect(r.history.filter((h) => h.backend === undefined)).toHaveLength(1);
    expect(r.client.panel().earlier[0]?.parts).toHaveLength(1);
  });

  it('[L6] a refused event with an unknown code is dropped', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.client.onTurnEvent({ kind: 'refused', turnId: T1, code: 'weird', used: 7 });
    expect(r.client.panel().live?.used).toBe(0);
    r.client.onTurnEvent({ kind: 'refused', turnId: T1, code: 'budget_reached', used: 7 });
    expect(r.client.panel().live?.used).toBe(7);
  });

  it('[M1] New session with no session open clears what a crash left', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'failed', calls: 1 });
    await r.settle();
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'idle' });
    r.script.session = sessionOf({ state: 'none', turns: 0 });
    await r.client.newSession();
    expect(r.client.panel().earlier).toEqual([]);
  });

  it('[N4] a late call keeps used and budget of the ended turn', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.script.details.set(`${T1}:${C1}`, detail(T1, C1, SALES, 'asking'));
    r.ev(callEv(T1, C1, SALES, 'asking', 2));
    r.ev({ kind: 'ended', turnId: T1, outcome: 'offline', calls: 1 });
    await r.settle();
    r.ev({
      kind: 'call',
      turnId: T1,
      callId: C1,
      workspaceId: 'ws-a',
      state: 'answered',
      used: 9,
      budget: 20,
    });
    await r.settle();
    expect(r.client.panel().earlier[0]).toMatchObject({ used: 2, budget: 10 });
  });
});

describe('agent threads in Conversations (ADR 0121)', () => {
  const NOTE = { answerKind: 'summary', answer: 'Both fall.', sources: [] } as never;
  const turnRows = (r: ReturnType<typeof rig>) =>
    r.history.filter((h) => h.backend === 'genie_agent');

  async function ask(
    r: ReturnType<typeof rig>,
    turnId: string,
    q: string,
    mode: 'ask' | 'follow_up' = 'ask',
    threadId?: string,
    outcome: AgentTurnEvent & { kind: 'ended' } = {
      kind: 'ended',
      turnId,
      outcome: 'answered',
      calls: 0,
    },
  ) {
    r.script.submit.push(accepted(turnId));
    await r.client.submit(turn(q), mode, undefined, threadId);
    r.script.result = NOTE;
    r.ev(outcome);
    await r.settle();
  }

  it('[AC-C1] [AC-C2] one genie_agent turn row at the end, with the question and the combined answer', async () => {
    const r = rig();
    await ask(r, T1, 'How is churn?');
    const rows = turnRows(r);
    expect(rows).toHaveLength(1);
    const row = rows[0];
    if (!row) throw new Error('no turn row');
    expect(row.opId).toBe(uuidOfTurn(T1));
    expect(row.question).toBe('How is churn?');
    expect(row.end).toMatchObject({ t: 'answer', md: 'Both fall.', keyNumbers: [] });
    expect(row.homeWorkspace).toBe(true);
    expect(row.mogId).toBe(MOG);
    expect(row.profile).toBe('fake-sales');
    // A new ask starts a thread: its root is its own turn UUID.
    expect(row.threadId).toBe(uuidOfTurn(T1));
    expect(row.splitGroupId).toBeUndefined();
  });

  it('[AC-C2] a hidden summary saves an empty answer; stop, timeout and the rest map to their end', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Q1'), 'ask');
    r.script.result = { answerKind: 'hidden', sources: [] };
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 0 });
    await r.settle();
    expect(turnRows(r)[0]?.end).toMatchObject({ t: 'answer', md: '' });
    for (const [id, outcome, t] of [
      [T2, 'stopped', 'cancel_requested'],
      [T3, 'timeout', 'abandoned'],
    ] as const) {
      r.script.submit.push(accepted(id, { newSession: false }));
      await r.client.submit(turn(`Q-${outcome}`), 'ask');
      r.ev({ kind: 'ended', turnId: id, outcome, calls: 1 });
      await r.settle();
      expect(turnRows(r).at(-1)?.end.t).toBe(t);
    }
  });

  it('writes the turn row once: a second ended event and a later session end add nothing', async () => {
    const r = rig();
    await ask(r, T1, 'Once?');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 0 });
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'quit' });
    await r.settle();
    expect(turnRows(r)).toHaveLength(1);
  });

  it('[AC-C4] a reply with a thread id joins that thread', async () => {
    const r = rig();
    await ask(r, T1, 'First?');
    await ask(r, T2, 'And then?', 'follow_up', uuidOfTurn(T1));
    expect(turnRows(r).map((h) => h.threadId)).toEqual([uuidOfTurn(T1), uuidOfTurn(T1)]);
    expect(r.client.inSession(uuidOfTurn(T1))).toBe(true);
    expect(r.client.inSession(uuidOfTurn(T3))).toBe(false);
  });

  it('[AC-C5] a follow-up with no thread id goes to the thread of the last answered turn', async () => {
    const r = rig();
    await ask(r, T1, 'First?');
    await ask(r, T2, 'Second?', 'follow_up');
    expect(turnRows(r)[1]?.threadId).toBe(uuidOfTurn(T1));
    // A new ask starts a new thread; with no answer yet, a follow-up does too.
    await ask(r, T3, 'New topic?', 'ask');
    expect(turnRows(r)[2]?.threadId).toBe(uuidOfTurn(T3));
    const r2 = rig();
    await ask(r2, T1, 'Alone?', 'follow_up');
    expect(turnRows(r2)[0]?.threadId).toBe(uuidOfTurn(T1));
  });

  it('a session end forgets the last answered thread', async () => {
    const r = rig();
    await ask(r, T1, 'First?');
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'idle' });
    await ask(r, T2, 'Later?', 'follow_up');
    expect(turnRows(r)[1]?.threadId).toBe(uuidOfTurn(T2));
  });

  it('HubAgentView.threads links each turn in the panel to its thread', async () => {
    const r = rig();
    await ask(r, T1, 'First?');
    await ask(r, T2, 'Second?', 'follow_up');
    expect(r.client.panel().threads).toEqual([
      { turnId: T1, threadId: uuidOfTurn(T1) },
      { turnId: T2, threadId: uuidOfTurn(T1) },
    ]);
    expect(r.client.threadOf(T2)).toBe(uuidOfTurn(T1));
    expect(r.client.threadMog(uuidOfTurn(T1))).toBe(MOG);
  });

  it('[AC-C15] turns() marks a live or queued turn as in progress, a finished one not', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1), accepted(T2, { position: 1, newSession: false }));
    await r.client.submit(turn('Live?'), 'ask');
    await r.client.submit(turn('Queued?'), 'ask');
    expect(r.client.turns().map((t) => t.inProgress)).toEqual([true, true]);
    r.script.result = NOTE;
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 0 });
    await r.settle();
    expect(r.client.turns().map((t) => [t.turnUuid, t.inProgress])).toEqual([
      [uuidOfTurn(T1), false],
      [uuidOfTurn(T2), true],
    ]);
  });

  it('[AC-C19] forgetThread drops the finished turns of a thread and their card, and tells the hub', async () => {
    const r = rig();
    await ask(r, T1, 'First?');
    await ask(r, T2, 'Second?', 'follow_up');
    const before = r.changes();
    r.client.forgetThread(uuidOfTurn(T1));
    expect(r.client.panel().earlier).toEqual([]);
    expect(r.client.panel().threads).toEqual([]);
    expect(r.dropped).toEqual([T1, T2]);
    expect(r.changes()).toBeGreaterThan(before);
    // A late event of a forgotten turn brings nothing back.
    r.ev(callEv(T1, C1, SALES, 'answered', 1));
    await r.settle();
    expect(r.client.panel().earlier).toEqual([]);
  });

  it('forgetThread leaves a turn in progress and other threads', async () => {
    const r = rig();
    await ask(r, T1, 'First?');
    r.script.submit.push(accepted(T2, { newSession: false }));
    await r.client.submit(turn('Other?'), 'ask');
    r.client.forgetThread(uuidOfTurn(T2));
    expect(r.client.turn(T2)).toBeDefined();
    r.client.forgetThread(uuidOfTurn(T3));
    expect(r.client.turn(T1)).toBeDefined();
  });

  it('a turn that a Wipe overtook (the session event first) writes no turn row', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Wiped?'), 'ask');
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'wipe' });
    r.script.result = NOTE;
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 0 });
    await r.settle();
    expect(turnRows(r)).toEqual([]);
  });

  const END = { kind: 'ended' as const, sessionId: 'd'.repeat(32) };

  it('[Wipe] [HIGH] the real core order, turn ended{session_ended} then session ended{wipe}: no turn row', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Wiped?'), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'session_ended', calls: 0 });
    r.client.onSessionEvent({ ...END, reason: 'wipe' });
    await r.settle();
    expect(turnRows(r)).toEqual([]);
  });

  it('[Wipe] [HIGH] a turn that ended answered just before the Wipe writes no row, and no call row', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Wiped?'), 'ask');
    r.ev(callEv(T1, C1, SALES, 'asking'));
    r.script.details.set(`${T1}:${C1}`, detail(T1, C1, SALES, 'answered'));
    r.ev(callEv(T1, C1, SALES, 'answered', 1));
    r.script.result = NOTE;
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 1 });
    // The Wipe comes while the writes are still waiting for the home profile.
    r.client.onSessionEvent({ ...END, reason: 'wipe' });
    await r.settle();
    expect(r.history).toEqual([]);
  });

  it('[Wipe] [L3] a write that started before the Wipe sees a moved fence', async () => {
    const r = rig();
    await ask(r, T1, 'Before?');
    const row = turnRows(r)[0];
    expect(row?.isStale?.()).toBe(false);
    r.client.onSessionEvent({ ...END, reason: 'wipe' });
    expect(row?.isStale?.()).toBe(true);
  });

  it('a turn that the session end cut off writes its row when the session event comes, not before', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Cut off?'), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'session_ended', calls: 0 });
    await r.settle();
    expect(turnRows(r)).toEqual([]);
    r.client.onSessionEvent({ ...END, reason: 'idle' });
    await r.settle();
    expect(turnRows(r)).toHaveLength(1);
    expect(turnRows(r)[0]?.end.t).toBe('failed');
  });

  it('turns() says which turns the panel sends: the 20 newest ended turns only', async () => {
    const r = rig();
    const ids = Array.from({ length: 22 }, (_, i) => i.toString(16).padStart(32, '0'));
    for (const id of ids) {
      r.script.submit.push(accepted(id, { newSession: false }));
      await r.client.submit(turn(`Q ${id}`), 'ask');
      r.script.result = NOTE;
      r.ev({ kind: 'ended', turnId: id, outcome: 'answered', calls: 0 });
      await r.settle();
    }
    const t = r.client.turns();
    expect(t).toHaveLength(22);
    expect(t.filter((x) => x.shown)).toHaveLength(20);
    expect(t.filter((x) => !x.shown).map((x) => x.turnUuid)).toEqual([
      uuidOfTurn(ids[0] as string),
      uuidOfTurn(ids[1] as string),
    ]);
    expect(r.client.panel().earlier).toHaveLength(20);
  });

  it('a turn stays in progress until finish is over, and the change that settles it is announced', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Settling?'), 'ask');
    r.script.result = NOTE;
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 0 });
    // `finish` waits for its pulls: the turn is ended but not settled yet.
    expect(r.client.turn(T1)?.state).toBe('done');
    expect(r.client.turns()[0]?.inProgress).toBe(true);
    const before = r.changes();
    await r.settle();
    expect(r.client.turns()[0]?.inProgress).toBe(false);
    expect(r.changes()).toBeGreaterThan(before);
  });

  it('[Wipe] clear() (every Wipe scope) also fences the writes of a turn in flight', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Wiped?'), 'ask');
    r.script.result = NOTE;
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 0 });
    r.client.clear();
    await r.settle();
    expect(r.history).toEqual([]);
  });

  it('[M-1] a queued turn that the session end cut off writes no row at the next session end', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1), accepted(T2, { position: 1, newSession: false }));
    await r.client.submit(turn('Live?'), 'ask');
    await r.client.submit(turn('Queued?'), 'ask');
    // The core order: the live turn ended, the session ended, then the queued turn ended.
    r.ev({ kind: 'ended', turnId: T1, outcome: 'session_ended', calls: 0 });
    r.client.onSessionEvent({ ...END, reason: 'new_session' });
    r.ev({ kind: 'ended', turnId: T2, outcome: 'session_ended', calls: 0 });
    await r.settle();
    // A new session starts and ends idle: the queued turn must not get a row now.
    r.client.onSessionEvent({ kind: 'started', sessionId: 'e'.repeat(32) });
    r.client.onSessionEvent({ ...END, sessionId: 'e'.repeat(32), reason: 'idle' });
    await r.settle();
    expect(turnRows(r).map((h) => h.question)).toEqual(['Live?']);
  });

  it('[L-1] fence() stops the rows of a turn in flight until the Wipe is over, and a lift lets later turns write', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Wiping?'), 'ask');
    const lift = r.client.fence();
    r.script.result = NOTE;
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 0 });
    await r.settle();
    expect(r.history).toEqual([]);
    // The Wipe failed: the held row is written, and a turn that starts after the lift writes too.
    lift();
    r.script.submit.push(accepted(T2, { newSession: false }));
    await r.client.submit(turn('After?'), 'ask');
    r.ev({ kind: 'ended', turnId: T2, outcome: 'answered', calls: 0 });
    await r.settle();
    expect(turnRows(r).map((h) => h.question)).toEqual(['Wiping?', 'After?']);
  });

  it('[L-1b] a turn that ended in the fence of a failed Wipe gets its row once, at the lift', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Kept?'), 'ask');
    const lift = r.client.fence();
    r.script.result = NOTE;
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 0 });
    await r.settle();
    expect(r.history).toEqual([]);
    lift();
    lift();
    await r.settle();
    expect(turnRows(r).map((h) => h.question)).toEqual(['Kept?']);
  });

  it('[L-1c] a turn submitted in the fence writes nothing in it, nothing after a good Wipe, a row after a lift', async () => {
    for (const wiped of [true, false]) {
      const r = rig();
      const lift = r.client.fence();
      r.script.submit.push(accepted(T1));
      await r.client.submit(turn('Inside?'), 'ask');
      r.script.result = NOTE;
      r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 0 });
      await r.settle();
      expect(r.history).toEqual([]);
      if (wiped) {
        r.client.clear();
        lift();
        await r.settle();
        expect(r.history).toEqual([]);
      } else {
        lift();
        await r.settle();
        expect(turnRows(r).map((h) => h.question)).toEqual(['Inside?']);
      }
    }
  });

  /** One turn that ends: its row is asked for, and `settle` lets the write reach its first await. */
  async function endTurn(r: ReturnType<typeof rig>, turnId: string, q: string, newSession = true) {
    r.script.submit.push(accepted(turnId, { newSession }));
    await r.client.submit(turn(q), 'ask');
    r.script.result = NOTE;
    r.ev({ kind: 'ended', turnId, outcome: 'answered', calls: 0 });
  }

  it('[L-1e] a fence during the homeProfile await: a good Wipe leaves no row, a failed one exactly one', async () => {
    for (const wiped of [true, false]) {
      const r = rig({ real: true });
      const home = gate();
      r.script.homeGate = home.p;
      await endTurn(r, T1, 'Held?');
      await r.settle();
      const lift = r.client.fence();
      home.open();
      await r.settle();
      expect(r.rows).toEqual([]);
      if (wiped) r.client.clear();
      lift();
      await r.settle();
      expect(r.rows).toEqual(wiped ? [] : ['Held?']);
    }
  });

  it('[L-1f] [HIGH] a fence after the handoff to the writer (case A: isStale defers; case B: lift first)', async () => {
    for (const order of ['A', 'B'] as const) {
      for (const wiped of [true, false]) {
        const r = rig({ real: true });
        const write = gate();
        r.script.writeGate = write.p;
        await endTurn(r, T1, 'Handed?');
        await r.settle();
        const lift = r.client.fence();
        if (order === 'B') {
          if (wiped) r.client.clear();
          lift();
        }
        write.open();
        await r.settle();
        if (order === 'A') {
          expect(r.rows).toEqual([]);
          if (wiped) r.client.clear();
          lift();
          await r.settle();
        }
        expect(r.rows, `${order} wiped=${wiped}`).toEqual(wiped ? [] : ['Handed?']);
      }
    }
  });

  it('[L-1g] clear() racing an in-flight write of a turn that the panel forgot: no row', async () => {
    const r = rig({ real: true });
    const home = gate();
    r.script.homeGate = home.p;
    await endTurn(r, T1, 'Forgotten?');
    await r.settle();
    // The session end forgets the ended turn (it is no longer in the panel), then a Wipe clears.
    r.client.onSessionEvent({ ...END, reason: 'new_session' });
    const lift = r.client.fence();
    r.client.clear();
    home.open();
    await r.settle();
    lift();
    await r.settle();
    expect(r.rows).toEqual([]);
  });

  it('[L-1h] the row of each Genie call waits for the lift, and a good Wipe drops it', async () => {
    for (const wiped of [true, false]) {
      const r = rig();
      const lift = r.client.fence();
      r.script.submit.push(accepted(T1));
      await r.client.submit(turn(), 'ask');
      r.ev({ kind: 'started', turnId: T1 });
      r.script.details.set(`${T1}:${C1}`, detail(T1, C1, SALES, 'answered', 'Sales: **4.2%**.'));
      r.script.details.set(`${T1}:${C2}`, detail(T1, C2, FIN, 'answered', 'Finance: **6%**.'));
      r.ev(callEv(T1, C1, SALES, 'asking', 1));
      r.ev(callEv(T1, C2, FIN, 'asking', 2));
      r.ev(callEv(T1, C1, SALES, 'answered', 2));
      r.ev(callEv(T1, C2, FIN, 'answered', 2));
      await r.settle();
      expect(r.history).toEqual([]);
      if (wiped) r.client.clear();
      lift();
      await r.settle();
      expect(r.history.map((h) => h.question)).toEqual(
        wiped ? [] : ['Churn in Sales?', 'Churn in Finance?'],
      );
    }
  });

  it('[L-1i] forgetThread drops the rows that the fence holds', async () => {
    const r = rig();
    const lift = r.client.fence();
    await endTurn(r, T1, 'Deleted?');
    await r.settle();
    r.client.forgetThread(uuidOfTurn(T1));
    lift();
    await r.settle();
    expect(r.history).toEqual([]);
  });

  it('[L-1j] a turn that starts during the remote deletes keeps its row', async () => {
    const r = rig({ real: true });
    const lift = r.client.fence();
    // The local wipe worked: clear() ends the fence, the remote deletes still run.
    r.client.clear();
    await endTurn(r, T1, 'During remote?');
    await r.settle();
    lift();
    await r.settle();
    expect(r.rows).toEqual(['During remote?']);
  });

  it('[L-1d] a lift does not undo a Wipe fence of the core that came meanwhile', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Gone?'), 'ask');
    const lift = r.client.fence();
    r.script.result = NOTE;
    r.ev({ kind: 'ended', turnId: T1, outcome: 'answered', calls: 0 });
    r.client.onSessionEvent({ ...END, reason: 'wipe' });
    lift();
    await r.settle();
    expect(r.history).toEqual([]);
  });
});
