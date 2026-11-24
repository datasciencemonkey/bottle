/**
 * Slice 2a: the brain client of the live Genie Code session. IPC doubles and a simulated clock.
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
import type { GenieEnd } from './genie-history';
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

function rig() {
  const clock = createSimClock(1_000);
  const calls: string[] = [];
  const shown: AgentTurnView[] = [];
  const dropped: string[] = [];
  const notes: string[] = [];
  const directs: { turn: AgentTurn; notice: string }[] = [];
  const history: GenieEnd[] = [];
  const phases: string[] = [];
  const logLines: string[] = [];
  let changes = 0;
  const script = {
    submit: [] as (AgentTurnAccepted | { error: string })[],
    details: new Map<string, AgentTurnCall>(),
    result: undefined as AgentTurnResult | undefined,
    session: sessionOf(),
  };
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
    homeProfile: async () => 'fake-sales',
    hostOf: async (p) => `https://${p}.fake.test`,
    newId: () => `00000000-0000-4000-8000-${String(++n).padStart(12, '0')}`,
    wallNow: () => clock.now(),
    recordCall: (x) => void history.push(x),
    direct: async (t, notice) => void directs.push({ turn: t, notice }),
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
    await r.client.submit(turn('Plain question'), 'ask_others', 'ticket-1');
    expect(r.calls[0]).toBe('submit:ask_others:op:ticket');
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
    ['genie_code_missing', 'Genie Code is not installed. Asked Sales only.'],
    ['genie_code_version', 'Genie Code is not the version that Bottle needs. Asked Sales only.'],
    ['agent_profile_unsupported', 'Genie Code is not available in Sales. Asked Sales only.'],
    ['agent_not_eligible', 'Genie Code is not available in Sales. Asked Sales only.'],
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
  it('a signed-out end keeps the turn as `needs_you` with its question, and does not fall back', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'signed_out', calls: 0 });
    await r.settle();
    expect(r.directs).toEqual([]);
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

  it('a crash keeps the failed turn on screen until the next session starts', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'failed', calls: 1 });
    await r.settle();
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'crashed' });
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
    '[H1] a %s ends live and waiting turns with session_ended: no direct call, no history row',
    async (reason) => {
      const r = rig();
      await submitTwo(r);
      r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason });
      r.ev({ kind: 'ended', turnId: T1, outcome: 'session_ended', calls: 0 });
      r.ev({ kind: 'ended', turnId: T2, outcome: 'session_ended', calls: 0 });
      await r.settle();
      expect(r.directs).toEqual([]);
      expect(r.history).toEqual([]);
    },
  );

  it.each(['lockdown', 'refusals', 'session_ended', 'signed_out', 'not_eligible'] as const)(
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

  it('[H2] an ask_others turn that ends before any call shows the notice and asks home no more', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn('Plain?'), 'ask_others', 't');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'failed', calls: 0 });
    await r.settle();
    expect(r.directs).toEqual([]);
    expect(r.client.panel().earlier[0]?.notice).toBe('The agent could not run.');
  });

  it('[H2] a refused ask_others submit shows only a note', async () => {
    const r = rig();
    r.script.submit.push({ error: 'genie_code_missing' });
    await r.client.submit(turn('Plain?'), 'ask_others', 't');
    expect(r.directs).toEqual([]);
    expect(r.notes).toEqual(['Genie Code is not installed.']);
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

  it('[M1] clear() forgets what a crash left on screen', async () => {
    const r = rig();
    r.script.submit.push(accepted(T1));
    await r.client.submit(turn(), 'ask');
    r.ev({ kind: 'ended', turnId: T1, outcome: 'failed', calls: 1 });
    await r.settle();
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'crashed' });
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
    expect(r.history).toHaveLength(1);
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
    r.client.onSessionEvent({ kind: 'ended', sessionId: 'd'.repeat(32), reason: 'crashed' });
    r.script.session = sessionOf({ state: 'none', turns: 0 });
    await r.client.newSession();
    expect(r.client.panel().earlier).toEqual([]);
  });
});
