import { asAgentAction, asAgentTurnView, asHubAgentView } from './hub-agent';

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
  threads: [{ turnId: T1, threadId: '0b5c1c58-6a0e-4f0e-9b62-1d2f3a4b5c6d' }],
};

describe('hub-agent shape checks (slice 2a)', () => {
  it('accepts each documented action and drops unknown fields', () => {
    expect(asAgentAction({ action: 'agent.ready', x: 1 })).toEqual({ action: 'agent.ready' });
    expect(asAgentAction({ action: 'agent.new_session' })).toBeUndefined();
    expect(asAgentAction({ action: 'agent.open_settings' })).toBeDefined();
    expect(asAgentAction({ action: 'agent.stop', turnId: T1, y: 2 })).toEqual({
      action: 'agent.stop',
      turnId: T1,
    });
    expect(asAgentAction({ action: 'agent.remove_queued', turnId: T1 })).toEqual({
      action: 'agent.remove_queued',
      turnId: T1,
    });
    // [ADR 0124 amendment] the "Copy Markdown" button.
    expect(asAgentAction({ action: 'agent.copy_markdown', turnId: T1, z: 3 })).toEqual({
      action: 'agent.copy_markdown',
      turnId: T1,
    });
    // [ADR 0124 amendment] the "Copy" button.
    expect(asAgentAction({ action: 'agent.copy', turnId: T1, z: 3 })).toEqual({
      action: 'agent.copy',
      turnId: T1,
    });
  });

  it('refuses bad actions', () => {
    expect(asAgentAction({ action: 'agent.stop' })).toBeUndefined();
    expect(asAgentAction({ action: 'agent.stop', turnId: 'nope' })).toBeUndefined();
    expect(
      asAgentAction({ action: 'agent.remove_queued', turnId: T1.toUpperCase() }),
    ).toBeUndefined();
    expect(asAgentAction({ action: 'agent.copy_markdown' })).toBeUndefined();
    expect(asAgentAction({ action: 'agent.copy_markdown', turnId: 'nope' })).toBeUndefined();
    expect(asAgentAction({ action: 'agent.copy' })).toBeUndefined();
    expect(asAgentAction({ action: 'agent.copy', turnId: 'nope' })).toBeUndefined();
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
    };
    expect(asHubAgentView({ ...empty, version: 'x' })).toEqual(empty);
  });

  it('accepts needs_you and seenAt on a turn', () => {
    const t = asAgentTurnView({ ...turn, state: 'needs_you', seenAt: 99, extra: 1 });
    expect(t?.state).toBe('needs_you');
    expect(t?.seenAt).toBe(99);
    expect(t).not.toHaveProperty('extra');
  });

  it('keeps a valid copyNote on a turn, so the card and Conversations show "Copied"', () => {
    expect(asAgentTurnView({ ...turn, copyNote: 'copied' })?.copyNote).toBe('copied');
    expect(asAgentTurnView({ ...turn, copyNote: 'failed' })?.copyNote).toBe('failed');
    expect(asAgentTurnView({ ...turn, copyNote: 'other' })).not.toHaveProperty('copyNote');
    expect(asAgentTurnView(turn)).not.toHaveProperty('copyNote');
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
    expect(asHubAgentView(undefined)).toBeUndefined();
  });

  it('keeps summary citations, drops their unknown fields and refuses bad ones (ADR 0122)', () => {
    const cite = { n: 1, callId: 'c-0123abcd', workspace: chip };
    const withCites = (citations: unknown) =>
      asAgentTurnView({ ...turn, summary: { ...turn.summary, citations } });
    expect(withCites(undefined)?.summary).not.toHaveProperty('citations');
    expect(withCites([])?.summary?.citations).toEqual([]);
    const deepLink = 'https://example.cloud.databricks.com/genie/rooms/1';
    expect(
      withCites([cite, { ...cite, n: 2, deepLink, raw: 'x', workspace: { ...chip, z: 1 } }])
        ?.summary?.citations,
    ).toEqual([cite, { ...cite, n: 2, deepLink }]);
    expect(withCites('x')).toBeUndefined();
    expect(withCites([null])).toBeUndefined();
    expect(withCites([{ ...cite, n: '1' }])).toBeUndefined();
    expect(withCites([{ ...cite, callId: 5 }])).toBeUndefined();
    expect(withCites([{ ...cite, workspace: { name: 5 } }])).toBeUndefined();
    expect(withCites([{ ...cite, deepLink: 5 }])).toBeUndefined();
    expect(withCites([{ ...cite, workspaceHost: 5 }])).toBeUndefined();
    expect(
      withCites([{ ...cite, deepLink, workspaceHost: 'https://example.cloud.databricks.com' }])
        ?.summary?.citations?.[0]?.workspaceHost,
    ).toBe('https://example.cloud.databricks.com');
    for (const n of [-1, 0, 1.5]) expect(withCites([{ ...cite, n }])).toBeUndefined();
    expect(withCites([{ ...cite, callId: 'c-1' }])).toBeUndefined();
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
});
