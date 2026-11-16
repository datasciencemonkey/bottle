/**
 * T13 RecoveryManager (4.3, hidden TS brain window).
 *
 * Responsibilities: After a restart: reconciles open operations and late re-polls, discards
 * stale callbacks, offers journal recovery.
 */
import type { OperationState } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';

export interface RecoveryManager {
  reconcile(): Promise<{ opId: string; state: OperationState }[]>;
  isStale(callback: { opId: string; epoch: number }): boolean;
  offerJournalRecovery(): Promise<{ sessionIds: string[] }>;
}

const C = 'RecoveryManager';

export class RecoveryManagerStub implements RecoveryManager {
  async reconcile(): Promise<{ opId: string; state: OperationState }[]> {
    throw new NotImplementedError(C, 'reconcile', 39); // TODO(#39)
  }
  isStale(_callback: { opId: string; epoch: number }): boolean {
    throw new NotImplementedError(C, 'isStale', 39); // TODO(#39)
  }
  async offerJournalRecovery(): Promise<{ sessionIds: string[] }> {
    throw new NotImplementedError(C, 'offerJournalRecovery', 125); // TODO(#125)
  }
}

export function createRecoveryManager(): RecoveryManager {
  return new RecoveryManagerStub();
}
