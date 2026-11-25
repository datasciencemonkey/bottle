import {
  asAgentThreadSendAction,
  asConversationsAction,
  asConversationsOpen,
  asHubConversations,
  asThreadFocusAction,
} from './hub-conversations';

const U1 = '0b5c1c58-6a0e-4f0e-9b62-1d2f3a4b5c6d';
const U2 = '1c6d2d69-7b1f-4a1f-8c73-2e3f4a5b6c7e';
const part = {
  requestId: 'r1',
  workspace: { name: 'Sales', colorVariant: 'teal' },
  profile: 'sales',
  workspaceHost: 'https://x.cloud.databricks.com',
  status: 'answered',
  answerMd: 'ok',
  keyNumbers: [{ text: '5', label: 'rows' }],
};
const turn = {
  turnUuid: U2,
  question: 'q',
  askedAt: 1,
  status: 'stopped',
  summaryMd: 's',
  parts: [part],
};
const thread = { threadId: U1, mogId: U2, mogName: 'MOG', turns: [turn] };

describe('conversations.delete', () => {
  it('accepts 1..200 keys of the three kinds', () => {
    const threads = ['c:conv-1', 'r:req-1', `a:${U1}`];
    expect(asConversationsAction({ action: 'conversations.delete', threads, x: 1 })).toEqual({
      action: 'conversations.delete',
      threads,
    });
    const max = Array.from({ length: 200 }, (_, i) => `r:${i}`);
    expect(asConversationsAction({ action: 'conversations.delete', threads: max })).toBeDefined();
  });

  it('keeps onlyFailed:true and drops any other value', () => {
    const threads = ['c:conv-1'];
    const d = (extra: Record<string, unknown>) =>
      asConversationsAction({ action: 'conversations.delete', threads, ...extra });
    expect(d({ onlyFailed: true })).toEqual({
      action: 'conversations.delete',
      threads,
      onlyFailed: true,
    });
    for (const v of [false, 'true', 1, null]) {
      expect(d({ onlyFailed: v })).toEqual({ action: 'conversations.delete', threads });
    }
  });

  it('refuses a bad batch', () => {
    const d = (threads: unknown) =>
      asConversationsAction({ action: 'conversations.delete', threads });
    expect(d([])).toBeUndefined();
    expect(d(Array.from({ length: 201 }, (_, i) => `r:${i}`))).toBeUndefined();
    expect(d('c:x')).toBeUndefined();
    expect(d(['x:1'])).toBeUndefined();
    expect(d(['c:'])).toBeUndefined();
    expect(d(['a:not-a-uuid'])).toBeUndefined();
    expect(d(['r:' + 'x'.repeat(65)])).toBeUndefined();
    expect(d([5])).toBeUndefined();
    expect(asConversationsAction({ action: 'conversations.delete' })).toBeUndefined();
  });

  it('keeps the old actions', () => {
    expect(asConversationsAction({ action: 'conversations.ready' })).toEqual({
      action: 'conversations.ready',
    });
    expect(asConversationsAction({ action: 'nope' })).toBeUndefined();
  });
});

describe('[ADR 0124 amendment] conversations.copy_markdown', () => {
  it('accepts a valid requestId and drops unknown fields', () => {
    expect(
      asConversationsAction({ action: 'conversations.copy_markdown', requestId: 'r1', z: 1 }),
    ).toEqual({
      action: 'conversations.copy_markdown',
      requestId: 'r1',
    });
  });

  it('refuses a missing, empty or over-long requestId', () => {
    expect(asConversationsAction({ action: 'conversations.copy_markdown' })).toBeUndefined();
    expect(
      asConversationsAction({ action: 'conversations.copy_markdown', requestId: '' }),
    ).toBeUndefined();
    expect(
      asConversationsAction({
        action: 'conversations.copy_markdown',
        requestId: 'x'.repeat(65),
      }),
    ).toBeUndefined();
    expect(
      asConversationsAction({ action: 'conversations.copy_markdown', requestId: 5 }),
    ).toBeUndefined();
  });
});

describe('agent.thread_send', () => {
  const ok = { action: 'agent.thread_send', threadId: U1, mogId: U2, question: 'Why?' };
  it('accepts a valid send and drops unknown fields', () => {
    expect(asAgentThreadSendAction({ ...ok, z: 1 })).toEqual(ok);
  });
  it('limits the question to 1..1000 code points, not blank', () => {
    expect(asAgentThreadSendAction({ ...ok, question: '😀'.repeat(1000) })).toBeDefined();
    expect(asAgentThreadSendAction({ ...ok, question: '😀'.repeat(1001) })).toBeUndefined();
    expect(asAgentThreadSendAction({ ...ok, question: '   ' })).toBeUndefined();
    expect(asAgentThreadSendAction({ ...ok, question: '' })).toBeUndefined();
    expect(asAgentThreadSendAction({ ...ok, question: 5 })).toBeUndefined();
  });
  it('refuses a bad id or action', () => {
    expect(asAgentThreadSendAction({ ...ok, threadId: 'x' })).toBeUndefined();
    expect(asAgentThreadSendAction({ ...ok, mogId: 'conversations' })).toBeUndefined();
    expect(asAgentThreadSendAction({ ...ok, action: 'thread.send' })).toBeUndefined();
    expect(asAgentThreadSendAction(null)).toBeUndefined();
  });
});

describe('thread.active_agent', () => {
  it('accepts a UUID thread id', () => {
    expect(asThreadFocusAction({ action: 'thread.active_agent', threadId: U1, x: 1 })).toEqual({
      action: 'thread.active_agent',
      threadId: U1,
    });
  });
  it('refuses a bad thread id and keeps the old actions', () => {
    expect(asThreadFocusAction({ action: 'thread.active_agent', threadId: 'x' })).toBeUndefined();
    expect(asThreadFocusAction({ action: 'thread.active_agent' })).toBeUndefined();
    expect(asThreadFocusAction({ action: 'thread.inactive' })).toEqual({
      action: 'thread.inactive',
    });
    expect(
      asThreadFocusAction({ action: 'thread.active', conversationId: 'c', profile: 'p' }),
    ).toBeDefined();
  });
});

describe('conversations open', () => {
  it('accepts a request id, a thread id, both or none', () => {
    expect(asConversationsOpen({ requestId: 'r1' })).toEqual({ requestId: 'r1' });
    expect(asConversationsOpen({ threadId: U1 })).toEqual({ threadId: U1 });
    expect(asConversationsOpen({})).toEqual({});
  });
  it('refuses a bad id', () => {
    expect(asConversationsOpen({ threadId: 'x' })).toBeUndefined();
    expect(asConversationsOpen({ threadId: 5 })).toBeUndefined();
    expect(asConversationsOpen({ requestId: '' })).toBeUndefined();
  });
});

describe('hub:conversations agentThreads', () => {
  it('accepts a payload with no agent threads or with valid ones', () => {
    expect(asHubConversations({ rows: [] })).toBeDefined();
    expect(asHubConversations({ rows: [], agentThreads: [thread] })).toBeDefined();
  });
  it('refuses a bad thread, turn or part', () => {
    const bad = (agentThreads: unknown) => asHubConversations({ rows: [], agentThreads });
    expect(bad('x')).toBeUndefined();
    expect(bad([{ ...thread, threadId: 'x' }])).toBeUndefined();
    expect(bad([{ ...thread, turns: [{ ...turn, status: 'done' }] }])).toBeUndefined();
    expect(bad([{ ...thread, turns: [{ ...turn, askedAt: NaN }] }])).toBeUndefined();
    expect(
      bad([{ ...thread, turns: [{ ...turn, parts: [{ ...part, status: 'stopped' }] }] }]),
    ).toBeUndefined();
    expect(
      bad([{ ...thread, turns: [{ ...turn, parts: [{ ...part, workspace: {} }] }] }]),
    ).toBeUndefined();
  });
  it('[ADR 0122] takes the stored citations of a turn and refuses a bad one', () => {
    const withCites = (citations: unknown) =>
      asHubConversations({
        rows: [],
        agentThreads: [{ ...thread, turns: [{ ...turn, citations }] }],
      });
    const chip = part.workspace;
    expect(withCites([{ n: 1, workspace: chip, requestId: part.requestId }])).toBeDefined();
    expect(withCites([{ n: 2, workspace: chip }])).toBeDefined();
    expect(withCites('x')).toBeUndefined();
    expect(withCites([{ n: 0, workspace: chip }])).toBeUndefined();
    expect(withCites([{ n: 1.5, workspace: chip }])).toBeUndefined();
    expect(withCites([{ n: 1, workspace: {} }])).toBeUndefined();
    expect(withCites([{ n: 1, workspace: chip, requestId: 7 }])).toBeUndefined();
  });
});
