/**
 * The brain client of the content-free audit log (#140, SEC-23, RTR-30, PG-15, ADR 0059).
 *
 * The call sites (ask #33, context send #152, Page call #203, agent tool #141, v1 sync) turn their
 * text into sizes and hashes with `textSizes` and `sha256Hex`, then call `record`. The client
 * checks the same closed sets as `mog_core::audit` and sends `audit_append`. A record holds IDs,
 * sizes and hashes only: it never takes a text field, and errors never echo a value.
 *
 * TODO(#442): `audit_append` has no `hashes` field yet, so the call sends kind, mogId, opId and
 * sizes only. Rust R10 stores the entry once `audit_append` is routed (#442).
 */
import type { BottleTool, MogId } from '../shared/interfaces';
import { call, type CommandArgs, type CommandReturns } from '../shared/ipc';

/** The kinds of SEC-23. An agent action also names its tool (RTR-30). */
export const AUDIT_KINDS = ['ask', 'context_send', 'page_call', 'agent_action', 'sync'] as const;
export type AuditKind = (typeof AUDIT_KINDS)[number];

/** The 7 `bottle-mcp` tools (4.7 `BottleTool`) and Genie One MCP through the proxy (AT-42). */
export const AGENT_TOOLS = [
  'transcript.search',
  'memory.recall',
  'memory.write',
  'show_blurb',
  'set_mood',
  'propose_page',
  'ask_user',
  'genie_one',
] as const satisfies readonly (BottleTool | 'genie_one')[];
export type AgentTool = (typeof AGENT_TOOLS)[number];

/** The size names a record may hold. Each value is a whole count. */
export const SIZE_KEYS = [
  'chars',
  'bytes',
  'items',
  'question_chars',
  'preamble_chars',
  'preamble_items',
  'answer_chars',
  'args_bytes',
  'result_bytes',
  'synonyms',
  'rows',
] as const;
export type SizeKey = (typeof SIZE_KEYS)[number];

/** The hash names a record may hold: SHA-256, 64 lowercase hex chars. `sent` is AT-08's hash. */
export const HASH_KEYS = ['sent', 'payload', 'args', 'result'] as const;
export type HashKey = (typeof HASH_KEYS)[number];

export interface AuditEvent {
  kind: AuditKind;
  /** Only for `agent_action`, and then required. */
  tool?: AgentTool;
  mogId: MogId;
  /** A lowercase UUID. */
  opId: string;
  sizes?: Partial<Record<SizeKey, number>>;
  hashes?: Partial<Record<HashKey, string>>;
}

/** The checked record: the wire kind (`agent_action:<tool>` for an agent action) and maps. */
export interface AuditRecord {
  kind: string;
  mogId: MogId;
  opId: string;
  sizes: Record<string, number>;
  hashes: Record<string, string>;
}

export type AuditRejectCode =
  | 'unknown_field'
  | 'unknown_kind'
  | 'bad_mog_id'
  | 'bad_op_id'
  | 'unknown_size_key'
  | 'bad_size'
  | 'unknown_hash_key'
  | 'bad_hash';

/** A refused record. The message is the code only; it never holds the refused value. */
export class AuditRejectedError extends Error {
  override readonly name = 'AuditRejectedError';
  readonly code: AuditRejectCode;

  constructor(code: AuditRejectCode) {
    super(`the audit record was refused: ${code}`);
    this.code = code;
  }
}

const FIELDS = new Set(['kind', 'tool', 'mogId', 'opId', 'sizes', 'hashes']);
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const SHA256 = /^[0-9a-f]{64}$/;

const has = <T extends string>(list: readonly T[], v: unknown): v is T =>
  typeof v === 'string' && (list as readonly string[]).includes(v);

function entriesOf(v: unknown): [string, unknown][] {
  if (v === undefined) return [];
  if (typeof v !== 'object' || v === null || Array.isArray(v)) return [['', v]];
  return Object.entries(v);
}

/** Check an event and build its record. Throws `AuditRejectedError` on any text. */
export function buildAuditRecord(e: AuditEvent): AuditRecord {
  const reject = (code: AuditRejectCode): never => {
    throw new AuditRejectedError(code);
  };
  if (Object.keys(e).some((k) => !FIELDS.has(k))) reject('unknown_field');
  if (!has(AUDIT_KINDS, e.kind)) reject('unknown_kind');
  const isAgent = e.kind === 'agent_action';
  if (isAgent ? !has(AGENT_TOOLS, e.tool) : e.tool !== undefined) reject('unknown_kind');
  if (typeof e.mogId !== 'string' || !UUID.test(e.mogId)) reject('bad_mog_id');
  // A UUID from the OperationTracker or the AgentBridge: joined text (`What_was_revenue`) fails.
  if (typeof e.opId !== 'string' || !UUID.test(e.opId)) reject('bad_op_id');
  const sizes: Record<string, number> = {};
  for (const [k, v] of entriesOf(e.sizes)) {
    if (!has(SIZE_KEYS, k)) reject('unknown_size_key');
    if (typeof v !== 'number' || !Number.isSafeInteger(v) || v < 0) reject('bad_size');
    sizes[k] = v as number;
  }
  const hashes: Record<string, string> = {};
  for (const [k, v] of entriesOf(e.hashes)) {
    // RTR-30: an agent action holds sizes only, so it takes no hash.
    if (!has(HASH_KEYS, k) || isAgent) reject('unknown_hash_key');
    if (typeof v !== 'string' || !SHA256.test(v)) reject('bad_hash');
    hashes[k] = v as string;
  }
  return {
    kind: isAgent ? `agent_action:${e.tool}` : e.kind,
    mogId: e.mogId,
    opId: e.opId,
    sizes,
    hashes,
  };
}

/** The sizes of a text: Unicode code points and UTF-8 bytes. */
export function textSizes(text: string): { chars: number; bytes: number } {
  return { chars: [...text].length, bytes: new TextEncoder().encode(text).length };
}

/** The SHA-256 of the UTF-8 text, in lowercase hex (WebCrypto). */
export async function sha256Hex(text: string): Promise<string> {
  const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return Array.from(new Uint8Array(d), (b) => b.toString(16).padStart(2, '0')).join('');
}

export interface AuditLogDeps {
  /** The `audit_append` call. Default: the IPC `call`. */
  invoke?: (
    name: 'audit_append',
    args: CommandArgs['audit_append'],
  ) => Promise<CommandReturns['audit_append']>;
}

export interface AuditLogClient {
  /** Check the event, send `audit_append` and return the record. A refused event sends nothing. */
  record(e: AuditEvent): Promise<AuditRecord>;
}

export function createAuditLog(deps: AuditLogDeps = {}): AuditLogClient {
  const send = deps.invoke ?? call;
  return {
    async record(e) {
      const r = buildAuditRecord(e);
      // TODO(#442): add `hashes: r.hashes` when the contract has the field.
      await send('audit_append', { kind: r.kind, mogId: r.mogId, opId: r.opId, sizes: r.sizes });
      return r;
    },
  };
}
