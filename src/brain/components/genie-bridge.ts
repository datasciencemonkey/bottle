/**
 * T8 GenieBridge + BackendRegistry (4.3, hidden TS brain window). Implements the 4.7
 * `GenieBridge`.
 *
 * Responsibilities: Tool contracts, error-to-text mapping, conversation pinning, 3 backend
 * kinds, companion binding.
 */
import type {
  AskEvent,
  AskRequest,
  BackendBinding,
  BackendErrorCode,
  GenieBridge,
  MogBackend,
  MogId,
  OperationState,
  ProfileName,
} from '../../shared/interfaces';
import { NotImplementedError } from '../errors';

const C = 'GenieBridge';

export class GenieBridgeStub implements GenieBridge {
  backendFor(_b: BackendBinding): MogBackend {
    throw new NotImplementedError(C, 'backendFor', 28); // TODO(#28)
  }
  submit(_req: AskRequest): { opId: string; events: AsyncIterable<AskEvent> } {
    throw new NotImplementedError(C, 'submit', 33); // TODO(#33)
  }
  operations(_mogId?: MogId): { opId: string; state: OperationState }[] {
    throw new NotImplementedError(C, 'operations', 39); // TODO(#39)
  }
  mapError(_e: { httpStatus?: number; rpcError?: unknown; isErrorText?: string }): {
    code: BackendErrorCode;
    userText: string;
  } {
    throw new NotImplementedError(C, 'mapError', 32); // TODO(#32)
  }
  workspaceLine(_profile: ProfileName): string {
    throw new NotImplementedError(C, 'workspaceLine', 40); // TODO(#40)
  }
  pinnedProfile(_conversationId: string): ProfileName | undefined {
    throw new NotImplementedError(C, 'pinnedProfile', 145); // TODO(#145)
  }
}

export function createGenieBridge(): GenieBridge {
  return new GenieBridgeStub();
}
