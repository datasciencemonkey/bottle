/**
 * T10 MemoryService (4.3, hidden TS brain window). Implements the 4.7 `MemoryService`.
 *
 * Responsibilities: ContextAssembler, MemoryWriter (with Page candidates), RecallGate (300 ms),
 * embedding queue, SyncWorker (v1), D37/D60 storage rules.
 *
 * MVP (B2, MEM-41): memory is the local R10 store only, so `mode()` is `'local'` and nothing waits
 * for sync. The service reaches the store only through `LocalMemoryStore` (MEM-39a), which sends
 * catalog ids and params, never SQL (MEM-34b). The feature issues fill in the methods below.
 * TODO(#427): the live AT-31/AT-32 run on a workspace without Lakebase (MEM-41, needs:human).
 */
import type {
  ContextPreamble,
  GenieExchange,
  MemoryDraft,
  MemoryService,
  MogId,
} from '../../shared/interfaces';
import { NotImplementedError } from '../errors';
import { IpcLocalMemoryStore, type LocalMemoryStore } from './memory-store';

type GateQuery = Parameters<MemoryService['gate']>[0];
type GateResult = Awaited<ReturnType<MemoryService['gate']>>;
type ForgetResult = Awaited<ReturnType<MemoryService['forget']>>;

const C = 'MemoryService';

export class LocalMemoryService implements MemoryService {
  /** The local store; tests pass a double (MEM-39a). */
  constructor(readonly store: LocalMemoryStore = new IpcLocalMemoryStore()) {}

  async gate(_q: GateQuery, _budgetMs?: 300): Promise<GateResult> {
    throw new NotImplementedError(C, 'gate', 200); // TODO(#200)
  }
  assembleContext(
    _mogId: MogId,
    _target: { workspaceId: string },
    _question: string,
  ): ContextPreamble | null {
    throw new NotImplementedError(C, 'assembleContext', 92); // TODO(#92)
  }
  async write(_drafts: MemoryDraft[]): Promise<void> {
    throw new NotImplementedError(C, 'write', 114); // TODO(#114)
  }
  async recordExchange(_x: GenieExchange): Promise<void> {
    throw new NotImplementedError(C, 'recordExchange', 38); // TODO(#38)
  }
  async forget(_scope: {
    sessionId?: string;
    mogId?: MogId;
    itemIds?: string[];
  }): Promise<ForgetResult> {
    throw new NotImplementedError(C, 'forget', 123); // TODO(#123)
  }
  /** B2: the MVP has no Lakebase sync. The v1 SyncWorker (#102) adds the other modes. */
  mode(): 'local' | 'local_synced' | 'sync_paused' {
    return 'local';
  }
  /** Nothing waits for a remote sync in local mode. */
  syncStatus(): { pending: number; lastOk?: string } {
    return { pending: 0 };
  }
}

export function createMemoryService(store?: LocalMemoryStore): MemoryService {
  return new LocalMemoryService(store);
}
