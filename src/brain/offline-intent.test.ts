/** The RTR-15c offline heuristic and the router fallback that uses it (#163). */
import { routerDecision } from '../test-support/model-invoke';
import type { RouterInput } from './components/router-step';
import {
  NO_HOME,
  OFFLINE_CONFIDENCE,
  offlineDecision,
  offlineIntent,
  withOfflineFallback,
} from './offline-intent';
import { ASK_CONFIDENCE, routeUtterance, type UtteranceRouteDeps } from './utterance-route';

const MOG = '00000000-0000-4000-8000-0000000000a1';
const SENTENCE = 'Hi Sam, the deck is attached and I will send the numbers tomorrow.';
const input = (profile: string): RouterInput => ({
  profile,
  speakerTrack: 'you',
  captureMode: 'ptt',
  pendingClarification: '',
  recentTopics: [],
  activeMogs: [{ id: MOG, name: 'Finance' }],
  profiles: [],
  currentTarget: profile,
});

function deps(model: 'ok' | 'fails') {
  const calls: string[] = [];
  const inserted: string[] = [];
  const base: UtteranceRouteDeps = {
    route: async () => {
      calls.push('model');
      if (model === 'fails') throw { error: 'auth_expired' };
      return routerDecision({ intent: 'dictation', cleaned_text: 'from the model' });
    },
    insertText: async (a) => {
      inserted.push(a.text);
      return { result: 'inserted' };
    },
  };
  return { d: withOfflineFallback(base), calls, inserted };
}

describe('the RTR-15c offline heuristic', () => {
  it('[RTR-15c] a question mark or a first wh-word gives question', () => {
    for (const t of [
      'What was revenue last quarter',
      'how many orders came in today',
      'Why did churn go up?',
      'Revenue for EMEA last week?',
      'who owns the sales pipeline',
    ])
      expect(offlineIntent(t)).toBe('question');
  });

  it('[RTR-15c] [RTR-01b] a plain sentence gives dictation', () => {
    for (const t of [SENTENCE, 'Thanks, see you at three.', 'I know what you mean.'])
      expect(offlineIntent(t)).toBe('dictation');
  });

  it('[RTR-15c] an empty utterance is noise', () => {
    expect(offlineIntent('  ')).toBe('noise');
  });

  it('[RTR-15c] an offline question is below the preview bar, so it waits for a click', () => {
    expect(OFFLINE_CONFIDENCE).toBeLessThan(ASK_CONFIDENCE);
    const d = offlineDecision('What was revenue?');
    expect(d).toMatchObject({ intent: 'question', genie_question: 'What was revenue?' });
    expect(d.confidence).toBeLessThan(ASK_CONFIDENCE);
    expect(d.injection_suspected).toBe(false);
    expect(d.target_profile_hint).toBe('');
  });
});

describe('the router fallback (RTR-15c)', () => {
  it('[RTR-15c] [RTR-01b] [RTR-01c] with no home profile a dictation is inserted and no model is called', async () => {
    const { d, calls, inserted } = deps('ok');
    const r = await routeUtterance(
      { mogId: MOG, text: SENTENCE, forceAsk: false, insertionOn: true, input: input(NO_HOME) },
      d,
    );
    expect(r).toMatchObject({ route: 'chip', dictated: { route: 'inserted' } });
    expect(inserted).toEqual([SENTENCE]);
    expect(calls).toEqual([]);
  });

  it('[RTR-15c] [RTR-01b] with no home profile and insertion off a dictation goes to Copy', async () => {
    const { d } = deps('ok');
    const r = await routeUtterance(
      { mogId: MOG, text: SENTENCE, forceAsk: false, insertionOn: false, input: input(NO_HOME) },
      d,
    );
    expect(r).toMatchObject({ route: 'chip', dictated: { route: 'copy', text: SENTENCE } });
  });

  it('[RTR-15c] with no home profile a question is a chip, never a preview (B1)', async () => {
    const { d } = deps('ok');
    const r = await routeUtterance(
      {
        mogId: MOG,
        text: 'What was revenue?',
        forceAsk: false,
        insertionOn: true,
        input: input(NO_HOME),
      },
      d,
    );
    expect(r.route).toBe('chip');
  });

  it('[RTR-15c] [RTR-01b] a failed router model call falls back to the heuristic', async () => {
    const { d, calls, inserted } = deps('fails');
    const r = await routeUtterance(
      { mogId: MOG, text: SENTENCE, forceAsk: false, insertionOn: true, input: input('home-a') },
      d,
    );
    expect(calls).toEqual(['model']);
    expect(r).toMatchObject({ route: 'chip', dictated: { route: 'inserted' } });
    expect(inserted).toEqual([SENTENCE]);
  });

  it('[RTR-15c] with a home profile the router model decides', async () => {
    const { d, calls, inserted } = deps('ok');
    await routeUtterance(
      { mogId: MOG, text: SENTENCE, forceAsk: false, insertionOn: true, input: input('home-a') },
      d,
    );
    expect(calls).toEqual(['model']);
    expect(inserted).toEqual(['from the model']);
  });
});
