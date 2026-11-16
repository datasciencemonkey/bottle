/**
 * T10 MemoryService (4.3, hidden TS brain window). Implements the 4.7 `MemoryService`.
 *
 * Responsibilities: ContextAssembler, MemoryWriter (with Page candidates), RecallGate (300 ms),
 * embedding queue, SyncWorker (v1), D37/D60 storage rules.
 */
import type {
  ContextPreamble,
  GenieExchange,
  MemoryDraft,
  MemoryService,
  MogId,
} from '../../shared/interfaces';
import { NotImplementedError } from '../errors';

type GateQuery = Parameters<MemoryService['gate']>[0];
type GateResult = Awaited<ReturnType<MemoryService['gate']>>;
type ForgetResult = Awaited<ReturnType<MemoryService['forget']>>;

const C = 'MemoryService';

export class MemoryServiceStub implements MemoryService {
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
  mode(): 'local' | 'local_synced' | 'sync_paused' {
    throw new NotImplementedError(C, 'mode', 102); // TODO(#102)
  }
  syncStatus(): { pending: number; lastOk?: string } {
    throw new NotImplementedError(C, 'syncStatus', 102); // TODO(#102)
  }
}

export function createMemoryService(): MemoryService {
  return new MemoryServiceStub();
}
