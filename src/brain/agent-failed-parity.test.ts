// @vitest-environment jsdom
/**
 * One definition of "failed" for Clear failed: the view counts a thread with `isFailedThread`
 * (AgentThread.tsx) and the brain re-checks it with `KeyRows.failed` (hub-conversations-wiring.ts).
 * A thread that the view counts but the brain skips shows in "Clear failed (n)", hides, and
 * comes back. This test builds each case through the real brain read and the real view, and
 * keeps the two equal.
 */
import type { LocalMemoryStore } from './components/memory-store';
import { readConversationsIndexed } from './hub-conversations-wiring';
import { buildAgentEntries, isFailedThread } from '../views/hub/AgentThread';
import type { HubAgentView } from '../shared/hub-agent';
import type { AgentTurnView } from '../shared/ipc';

vi.mock('@tauri-apps/api/core', async () => (await import('../views/test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../views/test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../views/test-tauri')).windowModule);

const MOG = '00000000-0000-4000-8000-00000000000a';
const T0 = 1_767_225_600_000;
const id = (n: number) => `41111111-2222-4333-8444-${n.toString(16).padStart(12, '0')}`;
type Row = Record<string, unknown>;
const row = (requestId: string, askedAt: number, over: Row): Row => ({
  requestId,
  status: 'completed',
  profileName: 'fake-sales',
  workspaceHost: 'sales.fake.test',
  deepLink: '',
  questionUser: '',
  keyNumbers: '[]',
  finalAnswerMd: '',
  conversationId: '',
  backendKind: 'genie_one_mcp',
  splitGroupId: '',
  askedAt,
  ...over,
});
const turn = (n: number, status: string, over: Row = {}) =>
  row(id(n), T0 + n, {
    backendKind: 'genie_agent',
    splitGroupId: id(1),
    questionUser: `Q${n}?`,
    status,
    ...over,
  });
const part = (n: number, of: number, status: string) =>
  row(id(n), T0 + n, {
    splitGroupId: id(of),
    status,
    finalAnswerMd: status === 'completed' ? 'A' : '',
  });

const CASES: [string, Row[], boolean][] = [
  ['an answered turn', [turn(1, 'completed'), part(10, 1, 'completed')], false],
  ['a turn that failed with a failed part', [turn(1, 'failed'), part(10, 1, 'failed')], true],
  ['a stopped turn with no part', [turn(1, 'cancelled')], true],
  [
    'a stopped turn where one call answered',
    [turn(1, 'cancelled'), part(10, 1, 'completed')],
    false,
  ],
  [
    'a timed-out turn where one call answered',
    [turn(1, 'abandoned'), part(10, 1, 'completed')],
    false,
  ],
  [
    'a fallback turn (row failed, home part answered)',
    [turn(1, 'failed'), part(10, 1, 'completed')],
    false,
  ],
  [
    'a second turn answered after a first one failed',
    [turn(1, 'failed'), turn(2, 'completed')],
    false,
  ],
  ['two turns that both failed', [turn(1, 'failed'), turn(2, 'cancelled')], true],
];

describe('Clear failed: the view and the brain agree on "failed"', () => {
  it.each(CASES)('%s', async (_name, rows, failed) => {
    const store: LocalMemoryStore = { exec: async () => ({ rows }) };
    const read = await readConversationsIndexed({
      store,
      mogs: async () => [{ mogId: MOG, name: 'Sales' }],
    });
    const threads = read.list.agentThreads ?? [];
    expect(threads).toHaveLength(1);
    const entries = buildAgentEntries(threads, undefined, undefined, 0);
    const entry = entries[0];
    if (!entry) throw new Error('no entry');
    const brain = read.keys.get(`a:${entry.threadId}`)?.failed;
    expect(isFailedThread(entry)).toBe(failed);
    expect(brain).toBe(failed);
  });
});

/** The turns that the client holds in memory: the view swaps or adds them, the brain reads rows. */
describe('Clear failed: in-memory turns', () => {
  const turnId = (n: number) => id(n).replaceAll('-', '');
  const ws = { name: 'Sales', colorVariant: 'coral' };
  const memTurn = (n: number, state: AgentTurnView['state'], callState: string): AgentTurnView =>
    ({
      turnId: turnId(n),
      question: `Q${n}?`,
      state,
      used: 1,
      budget: 10,
      agentWorkspace: ws,
      calls: [{ callId: `c-${n}`, workspace: ws, question: 'Q', state: callState }],
      parts: [],
    }) as AgentTurnView;
  const agentView = (turns: AgentTurnView[], thread: string): HubAgentView => ({
    session: { state: 'none', agentProfile: null, turns: 0, queued: 0, tokens: 0 },
    waiting: [],
    earlier: turns,
    threads: turns.map((t) => ({ turnId: t.turnId, threadId: thread })),
  });

  async function both(rows: Row[], turns: AgentTurnView[], thread: string) {
    const store: LocalMemoryStore = { exec: async () => ({ rows }) };
    const read = await readConversationsIndexed({
      store,
      mogs: async () => [{ mogId: MOG, name: 'Sales' }],
      memoryTurns: () =>
        turns.map((t) => ({
          turnUuid: id(Number(`0x${t.turnId.slice(-12)}`)),
          threadId: thread,
          inProgress: false,
        })),
    });
    const entry = buildAgentEntries(read.list.agentThreads, agentView(turns, thread), MOG, 0).find(
      (e) => e.threadId === thread,
    );
    if (!entry) throw new Error('no entry');
    return { view: isFailedThread(entry), brain: read.keys.get(`a:${thread}`)?.failed };
  }

  it('a needs_you turn is never counted by the view, whatever its stored row says', async () => {
    const r = await both([turn(1, 'failed')], [memTurn(1, 'needs_you', 'needs_you')], id(1));
    expect(r.view).toBe(false);
    // The stored row says failed (a signed-out turn saves a failed row, ADR 0121): the brain
    // would delete it, but the view never asks while the turn is in memory.
    expect(r.brain).toBe(true);
  });

  it('a stopped turn with one ready call is answered in both', async () => {
    const r = await both(
      [turn(1, 'cancelled'), part(10, 1, 'completed')],
      [memTurn(1, 'stopped', 'ready')],
      id(1),
    );
    expect(r).toEqual({ view: false, brain: false });
  });

  it('a stopped turn with no ready call is failed in both', async () => {
    const r = await both(
      [turn(1, 'cancelled'), part(10, 1, 'failed')],
      [memTurn(1, 'stopped', 'failed')],
      id(1),
    );
    expect(r).toEqual({ view: true, brain: true });
  });

  it('a memory-only thread with extra parts (no turn row) agrees with the view', async () => {
    const answered = await both([part(10, 1, 'completed')], [memTurn(1, 'failed', 'ready')], id(1));
    expect(answered).toEqual({ view: false, brain: false });
    const failed = await both([part(10, 1, 'failed')], [memTurn(1, 'failed', 'failed')], id(1));
    expect(failed).toEqual({ view: true, brain: true });
  });
});
