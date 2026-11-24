/**
 * ADR 0121: agent threads in hub Conversations, brain side. The rebuild of a thread from its turn
 * rows and part rows, the dedup against the turns in memory, Delete and Clear failed. The store is
 * a double that holds rows and runs `mem.exchange.recent` and `mem.exchange.delete`.
 */
import { BrainBus } from './bus';
import type { LocalMemoryStore } from './components/memory-store';
import {
  readConversations,
  wireHubConversations,
  type MemoryTurn,
} from './hub-conversations-wiring';
import type { HubConversations } from '../shared/hub-conversations';
import type { JsonObject } from '../shared/ipc';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const MOG_A = '00000000-0000-4000-8000-00000000000a';
const MOG_B = '00000000-0000-4000-8000-00000000000b';
const id = (n: number) => `41111111-2222-4333-8444-${n.toString(16).padStart(12, '0')}`;
const T0 = 1_767_225_600_000;
const ROOT = id(100);
const TURN1 = ROOT;
const TURN2 = id(200);

type Row = Record<string, unknown> & { mogId?: string };

const base = (requestId: string, askedAt: number, over: Row = {}): Row => ({
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
const turnRow = (turnUuid: string, root: string, askedAt: number, over: Row = {}) =>
  base(turnUuid, askedAt, {
    backendKind: 'genie_agent',
    splitGroupId: root,
    questionUser: `Q ${turnUuid.slice(-3)}?`,
    finalAnswerMd: 'Note from the MOG',
    ...over,
  });
const partRow = (n: number, turnUuid: string, askedAt: number, over: Row = {}) =>
  base(id(n), askedAt, {
    splitGroupId: turnUuid,
    finalAnswerMd: `Part ${n}`,
    conversationId: `conv-${n}`,
    ...over,
  });

function fakeStore(initial: Record<string, Row[]>) {
  const rows: Record<string, Row[]> = structuredClone(initial);
  const calls: { stmtId: string; params: JsonObject }[] = [];
  const store: LocalMemoryStore = {
    exec: async (stmtId, params) => {
      calls.push({ stmtId, params });
      const mog = params['mogId'] as string;
      if (stmtId === 'mem.exchange.recent') return { rows: rows[mog] ?? [] };
      if ((stmtId as string) === 'mem.exchange.delete') {
        const ids = params['requestIds'] as string[];
        const before = rows[mog]?.length ?? 0;
        rows[mog] = (rows[mog] ?? []).filter((r) => !ids.includes(r['requestId'] as string));
        return { rows: [{ changes: before - (rows[mog]?.length ?? 0) }] };
      }
      throw new Error(`unexpected ${stmtId}`);
    },
  };
  return { store, rows, calls };
}

const mogs = async () => [
  { mogId: MOG_A, name: 'Sales', colorVariant: 'coral' },
  { mogId: MOG_B, name: 'Ops' },
];
const chipOf = (profile: string) => ({
  name: profile === 'fake-sales' ? 'Sales' : 'Fin',
  colorVariant: 'sky',
});

/** Two turns of one thread (turn rows in MOG A), parts in A and B, and one plain Genie row. */
function threadRows(): Record<string, Row[]> {
  return {
    [MOG_A]: [
      turnRow(TURN1, ROOT, T0 + 1000),
      turnRow(TURN2, ROOT, T0 + 5000, { status: 'cancelled', finalAnswerMd: '' }),
      partRow(1, TURN1, T0 + 1100),
      partRow(2, TURN2, T0 + 5100, { status: 'abandoned' }),
      base(id(900), T0 + 9000, {
        questionUser: 'Plain?',
        finalAnswerMd: 'Plain.',
        conversationId: 'plain',
      }),
    ],
    [MOG_B]: [
      partRow(3, TURN1, T0 + 1200, { profileName: 'fake-fin', questionUser: '', status: 'failed' }),
    ],
  };
}

let bus: BrainBus;
beforeEach(() => {
  bus = new BrainBus();
});
function input(payload: unknown): void {
  bus.dispatch({
    event: 'view:input',
    payload: { from: 'hub', input: { mogId: 'conversations', kind: 'click', payload } },
  });
}

describe('the rebuild of agent threads', () => {
  it('[AC-C3] a thread has its turns oldest first, each with its summary and parts; parts never go in rows', async () => {
    const { store } = fakeStore(threadRows());
    const list = await readConversations({ store, mogs, chipOf });
    expect(list.rows.map((r) => r.requestId)).toEqual([id(900)]);
    expect(list.agentThreads).toHaveLength(1);
    const t = list.agentThreads?.[0];
    expect(t).toMatchObject({
      threadId: ROOT,
      mogId: MOG_A,
      mogName: 'Sales',
      mogColorVariant: 'coral',
    });
    expect(t?.turns.map((x) => [x.turnUuid, x.status, x.summaryMd])).toEqual([
      [TURN1, 'answered', 'Note from the MOG'],
      [TURN2, 'stopped', ''],
    ]);
    // The parts of a turn can sit in another MOG; they are ordered by time.
    expect(
      t?.turns[0]?.parts.map((p) => [p.requestId, p.workspace.name, p.status, p.answerMd]),
    ).toEqual([
      [id(1), 'Sales', 'answered', 'Part 1'],
      [id(3), 'Fin', 'failed', 'Part 3'],
    ]);
    expect(t?.turns[1]?.parts.map((p) => p.status)).toEqual(['failed']);
  });

  it('a fallback turn (failed on its own row, answered home part) shows answered', async () => {
    const { store } = fakeStore({
      [MOG_A]: [
        turnRow(TURN1, ROOT, T0, { status: 'failed', finalAnswerMd: '' }),
        partRow(1, TURN1, T0 + 10),
      ],
    });
    const list = await readConversations({ store, mogs });
    expect(list.agentThreads?.[0]?.turns[0]?.status).toBe('answered');
    // Without a chip source, the chip is the profile name.
    expect(list.agentThreads?.[0]?.turns[0]?.parts[0]?.workspace.name).toBe('fake-sales');
  });

  it('a follow-up turn joins the thread of its root; another root is another thread', async () => {
    const other = id(300);
    const { store } = fakeStore({
      [MOG_A]: [
        turnRow(TURN1, ROOT, T0),
        turnRow(TURN2, ROOT, T0 + 10),
        turnRow(other, other, T0 + 20),
      ],
    });
    const list = await readConversations({ store, mogs });
    expect(list.agentThreads?.map((t) => [t.threadId, t.turns.length])).toEqual([
      [other, 1],
      [ROOT, 2],
    ]);
  });

  it('[0.1.5] call rows with no turn row become one thread: "Question not saved" with its parts', async () => {
    const group = id(400);
    const { store } = fakeStore({
      [MOG_A]: [
        partRow(1, group, T0 + 1, { questionUser: 'sub question' }),
        partRow(2, group, T0 + 2, { status: 'failed' }),
      ],
      [MOG_B]: [partRow(3, group, T0 + 3, { profileName: 'fake-fin' })],
    });
    const list = await readConversations({ store, mogs, chipOf });
    expect(list.rows).toEqual([]);
    const t = list.agentThreads?.[0];
    expect(t?.threadId).toBe(group);
    expect(t?.mogId).toBe(MOG_A);
    expect(t?.turns).toHaveLength(1);
    expect(t?.turns[0]).toMatchObject({
      turnUuid: group,
      question: '',
      summaryMd: '',
      status: 'answered',
    });
    expect(t?.turns[0]?.parts).toHaveLength(3);
  });

  it('a group of parts with a bad group id, and a pending row, are not a thread', async () => {
    const { store } = fakeStore({
      [MOG_A]: [partRow(1, 'not-a-uuid', T0), turnRow(TURN1, ROOT, T0 + 1, { status: 'pending' })],
    });
    const list = await readConversations({ store, mogs });
    expect(list.agentThreads).toBeUndefined();
    expect(list.rows).toEqual([]);
  });

  it('[dedup] a turn that the client holds in memory wins: the stored turn is not sent', async () => {
    const { store } = fakeStore(threadRows());
    const mem: MemoryTurn[] = [{ turnUuid: TURN1, threadId: ROOT, inProgress: false }];
    const list = await readConversations({ store, mogs, memoryTurns: () => mem });
    expect(list.agentThreads?.[0]?.turns.map((t) => t.turnUuid)).toEqual([TURN2]);
    // Every turn in memory: the thread stays (its MOG), with no stored turn.
    mem.push({ turnUuid: TURN2, threadId: ROOT, inProgress: false });
    const all = await readConversations({ store, mogs, memoryTurns: () => mem });
    expect(all.agentThreads?.[0]).toMatchObject({ threadId: ROOT, turns: [] });
  });

  it('[dedup] the call rows of a live turn that has no turn row yet are no thread', async () => {
    const live = id(500);
    const { store } = fakeStore({ [MOG_A]: [partRow(1, live, T0)] });
    const list = await readConversations({
      store,
      mogs,
      memoryTurns: () => [{ turnUuid: live, threadId: live, inProgress: true }],
    });
    expect(list.agentThreads).toBeUndefined();
  });

  it('[Wipe] after a Wipe the store and the client are empty: no thread', async () => {
    const f = fakeStore(threadRows());
    f.rows[MOG_A] = [];
    f.rows[MOG_B] = [];
    const list = await readConversations({ store: f.store, mogs, memoryTurns: () => [] });
    expect(list).toEqual({ rows: [] });
  });
});

describe('Delete and Clear failed', () => {
  function rig(rows: Record<string, Row[]>, mem: MemoryTurn[] = []) {
    const f = fakeStore(rows);
    const sent: HubConversations[] = [];
    const forgotten: string[] = [];
    const w = wireHubConversations(bus, {
      store: f.store,
      mogs,
      chipOf,
      memoryTurns: () => mem,
      forgetThread: (t) => void forgotten.push(t),
      send: async (l) => void sent.push(l),
      sendOpen: async () => undefined,
    });
    return { ...f, w, sent, forgotten };
  }
  const writes = (calls: { stmtId: string }[]) =>
    calls.filter((c) => c.stmtId !== 'mem.exchange.recent').map((c) => c.stmtId);

  it('[AC-C18] [AC-C16] [AC-C17] deletes the turn rows, call rows and fallback rows in each MOG, locally only', async () => {
    const r = rig(threadRows());
    await r.w.refresh();
    input({ action: 'conversations.delete', threads: [`a:${ROOT}`] });
    await vi.waitFor(() => expect(r.sent.length).toBe(2));
    // Only the plain row stays; one delete for each MOG; no Genie delete, no memory delete.
    expect(r.rows[MOG_A]?.map((x) => x['requestId'])).toEqual([id(900)]);
    expect(r.rows[MOG_B]).toEqual([]);
    expect(writes(r.calls)).toEqual(['mem.exchange.delete', 'mem.exchange.delete']);
    expect(
      r.calls
        .filter((c) => c.stmtId === 'mem.exchange.delete')
        .map((c) => c.params['mogId'])
        .sort(),
    ).toEqual([MOG_A, MOG_B]);
    // The client forgets the turns, then the list is sent again with no thread.
    expect(r.forgotten).toEqual([ROOT]);
    expect(r.sent.at(-1)?.agentThreads).toBeUndefined();
    expect(r.sent.at(-1)?.rows.map((x) => x.requestId)).toEqual([id(900)]);
  });

  it('deletes a Genie thread by conversation key and a lone row by request key; unknown keys do nothing', async () => {
    const r = rig({
      [MOG_A]: [
        base(id(1), T0, { conversationId: 'cv', questionUser: 'a' }),
        base(id(2), T0 + 1, { conversationId: 'cv', questionUser: 'b' }),
        base(id(3), T0 + 2, { questionUser: 'lone' }),
        base(id(4), T0 + 3, { conversationId: 'keep' }),
      ],
    });
    await r.w.refresh();
    await r.w.remove(['c:cv', `r:${id(3)}`, 'c:ghost', `a:${id(777)}`]);
    expect(r.rows[MOG_A]?.map((x) => x['requestId'])).toEqual([id(4)]);
    // An agent key with no stored row still makes the client forget (a turn only in memory).
    expect(r.forgotten).toEqual([id(777)]);
  });

  it('[AC-C15] refuses a thread in progress, also with no stored row; others go', async () => {
    const r = rig(threadRows(), [{ turnUuid: id(600), threadId: ROOT, inProgress: true }]);
    await r.w.refresh();
    await r.w.remove([`a:${ROOT}`, `r:${id(901)}`]);
    expect(r.rows[MOG_A]).toHaveLength(5);
    expect(r.forgotten).toEqual([]);
    expect(writes(r.calls)).toEqual([]);
  });

  it('[AC-C14] Clear failed re-checks each key: an answered thread stays, a failed one goes', async () => {
    const failedRoot = id(700);
    const r = rig({
      [MOG_A]: [
        ...(threadRows()[MOG_A] ?? []),
        turnRow(failedRoot, failedRoot, T0 + 20_000, { status: 'failed', finalAnswerMd: '' }),
        partRow(50, failedRoot, T0 + 20_001, { status: 'failed' }),
        base(id(61), T0 + 30, { status: 'failed', conversationId: 'bad' }),
        base(id(62), T0 + 31, { status: 'failed', conversationId: 'mixed' }),
        base(id(63), T0 + 32, { conversationId: 'mixed' }),
        base(id(64), T0 + 33, { status: 'cancelled' }),
      ],
      [MOG_B]: threadRows()[MOG_B] ?? [],
    });
    await r.w.refresh();
    // The view sends every key it showed as failed; the first thread has an answered turn now.
    await r.w.remove([`a:${ROOT}`, `a:${failedRoot}`, 'c:bad', 'c:mixed', `r:${id(64)}`], {
      onlyFailed: true,
    });
    const left = (r.rows[MOG_A] ?? []).map((x) => x['requestId']);
    expect(left).toContain(TURN1);
    expect(left).toContain(id(62));
    expect(left).not.toContain(failedRoot);
    expect(left).not.toContain(id(50));
    expect(left).not.toContain(id(61));
    expect(left).not.toContain(id(64));
    expect(r.forgotten).toEqual([failedRoot]);
  });

  it('[AC-C14] a stopped or timed-out thread counts as failed; the bus action carries onlyFailed', async () => {
    const root = id(800);
    const r = rig({
      [MOG_A]: [turnRow(root, root, T0, { status: 'cancelled', finalAnswerMd: '' })],
    });
    await r.w.refresh();
    input({ action: 'conversations.delete', threads: [`a:${root}`], onlyFailed: true });
    await vi.waitFor(() => expect(r.forgotten).toEqual([root]));
    expect(r.rows[MOG_A]).toEqual([]);
  });

  it('[AC-C15] Clear failed never deletes a thread in progress', async () => {
    const root = id(800);
    const r = rig({ [MOG_A]: [turnRow(root, root, T0, { status: 'failed', finalAnswerMd: '' })] }, [
      { turnUuid: id(801), threadId: root, inProgress: true },
    ]);
    await r.w.refresh();
    await r.w.remove([`a:${root}`], { onlyFailed: true });
    expect(r.rows[MOG_A]).toHaveLength(1);
  });

  it('a delete from another window, or with a bad key, does nothing', async () => {
    const r = rig(threadRows());
    await r.w.refresh();
    bus.dispatch({
      event: 'view:input',
      payload: {
        from: 'card-x',
        input: {
          mogId: 'conversations',
          kind: 'click',
          payload: { action: 'conversations.delete', threads: [`a:${ROOT}`] },
        },
      },
    });
    input({ action: 'conversations.delete', threads: ['a:not-a-uuid'] });
    input({ action: 'conversations.delete', threads: [] });
    await new Promise((x) => setTimeout(x, 0));
    expect(writes(r.calls)).toEqual([]);
  });

  it('Delete before any list was read reads first, then deletes', async () => {
    const r = rig(threadRows());
    await r.w.remove([`a:${ROOT}`]);
    expect(r.rows[MOG_B]).toEqual([]);
  });

  it('a failed store delete never rejects; the rows show again on the next list', async () => {
    const f = fakeStore(threadRows());
    const sent: HubConversations[] = [];
    const w = wireHubConversations(bus, {
      store: {
        exec: (s, p) =>
          (s as string) === 'mem.exchange.delete'
            ? Promise.reject(new Error('x'))
            : f.store.exec(s, p),
      },
      mogs,
      send: async (l) => void sent.push(l),
      sendOpen: async () => undefined,
    });
    await w.refresh();
    await expect(w.remove([`a:${ROOT}`])).resolves.toBeUndefined();
    expect(sent.at(-1)?.agentThreads).toHaveLength(1);
  });
});

describe('open a thread', () => {
  it('openThread sends the thread id now, and again when the hub asks, once', async () => {
    const { store } = fakeStore({});
    const opens: unknown[] = [];
    const w = wireHubConversations(bus, {
      store,
      mogs,
      send: async () => undefined,
      sendOpen: async (o) => void opens.push(o),
    });
    await w.openThread(ROOT);
    input({ action: 'conversations.ready' });
    await vi.waitFor(() => expect(opens).toHaveLength(2));
    expect(opens).toEqual([{ threadId: ROOT }, { threadId: ROOT }]);
    input({ action: 'conversations.opened' });
    input({ action: 'conversations.ready' });
    await new Promise((x) => setTimeout(x, 0));
    expect(opens).toHaveLength(2);
  });

  it('threadMog gives the MOG of a listed thread', async () => {
    const { store } = fakeStore(threadRows());
    const w = wireHubConversations(bus, {
      store,
      mogs,
      send: async () => undefined,
      sendOpen: async () => undefined,
    });
    expect(w.threadMog(ROOT)).toBeUndefined();
    await w.refresh();
    expect(w.threadMog(ROOT)).toBe(MOG_A);
  });
});
