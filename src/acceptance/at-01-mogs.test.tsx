// @vitest-environment jsdom
/**
 * AT-01 "Create two MOGs" (#230), section 15: the TS part. Pass: both configs survive a restart,
 * chords stay distinct, no secret is in either config.
 *
 * The MOG tests run T1 MogManager (#26) on the store and window doubles of
 * `src/test-support/mog-doubles.ts`. The wizard (#43) is not built, so its test stays `it.skip`. The
 * window part (two panels, no focus steal, drag, the 8-MOG cap) and the chord check (MB-14 core)
 * run in `src-tauri/tests/at_01_mogs.rs`.
 */
import { act } from 'react';
import type { MogBinding, MogId } from '../shared/interfaces';
import { createMogManager, type MogDefinition } from '../brain/components/mog-manager';
import { fakeWindows, memoryConfigs, sequentialIds } from '../test-support/mog-doubles';
import { render, tauri } from '../views/test-tauri';
import { Hub } from '../views/hub/Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../views/test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../views/test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../views/test-tauri')).windowModule);

// `sequentialIds` gives ...0001 to Sales and ...0002 to Ops; the list sorts by name.
const B: MogId = '00000000-0000-4000-8000-000000000001';
const A: MogId = '00000000-0000-4000-8000-000000000002';
const binding = (home: string): MogBinding => ({
  home,
  backend: { kind: 'genie_one_mcp', profile: home },
  genieCompanion: false,
});

/** Strings that must never be in a MOG config (SEC rules): token shapes and secret keys. */
const SECRET = /dapi[0-9a-f]{16,}|eyJ[A-Za-z0-9_-]{10,}|"(token|secret|password|client_secret)"/i;

describe('AT-01 create two MOGs (TS part)', () => {
  beforeEach(() => tauri.reset());

  it('the secret check of the pending config tests finds token shapes and secret keys', () => {
    // The check that the pending tests below use, so a config that leaks a token fails them.
    expect(SECRET.test(JSON.stringify({ mogId: A, name: 'Sales', binding: binding('p') }))).toBe(
      false,
    );
    expect(SECRET.test('{"token":"x"}')).toBe(true);
    expect(SECRET.test(`dapi${'a'.repeat(32)}`)).toBe(true);
  });

  it('[AT-01] [MB-01] [MB-03a] two MOG configs survive a restart and hold no secret', async () => {
    const configs = memoryConfigs();
    const windows = fakeWindows();
    const before = createMogManager({ configs, windows, newId: sequentialIds() });
    const defs: { name: string; home: string }[] = [
      { name: 'Sales', home: 'sales-ws' },
      { name: 'Ops', home: 'ops-ws' },
    ];
    for (const d of defs) {
      const s = await before.create(d);
      await before.start(s.mogId);
    }
    expect(before.runningCount()).toBe(2);
    // The restart is a new MogManager on the same store; no MOG runs after it.
    const after: MogDefinition[] = (await createMogManager({ configs, windows }).list()).mogs;
    expect(after.map((d) => [d.mogId, d.binding.home])).toEqual([
      [A, 'ops-ws'],
      [B, 'sales-ws'],
    ]);
    for (const d of after) expect(SECRET.test(JSON.stringify(d))).toBe(false);
    for (const c of configs.files.values()) expect(SECRET.test(JSON.stringify(c))).toBe(false);
  });

  it('[AT-01] [MB-03b] the 9th start shows "8 MOGs are running. Stop one to start <name>."', async () => {
    const m = createMogManager({
      configs: memoryConfigs(),
      windows: fakeWindows(),
      newId: sequentialIds(),
    });
    const ids: MogId[] = [];
    for (let i = 1; i <= 9; i++) ids.push((await m.create({ name: `MOG ${i}`, home: 'p' })).mogId);
    for (const id of ids.slice(0, 8)) await m.start(id);
    await expect(m.start(ids[8] ?? '')).rejects.toThrow(
      /^8 MOGs are running\. Stop one to start MOG 9\.$/,
    );
  });

  it.skip('[AT-01] [MB-07] the wizard has 8 steps and resumes at the first failed step TODO(#43)', async () => {
    const { el } = await render(<Hub />);
    const steps = [...el.querySelectorAll('[data-wizard-step]')].map((s) => s.textContent);
    expect(steps).toHaveLength(8);
    await act(async () => {});
    expect(el.querySelector('[data-wizard-step][aria-current="step"]')).not.toBeNull();
  });
});
