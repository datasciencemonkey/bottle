import type { RouterDecision } from '../shared/interfaces';
import type { RouterInput } from './components/router-step';
import { chatCompletion, routerDecision } from '../test-support/model-invoke';
import { core } from './test-core';
import {
  ASK_CONFIDENCE,
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
    expect(r).toMatchObject({ route: 'chip', dictated: { route: 'inserted' } });
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
      expect(await routeUtterance(utterance(), d)).toMatchObject({
        route: 'chip',
        dictated: { route: 'insert_failed', reason: result, text: SENTENCE },
      });
    }
    const { d } = deps(dictation(), new Error('host'));
    expect(await routeUtterance(utterance(), d)).toMatchObject({
      route: 'chip',
      dictated: { route: 'insert_failed', reason: 'host_failed' },
    });
  });

  it('[RTR-01c] with insertion off a dictation goes to Copy and never to insert_text', async () => {
    const { d, calls } = deps(dictation());
    expect(await routeUtterance(utterance({ insertionOn: false }), d)).toMatchObject({
      route: 'chip',
      dictated: { route: 'copy', text: SENTENCE },
    });
    expect(calls).toEqual(['route']);
  });

  it('[#557] [RTR-01b] [RTR-17a] a question with no ⌥G is dictated: it is inserted and gets the Ask Genie chip', async () => {
    const asked = 'What was revenue last quarter?';
    for (const confidence of [0.95, ASK_CONFIDENCE, 0.5]) {
      const q = routerDecision({ confidence, cleaned_text: asked, genie_question: 'Revenue Q3?' });
      const { d, calls, inserted } = deps(q);
      const r = await routeUtterance(utterance({ text: asked }), d);
      // ADR 0098: no classifier gate. With no ⌥G the utterance is dictation, at any confidence.
      expect(r).toMatchObject({
        route: 'chip',
        warning: undefined,
        dictated: { route: 'inserted' },
      });
      expect(calls).toEqual(['route', 'insert_text']);
      expect(inserted).toEqual([{ mogId: MOG, text: asked }]);
      // The fallback chip holds the exact text that was dictated, not a rewritten question.
      if (r.route !== 'chip') throw new Error('not a chip');
      expect(r.decision.genie_question).toBe(asked);
    }
  });

  it('[RTR-01c] [RTR-17a] every unforced intent is dictated, and its fallback chip holds the dictated text', async () => {
    for (const intent of ['dictation', 'memory_question', 'context', 'command', 'noise'] as const) {
      const { d, calls, inserted } = deps(
        routerDecision({ intent, genie_question: 'rewritten', cleaned_text: SENTENCE }),
      );
      const r = await routeUtterance(utterance(), d);
      expect(r).toMatchObject({ route: 'chip', dictated: { route: 'inserted' } });
      if (r.route !== 'chip') throw new Error('not a chip');
      expect(r.decision.genie_question).toBe(SENTENCE);
      expect(calls).toEqual(['route', 'insert_text']);
      expect(inserted).toEqual([{ mogId: MOG, text: SENTENCE }]);
    }
  });

  it('[#557] [MOG-06d] a dictation whose paste is refused goes to Copy and still gets the chip', async () => {
    const q = routerDecision({ confidence: 0.9 });
    const r = await routeUtterance(utterance(), deps(q, 'no_permission').d);
    expect(r).toMatchObject({
      route: 'chip',
      dictated: { route: 'insert_failed', reason: 'no_permission', text: q.cleaned_text },
    });
    const off = await routeUtterance(utterance({ insertionOn: false }), deps(q).d);
    expect(off).toMatchObject({ route: 'chip', dictated: { route: 'copy', text: q.cleaned_text } });
  });

  it('[RTR-17a] [RTR-17b] ⌥G gives the ask route with the exact text: no preview, nothing inserted', async () => {
    for (const intent of [
      'dictation',
      'question',
      'noise',
      'context',
      'memory_question',
    ] as const) {
      const decision = routerDecision({
        intent,
        confidence: 0.1,
        cleaned_text: SENTENCE,
        genie_question: 'Rewritten question?',
      });
      const { d, calls } = deps(decision);
      const r = await routeUtterance(utterance({ forceAsk: true }), d);
      expect(r).toEqual({ route: 'ask', question: SENTENCE, decision });
      // The router step still runs (RTR-14); only the intent gate is skipped. No insertion.
      expect(calls).toEqual(['route']);
    }
  });

  it('[RTR-17a] a forced ask uses the STT text when the router gives no cleaned text', async () => {
    const r = await routeUtterance(
      utterance({ forceAsk: true }),
      deps(routerDecision({ cleaned_text: '', genie_question: 'x' })).d,
    );
    expect(r).toMatchObject({ route: 'ask', question: SENTENCE });
  });

  it('[RTR-07] a session utterance never gets the ask route, also when it is forced', async () => {
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

  it('[RTR-17a] a forced ask with no text sends nothing', async () => {
    const empty = routerDecision({ intent: 'noise', genie_question: '', cleaned_text: '  ' });
    const r = await routeUtterance(utterance({ forceAsk: true, text: ' \n ' }), deps(empty).d);
    expect(r.route).toBe('other');
  });

  it('[RTR-14] an injection-suspected utterance gets the warning chip, never the ask route', async () => {
    const suspect = routerDecision({ confidence: 0.95, injection_suspected: true });
    const forced = await routeUtterance(utterance({ forceAsk: true }), deps(suspect).d);
    expect(forced).toMatchObject({ route: 'chip', warning: 'injection_suspected' });
    const { d, calls } = deps(suspect);
    const plain = await routeUtterance(utterance(), d);
    // Unforced, it is still dictated; its chip is the warning, with no Ask Genie action.
    expect(plain).toMatchObject({
      route: 'chip',
      warning: 'injection_suspected',
      dictated: { route: 'inserted' },
    });
    expect(calls).toEqual(['route', 'insert_text']);
  });

  describe('a failed router step (the `context` fallback, ADR 0032)', () => {
    beforeEach(() => {
      core.reset();
      core.invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'model_invoke') throw new Error('gateway down');
        return {};
      });
    });

    it('[RTR-15c] [RTR-01c] with the router model down, a plain sentence is a dictation and nothing is sent to Genie', async () => {
      // ADR 0092: a model that cannot be reached is RTR-15c, not the RTR-13d parse fallback.
      const r = await routeUtterance(utterance({ insertionOn: false }), ipcUtteranceDeps());
      expect(r).toMatchObject({ route: 'chip', dictated: { route: 'copy', text: SENTENCE } });
      const cmds = core.invoke.mock.calls.map(([cmd]) => cmd);
      for (const c of ['mcp_call', 'agent_start', 'agent_send']) expect(cmds).not.toContain(c);
    });

    it('[RTR-13d] [RTR-01c] router output that does not parse sends nothing; the STT text is dictated', async () => {
      core.invoke.mockImplementation(async (cmd: string) =>
        cmd === 'model_invoke' ? chatCompletion('not json') : {},
      );
      const r = await routeUtterance(utterance({ insertionOn: false }), ipcUtteranceDeps());
      expect(r).toMatchObject({ route: 'chip', dictated: { route: 'copy', text: SENTENCE } });
      const cmds = core.invoke.mock.calls.map(([cmd]) => cmd);
      for (const c of ['mcp_call', 'agent_start', 'agent_send']) expect(cmds).not.toContain(c);
    });

    it('[RTR-17a] a forced one still asks with the STT text', async () => {
      const r = await routeUtterance(utterance({ forceAsk: true }), ipcUtteranceDeps());
      expect(r).toMatchObject({ route: 'ask', question: SENTENCE });
    });
  });

  it('[RTR-01c] [#557] the model reply of the IPC double reaches the route (a question is a chip)', async () => {
    core.reset();
    core.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'model_invoke' ? chatCompletion(JSON.stringify(routerDecision())) : {},
    );
    expect((await routeUtterance(utterance(), ipcUtteranceDeps())).route).toBe('chip');
  });
});
