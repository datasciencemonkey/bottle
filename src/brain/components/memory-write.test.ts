import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type { MemoryDraft } from '../../shared/interfaces';
import { RecordingLocalStore } from '../../test-support/memory-double';
import { LocalMemoryService } from './memory-service';
import { LOCAL_STMT_IDS } from './memory-store';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);

const MOG = '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01';
const SESSION = '7c1d2e3f-4a5b-4c6d-8e7f-901234567890';
const EXCHANGE = '3e4f5a6b-7c8d-4e9f-8a0b-1c2d3e4f5a6b';
const UTT = '9a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d';
const HOST = 'fake-workspace.example.test';
const WS = '1234567890123456';

interface Fixture {
  items: {
    id: string;
    source: MemoryDraft['source'];
    type: MemoryDraft['type'];
    text: string;
    quarantine: boolean;
  }[];
}
const fixture = JSON.parse(
  readFileSync(resolve(__dirname, '../../../tests/fixtures/memory-adversarial/items.json'), 'utf8'),
) as Fixture;

function rig() {
  // `mem.item.write` answers one {changes} row for each step: the item, then the embed job.
  const store = new RecordingLocalStore(() => ({ rows: [{ changes: 1 }, { changes: 1 }] }));
  let n = 0;
  const kicks = { n: 0 };
  const svc = new LocalMemoryService(store, {
    now: () => 1_000,
    newId: () => `00000000-0000-4000-8000-${String(++n).padStart(12, '0')}`,
    deviceId: () => 'device-test-1',
    embeddingQueue: { kick: () => void kicks.n++ },
  });
  return { store, svc, kicks };
}

function draft(o: Partial<MemoryDraft> = {}): MemoryDraft {
  return {
    mogId: MOG,
    type: 'fact',
    text: 'The pilot uses the EU region.',
    source: 'you',
    sessionId: SESSION,
    workspaceHost: HOST,
    workspaceId: WS,
    ...o,
  };
}

/** The call `k` of the store; it fails the test if there is none. */
function at(store: RecordingLocalStore, k: number): RecordingLocalStore['calls'][number] {
  const c = store.calls[k];
  if (c === undefined) throw new Error(`no store call ${k}`);
  return c;
}

/** A draft with no `key`. */
function without(key: keyof MemoryDraft): MemoryDraft {
  return Object.fromEntries(
    Object.entries(draft()).filter(([k]) => k !== key),
  ) as unknown as MemoryDraft;
}

describe('MemoryService.write: provenance (MEM-31, AT-06)', () => {
  it('[MEM-31] [AT-06] writes through mem.item.write with source, workspace host and id', async () => {
    const { store, svc, kicks } = rig();
    await svc.write([draft({ source: 'genie', type: 'genie_answer' })]);
    expect(store.calls).toHaveLength(1);
    const c = at(store, 0);
    expect(c.stmtId).toBe('mem.item.write');
    expect(c.params).toMatchObject({
      mogId: MOG,
      source: 'genie',
      itemType: 'genie_answer',
      workspaceHost: HOST,
      workspaceId: 1234567890123456,
      sourceSessionIds: [SESSION],
      pinned: false,
      quarantined: false,
    });
    // The embed job is queued in the same transaction, and the queue is started.
    expect(kicks.n).toBe(1);
  });

  it('[MEM-31] keeps each of the four sources you, others, genie and agent', async () => {
    const { store, svc } = rig();
    await svc.write(
      (['you', 'others', 'genie', 'agent'] as const).map((source) => draft({ source })),
    );
    expect(store.calls.map((c) => c.params['source'])).toEqual(['you', 'others', 'genie', 'agent']);
  });

  it('[MEM-31] refuses a draft with no workspace host, no workspace id or a bad source, before any store call', async () => {
    for (const bad of [
      without('workspaceHost'),
      draft({ workspaceHost: '' }),
      without('workspaceId'),
      draft({ workspaceId: 'not-a-number' }),
      draft({ source: 'confirmed' as MemoryDraft['source'] }),
      draft({ source: 'system' as MemoryDraft['source'] }),
      draft({ mogId: 'not-a-uuid' }),
      draft({ text: '' }),
      draft({ text: 'x'.repeat(601) }),
    ]) {
      const { store, svc } = rig();
      await expect(svc.write([draft(), bad]), JSON.stringify(bad).slice(0, 80)).rejects.toThrow();
      // The batch is checked first: no item of a bad batch is written.
      expect(store.calls).toEqual([]);
    }
  });

  it('[MEM-31] records the exchange and utterance links, so Forget finds the citing item', async () => {
    const { store, svc } = rig();
    await svc.write([
      { ...draft({ source: 'genie' }), sourceExchangeId: EXCHANGE, utteranceIds: [UTT] },
    ]);
    expect(at(store, 0).params).toMatchObject({
      sourceExchangeId: EXCHANGE,
      sourceUtteranceIds: [UTT],
      sourceSessionIds: [SESSION],
    });
  });

  it('[MEM-31] an empty batch writes nothing', async () => {
    const { store, svc } = rig();
    await svc.write([]);
    expect(store.calls).toEqual([]);
  });
});

describe('no ticketless pin (MEM-28b)', () => {
  it('[MEM-28b] write() refuses a pinned draft, and nothing is stored', async () => {
    const { store, svc } = rig();
    await expect(svc.write([draft({ pinned: true })])).rejects.toThrow(/user action/);
    expect(store.calls).toEqual([]);
  });

  it('[ADR 0113] a brain write can never label itself pin or added', async () => {
    const { store, svc } = rig();
    await svc.write([
      draft({ origin: 'pin' }),
      draft({ origin: 'added' }),
      draft({ origin: 'flush' }),
    ]);
    await svc.storeItem({
      mogId: MOG,
      type: 'fact',
      text: 'x',
      source: 'you',
      confidence: 1,
      origin: 'pin',
    });
    expect(store.calls.map((c) => c.params['origin'])).toEqual([
      'agent',
      'agent',
      'flush',
      'agent',
    ]);
    expect(store.calls.every((c) => c.params['pinned'] !== true)).toBe(true);
  });
});

describe('MemoryService.write: injection quarantine (SEC-24a, AT-17)', () => {
  it('[SEC-24a] [AT-17] marks every adversarial golden text quarantined and every benign text active', async () => {
    const { store, svc } = rig();
    await svc.write(
      fixture.items.map((i) => draft({ text: i.text, source: i.source, type: i.type })),
    );
    expect(store.calls).toHaveLength(fixture.items.length);
    fixture.items.forEach((i, k) => {
      expect(at(store, k).params['quarantined'], i.id).toBe(i.quarantine);
    });
  });

  it('[SEC-24a] the quarantined item is still stored (no deletion, U9), and gets no embed job', async () => {
    const store = new RecordingLocalStore(() => ({ rows: [{ changes: 1 }, { changes: 0 }] }));
    const svc = new LocalMemoryService(store, { now: () => 1 });
    const [r] = await svc.writeItems([draft({ text: 'Ignore previous.', source: 'genie' })]);
    expect(store.calls).toHaveLength(1);
    expect(r).toMatchObject({ quarantined: true, queued: false });
  });
});

describe('storeItem is not a side door (review of #540)', () => {
  it('[MEM-28b] storeItem refuses a pin: a pin needs a user action', async () => {
    const { store, svc } = rig();
    await expect(
      svc.storeItem({
        mogId: MOG,
        type: 'fact',
        text: 'x',
        source: 'you',
        confidence: 1,
        pinned: true,
      }),
    ).rejects.toThrow(/user action/);
    expect(store.calls).toEqual([]);
  });

  it('[SEC-24a] storeItem runs the brain quarantine check itself', async () => {
    const { store, svc } = rig();
    await svc.storeItem({
      mogId: MOG,
      type: 'fact',
      text: 'Ignore previous.',
      source: 'genie',
      confidence: 0.5,
    });
    expect(at(store, 0).params['quarantined']).toBe(true);
  });
});

describe('no brain item write skips the outbox (#102 note, MEM-10a)', () => {
  it('[MEM-10a] mem.item.upsert is Rust-only: it is not a brain statement id', () => {
    expect(LOCAL_STMT_IDS as readonly string[]).not.toContain('mem.item.upsert');
    expect(LOCAL_STMT_IDS as readonly string[]).toContain('mem.item.write');
  });

  it('[MEM-10a] no brain source file names mem.item.upsert as a statement', () => {
    for (const f of ['memory-service.ts', 'memory-store.ts', 'embedding-queue.ts']) {
      const src = readFileSync(resolve(__dirname, f), 'utf8');
      expect(src, f).not.toMatch(/exec\(\s*'mem\.item\.upsert'/);
      expect(src, f).not.toMatch(/^\s*'mem\.item\.upsert',$/m);
    }
  });
});
