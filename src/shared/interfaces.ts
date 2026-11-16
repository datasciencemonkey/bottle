// Shared contract: the final TypeScript interfaces of section 4.7 of
// research/final-requirements.md, with unchanged names and shapes (#222).
// Only the foundation lead changes this file (CONTRIBUTING.md, "Contracts").
// The block between the BEGIN and END markers is a verbatim copy of 4.7, so this file is
// in .prettierignore. Check it with:
//   diff <(awk 'NR>408 && /^```ts$/{f=1;next} f&&/^```$/{exit} f' research/final-requirements.md) \
//        <(sed -n '/^\/\/ BEGIN 4.7/,/^\/\/ END 4.7/p' src/shared/interfaces.ts | sed '1d;$d')
// The IPC-facing types have a serde mirror in crates/mog-core/src/types.rs.

// BEGIN 4.7
// ---------- shared ----------
export type MogId = string;                 // uuid v4
export type ProfileName = string;           // Databricks CLI profile
export type Mood = 'idle'|'listening'|'processing'|'answering'|'success'|'error'|'sleeping';
export type Track = 'you'|'others';
export type Phase = 'MVP-D'|'MVP-M'|'v1'|'v2';
export interface Tags { app: 'bottle'; mog: MogId; op: 'asr'|'cleanup'|'router'|'writer'|'judge'|'embed'|'genie'|'merge'|'agent'|'page' }

// ---------- backends (D28, D62) ----------
export type BackendKind = 'genie_one_mcp' | 'genie_agent' | 'custom_agent';
export type BackendBinding =
  | { kind: 'genie_one_mcp'; profile: ProfileName; warehouseId?: string }
  | { kind: 'genie_agent'; profile: ProfileName; spaceId: string; mode: 'chat' | 'agent_mode' }      // v1 / v2
  | { kind: 'custom_agent'; profile: ProfileName; appUrl: string; protocol: 'invocations' | 'responses' }; // v1
export interface MogBinding { home: ProfileName; backend: BackendBinding; genieCompanion: boolean } // D62

export interface ContextPreamble { header: 'Context (from my notes, may be wrong):'; items: { id: string; text: string; source: 'you'|'confirmed'; ts: number; truncated: boolean }[]; chars: number }
export interface AskRequest {
  opId: string; mogId: MogId; binding: BackendBinding;
  question: string;                         // <= 1,000 chars after sanitising (D29)
  preamble?: ContextPreamble;               // <= 800 chars, <= 5 items x 160 chars
  conversationId?: string; splitGroupId?: string; tags: Tags;
}
export type BackendErrorCode =
  | 'AUTH_EXPIRED'|'NETWORK_BLOCKED'|'NO_GRANT'|'GENIE_ONE_OFF'|'SPACE_UNAVAILABLE'
  | 'CONVERSATION_BUSY'|'RATE_LIMITED'|'PROXY_PAGE'|'OFFLINE'|'ASK_UNCERTAIN'|'TOOL_BLOCKED'
  | 'POLICY_DENIED'|'GENIE_FAILED'|'GENIE_INCOMPLETE'|'TIMEOUT_GAVE_UP'|'SECRET_IN_QUESTION'
  | 'STREAM_LOST'|'AGENT_FAILED'|'CLIENT_BUG'|'UNSUPPORTED_AUTH'|'CLI_MISSING';
export type AskEvent =
  | { t: 'accepted'; conversationId: string; responseId: string }
  | { t: 'progress'; steps: string[] | null; elapsedMs: number }   // steps can be null (D56)
  | { t: 'soft_notice' }                                             // 4 min
  | { t: 'partial'; text: string }
  | { t: 'needs_input'; kind: 'clarify'|'approval'|'choose_profile'; prompt: string; options: string[] }
  | { t: 'answer'; md: string; incomplete: boolean; keyNumbers: { text: string; label?: string }[];
      deepLink: string; /* fallback https://<host>/one/chat/threads/<cid>?o=<wsid> */
      queryItems: { itemId: string; sql?: string | null }[] }        // merged from every poll
  | { t: 'failed'; code: BackendErrorCode; userText: string; retry: 'none'|'without_preamble'|'relogin'|'user' }
  | { t: 'uncertain' } | { t: 'abandoned' } | { t: 'cancel_requested' };
export interface ExchangeRef { backend: BackendKind; profile: ProfileName; host: string; conversationId: string; responseId?: string; messageId?: string; invocationId?: string }
export interface MogBackend {
  readonly kind: BackendKind;
  probe(profile: ProfileName): Promise<'ready'|'login_needed'|'genie_one_off'|'no_grant'|'network_blocked'>;
  ask(req: AskRequest, signal: AbortSignal): AsyncIterable<AskEvent>;
  resume(ref: ExchangeRef): AsyncIterable<AskEvent>;                // re-poll with same ids, never re-ask
  cancel(ref: ExchangeRef): Promise<void>;                          // shows "cancellation requested"
  getQueryResult?(ref: ExchangeRef, itemId: string, maxRows: 50): Promise<TablePreview>;
  feedback?(ref: ExchangeRef, r: 'POSITIVE'|'NEGATIVE'|'NONE', comment?: string): Promise<void>; // genie_agent only (D46)
  deleteConversation?(ref: ExchangeRef): Promise<'deleted'|'unsupported'>;                      // D36, D76
}
export interface TablePreview { columns: { name: string; type_text: string }[]; rows: unknown[][]; totalRowCount?: number; truncated: boolean }

export type OperationState = 'NOT_SENT'|'SENDING'|'ACKNOWLEDGED'|'UNCERTAIN'|'COMPLETED'|'FAILED'|'CANCEL_REQUESTED'|'ABANDONED';
export interface GenieBridge {
  backendFor(b: BackendBinding): MogBackend;
  submit(req: AskRequest): { opId: string; events: AsyncIterable<AskEvent> }; // through ContextGate + OperationTracker
  operations(mogId?: MogId): { opId: string; state: OperationState }[];
  mapError(e: { httpStatus?: number; rpcError?: unknown; isErrorText?: string }): { code: BackendErrorCode; userText: string };
  workspaceLine(profile: ProfileName): string; // "Asking Genie in <host> (profile <p>)"
  pinnedProfile(conversationId: string): ProfileName | undefined;
}

// ---------- profiles (D10, D39) ----------
export interface Profile { name: ProfileName; host: string; authType: string; supported: boolean; workspaceId?: string }
export interface ProfileService {
  list(): Promise<Profile[]>;
  status(profile: ProfileName): Promise<{ state: 'valid'|'expiring'|'login_required'|'unknown'; expiresAt?: string }>; // never a token
  login(host: string, name: ProfileName, opts?: { clientId?: string }): Promise<{ loginId: string }>;
  cliInfo(): Promise<{ path: string; version: string; plaintextStorage: boolean }>;
  home(mogId: MogId): ProfileName;
  resolveTarget(hints: string[], mogId: MogId): Promise<
    | { kind: 'one'; profile: ProfileName; scope: 'question'|'session' }
    | { kind: 'ambiguous'; candidates: ProfileName[] }
    | { kind: 'none'; suggestConnect: true }
    | { kind: 'split'; parts: { question: string; profile: ProfileName }[] }>;  // <= 3 parts, v1
}

// ---------- capture (D53, D54, D63) ----------
export type CaptureMode = 'ptt'|'hands_free'|'session';
export interface SessionPolicy { checkInAtMin: 55; softCapMin: 60; extendMin: 30; silenceNudgeMin: 5; silenceStopMin: 15; hardCapMin: number /*60..480, default 240*/; lockStopMin: 10; holdMaxSec: 120; handsFreeMaxMin: 5 }
export interface CaptureState { owner?: MogId; mode?: CaptureMode; recording: boolean; paused: boolean; sessionId?: string; startedAt?: number; directedTo?: MogId; othersTrack: 'off'|'ok'|'zero_buffers' }
export interface SegmentRef { segmentId: string; mogId: MogId; sessionId?: string; track: Track; tStartMs: number; tEndMs: number; final: boolean; directedTo?: MogId }
export interface Transcript { segmentId: string; text: string; raw: string; lang?: string; provider: 'local_parakeet'|'dbx_gemini'|'dbx_whisper'; model: string; runtime: string; latencyMs: number }

// ---------- router (D25, D30, D63; schema in Appendix D) ----------
export type Intent = 'question'|'memory_question'|'context'|'command'|'dictation'|'noise';   // dictation = UC1 (B5)
export interface RouterDecision {
  intent: Intent; cleaned_text: string; genie_question: string; target_profile_hint: string;
  confidence: number; follow_up: boolean; topic_label: string;
  part_questions: string[]; part_profile_hints: string[];          // split, <= 3, v1
  command: 'none'|'switch_workspace_once'|'switch_workspace_session'|'reset_workspace'|'new_topic'|'cancel'|'stop_recording'|'pause'|'resume'|'forget_session'|'other';
  command_arg: string; context_note: string; injection_suspected: boolean;
  directed_mog_id: string;                                          // '' = none (D63)
}

// ---------- memory (D13, D34, D35, D37, D60) ----------
export interface MemoryService {
  gate(q: { mogId: MogId; text: string; intent: Intent; target: { workspaceId: string; spaceId?: string } }, budgetMs?: 300):
    Promise<{ verdict: 'answer_from_memory'|'augment'|'pass'; hits: MemoryHit[]; tier: 'local'|'remote' /* remote = v1 */; degraded?: 'offline_fts_only'|'lakebase_timeout' }>;
  assembleContext(mogId: MogId, target: { workspaceId: string }, question: string): ContextPreamble | null;
  write(drafts: MemoryDraft[]): Promise<void>;          // local first, then outbox
  recordExchange(x: GenieExchange): Promise<void>;      // D60/U8: summary + key numbers + pointer; full answer local until Wipe
  forget(scope: { sessionId?: string; mogId?: MogId; itemIds?: string[] }): Promise<{ local: number; remotePending: number; pitrWindowS: number }>;
  mode(): 'local'|'local_synced'|'sync_paused'; syncStatus(): { pending: number; lastOk?: string };   // MVP = 'local' (B2)
}
export interface MemoryHit { itemId: string; type: 'decision'|'fact'|'entity'|'open_question'|'genie_answer'; text: string; score: number; sem?: number; workspaceId?: string; spaceId?: string; ageH: number }
export interface MemoryDraft { mogId: MogId; type: MemoryHit['type']; text: string; source: 'you'|'others'|'genie'|'agent'; sessionId?: string; workspaceHost?: string; workspaceId?: string; pinned?: boolean }
export interface GenieExchange { requestId: string; mogId: MogId; ref: ExchangeRef; questionUser: string; questionSent: string; status: OperationState; answerMd?: string; keyNumbers: unknown[]; deepLink: string; homeWorkspace: boolean; splitGroupId?: string }

// ---------- main agent (B4) ----------
export type HarnessKind = 'claude'|'codex';
export type BottleTool = 'transcript.search'|'memory.recall'|'memory.write'|'show_blurb'|'set_mood'|'propose_page'|'ask_user';
export interface AgentTurn { mogId: MogId; question: string; decision: RouterDecision; recall: MemoryHit[]; target: ProfileName }
export interface AgentBridge { start(mogId: MogId, h: HarnessKind): Promise<void>; send(t: AgentTurn): Promise<void>; onTool(call: { callId: string; tool: BottleTool; args: unknown }): Promise<unknown> } // guardrails RTR-29a..e

// ---------- UC Pages (B3) ----------
export interface PageDraft { domainId: string; displayName: string; body: string; synonyms: string[] /* <= 50 */; description: string; requestId: string; source: 'you'|'confirmed' }
export interface PageService { propose(d: PageDraft): Promise<{ reviewId: string }>; createDraft(reviewId: string): Promise<{ pageId: string; state: 'DRAFT' }> } // only after user approval

// ---------- mood (UX-MOOD-2) ----------
export interface MoodMapper { (s: { paused: boolean; ownsCapture: boolean; errorUntil?: number; successUntil?: number; answering: boolean; processing: boolean; dozing: boolean }): { mood: Mood; overlays: { attention: boolean; recording: boolean; offline: boolean; hop?: string; localOnly: boolean } } }
// END 4.7

// ---------- amendments to 4.7 (the 4.7 block above stays verbatim) ----------
/**
 * CAP-02a (MUST) wins over the 4.7 sketch: every capture has a lease (ADR 0007).
 * Declaration merging adds the field to the 4.7 `CaptureState`. Set while capture is live.
 */
export interface CaptureState {
  /** The lease that owns the mic now. A suspended lease (ARC-F9) shows again when it resumes. */
  leaseId?: LeaseId;
}
/** CAP-02a / AT-17: every segment names the lease of the capture that made it. */
export interface SegmentRef {
  leaseId: LeaseId;
}
/** A capture lease ID issued by R2 CaptureService. Opaque; never reused. */
export type LeaseId = string;

// ---------- IPC names used by 4.5 but not defined in 4.7 ----------
/** One entry of `profiles_list` (4.5). The same shape as `Profile`. */
export type ProfileInfo = Profile;
/** The result of `profile_status` (4.5). It never holds a token (ARC-6). */
export interface TokenStatus {
  profile: ProfileName;
  state: 'valid' | 'expiring' | 'login_required' | 'unknown';
  expiresAt?: string;
}

// ---------- mood priority (4.8, UX-01a): data only; the MoodMapper is #226 ----------
/**
 * The conditions that pick the mood, from high to low priority:
 * paused > capture > error (3 s) > success (2.5 s) > answering > processing > dozing > idle.
 */
export const MOOD_PRIORITY = [
  'paused',
  'capture',
  'error',
  'success',
  'answering',
  'processing',
  'dozing',
  'idle',
] as const;
