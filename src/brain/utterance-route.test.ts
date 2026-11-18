import type { RouterDecision } from '../shared/interfaces';
import type { RouterInput } from './components/router-step';
import { chatCompletion, routerDecision } from '../test-support/model-invoke';
import { core } from './test-core';
import {
  ASK_CONFIDENCE,
  PREVIEW_MS,
  ipcUtteranceDeps,
  routeUtterance,
  type InsertResult,
  type Utterance,
  type UtteranceRouteDeps,
} from './utterance-route';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-0000000000a1';
const SENTENCE = 'Hi Sam, the deck is attached and I will send the numbers tomorrow.';
const input: RouterInput = {
  profile: 'caller-profile',
  speakerTrack: 'you',
  captureMode: 'ptt',
  pendingClarification: '',
  recentTopics: [],
  activeMogs: [{ id: MOG, name: 'Finance' }],
  profiles: [{ name: 'home', host: 'https://home.example.test', aliases: [] }],
  currentTarget: 'home',
};
const utterance = (over: Partial<Utterance> = {}): Utterance => ({
  mogId: MOG,
  text: SENTENCE,
  forceAsk: false,
  insertionOn: true,
  input,
  ...over,
});
const dictation = (over: Partial<RouterDecision> = {}) =>
  routerDecision({
    intent: 'dictation',
    cleaned_text: SENTENCE,
    genie_question: '',
    topic_label: '',
    ...over,
  });

function deps(decision: RouterDecision, result: InsertResult | Error = 'inserted') {
  const calls: string[] = [];
  const inserted: { mogId: string; text: string }[] = [];
  const d: UtteranceRouteDeps = {
    route: async () => {
      calls.push('route');
      return decision;
    },
    insertText: async (args) => {
      calls.push('insert_text');
      inserted.push(args);
      if (result instanceof Error) throw result;
      return { result };
    },
  };
  return { d, calls, inserted };
}

describe('utterance route (#59)', () => {
  it('[RTR-01b] [RTR-01c] a dictation ends at insert_text with the cleaned text', async () => {
    const { d, calls, inserted } = deps(
      dictation({ cleaned_text: 'Hi Sam, the deck is attached.' }),
    );
    const r = await routeUtterance(utterance(), d);
    expect(r.route).toBe('inserted');
    expect(calls).toEqual(['route', 'insert_text']);
    expect(inserted).toEqual([{ mogId: MOG, text: 'Hi Sam, the deck is attached.' }]);
  });

  it('[RTR-01c] an empty cleaned text inserts the STT text', async () => {
    const { d, inserted } = deps(dictation({ cleaned_text: '' }));
    await routeUtterance(utterance(), d);
    expect(inserted[0]?.text).toBe(SENTENCE);
  });

  it('[MOG-06d] a refused insertion returns the reason and the text for Copy', async () => {
    for (const result of ['no_permission', 'no_focus', 'secure_field'] as const) {
      const { d } = deps(dictation(), result);
      expect(await routeUtterance(utterance(), d)).toEqual({
        route: 'insert_failed',
        reason: result,
        text: SENTENCE,
      });
    }
    const { d } = deps(dictation(), new Error('host'));
    expect(await routeUtterance(utterance(), d)).toMatchObject({
      route: 'insert_failed',
      reason: 'host_failed',
    });
  });

  it('[RTR-01c] with insertion off a dictation goes to Copy and never to insert_text', async () => {
    const { d, calls } = deps(dictation());
    expect(await routeUtterance(utterance({ insertionOn: false }), d)).toEqual({
      route: 'copy',
      text: SENTENCE,
    });
    expect(calls).toEqual(['route']);
  });

  it('[RTR-04a] a confident question gets the 1.5 s preview, a weak one the chip', async () => {
    const q = routerDecision({ confidence: ASK_CONFIDENCE });
    const r = await routeUtterance(utterance(), deps(q).d);
    expect(r).toMatchObject({
      route: 'preview',
      forced: false,
      previewMs: PREVIEW_MS,
      question: 'What was revenue last quarter?',
    });
    const weak = routerDecision({ confidence: 0.69 });
    expect((await routeUtterance(utterance(), deps(weak).d)).route).toBe('chip');
    expect(PREVIEW_MS).toBe(1500);
  });

  it('[RTR-17a] Shift with the talk key forces the ask route for a dictation decision', async () => {
    const { d, calls } = deps(dictation());
    const r = await routeUtterance(utterance({ forceAsk: true }), d);
    expect(r).toMatchObject({ route: 'preview', forced: true, question: SENTENCE });
    // The router step still runs; only the intent gate is skipped. Nothing is inserted.
    expect(calls).toEqual(['route']);
  });

  it('[RTR-17a] a forced route skips the confidence gate and keeps the router question', async () => {
    const weak = routerDecision({ confidence: 0.1, genie_question: 'Revenue by region?' });
    const r = await routeUtterance(utterance({ forceAsk: true }), deps(weak).d);
    expect(r).toMatchObject({ route: 'preview', forced: true, question: 'Revenue by region?' });
    for (const intent of ['noise', 'context', 'memory_question'] as const) {
      const other = routerDecision({ intent, genie_question: '', cleaned_text: 'x y' });
      expect((await routeUtterance(utterance({ forceAsk: true }), deps(other).d)).route).toBe(
        'preview',
      );
    }
  });

  it('[RTR-17b] a forced ask keeps the 1.5 s preview and never sends on its own', async () => {
    const r = await routeUtterance(utterance({ forceAsk: true }), deps(dictation()).d);
    expect(r.route).toBe('preview');
    if (r.route === 'preview') expect(r.previewMs).toBe(1500);
  });

  it('[RTR-01b] other intents are left to their owners', async () => {
    for (const intent of ['memory_question', 'context', 'command', 'noise'] as const) {
      const { d, calls } = deps(routerDecision({ intent, genie_question: '' }));
      expect((await routeUtterance(utterance(), d)).route).toBe('other');
      expect(calls).toEqual(['route']);
    }
  });

  it('[RTR-07] a session utterance never gets the preview, also when it is forced', async () => {
    const session = { ...input, captureMode: 'session' as const };
    const q = routerDecision({ confidence: 0.95 });
    for (const forceAsk of [false, true]) {
      const { d, calls } = deps(q);
      const r = await routeUtterance(utterance({ forceAsk, input: session }), d);
      expect(r.route).toBe('chip');
      expect(calls).toEqual(['route']);
    }
    // A session never inserts into the focused app, also if a decision says `dictation`.
    const { d, calls } = deps(dictation());
    expect((await routeUtterance(utterance({ input: session }), d)).route).toBe('other');
    expect(calls).toEqual(['route']);
  });

  it('[RTR-17a] a forced ask with no text gives no preview', async () => {
    const empty = routerDecision({ intent: 'noise', genie_question: '', cleaned_text: '  ' });
    const r = await routeUtterance(utterance({ forceAsk: true, text: ' \n ' }), deps(empty).d);
    expect(r.route).toBe('other');
  });

  it('[RTR-14] an injection-suspected question gets the warning chip, never the preview', async () => {
    const suspect = routerDecision({ confidence: 0.95, injection_suspected: true });
    for (const forceAsk of [false, true]) {
      const r = await routeUtterance(utterance({ forceAsk }), deps(suspect).d);
      expect(r).toMatchObject({ route: 'chip', warning: 'injection_suspected' });
    }
    const plain = await routeUtterance(utterance(), deps(routerDecision({ confidence: 0.4 })).d);
    expect(plain).toMatchObject({ route: 'chip', warning: undefined });
  });

  describe('a failed router step (the `context` fallback, ADR 0032)', () => {
    beforeEach(() => {
      core.reset();
      core.invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'model_invoke') throw new Error('gateway down');
        return {};
      });
    });

    it('[RTR-01c] a plain utterance is not inserted and nothing is sent', async () => {
      const r = await routeUtterance(utterance(), ipcUtteranceDeps());
      expect(r.route).toBe('other');
      expect(core.invoke.mock.calls.map(([cmd]) => cmd)).not.toContain('insert_text');
    });

    it('[RTR-17a] [RTR-17b] a forced one still gets the preview of the STT text', async () => {
      const r = await routeUtterance(utterance({ forceAsk: true }), ipcUtteranceDeps());
      expect(r).toMatchObject({ route: 'preview', forced: true, question: SENTENCE });
    });
  });

  it('[RTR-04a] the model reply of the IPC double reaches the route', async () => {
    core.reset();
    core.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'model_invoke' ? chatCompletion(JSON.stringify(routerDecision())) : {},
    );
    expect((await routeUtterance(utterance(), ipcUtteranceDeps())).route).toBe('preview');
  });
});
