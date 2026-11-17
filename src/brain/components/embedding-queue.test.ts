import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  ItemEmbedderDouble,
  NetworkStatusDouble,
  OutboxDataDouble,
  OutboxStoreDouble,
} from '../../test-support/memory-double';
import { core } from '../test-core';
import {
  EMBED_BATCH,
  EmbeddingQueue,
  UnwiredItemEmbedder,
  UnwiredNetworkStatus,
  retryDelayMs,
} from './embedding-queue';
import { LocalMemoryService, type ItemWrite } from './memory-service';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);

const A = '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01';
const B = '6a7f0c2e-91d4-4b8e-8f1e-2c4b5d6e7f02';

function itemId(n: number): string {
  return `21111111-2222-4333-8444-${String(n).padStart(12, '0')}`;
}

function fact(mogId: string, n: number): ItemWrite {
  return {
    mogId,
    itemId: itemId(n),
    type: 'fact',
    text: `fact ${n}`,
    source: 'you',
    confidence: 0.9,
  };
}

interface Rig {
  data: OutboxDataDouble;
  store: OutboxStoreDouble;
  net: NetworkStatusDouble;
  embedder: ItemEmbedderDouble;
  queue: EmbeddingQueue;
  mem: LocalMemoryService;
  clock: { t: number };
}

/** A service and a queue on one store. `data` is the database file, so a new rig is a restart. */
function rig(data = new OutboxDataDouble(), online = false, mogs: string[] = [A]): Rig {
  const store = new OutboxStoreDouble(data);
  const net = new NetworkStatusDouble(online);
  const embedder = new ItemEmbedderDouble(data);
  const clock = { t: 10_000 };
  const now = () => clock.t;
  const queue = new EmbeddingQueue({ store, net, embedder, mogIds: () => mogs, now });
  const mem = new LocalMemoryService(store, { now, embeddingQueue: queue });
  return { data, store, net, embedder, queue, mem, clock };
}

describe('T10 embedding queue (#102)', () => {
  beforeEach(() => core.reset());

  it('[MEM-10a] storeItem writes the item and its outbox row with one mem.item.write', async () => {
    const r = rig();
    const out = await r.mem.storeItem(fact(A, 1));
    expect(out).toEqual({ itemId: itemId(1), queued: true });
    const writes = r.store.calls.filter((c) => c.stmtId.startsWith('mem.item'));
    expect(writes.map((c) => c.stmtId)).toEqual(['mem.item.write']);
    expect(writes[0].params).toMatchObject({
      mogId: A,
      itemId: itemId(1),
      itemType: 'fact',
      text: 'fact 1',
      source: 'you',
      confidence: 0.9,
      seenAt: 10_000,
      now: 10_000,
    });
    expect(r.store.jobs(A)).toHaveLength(1);
    // No IPC and no cloud call: the store is the double.
    expect(core.invoke).not.toHaveBeenCalled();
  });

  it('[MEM-10a] [MEM-01] storeItem refuses an item id that another MOG owns', async () => {
    const r = rig();
    await r.mem.storeItem(fact(B, 1));
    await expect(r.mem.storeItem(fact(A, 1))).rejects.toThrow(/another MOG/);
    expect(r.store.jobs(A)).toEqual([]);
  });

  it('[MEM-19d] [ARC-F7] offline, an item write queues its embedding and nothing runs', async () => {
    const r = rig();
    r.queue.start();
    for (let n = 0; n < 3; n++) await r.mem.storeItem(fact(A, n));
    await r.queue.idle();
    expect(r.embedder.seen).toEqual([]);
    expect(r.store.jobs(A)).toHaveLength(3);
    expect(await r.queue.drain()).toEqual({ done: 0, failed: 0, stopped: 'offline' });
  });

  it('[MEM-19d] online again, the queued requests run in FIFO order and their rows go', async () => {
    const r = rig();
    r.queue.start();
    for (let n = 0; n < 3; n++) await r.mem.storeItem(fact(A, n));
    r.net.set(true);
    await r.queue.idle();
    expect(r.embedder.seen.map((j) => j.itemId)).toEqual([itemId(0), itemId(1), itemId(2)]);
    expect(r.store.jobs(A)).toEqual([]);
    expect(r.data.items.every((i) => i.embedded)).toBe(true);
    // Each job is a pointer: the embedder reads the item; the job carries no text.
    expect(r.embedder.seen[0]).toEqual({ mogId: A, oid: 1, itemId: itemId(0), attempts: 0 });
  });

  it('[MEM-19d] online, a new item write runs its embedding at once', async () => {
    const r = rig(new OutboxDataDouble(), true);
    r.queue.start();
    await r.mem.storeItem(fact(A, 7));
    await r.queue.idle();
    expect(r.embedder.seen.map((j) => j.itemId)).toEqual([itemId(7)]);
    expect(r.store.jobs(A)).toEqual([]);
  });

  it('[MEM-08f] [AT-04] a queued request survives a restart and runs online', async () => {
    const data = new OutboxDataDouble();
    const before = rig(data);
    before.queue.start();
    await before.mem.storeItem(fact(A, 1));
    before.queue.stop();
    expect(before.embedder.seen).toEqual([]);
    // Restart: a new service, queue and embedder on the same database.
    const after = rig(data);
    expect(after.store.jobs(A)).toHaveLength(1);
    after.net.set(true);
    after.queue.start();
    await after.queue.idle();
    expect(after.embedder.seen.map((j) => j.itemId)).toEqual([itemId(1)]);
    expect(after.store.jobs(A)).toEqual([]);
  });

  it('[MEM-08f] the queue drains at start when the device is already online', async () => {
    const data = new OutboxDataDouble();
    await rig(data).mem.storeItem(fact(A, 1));
    const r = rig(data, true);
    r.queue.start();
    await r.queue.idle();
    expect(r.store.jobs(A)).toEqual([]);
  });

  it('[MEM-19d] [SP-21] the embedder goes offline mid-batch: the rest stays, nothing is lost or run twice', async () => {
    const r = rig();
    for (let n = 0; n < 250; n++) await r.mem.storeItem(fact(A, n));
    r.embedder.offlineAfter = 137;
    r.net.set(true);
    const first = await r.queue.drain();
    expect(first).toEqual({ done: 137, failed: 0, stopped: 'offline' });
    // The job the embedder refused is not counted as an attempt; it stays at the head.
    const left = r.store.jobs(A);
    expect(left).toHaveLength(113);
    expect(left[0]).toMatchObject({ refId: itemId(137), attempts: 0 });
    // The embedder is back. The drain picks up where it stopped.
    r.embedder.offlineAfter = Number.POSITIVE_INFINITY;
    expect(await r.queue.drain()).toEqual({ done: 113, failed: 0, stopped: 'idle' });
    const done = r.embedder.seen.filter((_, i) => i !== 137).map((j) => j.itemId);
    expect(new Set(done).size).toBe(250);
    expect(done).toHaveLength(250);
    expect(r.store.jobs(A)).toEqual([]);
    expect(r.data.items.filter((i) => i.embedded)).toHaveLength(250);
  });

  it('[MEM-19d] [SP-21] the network goes offline mid-batch: the drain stops before the next job', async () => {
    const r = rig();
    for (let n = 10; n < 15; n++) await r.mem.storeItem(fact(A, n));
    // The network drops while the second job runs.
    r.embedder.script = (job) => {
      if (job.itemId === itemId(11)) r.net.set(false);
      return undefined;
    };
    r.net.set(true);
    expect(await r.queue.drain()).toEqual({ done: 2, failed: 0, stopped: 'offline' });
    expect(r.store.jobs(A).map((j) => j.refId)).toEqual([itemId(12), itemId(13), itemId(14)]);
  });

  it('[MEM-19d] a failed job backs off up to 5 min and the others still run', async () => {
    const r = rig();
    for (let n = 0; n < 3; n++) await r.mem.storeItem(fact(A, n));
    r.net.set(true);
    r.embedder.script = (job) =>
      job.itemId === itemId(1) ? { status: 'failed', code: 'endpoint_429' } : undefined;
    expect(await r.queue.drain()).toEqual({ done: 2, failed: 1, stopped: 'idle' });
    const [job] = r.store.jobs(A);
    expect(job).toMatchObject({ refId: itemId(1), attempts: 1, lastErrorCode: 'endpoint_429' });
    expect(job.nextAttemptAt).toBe(10_000 + retryDelayMs(0));
    // Not due yet: the next drain does not run it.
    expect(await r.queue.drain()).toEqual({ done: 0, failed: 0, stopped: 'idle' });
    // The delay grows and stops at 5 min (the MEM-11b curve).
    expect([0, 1, 2, 3, 4, 10, 60].map(retryDelayMs)).toEqual([
      30_000, 60_000, 120_000, 240_000, 300_000, 300_000, 300_000,
    ]);
    // A code that is not a fixed code is stored as `embed_failed`, never as free text.
    r.clock.t = job.nextAttemptAt;
    r.embedder.script = () => ({ status: 'failed', code: 'Error: text "Q3" at host x' });
    await r.queue.drain();
    expect(r.store.jobs(A)[0]).toMatchObject({ attempts: 2, lastErrorCode: 'embed_failed' });
  });

  it(`[MEM-19d] the drain reads FIFO batches of ${EMBED_BATCH} embed jobs only, for each MOG`, async () => {
    const r = rig(new OutboxDataDouble(), false, [A, B]);
    await r.store.exec('mem.outbox.enqueue', {
      mogId: A,
      kind: 'sync',
      op: 'item.upsert',
      refId: itemId(99),
      now: 1,
    });
    for (let n = 0; n < 3; n++) await r.mem.storeItem(fact(B, n));
    r.net.set(true);
    await r.queue.drain();
    const dues = r.store.calls.filter((c) => c.stmtId === 'mem.outbox.due');
    expect(dues.length).toBeGreaterThan(0);
    for (const d of dues) expect(d.params).toMatchObject({ kind: 'embed', limit: EMBED_BATCH });
    expect(new Set(dues.map((d) => d.params.mogId))).toEqual(new Set([A, B]));
    expect(r.store.jobs(A, 'sync')).toHaveLength(1);
    expect(r.store.jobs(B)).toEqual([]);
  });

  it('[MEM-19d] two triggers at once run one drain at a time, and no job runs twice', async () => {
    const r = rig();
    r.queue.start();
    for (let n = 0; n < 4; n++) await r.mem.storeItem(fact(A, n));
    r.net.set(true);
    r.queue.kick();
    r.queue.kick();
    r.net.set(false);
    r.net.set(true);
    await r.queue.idle();
    expect(r.embedder.seen.map((j) => j.itemId)).toEqual([0, 1, 2, 3].map(itemId));
  });

  it('[U9] the drain deletes outbox rows only, never an item', async () => {
    const r = rig();
    for (let n = 0; n < 3; n++) await r.mem.storeItem(fact(A, n));
    r.embedder.script = (job) =>
      job.itemId === itemId(2) ? { status: 'failed', code: 'bad_input' } : undefined;
    r.net.set(true);
    await r.queue.drain();
    const ids = r.store.calls.map((c) => c.stmtId);
    expect(ids.filter((id) => id.startsWith('mem.forget') || id.startsWith('mem.wipe'))).toEqual(
      [],
    );
    expect(r.data.items).toHaveLength(3);
    const src = readFileSync(resolve(import.meta.dirname, 'embedding-queue.ts'), 'utf8');
    expect(src).not.toMatch(/mem\.(forget|wipe)\./);
  });

  it('[MEM-19d] without the real embedder and network wiring, jobs stay queued', async () => {
    const r = rig();
    const q = new EmbeddingQueue({
      store: r.store,
      net: new UnwiredNetworkStatus(),
      embedder: new UnwiredItemEmbedder(),
      mogIds: () => [A],
    });
    await r.mem.storeItem(fact(A, 1));
    expect(await q.drain()).toEqual({ done: 0, failed: 0, stopped: 'offline' });
    expect(
      await new UnwiredItemEmbedder().embed({ mogId: A, oid: 1, itemId: itemId(1), attempts: 0 }),
    ).toEqual({ status: 'unavailable' });
    expect(r.store.jobs(A)).toHaveLength(1);
  });
});
