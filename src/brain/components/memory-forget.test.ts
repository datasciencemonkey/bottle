/**
 * #123: T10 `forget(scope)` over the R10 Forget groups (MEM-32a, MEM-01, U9, AT-16; ADR 0081).
 * The Rust tests (`local_store/forget_tests.rs`) check the rows on a real store. These tests check
 * what T10 sends. Every value here is fake.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type { JsonObject } from '../../shared/ipc';
import { RecordingLocalStore } from '../../test-support/memory-double';
import { LocalMemoryService } from './memory-service';
import type { LocalStmtId } from './memory-store';

const MOG = '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01';
const SESSION = '21111111-2222-4333-8444-555555555501';
const ITEM_1 = '51111111-2222-4333-8444-555555555501';
const ITEM_2 = '51111111-2222-4333-8444-555555555502';
const T0 = 1_767_225_600_000;

function setup(answer?: (id: LocalStmtId, p: JsonObject) => { rows: unknown[] }) {
  const store = new RecordingLocalStore(
    answer ??
      ((id) =>
        id.endsWith('_conversations')
          ? { rows: [] }
          : { rows: [{ changes: 2 }, { changes: 3 }, { changes: 1 }] }),
  );
  let n = 0;
  const svc = new LocalMemoryService(store, {
    now: () => T0,
    newId: () => `71111111-2222-4333-8444-${String(++n).padStart(12, '0')}`,
    deviceId: () => 'test-device',
  });
  return { store, svc };
}

describe('T10 forget (#123, MEM-32a)', () => {
  it('[MEM-32a] [AT-16] Forget a session is one mem.forget.session group call', async () => {
    const { store, svc } = setup();
    const r = await svc.forget({ mogId: MOG, sessionId: SESSION });
    expect(store.calls).toEqual([
      {
        stmtId: 'mem.forget.session',
        params: {
          mogId: MOG,
          sessionId: SESSION,
          forgetId: '71111111-2222-4333-8444-000000000001',
          requestedAt: T0,
          deviceId: 'test-device',
        },
      },
    ]);
    // The deleted rows; the last step is the forget_log row, which is not user content.
    expect(r).toEqual({ local: 5, remotePending: 0, pitrWindowS: 0 });
  });

  it('[MEM-32a] Forget items sends the item ids in one mem.forget.items group call', async () => {
    const { store, svc } = setup();
    await svc.forget({ mogId: MOG, itemIds: [ITEM_1, ITEM_2, ITEM_1] });
    expect(store.calls).toHaveLength(1);
    expect(store.calls[0]?.stmtId).toBe('mem.forget.items');
    expect(store.calls[0]?.params.itemIds).toEqual([ITEM_1, ITEM_2]);
  });

  it('[MEM-32a] Forget a MOG is one mem.forget.mog group call', async () => {
    const { store, svc } = setup();
    await svc.forget({ mogId: MOG });
    expect(store.calls.map((c) => c.stmtId)).toEqual(['mem.forget.mog']);
    expect(Object.keys(store.calls[0]?.params ?? {}).sort()).toEqual([
      'deviceId',
      'forgetId',
      'mogId',
      'requestedAt',
    ]);
  });

  it('[MEM-01] Forget needs a UUID mogId and one clear scope; a bad scope sends nothing', async () => {
    const { store, svc } = setup();
    await expect(svc.forget({ sessionId: SESSION })).rejects.toThrow();
    await expect(svc.forget({ mogId: 'not-a-uuid' })).rejects.toThrow();
    await expect(svc.forget({ mogId: MOG, sessionId: 'bad' })).rejects.toThrow();
    await expect(svc.forget({ mogId: MOG, itemIds: ['bad'] })).rejects.toThrow();
    await expect(svc.forget({ mogId: MOG, itemIds: [] })).rejects.toThrow();
    await expect(
      svc.forget({ mogId: MOG, sessionId: SESSION, itemIds: [ITEM_1] }),
    ).rejects.toThrow();
    expect(store.calls).toEqual([]);
  });

  it('[MEM-32a] [MEM-32c] conversations of a session or a MOG are read with fixed queries', async () => {
    const row = {
      backendKind: 'genie_one_mcp',
      profile: 'test-profile',
      host: 'example.cloud.databricks.com',
      workspaceId: 1234,
      spaceId: null,
      conversationId: 'conv-test-1',
    };
    const { store, svc } = setup((id) => (id.endsWith('_conversations') ? { rows: [row] } : { rows: [] }));
    expect(await svc.forgetConversations({ mogId: MOG, sessionId: SESSION })).toEqual([
      {
        backend: 'genie_one_mcp',
        profile: 'test-profile',
        host: 'example.cloud.databricks.com',
        workspaceId: '1234',
        conversationId: 'conv-test-1',
      },
    ]);
    await svc.forgetConversations({ mogId: MOG });
    // Forget of items deletes no conversation, so it reads nothing.
    expect(await svc.forgetConversations({ mogId: MOG, itemIds: [ITEM_1] })).toEqual([]);
    expect(store.calls).toEqual([
      { stmtId: 'mem.forget.session_conversations', params: { mogId: MOG, sessionId: SESSION } },
      { stmtId: 'mem.forget.mog_conversations', params: { mogId: MOG } },
    ]);
  });

  it('[U9] T10 calls Forget only from forget(); no timer deletes', () => {
    const src = readFileSync(resolve(import.meta.dirname, 'memory-service.ts'), 'utf8');
    expect(src).not.toMatch(/setInterval|setTimeout/);
    expect(src.match(/'mem\.forget\.(items|session|mog)'/g)?.length).toBe(3);
  });
});
