/**
 * Test doubles for memory (MEM-39a, #37). They hold everything in process memory: no IPC, no
 * Rust store, no Lakebase.
 * - `RecordingLocalStore` replaces the R10 store behind `LocalMemoryService`.
 * - `InMemoryMemoryService` replaces the whole 4.7 MemoryService in tests of its callers.
 */
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
