/** The Copy card sink of the utterance handler (#498): no dictation text is dropped silently. */
import type { MogId } from '../shared/interfaces';
import type { ChipInput } from './card-chips';
import type { BlurbInput } from './blurb-stack';
import { routerDecision } from '../test-support/model-invoke';
import { copyReasonText, createDictationSink, type DictationSinkDeps } from './dictation-copy';

const A = '00000000-0000-4000-8000-0000000000a1';
const SENTENCE = 'Hi Sam, the deck is attached and I will send the numbers tomorrow.';
const owned = { utteranceId: 'utt-1', mogId: A };

function setup() {
  const blurbs: [MogId, BlurbInput][] = [];
  const chips: [MogId, ChipInput][] = [];
  const shown: MogId[] = [];
  const deps: DictationSinkDeps = {
    stackFor: (mogId) => ({
      add: (b) => void blurbs.push([mogId, b]),
      offerChip: (c) => void chips.push([mogId, c]),
    }),
    showCard: async (mogId) => void shown.push(mogId),
  };
  return { sink: createDictationSink(deps), blurbs, chips, shown };
}

describe('dictation Copy card sink', () => {
  it('[MOG-05b] [RTR-01c] a copy route puts the text and a Copy chip on the owner card and shows it', () => {
    const { sink, blurbs, chips, shown } = setup();
    sink({ route: 'copy', text: SENTENCE }, owned);
    expect(blurbs).toEqual([[A, { id: 'utt-1', text: SENTENCE, completed: true }]]);
    expect(chips).toEqual([[A, { id: 'copy-utt-1', kind: 'copy', label: 'Copy', owner: 'utt-1' }]]);
    expect(shown).toEqual([A]);
  });

  it('[MOG-06d] [RTR-01c] each insert_failed reason puts the text, the reason and Copy on the card', () => {
    for (const reason of ['no_permission', 'no_focus', 'secure_field', 'host_failed'] as const) {
      const { sink, blurbs, chips, shown } = setup();
      sink({ route: 'insert_failed', reason, text: SENTENCE }, owned);
      const text = blurbs[0]?.[1].text ?? '';
      expect(text.startsWith(SENTENCE)).toBe(true);
      expect(text).toContain(`Could not insert: ${copyReasonText(reason)}`);
      expect(chips.map(([m, c]) => [m, c.kind, c.label])).toEqual([[A, 'copy', 'Copy']]);
      expect(shown).toEqual([A]);
    }
  });

  it('[MOG-06d] each failure reason has its own text', () => {
    const texts = (['no_permission', 'no_focus', 'secure_field', 'host_failed'] as const).map(
      copyReasonText,
    );
    expect(new Set(texts).size).toBe(4);
    for (const t of texts) expect(t.trim()).not.toBe('');
  });

  it('[RTR-01c] an inserted or other route adds no Copy card', () => {
    const { sink, blurbs, chips, shown } = setup();
    const decision = routerDecision({ intent: 'noise' });
    sink({ route: 'inserted', decision }, owned);
    sink({ route: 'other', decision }, owned);
    expect([blurbs, chips, shown]).toEqual([[], [], []]);
  });

  it('[MOG-05b] a copy card that fails to show still holds the text on the card', async () => {
    const { blurbs } = setup();
    const sink = createDictationSink({
      stackFor: () => ({ add: (b) => void blurbs.push([A, b]), offerChip: () => undefined }),
      showCard: () => Promise.reject(new Error('window gone')),
    });
    expect(() => sink({ route: 'copy', text: SENTENCE }, owned)).not.toThrow();
    await Promise.resolve();
    expect(blurbs[0]?.[1].text).toBe(SENTENCE);
  });
});
