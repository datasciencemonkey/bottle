import type { RouterDecision } from '../shared/interfaces';
import type { Utterance as CapturedUtterance } from './capture-lifecycle';
import type { RouterInput } from './components/router-step';
import { createRequestOwner } from './request-owner';
import { routerDecision } from '../test-support/model-invoke';
import type { UtteranceRoute } from './utterance-route';
import { createUtteranceHandler, type UtteranceHandlerDeps } from './utterance-wiring';

const A = '00000000-0000-4000-8000-0000000000a1';
const B = '00000000-0000-4000-8000-0000000000b2';
const SENTENCE = 'Hi Sam, the deck is attached and I will send the numbers tomorrow.';
const input: RouterInput = {
  profile: 'caller-profile',
  speakerTrack: 'you',
  captureMode: 'ptt',
  pendingClarification: '',
  recentTopics: [],
  activeMogs: [{ id: A, name: 'Finance' }],
  profiles: [{ name: 'home', host: 'https://home.example.test', aliases: [] }],
  currentTarget: 'home',
};
const captured = (over: Partial<CapturedUtterance> = {}): CapturedUtterance => ({
  mogId: A,
  text: SENTENCE,
  forceAsk: false,
  segment: {
    segmentId: 'seg-1',
    mogId: A,
    leaseId: 'lease-1',
    track: 'you',
    tStartMs: 0,
    tEndMs: 900,
    final: true,
  },
  mode: 'ptt',
  ...over,
});
const dictation = routerDecision({
  intent: 'dictation',
  cleaned_text: SENTENCE,
  genie_question: '',
  topic_label: '',
});

function setup(decision: RouterDecision | Error, over: Partial<UtteranceHandlerDeps> = {}) {
  const open = new Set([A, B]);
  let n = 0;
  const owner = createRequestOwner({
    isOpen: (id) => open.has(id),
    newId: () => `utt-${++n}`,
    sendCard: async () => undefined,
  });
  const calls: string[] = [];
  const routed: { text: string; mogId: string; input: RouterInput }[] = [];
  const inserted: { mogId: string; text: string }[] = [];
  const asks: { r: UtteranceRoute; utteranceId: string; mogId: string }[] = [];
  const deps: UtteranceHandlerDeps = {
    owner,
    routerInput: () => input,
    insertionOn: () => true,
    route: {
      route: async (text, mogId, inp) => {
        calls.push('route');
        routed.push({ text, mogId, input: inp });
        if (decision instanceof Error) throw decision;
        return decision;
      },
      insertText: async (args) => {
        calls.push('insert_text');
        inserted.push(args);
        return { result: 'inserted' };
      },
    },
    onAsk: (r, owned) => void asks.push({ r, ...owned }),
    onDone: () => undefined,
    ...over,
  };
  return { handle: createUtteranceHandler(deps), owner, open, calls, routed, inserted, asks };
}

describe('the utterance hook of the capture lifecycle (#59, #73)', () => {
  it('[RTR-01c] [MOG-13a] a dictation is owned by the key-down MOG, inserted there, then released', async () => {
    const s = setup(dictation);
    await s.handle(captured());
    expect(s.calls).toEqual(['route', 'insert_text']);
    expect(s.routed[0]).toMatchObject({ text: SENTENCE, mogId: A });
    expect(s.inserted).toEqual([{ mogId: A, text: SENTENCE }]);
    expect(s.asks).toEqual([]);
    // The exchange is done: the owner is dropped (utt-1 was the only utterance).
    expect(s.owner.ownerOf('utt-1')).toBeUndefined();
  });

  it('[MOG-13a] an utterance of a MOG that is no longer open is not routed', async () => {
    const s = setup(dictation);
    s.open.delete(A);
    await s.handle(captured());
    expect(s.calls).toEqual([]);
  });

  it('[RTR-17a] [RTR-17b] [MOG-13a] a forced ask goes to the preview with its live owner', async () => {
    const s = setup(dictation);
    await s.handle(captured({ mogId: B, forceAsk: true }));
    expect(s.calls).toEqual(['route']);
    expect(s.asks).toHaveLength(1);
    expect(s.asks[0]).toMatchObject({
      utteranceId: 'utt-1',
      mogId: B,
      r: { route: 'preview', forced: true, previewMs: 1500 },
    });
    // T3 owns the preview now, so the owner stays until T3 releases it.
    expect(s.owner.ownerOf('utt-1')).toBe(B);
  });

  it('[RTR-01b] the lifecycle mode is the capture mode the router sees', async () => {
    const s = setup(dictation);
    await s.handle(captured({ mode: 'hands_free' }));
    expect(s.routed[0]?.input.captureMode).toBe('hands_free');
  });

  it('[RTR-15c] [RTR-01b] an async router input (the home profile read) still routes the utterance', async () => {
    const s = setup(dictation, { routerInput: async () => input });
    await s.handle(captured());
    expect(s.calls).toEqual(['route', 'insert_text']);
    expect(s.owner.ownerOf('utt-1')).toBeUndefined();
  });

  it('[RTR-01c] a router input that fails releases the owner and inserts nothing', async () => {
    const s = setup(dictation, { routerInput: async () => Promise.reject(new Error('boom')) });
    await expect(s.handle(captured())).resolves.toBeUndefined();
    expect(s.calls).toEqual([]);
    expect(s.owner.ownerOf('utt-1')).toBeUndefined();
  });

  it('[RTR-01c] a route that throws releases the owner and inserts nothing', async () => {
    const s = setup(new Error('boom'));
    await expect(s.handle(captured())).resolves.toBeUndefined();
    expect(s.inserted).toEqual([]);
    expect(s.owner.ownerOf('utt-1')).toBeUndefined();
  });

  it('[MOG-13a] an onAsk that throws still releases the owner', async () => {
    const s = setup(dictation, {
      onAsk: () => {
        throw new Error('T3 failed');
      },
    });
    await expect(s.handle(captured({ forceAsk: true }))).resolves.toBeUndefined();
    expect(s.owner.ownerOf('utt-1')).toBeUndefined();
  });
});

describe('a dictated question (#557, ADR 0096)', () => {
  it('[#557] [RTR-01b] a question is inserted, its Ask Genie chip goes to onAsk, and the dictation goes to onDone after it', async () => {
    const order: string[] = [];
    const done: UtteranceRoute[] = [];
    const t = setup(routerDecision({ confidence: 0.9 }), {
      onAsk: (r) => void order.push(`ask:${r.route}`),
      onDone: (r) => {
        order.push(`done:${r.route}`);
        done.push(r);
      },
    });
    await t.handle(captured({ text: 'What was revenue last quarter?' }));
    expect(t.calls).toEqual(['route', 'insert_text']);
    expect(order).toEqual(['ask:chip', 'done:inserted']);
    expect(done[0]).toMatchObject({ route: 'inserted' });
  });
});
