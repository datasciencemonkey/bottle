// @vitest-environment jsdom
/**
 * AT-01 "Create two MOGs" (#230), section 15: the TS part. Pass: both configs survive a restart,
 * chords stay distinct, no secret is in either config.
 *
 * The MOG definitions (T1 MogManager, #26) and the wizard (#43) are not built, so these tests are
 * written against `MogManager` and the hub view and stay `it.skip` with the feature issue. The
 * window part (two panels, no focus steal, drag, the 8-MOG cap) and the chord check (MB-14 core)
 * run in `src-tauri/tests/at_01_mogs.rs`.
 */
import { act } from 'react';
import type { MogBinding, MogId } from '../shared/interfaces';
import { createMogManager, type MogDefinition } from '../brain/components/mog-manager';
import { render, tauri } from '../views/test-tauri';
import { Hub } from '../views/hub/Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../views/test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../views/test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../views/test-tauri')).windowModule);

const A: MogId = '00000000-0000-4000-8000-0000000000a1';
const B: MogId = '00000000-0000-4000-8000-0000000000b1';
const binding = (home: string): MogBinding => ({
  home,
  backend: { kind: 'genie_one_mcp', profile: home },
  genieCompanion: false,
});

/** Strings that must never be in a MOG config (SEC rules): token shapes and secret keys. */
const SECRET = /dapi[0-9a-f]{16,}|eyJ[A-Za-z0-9_-]{10,}|"(token|secret|password|client_secret)"/i;

describe('AT-01 create two MOGs (TS part)', () => {
  beforeEach(() => tauri.reset());

  it('[AT-01] the secret check finds token shapes and secret keys', () => {
    // The check that the pending tests below use, so a config that leaks a token fails them.
    expect(SECRET.test(JSON.stringify({ mogId: A, name: 'Sales', binding: binding('p') }))).toBe(
      false,
    );
    expect(SECRET.test('{"token":"x"}')).toBe(true);
    expect(SECRET.test(`dapi${'a'.repeat(32)}`)).toBe(true);
  });

  it.skip('[AT-01] [MB-01] [MB-03a] two MOG configs survive a restart and hold no secret TODO(#26)', async () => {
    const before = createMogManager();
    const defs: MogDefinition[] = [
      { mogId: A, name: 'Sales', binding: binding('sales-ws') },
      { mogId: B, name: 'Ops', binding: binding('ops-ws') },
    ];
    // TODO(#26): MogManager create; the restart is a new MogManager on the same store.
    for (const d of defs) await before.start(d.mogId);
    const after = await createMogManager().list();
    expect(after.map((d) => [d.mogId, d.binding.home])).toEqual([
      [A, 'sales-ws'],
      [B, 'ops-ws'],
    ]);
    for (const d of after) expect(SECRET.test(JSON.stringify(d))).toBe(false);
  });

  it.skip('[AT-01] [MB-03b] the 9th start shows "8 MOGs are running. Stop one to start <name>." TODO(#26)', async () => {
    const m = createMogManager();
    const ids = Array.from({ length: 9 }, (_, i) => `00000000-0000-4000-8000-00000000010${i}`);
    for (const id of ids.slice(0, 8)) await m.start(id);
    await expect(m.start(ids[8] ?? '')).rejects.toThrow(
      /^8 MOGs are running\. Stop one to start .+\.$/,
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
