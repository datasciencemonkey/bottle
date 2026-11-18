/**
 * AT-05 "Preserve transcript meaning" (#231), section 15. Pass: raw and clean text stay
 * separate; numbers and entities stay exact or the raw text shows; a late cleanup result does
 * not overwrite user work.
 *
 * The corpus (numbers, units, negation, names, table names, self-corrections) and the meaning
 * oracle are `src/test-support/meaning.ts`; their self-tests run now. CleanupProvider (#55) is
 * built: its tests run with a `model_invoke` double that returns good and meaning-changing
 * replies. The raw and cleaned store with "Use raw text" (#61) and the MOG text card (#60) are
 * not built, so those tests stay `it.skip` with the feature issue. A skipped test never counts
 * as a pass.
 */
import type { Tags } from '../shared/interfaces';
import { createCleanupProvider } from '../brain/components/cleanup-provider';
import { createSttProvider } from '../brain/components/stt-provider';
import { core } from '../brain/test-core';
import { callsTo } from '../test-support/cloud-calls';
import { MEANING_CORPUS, meaningFailures } from '../test-support/meaning';
import { chatCompletion, type ModelInvokeArgs } from '../test-support/model-invoke';

vi.mock('@tauri-apps/api/core', async () => (await import('../brain/test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-0000000000a1';
const tags: Tags = { app: 'bottle', mog: MOG, op: 'cleanup' };

/** The transcript of a `model_invoke` cleanup request, with the tag escapes undone. */
function transcriptOf(payload: { args: ModelInvokeArgs }): string {
  const messages = (payload.args.request as { messages: { role: string; content: string }[] })
    .messages;
  const user = messages.find((m) => m.role === 'user')?.content ?? '';
  const inner = user.replace(/^<transcript>/, '').replace(/<\/transcript>$/, '');
  return inner.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');
}

/** A spoken number word: the corpus items with one are number cases too. */
const NUMBER_WORD =
  /\b(?:zero|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|sixty|seventy|eighty|ninety|hundred|thousand|million|billion|first|second|third|fourth|fifth|sixth|seventh|eighth|ninth|tenth)\b/i;

const cap = (t: string) => `${t.charAt(0).toUpperCase()}${t.slice(1)}`;

/**
 * Cleanup model replies for the corpus test: a good polish, and replies that change a number,
 * drop an entity, answer, add a URL, add code, drop a negation or pad the text.
 */
const MUTATIONS: ((t: string) => string)[] = [
  function polish(t) {
    return `${cap(t)}.`;
  },
  function changeNumberWord(t) {
    return `${cap(t.replace(/\b(two|five|three|nine)\b/, (w) => ({ two: 'three', five: 'six', three: 'four', nine: 'eight' })[w] ?? w))}.`;
  },
  function changeOrdinal(t) {
    return `${cap(t.replace(/\bfirst\b/, 'second'))}.`;
  },
  function digitizeOrdinal(t) {
    return `${cap(t.replace(/\b(\w+) first\b/, '21st'))}.`;
  },
  function digitizeSpokenNumber(t) {
    return `${cap(t.replace(/\b(\w+) point (\w+)\b/, '2.5'))}.`;
  },
  function bumpNumber(t) {
    return `${cap(t.replace(/\d/, (d) => String((Number(d) + 1) % 10)))}.`;
  },
  function swapNumbers(t) {
    return `${cap(t.replace(/(\d+(?:[.,]\d+)*)/, '$1 or 9,999'))}.`;
  },
  function dropFirstCapitalWord(t) {
    return `${cap(t.replace(/\b\p{Lu}[\p{L}]+\s?/u, ''))}.`;
  },
  function renameTable(t) {
    return `${cap(t.replace(/\b(\w+)\.(\w+)/, '$1_x.$2'))}.`;
  },
  function answer(t) {
    return `${cap(t)}. The answer is yes.`;
  },
  function addUrl(t) {
    return `${cap(t)} at www.example.test.`;
  },
  function addCode(t) {
    return `${cap(t)} \`now\`.`;
  },
  function dropNegation(t) {
    return `${cap(t.replace(/\b(?:not|never|no|don't)\s/i, ''))}.`;
  },
  function pad(t) {
    return `${cap(t)}, and please also make sure that everybody in the team hears about it.`;
  },
];

describe('AT-05 preserve transcript meaning', () => {
  beforeEach(() => core.reset());

  it('[AT-05] [CAP-66a] [CAP-68] cleanup keeps numbers and entities exact or keeps the raw text', async () => {
    // Every corpus item meets every model reply of MUTATIONS through the real IPC path. The
    // failures are collected first, so each line below can fail on its own.
    const cleanup = createCleanupProvider();
    const numberCases: string[] = [];
    const numberLost: string[] = [];
    const otherLost: string[] = [];
    let cleanedCount = 0;
    for (const mutate of MUTATIONS) {
      core.invoke.mockImplementation(async (cmd: string, payload?: unknown) =>
        cmd === 'model_invoke'
          ? chatCompletion(mutate(transcriptOf(payload as { args: ModelInvokeArgs })))
          : {},
      );
      for (const x of MEANING_CORPUS) {
        const out = await cleanup.clean(x.raw, tags, { profile: 'caller-profile' });
        const failures = meaningFailures(x, out).map((f) => `${f} (${mutate.name})`);
        if (!out.usedRaw) cleanedCount++;
        // A number case: the raw text holds a digit or a spoken number word.
        if (/\d/.test(x.raw) || NUMBER_WORD.test(x.raw)) {
          numberCases.push(x.id);
          numberLost.push(...failures);
        } else {
          otherLost.push(...failures);
        }
      }
    }
    // The corpus test keeps the meaning in 100 % of number cases (CAP-68).
    expect(numberCases.length).toBeGreaterThanOrEqual(MUTATIONS.length * 20);
    expect(numberLost).toEqual([]);
    const kept = numberCases.length - new Set(numberLost.map((f) => f.split(':')[0])).size;
    expect(kept / numberCases.length).toBe(1);
    // Names, table names and negations stay exact too (AT-05).
    expect(otherLost).toEqual([]);
    // The checks are not so strict that a good reply is never used.
    expect(cleanedCount).toBeGreaterThanOrEqual(MEANING_CORPUS.length);
  });

  it('[AT-05] [CAP-66b] fewer than 4 words make no cleanup model call', async () => {
    const out = await createCleanupProvider().clean('Send it now', tags, {
      profile: 'caller-profile',
    });
    expect(out).toMatchObject({ text: 'Send it now', usedRaw: false, outcome: 'short' });
    expect(callsTo(core.invoke.mock.calls, ['model_invoke'])).toEqual([]);
  });

  it('[AT-05] [CAP-68] a cleanup model reply that changes a number falls back to the raw text', async () => {
    // The model double returns a changed number; CleanupProvider must keep the raw text.
    core.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'model_invoke' ? chatCompletion('Ship 1,205 units to Lisbon by Friday.') : {},
    );
    const raw = 'ship 1,250 units to Lisbon by Friday';
    expect(
      await createCleanupProvider().clean(raw, tags, { profile: 'caller-profile' }),
    ).toMatchObject({ text: raw, usedRaw: true, outcome: 'check_failed' });
    expect(callsTo(core.invoke.mock.calls, ['model_invoke'])).toHaveLength(1);
  });

  it.skip('[AT-05] [CAP-70a] [CAP-70b] raw and cleaned text are kept apart and "Use raw text" restores the raw TODO(#61)', async () => {
    const t = await createSttProvider().transcribe(
      {
        segmentId: 's1',
        leaseId: 'lease-1',
        mogId: MOG,
        track: 'you',
        tStartMs: 0,
        tEndMs: 2_000,
        final: true,
      },
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
