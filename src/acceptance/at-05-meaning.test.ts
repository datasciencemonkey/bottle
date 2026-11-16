/**
 * AT-05 "Preserve transcript meaning" (#231), section 15. Pass: raw and clean text stay
 * separate; numbers and entities stay exact or the raw text shows; a late cleanup result does
 * not overwrite user work.
 *
 * The corpus (numbers, units, negation, names, table names, self-corrections) and the meaning
 * oracle are `src/test-support/meaning.ts`; their self-tests run now. CleanupProvider (#55), the
 * raw and cleaned store with "Use raw text" (#61) and the MOG text card (#60) are not built, so
 * these tests stay `it.skip` with the feature issue. A skipped test never counts as a pass.
 */
import type { Tags } from '../shared/interfaces';
import { createCleanupProvider } from '../brain/components/cleanup-provider';
import { createSttProvider } from '../brain/components/stt-provider';
import { core } from '../brain/test-core';
import { callsTo } from '../test-support/cloud-calls';
import { MEANING_CORPUS, meaningFailures } from '../test-support/meaning';

vi.mock('@tauri-apps/api/core', async () => (await import('../brain/test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-0000000000a1';
const tags: Tags = { app: 'bottle', mog: MOG, op: 'cleanup' };

describe('AT-05 preserve transcript meaning', () => {
  beforeEach(() => core.reset());

  it.skip('[AT-05] [CAP-66a] [CAP-68] cleanup keeps numbers and entities exact or keeps the raw text TODO(#55)', async () => {
    const cleanup = createCleanupProvider();
    for (const x of MEANING_CORPUS) {
      const out = await cleanup.clean(x.raw, tags);
      expect(meaningFailures(x, out), x.id).toEqual([]);
    }
  });

  it.skip('[AT-05] [CAP-66b] fewer than 4 words make no cleanup model call TODO(#55)', async () => {
    const out = await createCleanupProvider().clean('Send it now', tags);
    expect(out).toEqual({ text: 'Send it now', usedRaw: true });
    expect(callsTo(core.invoke.mock.calls, ['model_invoke'])).toEqual([]);
  });

  it.skip('[AT-05] [CAP-68] a cleanup model reply that changes a number falls back to the raw text TODO(#55)', async () => {
    // The model double returns a changed number; CleanupProvider must keep the raw text.
    core.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'model_invoke' ? { json: { text: 'Ship 1,205 units to Lisbon by Friday.' } } : {},
    );
    const raw = 'ship 1,250 units to Lisbon by Friday';
    expect(await createCleanupProvider().clean(raw, tags)).toEqual({ text: raw, usedRaw: true });
  });

  it.skip('[AT-05] [CAP-70a] [CAP-70b] raw and cleaned text are kept apart and "Use raw text" restores the raw TODO(#61)', async () => {
    const t = await createSttProvider().transcribe(
      { segmentId: 's1', mogId: MOG, track: 'you', tStartMs: 0, tEndMs: 2_000, final: true },
      { ...tags, op: 'asr' },
    );
    // Both texts exist for the utterance (CAP-70a).
    expect(t.raw.length).toBeGreaterThan(0);
    expect(t.text).not.toBe(t.raw);
    // TODO(#61): the transcript store keeps both rows; "Use raw text" puts t.raw back on the
    // card and in the insertion (CAP-70b). Assert the restored text equals t.raw.
    expect.unreachable('TODO(#61): the transcript store and "Use raw text" are not built');
  });

  it.skip('[AT-05] [MOG-05a] [MOG-05b] raw text shows first, cleaned text replaces it, and a late result does not overwrite an edit TODO(#60)', () => {
    // TODO(#60): on the sim clock, the card shows the raw text when STT returns (MOG-05a); the
    // cleaned text and Copy replace it when cleanup returns (MOG-05b). Then the user edits the
    // text, and a cleanup result that comes after the 900 ms deadline changes nothing.
    expect.unreachable('TODO(#60): the MOG dictation card is not built');
  });
});
