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
  BackendKind,
  ExchangeRef,
  GenieBridge,
  MogBackend,
  MogId,
  OperationState,
  ProfileName,
} from '../../shared/interfaces';
import { NotImplementedError } from '../errors';
import { classifyGenieError } from './genie-errors';

const C = 'GenieBridge';

/** G-01 (D28): every backend kind, in the 4.7 order. The Rust config schema has the same list. */
export const BACKEND_KINDS = [
  'genie_one_mcp',
  'genie_agent',
  'custom_agent',
] as const satisfies readonly BackendKind[];

/** True only for an exact G-01 backend kind. */
export function isBackendKind(v: unknown): v is BackendKind {
  return typeof v === 'string' && (BACKEND_KINDS as readonly string[]).includes(v);
}

/** Genie One MCP is the only MVP backend (1.1, G-41b); `genie_agent` and `custom_agent` are v1. */
export class BackendNotAvailableError extends Error {
  override readonly name = 'BackendNotAvailableError';
  readonly kind: BackendKind;

  constructor(kind: BackendKind) {
    super(`The ${kind} backend is not available in this build. Genie One is the MVP backend.`);
    this.kind = kind;
  }
}

/**
 * The `genie_one_mcp` backend (G-02). Rust R8 sends every call to
 * `https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>`
 * (`mog_core::genie_endpoint`); the brain never builds a URL (4.6 rule 1).
 */
export class GenieOneMcpBackend implements MogBackend {
  readonly kind = 'genie_one_mcp' as const;
  probe(
    _profile: ProfileName,
  ): Promise<'ready' | 'login_needed' | 'genie_one_off' | 'no_grant' | 'network_blocked'> {
    throw new NotImplementedError('GenieOneMcpBackend', 'probe', 30); // TODO(#30)
  }
  ask(_req: AskRequest, _signal: AbortSignal): AsyncIterable<AskEvent> {
    throw new NotImplementedError('GenieOneMcpBackend', 'ask', 33); // TODO(#33)
  }
  resume(_ref: ExchangeRef): AsyncIterable<AskEvent> {
    throw new NotImplementedError('GenieOneMcpBackend', 'resume', 31); // TODO(#31)
  }
  cancel(_ref: ExchangeRef): Promise<void> {
    throw new NotImplementedError('GenieOneMcpBackend', 'cancel', 151); // TODO(#151)
  }
}

export class GenieBridgeStub implements GenieBridge {
  backendFor(b: BackendBinding): MogBackend {
    if (b.kind === 'genie_one_mcp') return new GenieOneMcpBackend();
    throw new BackendNotAvailableError(b.kind); // v1: #156 (genie_agent), #158 (custom_agent)
  }
  submit(_req: AskRequest): { opId: string; events: AsyncIterable<AskEvent> } {
    throw new NotImplementedError(C, 'submit', 33); // TODO(#33)
  }
  operations(_mogId?: MogId): { opId: string; state: OperationState }[] {
    throw new NotImplementedError(C, 'operations', 39); // TODO(#39)
  }
  /** G-14, G-15a: the GL-5 matrix (genie-errors.ts, ADR 0072). Never upstream text. */
  mapError(e: { httpStatus?: number; rpcError?: unknown; isErrorText?: string }): {
    code: BackendErrorCode;
    userText: string;
  } {
    const { code, userText } = classifyGenieError(e);
    return { code, userText };
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
