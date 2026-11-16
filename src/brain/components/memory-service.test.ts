import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type { MemoryService } from '../../shared/interfaces';
import { RecordingLocalStore } from '../../test-support/memory-double';
import { core } from '../test-core';
import { LocalMemoryService, createMemoryService } from './memory-service';
import { IpcLocalMemoryStore } from './memory-store';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);

describe('T10 LocalMemoryService (MEM-39a, MEM-41)', () => {
  beforeEach(() => core.reset());

  it('[MEM-39a] runs on the LocalMemoryStore interface, so a test double replaces the store', () => {
    const double = new RecordingLocalStore();
    const svc: MemoryService = new LocalMemoryService(double);
    expect((svc as LocalMemoryService).store).toBe(double);
    expect(svc.mode()).toBe('local');
    expect(double.calls).toEqual([]);
  });

  it('[MEM-39a] the default store is the IPC store over mem_local_exec', () => {
    const svc = createMemoryService();
    expect(svc).toBeInstanceOf(LocalMemoryService);
    expect((svc as LocalMemoryService).store).toBeInstanceOf(IpcLocalMemoryStore);
  });

  it('[MEM-41] mode() is local and nothing is pending, with no Lakebase and no IPC call', () => {
    const svc = createMemoryService();
    expect(svc.mode()).toBe('local');
    expect(svc.syncStatus()).toEqual({ pending: 0 });
    expect(core.invoke).not.toHaveBeenCalled();
  });

  it('[MEM-41] the MVP memory path never names a mem_remote_* command', () => {
    for (const f of ['memory-service.ts', 'memory-store.ts']) {
      const src = readFileSync(resolve(import.meta.dirname, f), 'utf8');
      expect(src, f).not.toMatch(/mem_remote_/);
    }
  });
});
