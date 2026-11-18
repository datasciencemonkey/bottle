/**
 * The T10 embedding queue (#102, MEM-08f, MEM-19d, ARC-F7; ADR 0068).
 *
 * Every item write queues its embedding in the R10 outbox, in the same transaction as the item
 * (`mem.item.write`, MEM-10a). This queue drains the `embed` jobs when the device is online:
 * - at `start()`, so the jobs of an earlier run go after a restart;
 * - when `NetworkStatus` goes online again;
 * - on `kick()`, after a new item write.
 *
 * It reads FIFO batches of 100 (`mem.outbox.due`, kind `embed`) for each MOG (MEM-01) and gives
 * each job to the `ItemEmbedder`, one at a time:
 * - `done`: the job row goes (`mem.outbox.done`).
 * - `offline` or `unavailable`: the drain stops. This job and the rest stay as they are, with no
 *   attempt counted, and run on the next trigger.
 * - `failed`: the job waits with a backoff that grows to 5 min (`mem.outbox.retry`). The error is
 *   a fixed code, never free text.
 * The drain deletes outbox rows only. It never deletes or changes a memory item (U9).
 * One drain runs at a time; a trigger during a drain runs one more drain after it.
 */
import type { MogId } from '../../shared/interfaces';
import { IpcLocalMemoryStore, type LocalMemoryStore } from './memory-store';

/** Is the device online (ARC-F7: two failed connects mark the host offline)? */
export interface NetworkStatus {
  online(): boolean;
  /** Call `cb` on each change. Returns the function that stops the calls. */
  onChange(cb: (online: boolean) => void): () => void;
}

/** One queued embedding request. It points to the item; it holds no text. */
export interface EmbedJob {
  mogId: MogId;
  /** The outbox row. */
  oid: number;
  itemId: string;
  /** The failed runs so far. */
  attempts: number;
}

/** How one run of a job ended. `code` is a fixed code (`[a-z0-9_.]{1,64}`), never free text. */
export type EmbedOutcome =
  | { status: 'done' }
  | { status: 'offline' }
  | { status: 'unavailable' }
  | { status: 'failed'; code: string };

/**
 * Runs one job: it reads the item, gets the vector and stores it on the item. It must be safe to
 * run the same job again (a crash after the store and before `mem.outbox.done` runs it twice).
 */
export interface ItemEmbedder {
  embed(job: EmbedJob): Promise<EmbedOutcome>;
}

/**
 * The embedder until #198 lands. It runs nothing, so every job stays queued.
 * TODO(#198): model_invoke (role embed), L2-normalise, store the vector and `embedding_model`.
 */
export class UnwiredItemEmbedder implements ItemEmbedder {
  async embed(_job: EmbedJob): Promise<EmbedOutcome> {
    return { status: 'unavailable' };
  }
}

/**
 * The network status until the `net.status` payload has a type. It says offline, so nothing
 * leaves the device and the jobs wait.
 * TODO(#466): read the typed `net.status` payload from the brain bus.
 */
export class UnwiredNetworkStatus implements NetworkStatus {
  online(): boolean {
    return false;
  }
  onChange(_cb: (online: boolean) => void): () => void {
    return () => undefined;
  }
}

/** The jobs read in one batch (the MEM-11a batch size). */
export const EMBED_BATCH = 100;
const FIRST_RETRY_MS = 30_000;
/** The backoff grows to 5 min, as for the SyncWorker (MEM-11b). */
const MAX_RETRY_MS = 300_000;
const CODE_RE = /^[a-z0-9_.]{1,64}$/;

/** The wait after a failed run, for a job that failed `attempts` times before. */
export function retryDelayMs(attempts: number): number {
  return Math.min(FIRST_RETRY_MS * 2 ** Math.min(attempts, 16), MAX_RETRY_MS);
}

export interface EmbeddingQueueOptions {
  /** The R10 store. The default is the IPC store over `mem_local_exec`. */
  store?: LocalMemoryStore;
  net: NetworkStatus;
  embedder: ItemEmbedder;
  /** The MOGs to drain. The outbox is read one MOG at a time (MEM-01). */
  mogIds: () => readonly MogId[];
  /** The clock in epoch ms. */
  now?: () => number;
}

export interface DrainResult {
  done: number;
  failed: number;
  /** `idle`: no job is due. `offline`: the network or the embedder is not there. */
  stopped: 'idle' | 'offline';
}

interface DueRow {
  oid: number;
  refId: string;
  attempts: number;
}

function dueRow(r: unknown): DueRow | null {
  if (typeof r !== 'object' || r === null) return null;
  const { oid, refId, attempts } = r as Record<string, unknown>;
  if (typeof oid !== 'number' || typeof refId !== 'string') return null;
  return { oid, refId, attempts: typeof attempts === 'number' ? attempts : 0 };
}

export class EmbeddingQueue {
  readonly #store: LocalMemoryStore;
  readonly #net: NetworkStatus;
  readonly #embedder: ItemEmbedder;
  readonly #mogIds: () => readonly MogId[];
  readonly #now: () => number;
  #running: Promise<void> | null = null;
  #again = false;
  #unsubscribe: (() => void) | null = null;

  constructor(o: EmbeddingQueueOptions) {
    this.#store = o.store ?? new IpcLocalMemoryStore();
    this.#net = o.net;
    this.#embedder = o.embedder;
    this.#mogIds = o.mogIds;
    this.#now = o.now ?? (() => Date.now());
  }

  /** Drain now if online, and again each time the device goes online. */
  start(): void {
    if (!this.#unsubscribe) {
      this.#unsubscribe = this.#net.onChange((online) => {
        if (online) this.kick();
      });
    }
    this.kick();
  }

  /** Stop the network trigger. A drain that runs ends on its own. */
  stop(): void {
    this.#unsubscribe?.();
    this.#unsubscribe = null;
  }

  /** Start a drain if online. During a drain, run one more drain after it. */
  kick(): void {
    if (!this.#net.online()) return;
    if (this.#running) {
      this.#again = true;
      return;
    }
    this.#running = (async () => {
      try {
        do {
          this.#again = false;
          await this.drain();
        } while (this.#again && this.#net.online());
      } catch {
        // The jobs stay in the outbox; the next trigger tries again. No detail is logged.
      } finally {
        this.#running = null;
      }
    })();
  }

  /** Resolves when no drain runs. */
  async idle(): Promise<void> {
    while (this.#running) await this.#running;
  }

  /** Run the due embed jobs of every MOG, oldest first, until none is due or the device is offline. */
  async drain(): Promise<DrainResult> {
    let done = 0;
    let failed = 0;
    for (const mogId of this.#mogIds()) {
      for (;;) {
        if (!this.#net.online()) return { done, failed, stopped: 'offline' };
        const { rows } = await this.#store.exec('mem.outbox.due', {
          mogId,
          kind: 'embed',
          now: this.#now(),
          limit: EMBED_BATCH,
        });
        const jobs = rows.map(dueRow).filter((r): r is DueRow => r !== null);
        if (jobs.length === 0) break;
        for (const j of jobs) {
          if (!this.#net.online()) return { done, failed, stopped: 'offline' };
          const job: EmbedJob = { mogId, oid: j.oid, itemId: j.refId, attempts: j.attempts };
          const out = await this.#embedder.embed(job);
          if (out.status === 'offline' || out.status === 'unavailable') {
            return { done, failed, stopped: 'offline' };
          }
          if (out.status === 'done') {
            await this.#store.exec('mem.outbox.done', { mogId, oid: j.oid });
            done += 1;
          } else {
            await this.#store.exec('mem.outbox.retry', {
              mogId,
              oid: j.oid,
              nextAttemptAt: this.#now() + retryDelayMs(j.attempts),
              errorCode: CODE_RE.test(out.code) ? out.code : 'embed_failed',
            });
            failed += 1;
          }
        }
        if (rows.length < EMBED_BATCH) break;
      }
    }
    return { done, failed, stopped: 'idle' };
  }
}
