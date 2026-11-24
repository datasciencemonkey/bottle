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
  ExchangeRef,
  GenieExchange,
  MemoryDraft,
  MemoryOrigin,
  MemoryService,
  MogId,
  OperationState,
} from '../../shared/interfaces';
import type { JsonObject } from '../../shared/ipc';
import { NotImplementedError } from '../errors';
import { distilSummary } from './exchange-summary';
import { isBackendKind } from './genie-bridge';
import { isQuarantined } from './memory-quarantine';
import { IpcLocalMemoryStore, type LocalMemoryStore } from './memory-store';
import type { MemoryFileView } from '../../shared/hub-memory';

export interface MemoryServiceOptions {
  /** The clock in epoch ms. Tests pass a simulated clock. */
  now?: () => number;
  /** A new UUID. */
  newId?: () => string;
  /**
   * The device id for `origin_device_id`. The default is one random id for each service.
   * TODO(#195): the v1 SyncWorker needs an id that stays the same across starts.
   */
  deviceId?: () => string;
  /** The embedding queue to start after an item write (#102). Without one, jobs wait in R10. */
  embeddingQueue?: { kick(): void };
}

/**
 * One memory item for `storeItem` (#102). The columns of `mem.item.upsert`; `itemId` is new when
 * it is left out. #114 adds the provenance rules and the quarantine before it calls `storeItem`.
 */
export interface ItemWrite {
  mogId: MogId;
  itemId?: string;
  type: MemoryDraft['type'];
  /** 1-600 characters. */
  text: string;
  source: MemoryDraft['source'] | 'confirmed';
  confidence: number;
  pinned?: boolean;
  sensitivity?: 'normal' | 'restricted';
  workspaceHost?: string;
  workspaceId?: string;
  spaceId?: string;
  sourceExchangeId?: string;
  /** The sessions that the item comes from (`source_session_ids`); Forget of a session finds it. */
  sourceSessionIds?: string[];
  /** The utterances that the item comes from (`source_utterance_ids`). */
  sourceUtteranceIds?: string[];
  /** ADR 0112: why the item exists. A user origin (`pin`, `added`) needs a ticket, not this path. */
  origin?: MemoryOrigin;
  /**
   * The brain's SEC-24a result. It can only raise the quarantine: the Rust catalog also checks
   * the text itself and quarantines on either result (ADR 0089).
   */
  quarantined?: boolean;
}

/** The MOG row of `mem.mog.upsert` (#40). */
export interface MogRow {
  mogId: MogId;
  displayName: string;
  /** The backend of the MOG, a fixed code. T10 passes it on and never reads it (G-21a). */
  backendKind: string;
}

/**
 * A `MemoryDraft` with its exchange and utterance links (MEM-31 provenance).
 * TODO(#533): the contract adds `sourceExchangeId` and `utteranceIds` to `MemoryDraft`; then
 * this type is `MemoryDraft` again.
 */
export type MemoryDraftLinks = MemoryDraft & { sourceExchangeId?: string; utteranceIds?: string[] };

/** The result of one item write. */
export interface ItemWriteResult {
  itemId: string;
  /** The embed job was queued. A quarantined item gets none. */
  queued: boolean;
  quarantined: boolean;
  pinned: boolean;
}

const SOURCES: ReadonlySet<string> = new Set(['you', 'others', 'genie', 'agent']);
const ITEM_TYPES: ReadonlySet<string> = new Set([
  'decision',
  'fact',
  'entity',
  'open_question',
  'genie_answer',
]);
/** The confidence of a draft by its source (ADR 0089). A user pin is always 1.0 (MEM-28b). */
const CONFIDENCE: Record<MemoryDraft['source'], number> = {
  you: 0.9,
  genie: 0.8,
  agent: 0.8,
  others: 0.6,
};
const TEXT_MAX = 600;
const HOST_RE = /^[a-z0-9](?:[a-z0-9.-]{0,253}[a-z0-9])?$/i;
const WS_RE = /^\d{1,18}$/;

/** `OperationState` to the `genie_exchange.status` value, and whether it is terminal. */
const STATUS: Record<OperationState, [string, boolean]> = {
  NOT_SENT: ['pending', false],
  SENDING: ['pending', false],
  ACKNOWLEDGED: ['pending', false],
  UNCERTAIN: ['uncertain', false],
  COMPLETED: ['completed', true],
  FAILED: ['failed', true],
  // A cancel request can still end in an answer, so it is not terminal.
  CANCEL_REQUESTED: ['cancelled', false],
  ABANDONED: ['abandoned', true],
};

/**
 * A write of one exchange. `noSummary`: save no `answerSummary` (the row of an agent turn).
 * `citations`: the workspace (profile) that each `[n]` of an agent turn's answer names (ADR 0122).
 */
export type ExchangeWrite = GenieExchange & {
  noSummary?: true;
  citations?: { n: number; profile: string; requestId?: string }[];
  /**
   * Epoch ms: when the question was asked, if the caller knows it (an agent call: its start).
   * Conversations orders the parts of a turn by it, as the live card does. Default: now.
   */
  askedAt?: number;
};

/** The most bytes of the `citations` JSON of a turn row (`mem.exchange.upsert`). */
export const CITATIONS_MAX_BYTES = 4096;

/** The citations that fit the column: the last entries go first, so the row is never refused. */
export function fitCitations(
  c: ExchangeWrite['citations'],
): ExchangeWrite['citations'] | undefined {
  if (c === undefined) return undefined;
  const out = [...c];
  while (
    out.length > 0 &&
    new TextEncoder().encode(JSON.stringify(out)).length > CITATIONS_MAX_BYTES
  )
    out.pop();
  return out.length > 0 ? out : undefined;
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
/** The most key numbers kept for one exchange. */
const KEY_NUMBERS_MAX = 32;
/** The catalog limits of `mem.exchange.upsert` (`catalog.rs`). */
const KEY_NUMBERS_JSON_MAX = 16_384;
const ANSWER_MAX = 200_000;
const QUESTION_USER_MAX = 1000;
/** The catalog limit of `mem.mog.upsert` `displayName`. */
const DISPLAY_NAME_MAX = 80;
/** The end of a full answer that is longer than the catalog limit. */
export const ANSWER_CUT_MARK = '\n\n… (answer cut at 200,000 characters)';

/** The first `max` code points of `s`, so no surrogate pair is split. */
function codePoints(s: string, max: number): string {
  const cps = Array.from(s);
  return cps.length <= max ? s : cps.slice(0, max).join('');
}

/** The full answer, cut with a mark only if it is over the catalog limit, so the row still saves. */
function fitAnswer(md: string | undefined): string | undefined {
  if (md === undefined || Array.from(md).length <= ANSWER_MAX) return md;
  return codePoints(md, ANSWER_MAX - Array.from(ANSWER_CUT_MARK).length) + ANSWER_CUT_MARK;
}

/** The key numbers that fit the catalog limit, in order (at most 32). */
function fitKeyNumbers(k: unknown[]): unknown[] | undefined {
  const out = k.slice(0, KEY_NUMBERS_MAX);
  while (
    out.length &&
    new TextEncoder().encode(JSON.stringify(out)).length > KEY_NUMBERS_JSON_MAX
  ) {
    out.pop();
  }
  return out.length ? out : undefined;
}

async function sha256Hex(text: string): Promise<string> {
  const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return Array.from(new Uint8Array(d), (b) => b.toString(16).padStart(2, '0')).join('');
}

/** Drop `undefined` values, so the store gets only the params that have a value. */
function defined(o: Record<string, unknown>): JsonObject {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)) as JsonObject;
}

type GateQuery = Parameters<MemoryService['gate']>[0];
type GateResult = Awaited<ReturnType<MemoryService['gate']>>;
type ForgetResult = Awaited<ReturnType<MemoryService['forget']>>;

/** The 4.7 Forget scope. `mogId` is required (MEM-01); then `sessionId`, `itemIds` or neither. */
export type ForgetScope = Parameters<MemoryService['forget']>[0];

/** One Genie conversation that a Forget scope holds (MEM-32c), from the local exchange rows. */
export interface ForgetConversation {
  /**
   * The backend of the exchange, kept as data. G-21a (ADR 0084): memory code never names or
   * branches on a backend kind; the bridge owns the list (`isBackendKind`).
   */
  backend: ExchangeRef['backend'];
  /** The profile that made the conversation. A delete goes to this workspace only. */
  profile: string;
  host: string;
  /** The numeric workspace id as a decimal string (`?o=`), if known. */
  workspaceId?: string;
  /** The Genie space, if the backend has one. */
  spaceId?: string;
  conversationId: string;
}

/** A checked Forget scope: the group to run and its params. */
type CheckedScope =
  | { kind: 'items'; mogId: MogId; itemIds: string[] }
  | { kind: 'session'; mogId: MogId; sessionId: string }
  | { kind: 'mog'; mogId: MogId };

/** The most items in one Forget: the `itemIds` JSON stays under the 64 KiB catalog limit. */
const FORGET_ITEMS_MAX = 1000;

/** Check a Forget scope (MEM-01). Throws on a bad or unclear scope, before any store call. */
export function checkForgetScope(s: ForgetScope): CheckedScope {
  const mogId = s.mogId;
  if (mogId === undefined || !UUID_RE.test(mogId)) throw new Error('forget needs a UUID mogId');
  if (s.sessionId !== undefined && s.itemIds !== undefined) {
    throw new Error('forget takes a session or items, not both');
  }
  if (s.sessionId !== undefined) {
    if (!UUID_RE.test(s.sessionId)) throw new Error('forget needs a UUID sessionId');
    return { kind: 'session', mogId, sessionId: s.sessionId };
  }
  if (s.itemIds !== undefined) {
    const itemIds = [...new Set(s.itemIds)];
    if (itemIds.length === 0 || itemIds.length > FORGET_ITEMS_MAX) {
      throw new Error('forget needs 1 to 1000 item ids');
    }
    if (!itemIds.every((id) => UUID_RE.test(id))) throw new Error('forget needs UUID item ids');
    return { kind: 'items', mogId, itemIds };
  }
  return { kind: 'mog', mogId };
}

const str = (v: unknown): string | undefined => (typeof v === 'string' && v !== '' ? v : undefined);

/** One `mem.forget.*_conversations` row as a {@link ForgetConversation}, or undefined. */
export function conversationOf(r: unknown): ForgetConversation | undefined {
  if (typeof r !== 'object' || r === null) return undefined;
  const o = r as Record<string, unknown>;
  const backend = o['backendKind'];
  const profile = str(o['profile']);
  const host = str(o['host']);
  const conversationId = str(o['conversationId']);
  if (!isBackendKind(backend)) return undefined;
  if (profile === undefined || host === undefined || conversationId === undefined) return undefined;
  const c: ForgetConversation = {
    backend,
    profile,
    host,
    conversationId,
  };
  const ws = o['workspaceId'];
  if (typeof ws === 'number' && Number.isSafeInteger(ws) && ws >= 0) c.workspaceId = String(ws);
  const space = str(o['spaceId']);
  if (space !== undefined) c.spaceId = space;
  return c;
}

const C = 'MemoryService';

/** MEM-31: check one draft before any store call. Throws with a fixed message, never the text. */
function checkDraft(d: MemoryDraftLinks): void {
  if (!UUID_RE.test(d.mogId)) throw new Error('write needs a UUID mogId');
  if (!SOURCES.has(d.source)) throw new Error('write needs a source: you, others, genie or agent');
  if (!ITEM_TYPES.has(d.type)) throw new Error('write needs a memory item type');
  const n = typeof d.text === 'string' ? Array.from(d.text).length : 0;
  if (n < 1 || n > TEXT_MAX || d.text.trim() === '')
    throw new Error('write needs 1-600 characters');
  if (d.workspaceHost === undefined || !HOST_RE.test(d.workspaceHost)) {
    throw new Error('write needs the workspace host');
  }
  if (d.workspaceId === undefined || !WS_RE.test(d.workspaceId)) {
    throw new Error('write needs the decimal workspace id');
  }
  if (d.sessionId !== undefined && !UUID_RE.test(d.sessionId)) {
    throw new Error('write needs a UUID sessionId');
  }
  if (d.sourceExchangeId !== undefined && !UUID_RE.test(d.sourceExchangeId)) {
    throw new Error('write needs a UUID sourceExchangeId');
  }
  if (d.utteranceIds !== undefined && !d.utteranceIds.every((u) => UUID_RE.test(u))) {
    throw new Error('write needs UUID utterance ids');
  }
}

/** The result code of a user memory write (ADR 0113). It never holds text. */
export type MemoryWriteCode = 'secret_refused' | 'invalid' | 'failed';

/** A user memory write that failed. The message is the code only: no title, body or path. */
export class MemoryWriteError extends Error {
  constructor(readonly code: MemoryWriteCode) {
    super(code);
    this.name = 'MemoryWriteError';
  }
}

/** The code of a refused store call: `secret_refused` and `invalid_args` are named, the rest fails. */
export function writeCodeOf(e: unknown): MemoryWriteCode {
  if (e instanceof MemoryWriteError) return e.code;
  const code = typeof e === 'object' && e !== null ? (e as { error?: unknown }).error : undefined;
  if (code === 'secret_refused') return 'secret_refused';
  return code === 'invalid_args' ? 'invalid' : 'failed';
}

/** One pinned item the user made: a card "remember", a voice "remember", or a hub add. */
export interface PinWrite {
  mogId: MogId;
  itemType: 'fact' | 'definition' | 'decision' | 'open_question' | 'genie_answer';
  origin: 'pin' | 'added';
  /** A voice pin sends `text` only. A hub add sends `title` and `body` and no `text`. */
  title?: string;
  text?: string;
  body?: string;
  workspaceHost?: string;
  workspaceId?: string | number;
  spaceId?: string;
  profile?: string;
  sourceExchangeId?: string;
}

/** The fields of one item that the editor needs (`mem.item.get`). */
export interface MemoryItemRead {
  kind: string;
  title: string;
  body: string;
  status: string;
  pinned: boolean;
  releasedAt?: number;
}

const changesOf = (rows: unknown[]): number[] =>
  rows.map((r) => (r as { changes?: number }).changes ?? 0);

export class LocalMemoryService implements MemoryService {
  readonly #now: () => number;
  readonly #newId: () => string;
  readonly #deviceId: () => string;
  readonly #queue: { kick(): void } | undefined;
  #lastHlc = 0;

  /** The local store; tests pass a double (MEM-39a). */
  constructor(
    readonly store: LocalMemoryStore = new IpcLocalMemoryStore(),
    o: MemoryServiceOptions = {},
  ) {
    this.#now = o.now ?? (() => Date.now());
    this.#newId = o.newId ?? (() => crypto.randomUUID());
    const device = crypto.randomUUID();
    this.#deviceId = o.deviceId ?? (() => device);
    this.#queue = o.embeddingQueue;
  }

  /** A row HLC that never goes back: the clock, or one more than the last value. */
  #hlc(now: number): number {
    this.#lastHlc = Math.max(now, this.#lastHlc + 1);
    return this.#lastHlc;
  }

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
  /**
   * The MemoryWriter write (#114). Each draft goes through {@link storeItem}, so its row and its
   * embed job commit in one transaction (MEM-10a); no brain item write skips the outbox.
   * - MEM-31: each draft must have its source (`you`, `others`, `genie`, `agent`), the workspace
   *   host and the decimal workspace id; the session, exchange and utterance links are kept.
   * - SEC-24a: text that looks like an instruction to the assistant is quarantined.
   * - MEM-28b: a draft with `pinned: true` is refused; only a ticketed {@link pinItem} pins.
   * The whole batch is checked before the first write, so a bad draft writes nothing.
   */
  async write(drafts: MemoryDraftLinks[]): Promise<void> {
    await this.writeItems(drafts);
  }
  /** {@link write} with one result for each draft: the item id, the embed job and the status. */
  async writeItems(drafts: MemoryDraftLinks[]): Promise<ItemWriteResult[]> {
    for (const d of drafts) {
      if (d.pinned === true) throw new Error('write: a pin needs a user action (pinItem)');
      checkDraft(d);
    }
    const out: ItemWriteResult[] = [];
    for (const d of drafts) out.push(await this.#writeDraft(d));
    return out;
  }
  async #writeDraft(d: MemoryDraftLinks): Promise<ItemWriteResult> {
    const quarantined = isQuarantined(d.text);
    const w: ItemWrite = {
      mogId: d.mogId,
      type: d.type,
      text: d.text,
      source: d.source,
      confidence: CONFIDENCE[d.source],
      pinned: false,
      quarantined,
      ...(d.origin !== undefined ? { origin: d.origin } : {}),
    };
    // checkDraft made sure that both workspace fields are there (MEM-31).
    if (d.workspaceHost !== undefined) w.workspaceHost = d.workspaceHost;
    if (d.workspaceId !== undefined) w.workspaceId = d.workspaceId;
    if (d.sourceExchangeId !== undefined) w.sourceExchangeId = d.sourceExchangeId;
    if (d.sessionId !== undefined) w.sourceSessionIds = [d.sessionId];
    if (d.utteranceIds !== undefined) w.sourceUtteranceIds = d.utteranceIds;
    const r = await this.#storeItem(w);
    return { ...r, quarantined, pinned: false };
  }
  /**
   * Write one memory item and queue its embedding with `mem.item.write` (#102, MEM-10a,
   * MEM-19d). R10 commits the item row and its outbox row in one transaction, so a crash loses
   * neither one alone. Offline or online, the embedding waits in the outbox; the queue runs it
   * when the device is online. A second write of the same item adds no second job.
   * It refuses an item id that another MOG owns (MEM-01): R10 changes no row then.
   */
  async storeItem(w: ItemWrite): Promise<{ itemId: string; queued: boolean }> {
    // MEM-28b: a pin needs a user action, so only the ticketed pinItem() pins (review of #540).
    if (w.pinned === true) throw new Error('storeItem: a pin needs a user action (pinItem)');
    return this.#storeItem(w);
  }
  /** The item write itself. SEC-24a: the brain check runs here, so no caller can skip it. */
  async #storeItem(w: ItemWrite): Promise<{ itemId: string; queued: boolean }> {
    if (!UUID_RE.test(w.mogId)) throw new Error('storeItem needs a UUID mogId');
    const itemId = w.itemId ?? this.#newId();
    const now = this.#now();
    const workspaceId =
      w.workspaceId !== undefined && /^\d{1,18}$/.test(w.workspaceId)
        ? Number(w.workspaceId)
        : undefined;
    const { rows } = await this.store.exec(
      'mem.item.write',
      defined({
        mogId: w.mogId,
        itemId,
        itemType: w.type,
        text: w.text,
        source: w.source,
        confidence: w.confidence,
        pinned: w.pinned,
        sensitivity: w.sensitivity,
        workspaceHost: w.workspaceHost,
        workspaceId,
        spaceId: w.spaceId,
        sourceExchangeId: w.sourceExchangeId,
        sourceSessionIds: w.sourceSessionIds,
        sourceUtteranceIds: w.sourceUtteranceIds,
        // A user origin needs a ticket (pinItem): no brain write can label itself `pin` or `added`.
        origin:
          w.origin === undefined || w.origin === 'pin' || w.origin === 'added' ? 'agent' : w.origin,
        quarantined: w.quarantined === true || isQuarantined(w.text),
        seenAt: now,
        rowHlc: this.#hlc(now),
        deviceId: this.#deviceId(),
        now,
      }),
    );
    const changes = rows.map((r) => (r as { changes?: number }).changes ?? 0);
    if (changes[0] !== 1) throw new Error('storeItem: the item id belongs to another MOG');
    this.#queue?.kick();
    return { itemId, queued: changes[1] === 1 };
  }
  /**
   * Register the MOG row in R10 with `mem.mog.upsert` (#40, ADR 0061): every other table links
   * to it, so it goes before the first exchange of a MOG. The upsert is idempotent; a later call
   * updates the name. The name is cut to the 80-character catalog limit; an empty name is
   * refused (the table CHECK).
   */
  async registerMog(m: MogRow): Promise<void> {
    if (!UUID_RE.test(m.mogId)) throw new Error('registerMog needs a UUID mogId');
    // `mog.display_name` has CHECK (length BETWEEN 1 AND 80).
    if (m.displayName.length === 0) throw new Error('registerMog needs a name');
    await this.store.exec('mem.mog.upsert', {
      mogId: m.mogId,
      displayName: codePoints(m.displayName, DISPLAY_NAME_MAX),
      backendKind: m.backendKind,
      rowHlc: this.#hlc(this.#now()),
      deviceId: this.#deviceId(),
    });
  }
  /**
   * Store one Genie exchange in R10 with `mem.exchange.upsert` (#38, MEM-25a, MEM-26, D60, U8).
   * - Home workspace: the distilled summary, the key numbers verbatim, the pointer (deep link and
   *   conversation ids) and the full `final_answer_md`.
   * - Other workspaces: the pointer and the full answer only (D60 and the table CHECK).
   * - The sent question is stored only as its SHA-256; it can hold a context preamble.
   * - Nothing sets an expiry, and T10 deletes no exchange: the answer stays until Forget or
   *   Settings → Wipe (U9). The upsert keeps a stored answer when a later event has none.
   * - `asked_at` is the clock at the first call for a request; Wipe ages the row by it. So the
   *   caller records the exchange when the question is sent, then again at the terminal event.
   * - A value over a catalog limit is fitted, so the row and its pointer always save: an answer
   *   over 200,000 characters ends with `ANSWER_CUT_MARK`, and key numbers stop at 16 KiB.
   * The caller registers the MOG row first (`mem.mog.upsert`); the table links to it.
   */
  async recordExchange(x: ExchangeWrite): Promise<void> {
    if (!UUID_RE.test(x.mogId) || !UUID_RE.test(x.requestId)) {
      throw new Error('recordExchange needs a UUID mogId and requestId');
    }
    const now = this.#now();
    const [status, terminal] = STATUS[x.status];
    const home = x.homeWorkspace;
    const workspaceId =
      x.workspaceId !== undefined && /^\d{1,18}$/.test(x.workspaceId)
        ? Number(x.workspaceId)
        : undefined;
    await this.store.exec(
      'mem.exchange.upsert',
      defined({
        exchangeId: this.#newId(),
        requestId: x.requestId,
        mogId: x.mogId,
        sessionId: x.sessionId,
        backendKind: x.ref.backend,
        profileName: x.ref.profile,
        workspaceHost: x.ref.host,
        workspaceId,
        homeWorkspace: home,
        spaceId: x.spaceId,
        conversationId: x.ref.conversationId || undefined,
        responseId: x.ref.responseId,
        questionUser: home ? codePoints(x.questionUser, QUESTION_USER_MAX) : undefined,
        questionSentSha256: await sha256Hex(x.questionSent),
        status,
        // An agent turn row holds the combined answer in `finalAnswerMd` only (`noSummary`, ADR 0121).
        answerSummary:
          home && x.noSummary !== true ? (distilSummary(x.answerMd) ?? undefined) : undefined,
        keyNumbers: home ? fitKeyNumbers(x.keyNumbers) : undefined,
        finalAnswerMd: fitAnswer(x.answerMd),
        deepLink: x.deepLink || undefined,
        splitGroupId: x.splitGroupId,
        askedAt:
          x.askedAt !== undefined && Number.isInteger(x.askedAt) && x.askedAt > 0 ? x.askedAt : now,
        completedAt: terminal ? now : undefined,
        rowHlc: this.#hlc(now),
        deviceId: this.#deviceId(),
        citations: fitCitations(x.citations),
      }),
    );
  }
  /**
   * The Genie conversations of a Forget scope (MEM-32c). Call it BEFORE {@link forget}: the
   * delete removes the exchange rows that hold the conversation ids. Forget of items deletes no
   * exchange, so it has no conversation.
   */
  async forgetConversations(scope: ForgetScope): Promise<ForgetConversation[]> {
    const s = checkForgetScope(scope);
    if (s.kind === 'items') return [];
    const { rows } =
      s.kind === 'session'
        ? await this.store.exec('mem.forget.session_conversations', {
            mogId: s.mogId,
            sessionId: s.sessionId,
          })
        : await this.store.exec('mem.forget.mog_conversations', { mogId: s.mogId });
    return rows.flatMap((r) => conversationOf(r) ?? []);
  }
  /**
   * Forget on a user action (MEM-32a, U9, ADR 0081). One R10 group deletes the scope in one
   * transaction: the items that cite it first, their embed jobs, vectors and FTS rows, then the
   * exchanges, utterances and session (or every row of the MOG). The last step writes the
   * `forget_log` row for the v1 sync. `local` counts the deleted rows.
   * MVP (B2): there is no Lakebase, so nothing is pending remotely and there is no PITR window.
   * TODO(#195): the v1 SyncWorker queues the remote deletes first (MEM-32b) and shows the PITR
   * window (MEM-32e).
   */
  async forget(scope: ForgetScope): Promise<ForgetResult> {
    const s = checkForgetScope(scope);
    const log = {
      forgetId: this.#newId(),
      requestedAt: this.#now(),
      deviceId: this.#deviceId(),
    };
    const { rows } =
      s.kind === 'items'
        ? await this.store.exec('mem.forget.items', { mogId: s.mogId, itemIds: s.itemIds, ...log })
        : s.kind === 'session'
          ? await this.store.exec('mem.forget.session', {
              mogId: s.mogId,
              sessionId: s.sessionId,
              ...log,
            })
          : await this.store.exec('mem.forget.mog', { mogId: s.mogId, ...log });
    const changes = rows.map((r) => (r as { changes?: number }).changes ?? 0);
    // The last step is the forget_log row: it records the Forget, it is not user content.
    const local = changes.slice(0, -1).reduce((a, b) => a + b, 0);
    return { local, remotePending: 0, pitrWindowS: 0 };
  }
  /**
   * One pinned item of the user (ADR 0113). The ticket comes from the host, never from here; the
   * core binds it to the click. No `quarantined`, `secretCheck` or `slug` goes in: the core
   * computes those. Throws {@link MemoryWriteError}; the message holds a code only.
   */
  async pinItem(
    w: PinWrite,
    ticket: string,
  ): Promise<{ itemId: string; status?: string | undefined }> {
    const itemId = this.#newId();
    const now = this.#now();
    const workspaceId =
      w.workspaceId === undefined
        ? undefined
        : typeof w.workspaceId === 'number'
          ? w.workspaceId
          : /^\d{1,18}$/.test(w.workspaceId)
            ? Number(w.workspaceId)
            : undefined;
    try {
      const { rows } = await this.store.exec(
        'mem.item.write',
        defined({
          mogId: w.mogId,
          itemId,
          itemType: w.itemType,
          source: 'you',
          confidence: 1,
          pinned: true,
          origin: w.origin,
          title: w.title,
          text: w.text,
          body: w.body,
          workspaceHost: w.workspaceHost,
          workspaceId,
          spaceId: w.spaceId,
          profile: w.profile,
          sourceExchangeId: w.sourceExchangeId,
          seenAt: now,
          rowHlc: this.#hlc(now),
          deviceId: this.#deviceId(),
          now,
        }),
        ticket,
      );
      if (changesOf(rows)[0] !== 1) throw new MemoryWriteError('invalid');
    } catch (e) {
      throw new MemoryWriteError(writeCodeOf(e));
    }
    this.#queue?.kick();
    return { itemId, status: (await this.#statusOf(w.mogId, itemId)) ?? undefined };
  }
  /** Edit one item (`mem.item.edit`, ADR 0113). Returns the new status. */
  async editItem(
    a: { mogId: MogId; itemId: string; itemType: string; title: string; body: string },
    ticket: string,
  ): Promise<{ status?: string | undefined }> {
    const now = this.#now();
    await this.#ticketed(
      'mem.item.edit',
      {
        mogId: a.mogId,
        itemId: a.itemId,
        itemType: a.itemType,
        title: a.title,
        body: a.body,
        now,
        rowHlc: this.#hlc(now),
      },
      ticket,
    );
    this.#queue?.kick();
    return { status: (await this.#statusOf(a.mogId, a.itemId)) ?? undefined };
  }
  /** A thumbs up (`mem.item.rate`, ADR 0113). Returns the new status. */
  async rateItem(
    a: { mogId: MogId; itemId: string },
    ticket: string,
  ): Promise<{ status?: string | undefined }> {
    const now = this.#now();
    await this.#ticketed(
      'mem.item.rate',
      { mogId: a.mogId, itemId: a.itemId, now, rowHlc: this.#hlc(now) },
      ticket,
    );
    this.#queue?.kick();
    return { status: (await this.#statusOf(a.mogId, a.itemId)) ?? undefined };
  }
  async #ticketed(stmt: 'mem.item.edit' | 'mem.item.rate', params: JsonObject, ticket: string) {
    try {
      const { rows } = await this.store.exec(stmt, params, ticket);
      // Step 1 changed no row: the item is gone, or the rule refused it.
      if (changesOf(rows)[0] !== 1) throw new MemoryWriteError('invalid');
    } catch (e) {
      throw new MemoryWriteError(writeCodeOf(e));
    }
  }
  /** `active` or `quarantined` after a write, else undefined. */
  async #statusOf(mogId: MogId, itemId: string): Promise<'active' | 'quarantined' | undefined> {
    try {
      const status = (await this.readItem(mogId, itemId))?.status;
      return status === 'active' || status === 'quarantined' ? status : undefined;
    } catch {
      return undefined;
    }
  }
  /** One item for the editor (`mem.item.get`), or undefined if there is none. */
  async readItem(mogId: MogId, itemId: string): Promise<MemoryItemRead | undefined> {
    const { rows } = await this.store.exec('mem.item.get', { mogId, itemId });
    const r = rows[0] as Record<string, unknown> | undefined;
    if (!r) return undefined;
    const str = (v: unknown) => (typeof v === 'string' ? v : '');
    return {
      kind: str(r['type']),
      title: str(r['title']),
      body: str(r['bodyMd']),
      status: str(r['status']),
      pinned: r['pinned'] === 1 || r['pinned'] === true,
      ...(typeof r['releasedAt'] === 'number' ? { releasedAt: r['releasedAt'] } : {}),
    };
  }
  /**
   * The rows of one MOG for the hub list (`mem.item.list`), and the folder name that Rust made
   * for it. The brain never computes a folder name.
   */
  async listItems(mogId: MogId): Promise<{ rows: unknown[]; folder?: string }> {
    const r = await this.store.exec('mem.item.list', { mogId });
    return { rows: r.rows, ...(typeof r.folder === 'string' ? { folder: r.folder } : {}) };
  }
  /**
   * Register the MOG row before a list or a pin. A MOG gets its store row with its first Genie
   * exchange, so before that `mem.item.list` and `mem.item.write` refuse it. A Forget of the MOG
   * or a Wipe deletes the row again, so there is no cache: the upsert is safe to repeat, and it
   * is the same call that the exchange history makes.
   */
  async ensureMog(m: { mogId: MogId; name: string; backendKind: string }): Promise<void> {
    await this.registerMog({
      mogId: m.mogId,
      displayName: m.name.trim() === '' ? 'MOG' : m.name,
      backendKind: m.backendKind,
    });
  }
  /** One rendered file or item section (`mem.file.get`). */
  async readFile(mogId: MogId, path: string): Promise<Pick<MemoryFileView, 'text'> | undefined> {
    const { rows } = await this.store.exec('mem.file.get', { mogId, path });
    const text = (rows[0] as { text?: unknown } | undefined)?.text;
    return typeof text === 'string' ? { text } : undefined;
  }
  /** B2: the MVP has no Lakebase sync. The v1 SyncWorker (#195) adds the other modes. */
  mode(): 'local' | 'local_synced' | 'sync_paused' {
    return 'local';
  }
  /** Nothing waits for a remote sync in local mode. */
  syncStatus(): { pending: number; lastOk?: string } {
    return { pending: 0 };
  }
}

/**
 * T10 for `startBrain`. It has no embedding queue yet, so item writes queue their jobs in R10 and
 * nothing drains them.
 * TODO(#198): `startBrain` builds one `EmbeddingQueue` with the real embedder, a `NetworkStatus`
 * on the typed `net.status` payload (#466) and the MOG ids of T1, passes it here as
 * `embeddingQueue`, and calls `start()`.
 */
export function createMemoryService(
  store?: LocalMemoryStore,
  o?: MemoryServiceOptions,
): MemoryService {
  return new LocalMemoryService(store, o);
}
