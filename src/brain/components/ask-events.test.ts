/**
 * The GL-1 event normaliser (#33, ADR 0084): one terminal AskEvent per request (G-21b), clarify
 * (G-22), and `incomplete` with and without an answer (G-23a, G-23b). The Genie One MCP double
 * of #229 runs on the sim scheduler through the PollActor rig; nothing is live.
 */
import type { AskEvent, AskRequest } from '../../shared/interfaces';
import { scenarios, type ScriptedResponse } from '../../test-support/genie-one-mcp';
import { HOST, MOG, PROFILE, rig, type Rig } from '../../test-support/poll-actor-rig';
import {
  CLARIFY_FOLLOW_UP_MS,
  CLARIFY_MAX_CHARS,
  createClarifyFollowUps,
  isClarify,
  isTerminal,
  keyNumbersOf,
  oneTerminal,
} from './ask-events';
import { createGenieBridge } from './genie-bridge';
import { userTextFor } from './genie-errors';
import { GIVE_UP_MS } from './poll-actor';

const MIN = 60_000;

function bridgeFor(r: Rig) {
  return createGenieBridge({
    call: r.call,
    conversations: r.cm,
    now: () => r.sched.clock.now(),
    hostOf: () => HOST,
  });
}

function request(over: Partial<AskRequest> = {}): AskRequest {
  return {
    opId: '00000000-0000-4000-8000-00000000a001',
    mogId: MOG as AskRequest['mogId'],
    binding: { kind: 'genie_one_mcp', profile: PROFILE },
    question: 'What was revenue last quarter?',
    tags: { app: 'bottle', mog: MOG, op: 'genie' },
    ...over,
  };
}

/** Submit one request and collect its events on sim time. */
async function submitted(
  r: Rig,
  req: AskRequest = request(),
  bridge = bridgeFor(r),
  untilMs = r.sched.clock.now() + GIVE_UP_MS + MIN,
): Promise<AskEvent[]> {
  const out: AskEvent[] = [];
  const done = (async () => {
    for await (const e of bridge.submit(req).events) out.push(e);
  })();
  await r.sched.runUntil(untilMs, { settle: true });
  await done;
  return out;
}

const terminals = (events: AskEvent[]) => events.filter(isTerminal);

describe('GL-1 terminal events (G-21b)', () => {
  const cases: [string, () => ScriptedResponse][] = [
    ['normalAnswer', scenarios.normalAnswer],
    ['unknownStatus', () => scenarios.unknownStatus(3)],
    ['nullDeepLink', scenarios.nullDeepLink],
    ['incompleteWithAnswer', scenarios.incompleteWithAnswer],
    ['incompleteNoAnswer', scenarios.incompleteNoAnswer],
    ['failed', scenarios.failed],
    ['truncated', () => scenarios.truncated(60)],
    ['needsInput', scenarios.needsInput],
    ['nullProgressSteps', scenarios.nullProgressSteps],
    ['gave up', () => scenarios.unknownStatus(100_000)],
  ];

  it.each(cases)(
    '[G-21b] [AT-09] %s gives exactly one terminal event, and it is the last event',
    async (_name, make) => {
      const r = rig({ responses: [make()] });
      const events = await submitted(r);
      expect(events[0]).toMatchObject({ t: 'accepted' });
      expect(terminals(events)).toHaveLength(1);
      expect(isTerminal(events.at(-1) as AskEvent)).toBe(true);
      expect(r.double.asks()).toHaveLength(1);
    },
  );

  it('[G-21b] a refused ask, a refused poll and an error reply each give one terminal event', async () => {
    const refusedAsk = rig({ rejectAsk: () => ({ error: 'offline' }) });
    const a = await submitted(refusedAsk);
    expect(a).toEqual([
      {
        t: 'failed',
        code: 'OFFLINE',
        userText: userTextFor('OFFLINE', { host: HOST }),
        retry: 'user',
      },
    ]);
    const refusedPoll = rig({ reject: () => ({ error: 'auth_expired' }) });
    const b = await submitted(refusedPoll);
    expect(terminals(b)).toEqual([expect.objectContaining({ t: 'failed', code: 'AUTH_EXPIRED' })]);
    // A busy conversation: the second ask in an open conversation gets the busy text.
    const busy = rig({ responses: [scenarios.unknownStatus(100_000)] });
    const bridge = bridgeFor(busy);
    const first = bridge.submit(request()).events[Symbol.asyncIterator]();
    const accepted = first.next();
    await busy.sched.runUntil(1, { settle: true });
    const cid = ((await accepted).value as Extract<AskEvent, { t: 'accepted' }>).conversationId;
    const c = await submitted(busy, request({ conversationId: cid }), bridge, 2);
    expect(c).toEqual([expect.objectContaining({ t: 'failed', code: 'CONVERSATION_BUSY' })]);
    expect(busy.double.asks()).toHaveLength(1);
  });

  it('[G-21b] oneTerminal drops events after the first terminal and ends a stream without one in a failure', async () => {
    async function* src(events: AskEvent[]) {
      yield* events;
    }
    const after = async (events: AskEvent[]) => {
      const out: AskEvent[] = [];
      for await (const e of oneTerminal(src(events))) out.push(e);
      return out;
    };
    const answer: AskEvent = {
      t: 'answer',
      md: 'x',
      incomplete: false,
      keyNumbers: [],
      deepLink: '',
      queryItems: [],
    };
    expect(await after([{ t: 'soft_notice' }, answer, { t: 'abandoned' }, answer])).toEqual([
      { t: 'soft_notice' },
      answer,
    ]);
    const lost = await after([{ t: 'progress', steps: null, elapsedMs: 1 }]);
    expect(terminals(lost)).toEqual([
      expect.objectContaining({ t: 'failed', code: 'STREAM_LOST' }),
    ]);
    // needs_input is terminal only for clarify (approval and choose_profile come before an ask).
    expect(isTerminal({ t: 'needs_input', kind: 'clarify', prompt: '?', options: [] })).toBe(true);
    expect(isTerminal({ t: 'needs_input', kind: 'approval', prompt: '', options: [] })).toBe(false);
    expect(isTerminal({ t: 'needs_input', kind: 'choose_profile', prompt: '', options: [] })).toBe(
      false,
    );
    expect(isTerminal({ t: 'cancel_requested' })).toBe(true);
    expect(isTerminal({ t: 'partial', text: 'x' })).toBe(false);
  });

  it('[G-21b] a thrown error inside the backend still gives one failed event', async () => {
    const r = rig({ rejectAsk: () => new Error('host exploded with private detail') });
    const events = await submitted(r);
    expect(events).toEqual([expect.objectContaining({ t: 'failed' })]);
    expect(JSON.stringify(events)).not.toContain('private detail');
  });
});

describe('completed and incomplete answers (G-22, G-23a, G-23b)', () => {
  it('[G-23a] incomplete with an answer is answer{incomplete:true} with its text and key numbers', async () => {
    const r = rig({ responses: [scenarios.incompleteWithAnswer()] });
    const last = (await submitted(r)).at(-1);
    expect(last).toMatchObject({
      t: 'answer',
      incomplete: true,
      md: 'Revenue in EMEA was **$410,500** in Q3; other regions did not finish.',
      keyNumbers: [{ text: '$410,500' }],
    });
  });

  it('[G-23b] incomplete with no answer is a failure, never an empty answer', async () => {
    const r = rig({ responses: [scenarios.incompleteNoAnswer()] });
    const last = (await submitted(r)).at(-1);
    expect(last).toEqual({
      t: 'failed',
      code: 'GENIE_INCOMPLETE',
      userText: userTextFor('GENIE_INCOMPLETE'),
      retry: 'user',
    });
    // A whitespace-only answer counts as no answer.
    const blank = rig({
      responses: [{ polls: [{ status: 'incomplete', final_answer: '  \n ', query_items: null }] }],
    });
    expect((await submitted(blank)).at(-1)).toMatchObject({
      t: 'failed',
      code: 'GENIE_INCOMPLETE',
    });
  });

  it('[G-22] a short completed answer that ends with "?" and has no items is needs_input(clarify)', async () => {
    const r = rig({ responses: [scenarios.needsInput()] });
    const last = (await submitted(r)).at(-1);
    expect(last).toEqual({
      t: 'needs_input',
      kind: 'clarify',
      prompt: 'Do you mean gross revenue or net revenue?',
      options: [],
    });
  });

  it('[G-22] the clarify rule: < 400 chars, ends with "?", no query items from any poll', () => {
    expect(CLARIFY_MAX_CHARS).toBe(400);
    const q = (n: number) => `${'a'.repeat(n - 1)}?`;
    expect(isClarify(q(399), [])).toBe(true);
    expect(isClarify(q(400), [])).toBe(false);
    expect(isClarify('Which region?  \n', [])).toBe(true);
    expect(isClarify('Which region?', [{ itemId: 'pub_q1_output' }])).toBe(false);
    expect(isClarify('Revenue was $1.2M.', [])).toBe(false);
    expect(isClarify('', [])).toBe(false);
  });

  it('[G-22] a question answer with a query item from an earlier poll is a normal answer', async () => {
    const r = rig({
      responses: [
        {
          polls: [
            { status: 'in_progress', query_items: [{ item_id: 'pub_q1_output', sql: 'SELECT 1' }] },
            {
              status: 'completed',
              final_answer: 'Is this the table you wanted?',
              query_items: null,
            },
          ],
        },
      ],
    });
    const last = (await submitted(r)).at(-1);
    expect(last).toMatchObject({
      t: 'answer',
      incomplete: false,
      queryItems: [{ itemId: 'pub_q1_output', sql: 'SELECT 1' }],
    });
  });

  it('[G-21a] a completed answer carries the md, the deep link and the items of the poll', async () => {
    const r = rig({ responses: [scenarios.normalAnswer()] });
    const last = (await submitted(r)).at(-1);
    expect(last).toMatchObject({
      t: 'answer',
      md: 'Revenue last quarter was **$1.2M**.',
      incomplete: false,
      keyNumbers: [{ text: '$1.2M' }],
      queryItems: [{ itemId: 'q1', sql: 'SELECT 1' }],
    });
    expect((last as Extract<AskEvent, { t: 'answer' }>).deepLink).toMatch(
      new RegExp(`^https://${HOST.replace(/\./g, '\\.')}/one/chat/threads/`),
    );
  });

  it('[UX-14a] key numbers are bold spans with a digit, verbatim, 3 or less', () => {
    expect(
      keyNumbersOf('Revenue was **$1.2M**, up **12.5 %** from **Q2**; **no** change.'),
    ).toEqual([{ text: '$1.2M' }, { text: '12.5 %' }, { text: 'Q2' }]);
    expect(keyNumbersOf('**1** **2** **3** **4**')).toHaveLength(3);
    expect(keyNumbersOf('Nothing bold 42.')).toEqual([]);
  });
});

describe('clarify follow-up (G-22, GL-3)', () => {
  it('[G-22] the next ask of the MOG within 5 min goes to the clarify conversation, and not after', async () => {
    const r = rig({
      responses: [scenarios.needsInput(), scenarios.needsInput(), scenarios.normalAnswer()],
    });
    const bridge = bridgeFor(r);
    const first = await submitted(r, request(), bridge, r.sched.clock.now() + MIN);
    const cid = (first[0] as Extract<AskEvent, { t: 'accepted' }>).conversationId;
    expect(first.at(-1)).toMatchObject({ t: 'needs_input', kind: 'clarify' });
    // 4 min later: the answer to the clarify question goes to the same conversation.
    await r.sched.runUntil(r.sched.clock.now() + 4 * MIN);
    const second = await submitted(
      r,
      request({ question: 'Net revenue' }),
      bridge,
      r.sched.clock.now() + MIN,
    );
    expect(r.double.asks()[1]?.args.conversation_id).toBe(cid);
    expect(second.at(-1)).toMatchObject({ t: 'needs_input', kind: 'clarify' });
    // More than 5 min after the second clarify: a new conversation.
    await r.sched.runUntil(r.sched.clock.now() + CLARIFY_FOLLOW_UP_MS + 1);
    await submitted(r, request({ question: 'Pipeline' }), bridge, r.sched.clock.now() + MIN);
    expect(r.double.asks()[2]?.args.conversation_id).toBeUndefined();
  });

  it('[G-22] the follow-up is used once, only by the same MOG and profile', () => {
    const f = createClarifyFollowUps();
    const MOG_B = '00000000-0000-4000-8000-0000000000b2' as AskRequest['mogId'];
    const mog = MOG as AskRequest['mogId'];
    f.note(mog, { conversationId: 'c1', profile: PROFILE }, 0);
    expect(f.take(MOG_B, PROFILE, 1)).toBeUndefined();
    expect(f.take(mog, 'other-profile', 1)).toBeUndefined();
    // A take by another profile clears it: that next utterance was not the clarify answer.
    expect(f.take(mog, PROFILE, 2)).toBeUndefined();
    f.note(mog, { conversationId: 'c2', profile: PROFILE }, 10);
    expect(f.take(mog, PROFILE, 10 + CLARIFY_FOLLOW_UP_MS)).toBe('c2');
    expect(f.take(mog, PROFILE, 10 + CLARIFY_FOLLOW_UP_MS)).toBeUndefined();
    f.note(mog, { conversationId: 'c3', profile: PROFILE }, 0);
    expect(f.take(mog, PROFILE, CLARIFY_FOLLOW_UP_MS + 1)).toBeUndefined();
  });

  it('[G-22] an explicit conversation ID wins over the clarify follow-up', async () => {
    const r = rig({ responses: [scenarios.needsInput(), scenarios.normalAnswer()] });
    const bridge = bridgeFor(r);
    await submitted(r, request(), bridge, r.sched.clock.now() + MIN);
    await submitted(
      r,
      request({ conversationId: 'conv-explicit' }),
      bridge,
      r.sched.clock.now() + MIN,
    );
    expect(r.double.asks()[1]?.args.conversation_id).toBe('conv-explicit');
  });
});
