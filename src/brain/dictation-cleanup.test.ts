/** Dictation cleanup around the router step (ADR 0122; CAP-66a, CAP-67a, CAP-67b, RTR-17a). */
import { routerDecision } from '../test-support/model-invoke';
import type { RouterInput } from './components/router-step';
import {
  CLAUDE_HAIKU_5_5,
  DEFAULT_ROUTER_MODEL,
  FALLBACK_ROUTER_MODEL,
  GPT_OSS_20B,
  isTestedRouterModel,
  PENDING_ROUTER_EVAL,
} from './components/router-model';
import {
  DICTATION_DEADLINE_MS,
  modelTextGrounded,
  withDictationCleanup,
} from './dictation-cleanup';
import { withOfflineFallback } from './offline-intent';
import { routeUtterance, type Utterance, type UtteranceRouteDeps } from './utterance-route';

const MOG = '00000000-0000-4000-8000-0000000000a1';
const RAW = 'um so the the report for Lisbon has 12 rows uh';
const PRE = 'So the report for Lisbon has 12 rows';
const input = (profile = 'home'): RouterInput => ({
  profile,
  speakerTrack: 'you',
  captureMode: 'ptt',
  pendingClarification: '',
  recentTopics: [],
  activeMogs: [{ id: MOG, name: 'Finance' }],
  profiles: [],
  currentTarget: profile,
});
const utterance = (over: Partial<Utterance> = {}): Utterance => ({
  mogId: MOG,
  text: RAW,
  leaseId: 'lease-1',
  forceAsk: false,
  insertionOn: true,
  input: input(),
  ...over,
});

function harness(route: UtteranceRouteDeps['route']) {
  const seen: string[] = [];
  const inserted: string[] = [];
  const base: UtteranceRouteDeps = {
    route: (t, m, i) => {
      seen.push(t);
      return route(t, m, i);
    },
    insertText: async (a) => {
      inserted.push(a.text);
      return { result: 'inserted' };
    },
  };
  return { base, seen, inserted };
}

const answer =
  (cleaned: string, over = {}) =>
  async () =>
    routerDecision({ intent: 'dictation', cleaned_text: cleaned, genie_question: '', ...over });

afterEach(() => vi.useRealTimers());

describe('the model answers (CAP-66a)', () => {
  it('[CAP-66a] the router input is the pre-pass text, and a grounded model text is pasted', async () => {
    const h = harness(answer('So, the report for Lisbon has 12 rows.'));
    const r = await routeUtterance(utterance(), withDictationCleanup(h.base));
    expect(h.seen).toEqual([PRE]);
    expect(h.inserted).toEqual(['So, the report for Lisbon has 12 rows.']);
    expect(r).toMatchObject({ route: 'chip', dictated: { route: 'inserted' } });
  });

  it('[CAP-66a] [CAP-68] a model text with a new number or name is rejected for the pre-pass text', async () => {
    for (const bad of [
      'So the report for Lisbon has 13 rows.',
      'So the report for Madrid has 12 rows.',
      'So the report for Lisbon has 12 rows and 4 columns.',
      'So the report for Lisbon has rows.',
      '',
    ]) {
      const h = harness(answer(bad));
      await routeUtterance(utterance(), withDictationCleanup(h.base));
      expect(h.inserted, bad).toEqual([PRE]);
    }
  });

  it('[CAP-66a] the grounding check keeps a rewrite of the same words', () => {
    expect(modelTextGrounded('So, the report has 12 rows.', 'So the report has 12 rows')).toBe(
      true,
    );
    expect(modelTextGrounded('The report has 12 rows.', 'So the report has 12 rows')).toBe(true);
    expect(modelTextGrounded('The report has 12 rows in Oslo.', 'So the report has 12 rows')).toBe(
      false,
    );
  });

  it('[CAP-66a] a rejected rewrite also resets a genie_question that the pre-pass text does not hold', async () => {
    const h = harness(
      answer('What is revenue for Oslo?', {
        intent: 'question',
        genie_question: 'What is revenue for Oslo?',
      }),
    );
    const d = await withDictationCleanup(h.base).route('what is revenue for Lisbon', MOG, input());
    expect(d.cleaned_text).toBe('What is revenue for Lisbon');
    expect(d.genie_question).toBe('What is revenue for Lisbon');
  });
});

describe('the deadline and the offline path (CAP-67b)', () => {
  it('[CAP-67b] [AT-21] a model that is late at 900 ms gives the pre-pass text, with no retry', async () => {
    vi.useFakeTimers();
    let calls = 0;
    const h = harness(() => {
      calls++;
      return new Promise(() => undefined);
    });
    const d = withDictationCleanup(h.base);
    const done = routeUtterance(utterance(), d);
    await vi.advanceTimersByTimeAsync(DICTATION_DEADLINE_MS - 1);
    expect(h.inserted).toEqual([]);
    await vi.advanceTimersByTimeAsync(1);
    await done;
    expect(h.inserted).toEqual([PRE]);
    expect(calls).toBe(1);
  });

  it('[CAP-67b] a late reply after the deadline changes nothing', async () => {
    vi.useFakeTimers();
    let late!: (d: ReturnType<typeof routerDecision>) => void;
    const h = harness(() => new Promise((res) => (late = res)));
    const done = routeUtterance(utterance(), withDictationCleanup(h.base));
    await vi.advanceTimersByTimeAsync(DICTATION_DEADLINE_MS);
    await done;
    late(routerDecision({ intent: 'dictation', cleaned_text: 'from the model' }));
    await vi.advanceTimersByTimeAsync(10);
    expect(h.inserted).toEqual([PRE]);
  });

  it('[CAP-67b] a model that answers before the deadline stops the timer', async () => {
    vi.useFakeTimers();
    const h = harness(answer(PRE));
    await routeUtterance(utterance(), withDictationCleanup(h.base));
    expect(vi.getTimerCount()).toBe(0);
  });

  it('[CAP-66a] [RTR-15c] with no home profile the offline fallback pastes the pre-pass text', async () => {
    const h = harness(answer('never called'));
    const d = withDictationCleanup(withOfflineFallback(h.base));
    await routeUtterance(utterance({ input: input('') }), d);
    expect(h.seen).toEqual([]);
    expect(h.inserted).toEqual([PRE]);
  });

  it('[CAP-66a] [RTR-15c] a failed model call pastes the pre-pass text', async () => {
    const h = harness(async () => {
      throw { error: 'auth_expired' };
    });
    await routeUtterance(utterance(), withDictationCleanup(withOfflineFallback(h.base)));
    expect(h.inserted).toEqual([PRE]);
  });
});

describe('the "Clean up dictation" toggle', () => {
  it('[CAP-66a] with the toggle off the raw text.trim() is pasted and the model text is skipped', async () => {
    const h = harness(answer('So the report has 12 rows.'));
    const d = withDictationCleanup(h.base, { cleanDictation: () => false });
    await routeUtterance(utterance({ text: `  ${RAW}  ` }), d);
    expect(h.seen).toEqual([`  ${RAW}  `]);
    expect(h.inserted).toEqual([RAW]);
  });

  it('[CAP-66a] with the toggle off and no home profile the raw text.trim() is pasted', async () => {
    const h = harness(answer('never called'));
    const d = withDictationCleanup(withOfflineFallback(h.base), { cleanDictation: () => false });
    await routeUtterance(utterance({ input: input('') }), d);
    expect(h.inserted).toEqual([RAW]);
  });

  it('[CAP-66a] with the toggle off a late model gets no deadline fallback', async () => {
    vi.useFakeTimers();
    let release!: (d: ReturnType<typeof routerDecision>) => void;
    const h = harness(() => new Promise((res) => (release = res)));
    const done = routeUtterance(
      utterance(),
      withDictationCleanup(h.base, { cleanDictation: () => false }),
    );
    await vi.advanceTimersByTimeAsync(5_000);
    expect(h.inserted).toEqual([]);
    release(routerDecision({ intent: 'dictation', cleaned_text: 'x' }));
    await done;
    expect(h.inserted).toEqual([RAW]);
  });

  it('[CAP-66a] the toggle is read for each utterance', async () => {
    let on = true;
    const h = harness(answer(PRE));
    const d = withDictationCleanup(h.base, { cleanDictation: () => on });
    await routeUtterance(utterance(), d);
    on = false;
    await routeUtterance(utterance(), d);
    expect(h.inserted).toEqual([PRE, RAW]);
  });
});

describe('the ⌥G ask (RTR-17a)', () => {
  it('[RTR-17a] [RTR-17b] the ask sends the exact text that a dictation would insert', async () => {
    const h = harness(answer('So, the report for Lisbon has 12 rows.'));
    const r = await routeUtterance(utterance({ forceAsk: true }), withDictationCleanup(h.base));
    expect(r).toMatchObject({ route: 'ask', question: 'So, the report for Lisbon has 12 rows.' });
    expect(h.inserted).toEqual([]);
  });

  it('[RTR-17a] a late model gives the ask the pre-pass text', async () => {
    vi.useFakeTimers();
    const h = harness(() => new Promise(() => undefined));
    const done = routeUtterance(utterance({ forceAsk: true }), withDictationCleanup(h.base));
    await vi.advanceTimersByTimeAsync(DICTATION_DEADLINE_MS);
    expect(await done).toMatchObject({ route: 'ask', question: PRE });
  });

  it('[RTR-17a] with the toggle off the ask sends the raw text', async () => {
    const h = harness(answer('So the report has 12 rows.'));
    const d = withDictationCleanup(h.base, { cleanDictation: () => false });
    expect(await routeUtterance(utterance({ forceAsk: true }), d)).toMatchObject({
      route: 'ask',
      question: RAW,
    });
  });
});

describe('the orchestrator model default (ADR 0122, NFR-22a)', () => {
  it('[NFR-22a] Haiku is the default and gpt-oss-20b the fallback, both system.ai ids', () => {
    expect(DEFAULT_ROUTER_MODEL).toBe(CLAUDE_HAIKU_5_5);
    expect(CLAUDE_HAIKU_5_5).toBe('system.ai.claude-haiku-5-5');
    expect(FALLBACK_ROUTER_MODEL).toBe(GPT_OSS_20B);
    expect(GPT_OSS_20B).toBe('system.ai.gpt-oss-20b');
  });

  it('[NFR-22a] Haiku is "tested" only with an eval record that passed', () => {
    expect(isTestedRouterModel(CLAUDE_HAIKU_5_5, PENDING_ROUTER_EVAL)).toBe(false);
    expect(isTestedRouterModel(CLAUDE_HAIKU_5_5, { [CLAUDE_HAIKU_5_5]: false })).toBe(false);
    expect(isTestedRouterModel(CLAUDE_HAIKU_5_5, { [CLAUDE_HAIKU_5_5]: true })).toBe(true);
  });
});
