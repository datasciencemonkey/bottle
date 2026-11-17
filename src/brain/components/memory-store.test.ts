import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { core } from '../test-core';
import { IpcLocalMemoryStore, LOCAL_STMT_IDS, isLocalStmtId } from './memory-store';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-000000000001';

/**
 * True if `text` holds SQL: a statement keyword followed by its clause keyword, or a DDL or
 * pragma statement. Plain words such as "select the EU region" do not match.
 */
function containsSqlText(text: string): boolean {
  const patterns = [
    /\bselect\b[\s\S]{0,200}?\bfrom\b\s+[a-z_"`[]/i,
    /\binsert\s+(or\s+\w+\s+)?into\b/i,
    /\bupdate\s+[a-z_"`[][\w"`\]]*\s+set\b/i,
    /\bdelete\s+from\b/i,
    /\b(create|drop|alter)\s+(table|index|trigger|view|virtual)\b/i,
    /\bpragma\s+\w+/i,
    /\battach\s+database\b/i,
    /\bwith\s+\w+\s+as\s*\(/i,
  ];
  return patterns.some((p) => p.test(text));
}

/** Every string inside a JSON value, keys included. */
function strings(v: unknown): string[] {
  if (typeof v === 'string') return [v];
  if (Array.isArray(v)) return v.flatMap(strings);
  if (v && typeof v === 'object') {
    return Object.entries(v).flatMap(([k, x]) => [k, ...strings(x)]);
  }
  return [];
}

/** Production TS files under `dir` (no tests). */
function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) return sourceFiles(p);
    return /\.(ts|tsx)$/.test(name) && !/\.test\.tsx?$/.test(name) ? [p] : [];
  });
}

describe('LocalMemoryStore over mem_local_exec (MEM-34a, MEM-34b)', () => {
  beforeEach(() => core.reset());

  it('[MEM-34b] the SQL detector finds SQL and passes plain words', () => {
    for (const sql of [
      'SELECT * FROM memory_item',
      'insert into kv (mog_id) values (1)',
      'UPDATE outbox SET attempts = 1',
      'DELETE FROM genie_exchange',
      'DROP TABLE mog',
      'PRAGMA key = 1',
      'WITH x AS (SELECT 1)',
    ]) {
      expect(containsSqlText(sql), sql).toBe(true);
    }
    for (const text of [
      'select the EU region',
      'Please update the forecast',
      'delete my last note',
      'mem.item.upsert',
    ]) {
      expect(containsSqlText(text), text).toBe(false);
    }
  });

  it('[MEM-34a] sends only a statement id and params through mem_local_exec', async () => {
    core.invoke.mockImplementation(async () => ({ rows: [{ itemId: 'i1' }] }));
    const store = new IpcLocalMemoryStore();
    const out = await store.exec('mem.item.get', { mogId: MOG, itemId: 'i1' });
    expect(out).toEqual({ rows: [{ itemId: 'i1' }] });
    expect(core.invoke).toHaveBeenCalledTimes(1);
    expect(core.invoke).toHaveBeenCalledWith('mem_local_exec', {
      args: { stmtId: 'mem.item.get', params: { mogId: MOG, itemId: 'i1' } },
    });
  });

  it('[MEM-34a] refuses an id outside the catalog before any IPC call', async () => {
    const store = new IpcLocalMemoryStore();
    for (const id of ['mem.nope', 'SELECT * FROM memory_item', 'mem.wipe.items_before']) {
      expect(isLocalStmtId(id), id).toBe(false);
      await expect(
        store.exec(id as (typeof LOCAL_STMT_IDS)[number], { mogId: MOG }),
      ).rejects.toThrow(/stmtId/);
    }
    expect(core.invoke).not.toHaveBeenCalled();
  });

  it('[MEM-34a] lists unique mem.* ids; Rust-only Wipe ids are not in the brain list', () => {
    expect(new Set(LOCAL_STMT_IDS).size).toBe(LOCAL_STMT_IDS.length);
    for (const id of LOCAL_STMT_IDS) {
      expect(id).toMatch(/^mem(\.[a-z_]+){2,}$/);
      expect(id.startsWith('mem.wipe.'), id).toBe(false);
    }
    for (const id of ['mem.exchange.upsert', 'mem.item.upsert', 'mem.recall.fts']) {
      expect(LOCAL_STMT_IDS).toContain(id);
    }
  });

  it('[MEM-34b] no mem_local_exec payload contains SQL text', async () => {
    core.invoke.mockImplementation(async () => ({ rows: [] }));
    const store = new IpcLocalMemoryStore();
    // User text that looks like SQL is data; the payload carries it as a param value only.
    const text = 'the EU region';
    for (const id of LOCAL_STMT_IDS) {
      await store.exec(id, { mogId: MOG, text, query: text });
    }
    expect(core.invoke).toHaveBeenCalledTimes(LOCAL_STMT_IDS.length);
    for (const [cmd, payload] of core.invoke.mock.calls) {
      expect(cmd).toBe('mem_local_exec');
      for (const s of [cmd, ...strings(payload)]) {
        expect(containsSqlText(s), s).toBe(false);
      }
    }
  });

  it('[MEM-34b] no brain, shared or view source file holds SQL text', () => {
    const root = resolve(import.meta.dirname, '../..');
    const files = ['brain', 'shared', 'views'].flatMap((d) => sourceFiles(join(root, d)));
    expect(files.length).toBeGreaterThan(10);
    for (const f of files) {
      expect(containsSqlText(readFileSync(f, 'utf8')), f).toBe(false);
    }
  });
});
