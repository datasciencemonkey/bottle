/**
 * The brain side of the hub Conversations list (#103, #40). The brain reads the newest exchanges
 * of each MOG with `mem.exchange.recent` and sends one merged list, newest first, to the hub
 * window only. A double-click open is held until the hub asks, so a late hub still selects it.
 */
import { BrainBus } from './bus';
import type { LocalMemoryStore, LocalStmtId } from './components/memory-store';
import { readConversations, wireHubConversations } from './hub-conversations-wiring';
import {
  asHubConversations,
  CONVERSATIONS_MAX,
  type HubConversations,
} from '../shared/hub-conversations';
import type { JsonObject } from '../shared/ipc';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const MOG_A = '00000000-0000-4000-8000-00000000000a';
const MOG_B = '00000000-0000-4000-8000-00000000000b';
const req = (n: number) => `41111111-2222-4333-8444-${n.toString(16).padStart(12, '0')}`;
const T0 = 1_767_225_600_000;

type Row = Record<string, unknown>;

function dbRow(n: number, askedAt: number, over: Row = {}): Row {
  return {
    requestId: req(n),
    status: 'completed',
    profileName: 'test-profile',
    workspaceHost: 'example.cloud.databricks.com',
    questionUser: `Question ${n}?`,
    keyNumbers: JSON.stringify([{ text: '4.2 %', label: 'growth' }, { text: '$1.3M' }]),
    finalAnswerMd: `**Answer** ${n}`,
    conversationId: `conv-${n}`,
    askedAt,
    completedAt: askedAt + 5_000,
    ...over,
  };
}

function fakeStore(byMog: Record<string, Row[]>, fail = false) {
  const calls: { stmtId: LocalStmtId; params: JsonObject }[] = [];
  const store: LocalMemoryStore = {
    exec: async (stmtId, params) => {
      calls.push({ stmtId, params });
      if (fail) throw { error: 'host_failed' };
      return { rows: byMog[params['mogId'] as string] ?? [] };
    },
  };
  return { store, calls };
}

const mogs = async () => [
  { mogId: MOG_A, name: 'Sales' },
  { mogId: MOG_B, name: 'Ops' },
];

let bus: BrainBus;
beforeEach(() => {
  bus = new BrainBus();
});

function input(payload: unknown, from = 'hub', mogId = 'conversations', kind = 'click'): void {
  bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
}

describe('readConversations (#103)', () => {
  it('[#103] merges the MOGs newest first, with the MOG name, status, profile and key numbers', async () => {
    const { store, calls } = fakeStore({
      [MOG_A]: [dbRow(3, T0 + 3000), dbRow(1, T0 + 1000)],
      [MOG_B]: [
        dbRow(2, T0 + 2000, {
          status: 'failed',
          finalAnswerMd: null,
          keyNumbers: '[]',
          completedAt: null,
        }),
      ],
    });
    const list = await readConversations({ store, mogs });
    expect(calls.map((c) => c.stmtId)).toEqual(['mem.exchange.recent', 'mem.exchange.recent']);
    expect(calls[0]?.params).toEqual({ mogId: MOG_A, limit: CONVERSATIONS_MAX });
    expect(list.rows.map((r) => r.requestId)).toEqual([req(3), req(2), req(1)]);
    expect(list.rows[0]).toEqual({
      requestId: req(3),
      mogId: MOG_A,
      mogName: 'Sales',
      question: 'Question 3?',
      answerMd: '**Answer** 3',
      keyNumbers: [{ text: '4.2 %', label: 'growth' }, { text: '$1.3M' }],
      askedAt: T0 + 3000,
      profile: 'test-profile',
      workspaceHost: 'example.cloud.databricks.com',
      status: 'answered',
      conversationId: 'conv-3',
    });
    expect(list.rows[1]).toMatchObject({
      mogName: 'Ops',
      status: 'failed',
      answerMd: '',
      keyNumbers: [],
      conversationId: 'conv-2',
    });
    // The payload passes the view's own shape check.
    expect(asHubConversations(list)).toEqual(list);
  });

  it('[#103] maps every unfinished status to failed and skips a pending row', async () => {
    const statuses = ['failed', 'abandoned', 'uncertain', 'cancelled', 'incomplete', 'pending'];
    const { store } = fakeStore({
      [MOG_A]: statuses.map((status, i) => dbRow(i + 1, T0 - i, { status })),
    });
    const list = await readConversations({ store, mogs });
    expect(list.rows.map((r) => r.status)).toEqual(Array(5).fill('failed'));
  });

  it('[#103] drops bad key numbers and a row with no request id; keeps at most CONVERSATIONS_MAX', async () => {
    const many = Array.from({ length: CONVERSATIONS_MAX }, (_, i) => dbRow(i + 10, T0 + i));
    const { store } = fakeStore({
      [MOG_A]: [
        dbRow(1, T0 + 999_999, { keyNumbers: 'not json' }),
        dbRow(2, T0 + 999_998, { keyNumbers: JSON.stringify([{ text: 5 }, { text: 'ok' }]) }),
        { ...dbRow(3, T0), requestId: null },
      ],
      [MOG_B]: many,
    });
    const list = await readConversations({ store, mogs });
    expect(list.rows).toHaveLength(CONVERSATIONS_MAX);
    expect(list.rows[0]).toMatchObject({ requestId: req(1), keyNumbers: [] });
    expect(list.rows[1]).toMatchObject({ requestId: req(2), keyNumbers: [{ text: 'ok' }] });
    expect(list.rows.some((r) => r.requestId === req(3))).toBe(false);
  });

  it('[#145] a row saved before #584 has no conversationId column: it reads as the empty string', async () => {
    const { store } = fakeStore({
      [MOG_A]: [dbRow(1, T0, { conversationId: null })],
    });
    const list = await readConversations({ store, mogs });
    expect(list.rows[0]).toMatchObject({ conversationId: '' });
  });

  it('[#103] a store that fails gives an empty list marked unavailable, with no error text', async () => {
    const { store } = fakeStore({}, true);
    await expect(readConversations({ store, mogs })).resolves.toEqual({
      rows: [],
      unavailable: true,
    });
    const bad = await readConversations({
      store,
      mogs: async () => Promise.reject(new Error('config')),
    });
    expect(bad).toEqual({ rows: [], unavailable: true });
  });
});

describe('wireHubConversations (#103)', () => {
  it('[#103] [ARC-5] sends the list on conversations.ready from the hub only', async () => {
    const { store } = fakeStore({ [MOG_A]: [dbRow(1, T0)] });
    const sent: HubConversations[] = [];
    wireHubConversations(bus, {
      store,
      mogs,
      send: async (l) => void sent.push(l),
      sendOpen: async () => undefined,
    });
    input({ action: 'conversations.ready' }, 'mog-x', MOG_A);
    input({ action: 'conversations.ready' }, 'hub', 'conversations', 'ready');
    input({ action: 'conversations.ready' }, 'hub', 'hub');
    input({ action: 'mogs.ready' });
    await Promise.resolve();
    expect(sent).toHaveLength(0);
    input({ action: 'conversations.ready' });
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0]?.rows.map((r) => r.requestId)).toEqual([req(1)]);
  });

  it('[#103] refresh() sends the list again after a new exchange is saved', async () => {
    const byMog: Record<string, Row[]> = { [MOG_A]: [] };
    const { store } = fakeStore(byMog);
    const sent: HubConversations[] = [];
    const w = wireHubConversations(bus, {
      store,
      mogs,
      send: async (l) => void sent.push(l),
      sendOpen: async () => undefined,
    });
    byMog[MOG_A] = [dbRow(1, T0)];
    await w.refresh();
    expect(sent.at(-1)?.rows).toHaveLength(1);
    // No hub is open: the send fails quietly.
    const quiet = wireHubConversations(bus, {
      store,
      mogs,
      send: async () => Promise.reject(new Error('no hub')),
      sendOpen: async () => undefined,
    });
    await expect(quiet.refresh()).resolves.toBeUndefined();
  });

  it('[#103] openConversations sends the selection now and again when the hub asks', async () => {
    const { store } = fakeStore({ [MOG_A]: [dbRow(1, T0)] });
    const order: string[] = [];
    const opens: unknown[] = [];
    const w = wireHubConversations(bus, {
      store,
      mogs,
      send: async () => void order.push('list'),
      sendOpen: async (o) => {
        order.push('open');
        opens.push(o);
      },
    });
    await w.openConversations(req(1));
    expect(opens).toEqual([{ requestId: req(1) }]);
    // A hub that opens late asks once: it gets the list first, then the held selection.
    input({ action: 'conversations.ready' });
    await vi.waitFor(() => expect(opens).toHaveLength(2));
    expect(order.slice(-2)).toEqual(['list', 'open']);
    // The selection is sent once only; a later ready sends the list only.
    input({ action: 'conversations.ready' });
    await vi.waitFor(() => expect(order.filter((o) => o === 'list')).toHaveLength(2));
    await Promise.resolve();
    expect(opens).toHaveLength(2);
    // With no id it opens the area with no selection.
    await w.openConversations();
    expect(opens.at(-1)).toEqual({});
  });

  it('[#103] the hub ack of an open clears the held selection, so a later show keeps the user pick', async () => {
    const { store } = fakeStore({ [MOG_A]: [dbRow(1, T0)] });
    const opens: unknown[] = [];
    const lists: unknown[] = [];
    const w = wireHubConversations(bus, {
      store,
      mogs,
      send: async (l) => void lists.push(l),
      sendOpen: async (o) => void opens.push(o),
    });
    await w.openConversations(req(1));
    input({ action: 'conversations.opened' });
    await Promise.resolve();
    input({ action: 'conversations.ready' });
    await vi.waitFor(() => expect(lists).toHaveLength(1));
    await Promise.resolve();
    expect(opens).toEqual([{ requestId: req(1) }]);
  });

  it('[#145] findMogId finds the MOG of a thread from its conversationId, across every MOG', async () => {
    const { store } = fakeStore({
      [MOG_A]: [dbRow(1, T0, { conversationId: 'conv-x' })],
      [MOG_B]: [dbRow(2, T0, { conversationId: 'conv-y' })],
    });
    const w = wireHubConversations(bus, {
      store,
      mogs,
      send: async () => undefined,
      sendOpen: async () => undefined,
    });
    await expect(w.findMogId('conv-y')).resolves.toBe(MOG_B);
    await expect(w.findMogId('conv-does-not-exist')).resolves.toBeUndefined();
  });

  it('[#145] a blank conversationId never matches (rows saved before #584 would collide)', async () => {
    const { store } = fakeStore({
      [MOG_A]: [dbRow(1, T0, { conversationId: null })],
    });
    const w = wireHubConversations(bus, {
      store,
      mogs,
      send: async () => undefined,
      sendOpen: async () => undefined,
    });
    await expect(w.findMogId('')).resolves.toBeUndefined();
  });

  it('[#103] dispose stops the hub input', async () => {
    const { store, calls } = fakeStore({});
    const w = wireHubConversations(bus, {
      store,
      mogs,
      send: async () => undefined,
      sendOpen: async () => undefined,
    });
    w.dispose();
    input({ action: 'conversations.ready' });
    await Promise.resolve();
    expect(calls).toHaveLength(0);
  });
});
