import { asAgentAction, asAgentTurnView, asGenieCodeStatus, asHubAgentView } from './hub-agent';

const T1 = 'a'.repeat(32);
const T2 = 'b'.repeat(32);
const chip = { name: 'Sales', colorVariant: 'teal' };
const call = { callId: 'c-0123abcd', workspace: chip, question: 'Revenue?', state: 'ready' };
const part = { callId: 'c-0123abcd', workspace: chip, sections: [{ id: 'answer', md: 'x' }] };
const turn = {
  turnId: T1,
  question: 'What is revenue?',
  state: 'done',
  used: 1,
  budget: 10,
  agentWorkspace: chip,
  calls: [call],
  parts: [part],
  summary: { md: 'Note', kind: 'note', workspaces: [chip] },
  tokens: 1200,
};
const genieCode = {
  binary: 'ready',
  version: '0.5.0',
  pinned: '0.5.0',
  agentProfile: 'sales',
  agentProfileState: 'ready',
};
const session = {
  state: 'busy',
  sessionId: T2,
  startedAt: 1,
  agentProfile: 'sales',
  turns: 2,
  queued: 1,
  tokens: 5,
};
const view = {
  session,
  live: { ...turn, state: 'running' },
  waiting: [{ ...turn, turnId: T2, state: 'queued', position: 1, calls: [], parts: [] }],
  earlier: [turn],
  genieCode,
  threads: [{ turnId: T1, threadId: '0b5c1c58-6a0e-4f0e-9b62-1d2f3a4b5c6d' }],
};

describe('hub-agent shape checks (slice 2a)', () => {
  it('accepts each documented action and drops unknown fields', () => {
    expect(asAgentAction({ action: 'agent.ready', x: 1 })).toEqual({ action: 'agent.ready' });
    expect(asAgentAction({ action: 'agent.new_session' })).toEqual({ action: 'agent.new_session' });
    expect(asAgentAction({ action: 'agent.open_settings' })).toBeDefined();
    expect(asAgentAction({ action: 'agent.stop', turnId: T1, y: 2 })).toEqual({
      action: 'agent.stop',
      turnId: T1,
    });
    expect(asAgentAction({ action: 'agent.remove_queued', turnId: T1 })).toEqual({
      action: 'agent.remove_queued',
      turnId: T1,
    });
  });

  it('refuses bad actions', () => {
    expect(asAgentAction({ action: 'agent.stop' })).toBeUndefined();
    expect(asAgentAction({ action: 'agent.stop', turnId: 'nope' })).toBeUndefined();
    expect(
      asAgentAction({ action: 'agent.remove_queued', turnId: T1.toUpperCase() }),
    ).toBeUndefined();
    expect(asAgentAction({ action: 'workspace.cross' })).toBeUndefined();
    expect(asAgentAction(null)).toBeUndefined();
  });

  it('parses a full panel view and keeps every part', () => {
    expect(asHubAgentView(view)).toEqual(view);
  });

  it('parses the empty panel (no session)', () => {
    const empty = {
      session: { state: 'none', agentProfile: null, turns: 0, queued: 0, tokens: 0 },
      waiting: [],
      earlier: [],
      threads: [],
      genieCode: { ...genieCode, binary: 'not_installed', agentProfile: null },
    };
    expect(asHubAgentView({ ...empty, version: 'x' })).toEqual(empty);
  });

  it('accepts needs_you and seenAt on a turn', () => {
    const t = asAgentTurnView({ ...turn, state: 'needs_you', seenAt: 99, extra: 1 });
    expect(t?.state).toBe('needs_you');
    expect(t?.seenAt).toBe(99);
    expect(t).not.toHaveProperty('extra');
  });

  it('drops unknown fields at every level', () => {
    const extra = {
      ...view,
      z: 1,
      session: { ...session, secret: 'no' },
      live: { ...view.live, leak: 1, calls: [{ ...call, raw: 1 }] },
    };
    const got = asHubAgentView(extra);
    expect(got).toEqual(view);
  });

  it('refuses more than 3 waiting turns', () => {
    const [w] = view.waiting;
    expect(asHubAgentView({ ...view, waiting: [w, w, w] })).toBeDefined();
    expect(asHubAgentView({ ...view, waiting: [w, w, w, w] })).toBeUndefined();
  });

  it('refuses bad turns, calls, parts and sessions', () => {
    expect(asHubAgentView({ ...view, live: { ...turn, state: 'maybe' } })).toBeUndefined();
    expect(asHubAgentView({ ...view, live: { ...turn, turnId: 'x' } })).toBeUndefined();
    expect(asHubAgentView({ ...view, live: { ...turn, used: -1 } })).toBeUndefined();
    expect(
      asHubAgentView({ ...view, live: { ...turn, calls: [{ ...call, callId: 'c-1' }] } }),
    ).toBeUndefined();
    expect(
      asHubAgentView({ ...view, live: { ...turn, parts: [{ ...part, sections: [5] }] } }),
    ).toBeUndefined();
    expect(asHubAgentView({ ...view, session: { ...session, state: 'x' } })).toBeUndefined();
    expect(asHubAgentView({ ...view, session: { ...session, turns: 1.5 } })).toBeUndefined();
    expect(asHubAgentView({ ...view, earlier: [{}] })).toBeUndefined();
    expect(asHubAgentView({ ...view, genieCode: { binary: 'ready' } })).toBeUndefined();
    expect(asHubAgentView(undefined)).toBeUndefined();
  });

  it('drops a bad notice instead of failing', () => {
    expect(asHubAgentView({ ...view, notice: { kind: 'bad', text: 'x' } })?.notice).toBeUndefined();
    expect(asHubAgentView({ ...view, notice: { kind: 'info', text: 'x' } })?.notice).toEqual({
      kind: 'info',
      text: 'x',
    });
  });

  it('checks the thread links', () => {
    expect(asHubAgentView({ ...view, threads: [{ turnId: T1, threadId: 'x' }] })).toBeUndefined();
    expect(
      asHubAgentView({ ...view, threads: [{ turnId: 'x', threadId: view.threads[0]?.threadId }] }),
    ).toBeUndefined();
    expect(asHubAgentView({ ...view, threads: 'x' })).toBeUndefined();
    expect(asHubAgentView({ ...view, threads: undefined })).toBeUndefined();
  });

  it('checks the Genie Code status', () => {
    expect(asGenieCodeStatus(genieCode)).toEqual(genieCode);
    expect(asGenieCodeStatus({ ...genieCode, binary: 'maybe' })).toBeUndefined();
    expect(asGenieCodeStatus({ ...genieCode, agentProfileState: 'x' })).toBeUndefined();
  });
});
