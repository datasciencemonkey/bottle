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
import { call as ipcCall } from '../../shared/ipc';
import { createAuditLog, sha256Hex, textSizes, type AuditLogClient } from '../audit-log';
import { NotImplementedError } from '../errors';
import {
  createClarifyFollowUps,
  faultsAsFailed,
  oneTerminal,
  pollEvents,
  type ClarifyFollowUps,
} from './ask-events';
import { createConversationManager, type ConversationManager } from './conversation-manager';
import {
  classifyGenieError,
  finalAskFailure,
  sendGenieAsk,
  toFailedEvent,
  classifyTransportCode,
  type Gl5Context,
} from './genie-errors';
import type { McpCaller } from './poll-actor';

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

/** The seams of the Genie One MCP backend. Tests give the #229 double and a sim clock. */
export interface GenieOneMcpDeps {
  /** One `mcp_call` to the Rust core. Default: the IPC `call`. */
  call?: McpCaller;
  /** Epoch milliseconds, on the clock of the PollActor (`PollTarget.askedAt`). */
  now?: () => number;
  /**
   * The workspace host of a profile, for the texts and the journal. TODO(#17): ProfileService
   * gives it. Without it the texts say "this workspace".
   */
  hostOf?: (profile: ProfileName) => string | undefined;
}

/**
 * The `genie_one_mcp` backend (G-02). Rust R8 sends every call to
 * `https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>`
 * (`mog_core::genie_endpoint`); the brain never builds a URL (4.6 rule 1).
 */
export class GenieOneMcpBackend implements MogBackend {
  readonly kind = 'genie_one_mcp' as const;
  readonly #conversations: ConversationManager | undefined;
  readonly #deps: GenieOneMcpDeps;

  constructor(conversations?: ConversationManager, deps: GenieOneMcpDeps = {}) {
    this.#conversations = conversations;
    this.#deps = deps;
  }
  probe(
    _profile: ProfileName,
  ): Promise<'ready' | 'login_needed' | 'genie_one_off' | 'no_grant' | 'network_blocked'> {
    throw new NotImplementedError('GenieOneMcpBackend', 'probe', 30); // TODO(#30)
  }
  /**
   * #33 (ADR 0084): send one `genie_ask` and poll it to one terminal event. The caller calls
   * this only after a user action (B1; the preview gate is #142). It never asks again (G-15b).
   */
  ask(req: AskRequest, signal: AbortSignal): AsyncIterable<AskEvent> {
    // G-21b: each backend ends each request in exactly one terminal event.
    return oneTerminal(faultsAsFailed(askEvents(req, signal, this.#cm(), this.#deps)));
  }
  resume(ref: ExchangeRef): AsyncIterable<AskEvent> {
    return resumePolls(ref, this.#cm());
  }
  cancel(_ref: ExchangeRef): Promise<void> {
    throw new NotImplementedError('GenieOneMcpBackend', 'cancel', 151); // TODO(#151)
  }
  #cm(): ConversationManager {
    return this.#conversations ?? createConversationManager();
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
  // G-21b: one terminal event (#33, ADR 0084).
  yield* oneTerminal(pollEvents(target, cm, { host: target.host, profile: target.profile }));
}

/** The Genie One ask path of `GenieOneMcpBackend.ask` (#33, ADR 0084). */
async function* askEvents(
  req: AskRequest,
  signal: AbortSignal,
  cm: ConversationManager,
  deps: GenieOneMcpDeps,
): AsyncGenerator<AskEvent> {
  const { binding } = req;
  if (binding.kind !== 'genie_one_mcp') {
    yield toFailedEvent(classifyTransportCode('not_implemented'));
    return;
  }
  const { profile } = binding;
  const host = deps.hostOf?.(profile) ?? '';
  const ctx: Gl5Context = host ? { host, profile } : { profile };
  const call = deps.call ?? ((args) => ipcCall('mcp_call', args));
  const now = deps.now ?? Date.now;
  const args: Record<string, unknown> = { question: req.question };
  if (req.conversationId) args.conversation_id = req.conversationId;
  // G-15b: one send; no failure sends it again. G-07a: no `meta` on the ask either.
  const sent = await sendGenieAsk(
    (tool, a) =>
      call({
        profile,
        server: 'genie_one',
        tool,
        args: a as McpCallArgsJson,
        opId: req.opId,
        tags: req.tags,
      }),
    args,
    ctx,
  );
  if (sent.t === 'failed') {
    yield finalAskFailure(sent);
    return;
  }
  const askedAt = now();
  yield sent;
  yield* pollEvents(
    {
      opId: req.opId,
      mogId: req.mogId,
      profile,
      host,
      conversationId: sent.conversationId,
      responseId: sent.responseId,
      askedAt,
    },
    cm,
    ctx,
    signal,
  );
}

type McpCallArgsJson = Parameters<McpCaller>[0]['args'];

/** The seams of the bridge. Tests give the #229 double, a sim clock and a fake audit log. */
export interface GenieBridgeDeps extends GenieOneMcpDeps {
  conversations?: ConversationManager;
  followUps?: ClarifyFollowUps;
  /**
   * SEC-23 (ADR 0059): the content-free `ask` entry of each accepted ask. Default: the
   * `audit_append` IPC client. TODO(#442): `audit_append` carries the hashes.
   */
  audit?: AuditLogClient;
}

export class GenieBridgeStub implements GenieBridge {
  readonly #deps: GenieBridgeDeps;
  readonly #followUps: ClarifyFollowUps;
  readonly #auditLog: AuditLogClient;

  constructor(deps: GenieBridgeDeps = {}) {
    this.#deps = deps;
    this.#followUps = deps.followUps ?? createClarifyFollowUps();
    this.#auditLog = deps.audit ?? createAuditLog();
  }

  backendFor(b: BackendBinding): MogBackend {
    if (b.kind === 'genie_one_mcp')
      return new GenieOneMcpBackend(this.#deps.conversations, this.#deps);
    throw new BackendNotAvailableError(b.kind); // v1: #156 (genie_agent), #158 (custom_agent)
  }
  /**
   * #33 (ADR 0084): one request, one terminal event (G-21b). The ask is sent when the caller
   * reads the events, and the caller calls `submit` only after a user action (B1, #142).
   * TODO(#39): the OperationTracker states. TODO(#151): cancel aborts the signal.
   */
  submit(req: AskRequest): { opId: string; events: AsyncIterable<AskEvent> } {
    return { opId: req.opId, events: oneTerminal(this.#submitEvents(req)) };
  }
  async *#submitEvents(req: AskRequest): AsyncGenerator<AskEvent> {
    const now = this.#deps.now ?? Date.now;
    // TODO(#152): the ContextGate approval of the preamble. Until then nothing with a preamble
    // goes to Genie (B1, AT-08).
    if (req.preamble) {
      yield toFailedEvent(classifyTransportCode('tool_not_allowed'));
      return;
    }
    let backend: MogBackend;
    try {
      backend = this.backendFor(req.binding);
    } catch {
      yield toFailedEvent(classifyTransportCode('not_implemented'));
      return;
    }
    const profile = req.binding.profile;
    // G-22: the answer to a clarify question goes to the same conversation in 5 min.
    const followUp = this.#followUps.take(req.mogId, profile, now());
    const ask: AskRequest =
      req.conversationId || !followUp ? req : { ...req, conversationId: followUp };
    let conversationId: string | undefined;
    try {
      for await (const e of backend.ask(ask, new AbortController().signal)) {
        if (e.t === 'accepted') {
          conversationId = e.conversationId;
          void this.#audit(ask);
        }
        if (e.t === 'needs_input' && e.kind === 'clarify' && conversationId)
          this.#followUps.note(req.mogId, { conversationId, profile }, now());
        yield e;
      }
    } catch {
      // G-21b: a fault still ends the request in one failed event, with no upstream text.
      yield toFailedEvent(classifyTransportCode('host_failed'));
    }
  }
  /** SEC-23: kind, MOG, op, sizes and the hash of the sent text. Never the text itself. */
  async #audit(req: AskRequest): Promise<void> {
    try {
      await this.#auditLog.record({
        kind: 'ask',
        mogId: req.mogId,
        opId: req.opId,
        sizes: { question_chars: textSizes(req.question).chars },
        hashes: { sent: await sha256Hex(req.question) },
      });
    } catch {
      // A failed audit write never stops the ask.
    }
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

export function createGenieBridge(deps: GenieBridgeDeps = {}): GenieBridge {
  return new GenieBridgeStub(deps);
}
