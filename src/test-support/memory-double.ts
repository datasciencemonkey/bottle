/**
 * Test doubles for memory (MEM-39a, #37). They hold everything in process memory: no IPC, no
 * Rust store, no Lakebase.
 * - `RecordingLocalStore` replaces the R10 store behind `LocalMemoryService`.
 * - `InMemoryMemoryService` replaces the whole 4.7 MemoryService in tests of its callers.
 * - `OutboxStoreDouble`, `NetworkStatusDouble` and `ItemEmbedderDouble` are the R10 outbox, the
 *   network status and the embedder behind the #102 embedding queue.
 * - `genieExchangeDouble` is a terminal Genie exchange with plainly fake values (#38). It has the
 *   shape a bridge gives `recordExchange`; it is never a live Genie result.
 */
import type {
  EmbedJob,
  EmbedOutcome,
  ItemEmbedder,
  NetworkStatus,
} from '../brain/components/embedding-queue';
import type { LocalMemoryStore, LocalStmtId } from '../brain/components/memory-store';
import type { JsonObject } from '../shared/ipc';
import type {
  ContextPreamble,
  GenieExchange,
  MemoryDraft,
  MemoryHit,
  MemoryService,
  MogId,
} from '../shared/interfaces';

/** A completed home-workspace exchange with fake ids, host, link and answer (#38). */
export function genieExchangeDouble(over: Partial<GenieExchange> = {}): GenieExchange {
  return {
    requestId: '41111111-2222-4333-8444-555555555501',
    mogId: '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01',
    ref: {
      backend: 'genie_one_mcp',
      profile: 'test-profile',
      host: 'example.cloud.databricks.com',
      conversationId: 'test-conv',
      responseId: 'test-response',
    },
    questionUser: 'What was Q3 revenue in the EU?',
    questionSent: 'Context: test preamble. Question: What was Q3 revenue in the EU?',
    status: 'COMPLETED',
    answerMd:
      '## Q3 revenue\n\n**Revenue** in the EU rose **4.2 %** to **$1.3M** in Q3. ' +
      'See [the dashboard](https://example.cloud.databricks.com/dash/test).\n\n' +
      '| region | rev |\n|---|---|\n| EU | 1.3M |',
    keyNumbers: [{ text: '4.2 %', label: 'growth' }, { text: '$1.3M' }],
    deepLink: 'https://example.cloud.databricks.com/genie/rooms/test-space/chats/test-conv?o=1234',
    homeWorkspace: true,
    ...over,
  };
}

type GateQuery = Parameters<MemoryService['gate']>[0];
type GateResult = Awaited<ReturnType<MemoryService['gate']>>;
type ForgetResult = Awaited<ReturnType<MemoryService['forget']>>;

export class RecordingLocalStore implements LocalMemoryStore {
  readonly calls: { stmtId: LocalStmtId; params: JsonObject }[] = [];

  constructor(
    private readonly answer: (
      stmtId: LocalStmtId,
      params: JsonObject,
    ) => { rows: unknown[] } = () => ({
      rows: [],
    }),
  ) {}

  async exec(stmtId: LocalStmtId, params: JsonObject): Promise<{ rows: unknown[] }> {
    this.calls.push({ stmtId, params });
    return this.answer(stmtId, params);
  }
}

interface StoredItem extends MemoryDraft {
  itemId: string;
}

function words(text: string): string[] {
  return text
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter((w) => w.length > 2);
}

export class InMemoryMemoryService implements MemoryService {
  readonly items: StoredItem[] = [];
  readonly exchanges: GenieExchange[] = [];
  private next = 0;

  async gate(q: GateQuery): Promise<GateResult> {
    const want = new Set(words(q.text));
    const hits: MemoryHit[] = this.items
      .filter((i) => i.mogId === q.mogId && words(i.text).some((w) => want.has(w)))
      .map((i) => ({ itemId: i.itemId, type: i.type, text: i.text, score: 1, ageH: 0 }));
    return { verdict: hits.length ? 'augment' : 'pass', hits, tier: 'local' };
  }

  assembleContext(
    _mogId: MogId,
    _target: { workspaceId: string },
    _q: string,
  ): ContextPreamble | null {
    return null;
  }

  async write(drafts: MemoryDraft[]): Promise<void> {
    for (const d of drafts) this.items.push({ ...d, itemId: `item-${++this.next}` });
  }

  async recordExchange(x: GenieExchange): Promise<void> {
    this.exchanges.push(x);
  }

  async forget(scope: {
    sessionId?: string;
    mogId?: MogId;
    itemIds?: string[];
  }): Promise<ForgetResult> {
    const hit = (i: StoredItem) =>
      (scope.mogId === undefined || i.mogId === scope.mogId) &&
      (scope.sessionId === undefined || i.sessionId === scope.sessionId) &&
      (scope.itemIds === undefined || scope.itemIds.includes(i.itemId));
    const before = this.items.length;
    const keep = this.items.filter((i) => !hit(i));
    this.items.splice(0, this.items.length, ...keep);
    return { local: before - keep.length, remotePending: 0, pitrWindowS: 0 };
  }

  mode(): 'local' | 'local_synced' | 'sync_paused' {
    return 'local';
  }

  syncStatus(): { pending: number; lastOk?: string } {
    return { pending: 0 };
  }
}

// ---- Outbox and embedding queue doubles (#102, MEM-08f, MEM-10a, MEM-19d) ----------------------

/** One outbox row of `OutboxStoreDouble`, with the columns of migration 0001. */
export interface OutboxRowDouble {
  oid: number;
  mogId: string;
  kind: 'embed' | 'sync';
  op: string;
  refId: string;
  payload: string;
  attempts: number;
  nextAttemptAt: number;
  lastErrorCode?: string;
}

/** One memory item row of `OutboxStoreDouble`. */
export interface ItemRowDouble {
  itemId: string;
  mogId: string;
  text: string;
  embedded: boolean;
  mentions: number;
}

/**
 * The data of `OutboxStoreDouble`. It outlives a store, so a new store on the same data is a
 * restart of the app on the same database file.
 */
export class OutboxDataDouble {
  readonly items: ItemRowDouble[] = [];
  readonly outbox: OutboxRowDouble[] = [];
  nextOid = 1;
}

const str = (v: unknown): string => (typeof v === 'string' ? v : '');
const num = (v: unknown): number => (typeof v === 'number' ? v : 0);

/**
 * A stateful R10 double for the item write and the outbox statements. It follows the SQL of the
 * Rust catalog: `mem.item.write` writes the item and its embed job together (MEM-10a); an item id
 * of another MOG changes nothing (MEM-01); a waiting job is not added again (SP-21). Every call
 * is also recorded. No SQL, no IPC.
 */
export class OutboxStoreDouble implements LocalMemoryStore {
  readonly calls: { stmtId: LocalStmtId; params: JsonObject }[] = [];

  constructor(readonly data: OutboxDataDouble = new OutboxDataDouble()) {}

  /** The rows of one MOG, oldest first. */
  jobs(mogId: string, kind: 'embed' | 'sync' = 'embed'): OutboxRowDouble[] {
    return this.data.outbox.filter((r) => r.mogId === mogId && r.kind === kind);
  }

  async exec(stmtId: LocalStmtId, params: JsonObject): Promise<{ rows: unknown[] }> {
    this.calls.push({ stmtId, params });
    const d = this.data;
    const mogId = str(params.mogId);
    const changes = (n: number) => ({ changes: n });
    switch (stmtId) {
      case 'mem.item.write': {
        const itemId = str(params.itemId);
        let it = d.items.find((i) => i.itemId === itemId);
        if (it && it.mogId !== mogId) return { rows: [changes(0), changes(0)] };
        if (it) {
          it.text = str(params.text);
          it.mentions += 1;
        } else {
          it = { itemId, mogId, text: str(params.text), embedded: false, mentions: 1 };
          d.items.push(it);
        }
        const waiting = this.jobs(mogId).some((r) => r.refId === itemId);
        if (it.embedded || waiting) return { rows: [changes(1), changes(0)] };
        const now = num(params.now);
        d.outbox.push({
          oid: d.nextOid++,
          mogId,
          kind: 'embed',
          op: 'item.embed',
          refId: itemId,
          payload: '{}',
          attempts: 0,
          nextAttemptAt: now,
        });
        return { rows: [changes(1), changes(1)] };
      }
      case 'mem.item.get': {
        const it = d.items.find((i) => i.mogId === mogId && i.itemId === str(params.itemId));
        return { rows: it ? [{ itemId: it.itemId, text: it.text }] : [] };
      }
      case 'mem.outbox.enqueue': {
        const now = num(params.now);
        d.outbox.push({
          oid: d.nextOid++,
          mogId,
          kind: str(params.kind) === 'sync' ? 'sync' : 'embed',
          op: str(params.op),
          refId: str(params.refId),
          payload: JSON.stringify(params.payload ?? {}),
          attempts: 0,
          nextAttemptAt: now,
        });
        return { rows: [changes(1)] };
      }
      case 'mem.outbox.due': {
        const kind = str(params.kind);
        if (kind === '') throw new Error('invalid_args');
        const rows = this.data.outbox
          .filter((r) => r.mogId === mogId && r.kind === kind && r.nextAttemptAt <= num(params.now))
          .sort((a, b) => a.oid - b.oid)
          .slice(0, num(params.limit))
          .map((r) => ({
            oid: r.oid,
            kind: r.kind,
            op: r.op,
            refId: r.refId,
            payload: r.payload,
            attempts: r.attempts,
            nextAttemptAt: r.nextAttemptAt,
          }));
        return { rows };
      }
      case 'mem.outbox.retry': {
        const r = d.outbox.find((x) => x.mogId === mogId && x.oid === num(params.oid));
        if (!r) return { rows: [changes(0)] };
        r.attempts += 1;
        r.nextAttemptAt = num(params.nextAttemptAt);
        if (typeof params.errorCode === 'string') r.lastErrorCode = params.errorCode;
        else delete r.lastErrorCode;
        return { rows: [changes(1)] };
      }
      case 'mem.outbox.done': {
        const i = d.outbox.findIndex((x) => x.mogId === mogId && x.oid === num(params.oid));
        if (i < 0) return { rows: [changes(0)] };
        d.outbox.splice(i, 1);
        return { rows: [changes(1)] };
      }
      default:
        return { rows: [] };
    }
  }
}

/** A network status the test sets by hand (ARC-F7). It starts offline. */
export class NetworkStatusDouble implements NetworkStatus {
  #online: boolean;
  readonly #subs = new Set<(online: boolean) => void>();

  constructor(online = false) {
    this.#online = online;
  }

  online(): boolean {
    return this.#online;
  }

  onChange(cb: (online: boolean) => void): () => void {
    this.#subs.add(cb);
    return () => this.#subs.delete(cb);
  }

  set(online: boolean): void {
    if (online === this.#online) return;
    this.#online = online;
    for (const cb of this.#subs) cb(online);
  }
}

/**
 * An embedder double. It never calls an endpoint and never makes a vector. By default each job is
 * `done` and marks its item embedded in `data`. `script` can answer a job another way, and
 * `offlineAfter` makes the embedder go offline after that many `done` jobs (the SP-21 mid-batch
 * case).
 */
export class ItemEmbedderDouble implements ItemEmbedder {
  readonly seen: EmbedJob[] = [];
  offlineAfter = Number.POSITIVE_INFINITY;
  script: (job: EmbedJob) => EmbedOutcome | undefined = () => undefined;
  #done = 0;

  constructor(private readonly data?: OutboxDataDouble) {}

  async embed(job: EmbedJob): Promise<EmbedOutcome> {
    this.seen.push(job);
    const scripted = this.script(job);
    if (scripted) return scripted;
    if (this.#done >= this.offlineAfter) return { status: 'offline' };
    this.#done += 1;
    const it = this.data?.items.find((i) => i.mogId === job.mogId && i.itemId === job.itemId);
    if (it) it.embedded = true;
    return { status: 'done' };
  }
}
