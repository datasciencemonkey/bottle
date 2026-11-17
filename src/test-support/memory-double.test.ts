import type { GenieExchange, MemoryService } from '../shared/interfaces';
import { InMemoryMemoryService, RecordingLocalStore } from './memory-double';

const A = '00000000-0000-4000-8000-00000000000a';
const B = '00000000-0000-4000-8000-00000000000b';
const ref = { backend: 'genie_one_mcp', profile: 'p', host: 'h', conversationId: 'c' } as const;

function exchange(mogId: string): GenieExchange {
  return {
    requestId: 'r1',
    mogId,
    ref,
    questionUser: 'q',
    questionSent: 'q',
    status: 'COMPLETED',
    keyNumbers: [],
    deepLink: '',
    homeWorkspace: true,
  };
}

describe('memory test doubles (MEM-39a)', () => {
  it('[MEM-39a] InMemoryMemoryService implements the 4.7 MemoryService in memory', async () => {
    const svc: MemoryService = new InMemoryMemoryService();
    expect(svc.mode()).toBe('local');
    expect(svc.syncStatus()).toEqual({ pending: 0 });
    await svc.write([
      { mogId: A, type: 'decision', text: 'Use the EU region', source: 'you', sessionId: 's1' },
      { mogId: B, type: 'fact', text: 'EU region is cheaper', source: 'you' },
    ]);
    const target = { workspaceId: 'w' };
    const g = await svc.gate({ mogId: A, text: 'which region', intent: 'question', target });
    expect(g.tier).toBe('local');
    expect(g.verdict).toBe('augment');
    expect(g.hits.map((h) => h.text)).toEqual(['Use the EU region']);
    const none = await svc.gate({ mogId: A, text: 'budget', intent: 'question', target });
    expect(none).toEqual({ verdict: 'pass', hits: [], tier: 'local' });
    expect(svc.assembleContext(A, target, 'q')).toBeNull();
    await svc.recordExchange(exchange(A));
    expect((svc as InMemoryMemoryService).exchanges).toHaveLength(1);
    const f = await svc.forget({ mogId: A, sessionId: 's1' });
    expect(f).toEqual({ local: 1, remotePending: 0, pitrWindowS: 0 });
    // MEM-01: the other MOG keeps its item.
    const gb = await svc.gate({ mogId: B, text: 'region', intent: 'question', target });
    expect(gb.hits).toHaveLength(1);
  });

  it('[MEM-39a] RecordingLocalStore records calls and answers from its handler', async () => {
    const store = new RecordingLocalStore((id) => ({ rows: [{ id }] }));
    expect(await store.exec('mem.item.get', { mogId: A })).toEqual({
      rows: [{ id: 'mem.item.get' }],
    });
    expect(store.calls).toEqual([{ stmtId: 'mem.item.get', params: { mogId: A } }]);
    expect(await new RecordingLocalStore().exec('mem.outbox.due', { mogId: A })).toEqual({
      rows: [],
    });
  });
});

describe('GenieExchange session and target ids (#449)', () => {
  it('[MEM-32a][SEC-14a] a GenieExchange carries sessionId, workspaceId and spaceId, and the double keeps them', async () => {
    const x: GenieExchange = {
      ...exchange(A),
      sessionId: '00000000-0000-4000-8000-0000000000e1',
      workspaceId: '1234567890',
      spaceId: 'space-test-not-real',
    };
    const svc = new InMemoryMemoryService();
    await svc.recordExchange(x);
    const [kept] = svc.exchanges;
    // Forget by session (MEM-32a) and "Stop and delete this session" (SEC-14a) find the exchange.
    expect(svc.exchanges.filter((e) => e.sessionId === x.sessionId)).toHaveLength(1);
    expect(kept?.workspaceId).toBe('1234567890');
    expect(kept?.spaceId).toBe('space-test-not-real');
  });

  it('[MEM-25a] the 3 fields are optional: an exchange without them still type-checks', () => {
    const x: GenieExchange = exchange(A);
    expect(x.sessionId).toBeUndefined();
    expect(x.workspaceId).toBeUndefined();
    expect(x.spaceId).toBeUndefined();
  });
});
