/**
 * The T10 side of the R10 local store (#37, MEM-34a, MEM-34b, MEM-39a; ADR 0054).
 *
 * The brain never sends SQL. It names a statement of the fixed Rust catalog
 * (`mog_core::local_store::catalog`) and sends its params through `mem_local_exec`. The Rust core
 * checks the id, the params and the MOG (MEM-01), and runs the prepared statement.
 * `LOCAL_STMT_IDS` lists the ids the brain may use; a Rust test keeps it equal to the catalog.
 * The Wipe statements are Rust-only (R14), so they are not in this list. `mem.item.write` is an
 * atomic group: the item row and its embed job commit in one transaction (MEM-10a, ADR 0068).
 * The three `mem.forget.*` groups (items, session, MOG) each delete in one transaction, citing
 * items first (MEM-32a, ADR 0081); their steps are Rust-only.
 */
import { call, type JsonObject } from '../../shared/ipc';

export const LOCAL_STMT_IDS = [
  'mem.mog.upsert',
  'mem.exchange.upsert',
  'mem.exchange.get',
  'mem.exchange.dependents',
  'mem.exchange.by_session',
  'mem.item.upsert',
  'mem.item.get',
  'mem.item.write',
  'mem.recall.fts',
  'mem.forget.session_conversations',
  'mem.forget.mog_conversations',
  'mem.forget.items',
  'mem.forget.session',
  'mem.forget.mog',
  'mem.outbox.enqueue',
  'mem.outbox.due',
  'mem.outbox.retry',
  'mem.outbox.done',
] as const;

export type LocalStmtId = (typeof LOCAL_STMT_IDS)[number];

const IDS: ReadonlySet<string> = new Set(LOCAL_STMT_IDS);

export function isLocalStmtId(id: string): id is LocalStmtId {
  return IDS.has(id);
}

/** The local store behind MemoryService (MEM-39a). Tests replace it with a double. */
export interface LocalMemoryStore {
  /** Run one catalog statement. A query gives its rows; a write gives `[{ changes }]`. */
  exec(stmtId: LocalStmtId, params: JsonObject): Promise<{ rows: unknown[] }>;
}

export type MemLocalExec = (args: {
  stmtId: string;
  params: JsonObject;
}) => Promise<{ rows: unknown[] }>;

/** The store over IPC: `mem_local_exec { stmtId, params }` to the Rust core. */
export class IpcLocalMemoryStore implements LocalMemoryStore {
  constructor(private readonly send: MemLocalExec = (a) => call('mem_local_exec', a)) {}

  async exec(stmtId: LocalStmtId, params: JsonObject): Promise<{ rows: unknown[] }> {
    if (!isLocalStmtId(stmtId)) {
      throw new Error('stmtId is not in the local statement catalog');
    }
    return this.send({ stmtId, params });
  }
}
