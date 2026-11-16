import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { NOTICE_VERSION } from '../shared/hub-notice';
import { core } from './test-core';
import { startCapture } from './capture-gate';
import { createConsentService, memoryConsentStore } from './components/consent-service';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const MOG = '00000000-0000-4000-8000-000000000001';
const args = { mogId: MOG, mode: 'ptt' } as const;

function consent(store = memoryConsentStore(), version = NOTICE_VERSION) {
  return createConsentService({ store, version, now: () => new Date(0), newId: () => 'd' });
}

describe('capture_start gate on the first-run notice (#27)', () => {
  beforeEach(() => core.reset());

  it('[MB-08a] [AT-24] refuses capture_start with consent_required before the notice is accepted', async () => {
    const onBlocked = vi.fn();
    await expect(startCapture({ consent: consent(), onBlocked }, args)).rejects.toEqual({
      error: 'consent_required',
      command: 'capture_start',
    });
    expect(core.invoke).not.toHaveBeenCalled();
    expect(onBlocked).toHaveBeenCalledWith({ version: NOTICE_VERSION, accepted: false });
  });

  it('[MB-08a] [AT-24] calls capture_start once the current notice is accepted', async () => {
    const c = consent();
    await c.acceptNotice(NOTICE_VERSION);
    const state = { mic: 'open' };
    core.invoke.mockImplementation(async () => state);
    const onBlocked = vi.fn();
    await expect(startCapture({ consent: c, onBlocked }, args)).resolves.toBe(state);
    expect(core.invoke).toHaveBeenCalledWith('capture_start', { args });
    expect(onBlocked).not.toHaveBeenCalled();
  });

  it('[MB-08b] [AT-24] a notice version change blocks capture again until the user accepts it', async () => {
    const store = memoryConsentStore();
    await consent(store, 1).acceptNotice(1);
    const v2 = consent(store, 2);
    await expect(startCapture({ consent: v2 }, args)).rejects.toEqual({
      error: 'consent_required',
      command: 'capture_start',
    });
    expect(core.invoke).not.toHaveBeenCalled();
    await v2.acceptNotice(2);
    await startCapture({ consent: v2 }, args);
    expect(core.invoke).toHaveBeenCalledTimes(1);
  });

  it('[MB-08a] no brain source except capture-gate.ts calls capture_start', () => {
    const dir = import.meta.dirname;
    const files = (d: string): string[] =>
      readdirSync(d).flatMap((f) => {
        const p = join(d, f);
        return statSync(p).isDirectory() ? files(p) : /\.tsx?$/.test(f) ? [p] : [];
      });
    const callers = files(dir)
      .filter((f) => !/\.test\.tsx?$/.test(f))
      .filter((f) => /['"`]capture_start['"`]/.test(readFileSync(f, 'utf8')))
      .map((f) => relative(dir, f).replaceAll('\\', '/'));
    expect(callers).toEqual(['capture-gate.ts']);
  });
});
