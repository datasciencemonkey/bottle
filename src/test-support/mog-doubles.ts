/**
 * Test doubles for T1 MogManager (#26): an in-memory MOG config store and a window host.
 *
 * `memoryConfigs` copies the rules of `mog_core::mog_config` that the brain depends on
 * (ADR 0035): label-safe lower-case IDs, a 1-64 character name, a secret check with a
 * content-free code, `config_not_found` and a cap of 256 configs. `fakeWindows` copies the
 * R11 `WindowLedger` cap (8 open MOG windows, `mog_cap_reached`). Both reject with the
 * `{ error: <code> }` shape of the hardened gateway (ADR 0040).
 */
import type { MogConfig } from '../brain/components/mog-config';
import type { MogConfigClient, MogWindowClient } from '../brain/components/mog-manager';

const ID = /^[a-z0-9-]{1,64}$/;
const SECRET_KEY = /^(token|password|secret|pat)$/i;
const reject = (error: string) => Promise.reject({ error });

function hasSecretKey(v: unknown): boolean {
  if (typeof v !== 'object' || v === null) return false;
  return Object.entries(v).some(([k, x]) => SECRET_KEY.test(k) || hasSecretKey(x));
}

export interface MemoryConfigs extends MogConfigClient {
  /** The saved configs by ID, as the files on disk. */
  readonly files: Map<string, MogConfig>;
  /** IDs that `list` reports as unreadable (a tampered file). */
  readonly unreadable: string[];
  /** Every call, in order: `[op, id]`. */
  readonly calls: [string, string][];
}

export function memoryConfigs(limit = 256): MemoryConfigs {
  const files = new Map<string, MogConfig>();
  const unreadable: string[] = [];
  const calls: [string, string][] = [];
  const copy = (c: MogConfig): MogConfig => structuredClone(c);
  return {
    files,
    unreadable,
    calls,
    async list() {
      calls.push(['list', '']);
      const items = [...files]
        .map(([id, c]) => ({ id, name: c.name }))
        .sort((a, b) => (a.id < b.id ? -1 : 1));
      return { items, unreadable: [...unreadable] };
    },
    async read(id) {
      calls.push(['read', id]);
      const c = files.get(id);
      return c ? copy(c) : reject('config_not_found');
    },
    async write(id, config) {
      calls.push(['write', id]);
      if (!ID.test(id)) return reject('invalid_config_id');
      if (hasSecretKey(config)) return reject('secret_named_key');
      const n = [...config.name].length;
      if (n < 1 || n > 64 || [...config.persona].length > 2000) return reject('invalid_config');
      if (!files.has(id) && files.size >= limit) return reject('config_limit_reached');
      files.set(id, copy(config));
      return copy(config);
    },
    async remove(id) {
      calls.push(['delete', id]);
      return files.delete(id);
    },
  };
}

export interface FakeWindows extends MogWindowClient {
  readonly open: Set<string>;
  readonly calls: [string, string][];
}

export function fakeWindows(cap = 8): FakeWindows {
  const open = new Set<string>();
  const calls: [string, string][] = [];
  return {
    open,
    calls,
    async openMog(mogId) {
      calls.push(['open', mogId]);
      if (open.has(mogId)) return;
      if (open.size >= cap) return reject('mog_cap_reached');
      open.add(mogId);
    },
    async closeMog(mogId) {
      calls.push(['close', mogId]);
      if (!open.delete(mogId)) return reject('unknown_mog');
    },
  };
}

/** Deterministic UUID v4-shaped IDs: `...-000000000001`, `...-000000000002`, ... */
export function sequentialIds(): () => string {
  let n = 0;
  return () => `00000000-0000-4000-8000-${(++n).toString(16).padStart(12, '0')}`;
}
