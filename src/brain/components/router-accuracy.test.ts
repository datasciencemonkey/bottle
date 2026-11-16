import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type { Intent, RouterDecision } from '../../shared/interfaces';
import { createRouterModelDouble } from '../../test-support/model-invoke';
import {
  INTENT_ACCURACY_TARGET,
  formatIntentAccuracyReport,
  measureIntentAccuracy,
  parseGoldenSet,
  type GoldenItem,
} from './router-accuracy';
import { runRouterStep, type RouterInput } from './router-step';

const MOG = '00000000-0000-4000-8000-000000000001';
const INTENTS: Intent[] = ['question', 'memory_question', 'context', 'command', 'dictation', 'noise'];
const GOLDEN_PATH = resolve(
  import.meta.dirname,
  '../../../tests/fixtures/router-golden/utterances.json',
);
const golden = parseGoldenSet(JSON.parse(readFileSync(GOLDEN_PATH, 'utf8')));

const baseInput: Omit<RouterInput, 'captureMode' | 'speakerTrack'> = {
  profile: 'caller-profile',
  pendingClarification: '',
  recentTopics: [],
  activeMogs: [{ id: MOG, name: 'Finance' }],
  profiles: [{ name: 'home', host: 'https://home.example.test', aliases: [] }],
  currentTarget: 'home',
};

/** A decision the double returns for `intent` that passes the GL-7.2 rules unchanged. */
function replyFor(item: GoldenItem, intent: Intent): Partial<RouterDecision> {
  const asks = intent === 'question' || intent === 'memory_question';
  return {
    intent,
    cleaned_text: item.text,
    genie_question: asks ? item.text : '',
    command: intent === 'command' ? 'other' : 'none',
  };
}

/** Classify the golden set through the real router step and a double that is wrong on `wrong`. */
function classifier(wrong: Set<string>) {
  return async (item: GoldenItem): Promise<Intent> => {
    const answer: Intent = wrong.has(item.id)
      ? item.expected === 'noise'
        ? 'question'
        : 'noise'
      : item.expected;
    const double = createRouterModelDouble([{ decision: replyFor(item, answer) }]);
    const d = await runRouterStep(
      item.text,
      MOG,
      { ...baseInput, captureMode: item.captureMode, speakerTrack: item.speakerTrack },
      { invoke: double.invoke },
    );
    return d.intent;
  };
}

describe('golden utterance set (RTR-01)', () => {
  it('[RTR-01, AT-06] covers the six intents with 10 or more utterances each, with unique IDs and texts', () => {
    for (const intent of INTENTS)
      expect(golden.filter((g) => g.expected === intent).length, intent).toBeGreaterThanOrEqual(10);
    expect(new Set(golden.map((g) => g.id)).size).toBe(golden.length);
    expect(new Set(golden.map((g) => g.text)).size).toBe(golden.length);
  });

  it('[RTR-01] rejects a golden set with an unknown intent or a missing field', () => {
    const item = { id: 'x', text: 't', captureMode: 'ptt', speakerTrack: 'you', expected: 'question' };
    expect(() => parseGoldenSet({ version: 1, items: [{ ...item, expected: 'chat' }] })).toThrow();
    expect(() => parseGoldenSet({ version: 1, items: [{ ...item, text: undefined }] })).toThrow();
    expect(() => parseGoldenSet({ version: 2, items: [item] })).toThrow();
    expect(parseGoldenSet({ version: 1, items: [item] })).toHaveLength(1);
  });
});

describe('intent-accuracy report (RTR-01)', () => {
  it('[RTR-01, AT-06] counts a double that is right on 57 of 60 as 95 % and meets the 90 % target', async () => {
    const wrong = new Set(golden.slice(0, 3).map((g) => g.id));
    const r = await measureIntentAccuracy(golden, classifier(wrong));
    expect(INTENT_ACCURACY_TARGET).toBe(0.9);
    expect(r.total).toBe(60);
    expect(r.correct).toBe(57);
    expect(r.accuracy).toBeCloseTo(0.95, 10);
    expect(r.meetsTarget).toBe(true);
    expect(r.missIds).toEqual([...wrong]);
  });

  it('[RTR-01, AT-06] fails the target below 90 %', async () => {
    const wrong = new Set(golden.filter((_, i) => i % 6 === 0).map((g) => g.id));
    const r = await measureIntentAccuracy(golden, classifier(wrong));
    expect(r.correct).toBe(50);
    expect(r.meetsTarget).toBe(false);
  });

  it('[RTR-01] gives per-intent counts and a confusion table', async () => {
    const wrong = new Set([golden.find((g) => g.expected === 'context')!.id]);
    const r = await measureIntentAccuracy(golden, classifier(wrong));
    expect(r.perIntent.context).toEqual({ total: 10, correct: 9 });
    expect(r.perIntent.question).toEqual({ total: 10, correct: 10 });
    expect(r.confusion.context.noise).toBe(1);
    expect(r.confusion.context.context).toBe(9);
  });

  it('[RTR-01, AT-06] the markdown report has IDs and counts but no utterance text', async () => {
    const wrong = new Set(golden.slice(0, 2).map((g) => g.id));
    const r = await measureIntentAccuracy(golden, classifier(wrong));
    const md = formatIntentAccuracyReport(r, {
      source: 'double',
      model: 'router-model-double',
      date: '2026-11-16',
    });
    expect(md).toContain('96.7 %');
    expect(md).toContain('g001');
    for (const g of golden) expect(md).not.toContain(g.text);
  });

  it('[RTR-01] a report from the double says it is not a model result and the live run is pending on #56', async () => {
    const r = await measureIntentAccuracy(golden, classifier(new Set()));
    const md = formatIntentAccuracyReport(r, { source: 'double', model: 'd', date: '2026-11-16' });
    expect(md).toMatch(/test double/i);
    expect(md).toMatch(/not a model result/i);
    expect(md).toContain('#56');
    const live = formatIntentAccuracyReport(r, { source: 'live', model: 'm', date: '2026-11-16' });
    expect(live).not.toMatch(/not a model result/i);
  });
});
