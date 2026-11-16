/**
 * T2 OperationTracker (4.3, hidden TS brain window).
 *
 * Responsibilities: NOT_SENT / SENDING / ACKNOWLEDGED / UNCERTAIN per ask. An UNCERTAIN ask is
 * never sent again automatically.
 */
import type { OperationState } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';

export interface OperationTracker {
  state(opId: string): OperationState | undefined;
  transition(opId: string, to: OperationState): void;
  /** False for an UNCERTAIN ask: it is never sent again automatically. */
  mayAutoResend(opId: string): boolean;
}

const C = 'OperationTracker';

export class OperationTrackerStub implements OperationTracker {
  state(_opId: string): OperationState | undefined {
    throw new NotImplementedError(C, 'state', 39); // TODO(#39)
  }
  transition(_opId: string, _to: OperationState): void {
    throw new NotImplementedError(C, 'transition', 39); // TODO(#39)
  }
  mayAutoResend(_opId: string): boolean {
    throw new NotImplementedError(C, 'mayAutoResend', 39); // TODO(#39)
  }
}

export function createOperationTracker(): OperationTracker {
  return new OperationTrackerStub();
}
