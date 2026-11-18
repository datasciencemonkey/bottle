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
import { createConversationManager, type ConversationManager } from './conversation-manager';
import type { PollOutcome } from './poll-actor';

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
  readonly #conversations: ConversationManager | undefined;

  constructor(conversations?: ConversationManager) {
    this.#conversations = conversations;
  }
  probe(
    _profile: ProfileName,
  ): Promise<'ready' | 'login_needed' | 'genie_one_off' | 'no_grant' | 'network_blocked'> {
    throw new NotImplementedError('GenieOneMcpBackend', 'probe', 30); // TODO(#30)
  }
  ask(_req: AskRequest, _signal: AbortSignal): AsyncIterable<AskEvent> {
    throw new NotImplementedError('GenieOneMcpBackend', 'ask', 33); // TODO(#33)
  }
  resume(ref: ExchangeRef): AsyncIterable<AskEvent> {
    return resumePolls(ref, this.#conversations ?? createConversationManager());
  }
  cancel(_ref: ExchangeRef): Promise<void> {
    throw new NotImplementedError('GenieOneMcpBackend', 'cancel', 151); // TODO(#151)
  }
}

/** `resume` cannot poll this exchange: it never asks again instead (G-13b, MOG-10b). */
export class ResumeRefusedError extends Error {
  override readonly name = 'ResumeRefusedError';
  readonly reason: 'not_genie_one' | 'no_response_id' | 'not_in_journal' | 'other_profile';

  constructor(reason: ResumeRefusedError['reason']) {
    super(`resume cannot poll this exchange (${reason}); it never asks again`);
    this.reason = reason;
  }
}

/** A poll loop ended in a Genie `failed` status or an error reply. TODO(#32): the GL-5 mapper. */
export class PollEndedError extends Error {
  override readonly name = 'PollEndedError';
  readonly outcome: PollOutcome;

  constructor(outcome: PollOutcome) {
    super(`the poll loop ended: ${outcome.kind}`);
    this.outcome = outcome;
  }
}

function progressSteps(v: unknown): string[] | null {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : null;
}

/**
 * `MogBackend.resume` for Genie One MCP (#31, ARC-F10, MOG-11a): poll the journaled ask again
 * with its original IDs, on the PollActor. It never sends `genie_ask`.
 */
async function* resumePolls(ref: ExchangeRef, cm: ConversationManager): AsyncGenerator<AskEvent> {
  if (ref.backend !== 'genie_one_mcp') throw new ResumeRefusedError('not_genie_one');
  if (!ref.responseId) throw new ResumeRefusedError('no_response_id');
  const target = cm.journal.get(ref.conversationId, ref.responseId);
  if (!target) throw new ResumeRefusedError('not_in_journal');
  // A conversation stays in its workspace: resume only on the profile that asked.
  if (target.profile !== ref.profile || target.host !== ref.host)
    throw new ResumeRefusedError('other_profile');
  const polls = cm.poll(target);
  for (;;) {
    const r = await polls.next();
    if (r.done) {
      const o = r.value;
      if (o.kind === 'terminal' && o.status !== 'failed') {
        // TODO(#33): normalise the answer (key numbers); TODO(#87): the fallback deep link and
        // the query item merge of every poll.
        yield {
          t: 'answer',
          md: typeof o.result.final_answer === 'string' ? o.result.final_answer : '',
          incomplete: o.status === 'incomplete',
          keyNumbers: [],
          deepLink: typeof o.result.deep_link === 'string' ? o.result.deep_link : '',
          queryItems: Array.isArray(o.result.query_items)
            ? (o.result.query_items as { item_id?: unknown; sql?: unknown }[])
                .filter((q) => typeof q?.item_id === 'string')
                .map((q) => ({
                  itemId: q.item_id as string,
                  sql: typeof q.sql === 'string' ? q.sql : null,
                }))
            : [],
        };
      } else if (o.kind === 'gave_up') {
        yield { t: 'abandoned' }; // TODO(#149): the late re-polls at +15 and +60 min.
      } else if (o.kind !== 'aborted') {
        throw new PollEndedError(o);
      }
      return;
    }
    if (
      r.value.status !== 'completed' &&
      r.value.status !== 'incomplete' &&
      r.value.status !== 'failed'
    )
      yield {
        t: 'progress',
        steps: progressSteps(r.value.result.progress_steps),
        elapsedMs: r.value.elapsedMs,
      };
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
