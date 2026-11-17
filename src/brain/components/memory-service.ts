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
  GenieExchange,
  MemoryDraft,
  MemoryService,
  MogId,
  OperationState,
} from '../../shared/interfaces';
import type { JsonObject } from '../../shared/ipc';
import { NotImplementedError } from '../errors';
import { distilSummary } from './exchange-summary';
import { IpcLocalMemoryStore, type LocalMemoryStore } from './memory-store';

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
}

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

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
/** The most key numbers kept for one exchange. */
const KEY_NUMBERS_MAX = 32;
/** The catalog limits of `mem.exchange.upsert` (`catalog.rs`). */
const KEY_NUMBERS_JSON_MAX = 16_384;
const ANSWER_MAX = 200_000;
const QUESTION_USER_MAX = 1000;
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

const C = 'MemoryService';

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
  async write(_drafts: MemoryDraft[]): Promise<void> {
    throw new NotImplementedError(C, 'write', 114); // TODO(#114)
  }
  /**
   * Write one memory item and queue its embedding with `mem.item.write` (#102, MEM-10a,
   * MEM-19d). R10 commits the item row and its outbox row in one transaction, so a crash loses
   * neither one alone. Offline or online, the embedding waits in the outbox; the queue runs it
   * when the device is online. A second write of the same item adds no second job.
   * It refuses an item id that another MOG owns (MEM-01): R10 changes no row then.
   */
  async storeItem(w: ItemWrite): Promise<{ itemId: string; queued: boolean }> {
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
  async recordExchange(x: GenieExchange): Promise<void> {
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
        answerSummary: home ? (distilSummary(x.answerMd) ?? undefined) : undefined,
        keyNumbers: home ? fitKeyNumbers(x.keyNumbers) : undefined,
        finalAnswerMd: fitAnswer(x.answerMd),
        deepLink: x.deepLink || undefined,
        splitGroupId: x.splitGroupId,
        askedAt: now,
        completedAt: terminal ? now : undefined,
        rowHlc: this.#hlc(now),
        deviceId: this.#deviceId(),
      }),
    );
  }
  async forget(_scope: {
    sessionId?: string;
    mogId?: MogId;
    itemIds?: string[];
  }): Promise<ForgetResult> {
    throw new NotImplementedError(C, 'forget', 123); // TODO(#123)
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
