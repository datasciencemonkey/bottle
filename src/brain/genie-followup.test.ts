/**
 * genie-followup (#145, #103, #584): sending a question from the hub thread composer is a true
 * Genie follow-up. It calls T8 `GenieBridge.submit` with the thread's exact `conversationId` and
 * `profile` (a conversation stays in its workspace), resolves the owning MOG from the stored
 * exchanges (the composer itself never carries a `mogId`), and streams the same words the card
 * shows (`ASKING_GENIE`, then a fixed stage) to the hub as `ThreadStatus`. The terminal event is
 * saved through the same local-history writer as a direct ask (`onEnd`), so the thread's own
 * list shows the finished exchange.
 */
import type { AskEvent, AskRequest, GenieBridge } from '../shared/interfaces';
import type { ThreadStatus } from '../shared/hub-conversations';
import { ASKING_GENIE } from '../shared/genie-status';
import { createGenieFollowup, type GenieFollowupDeps } from './genie-followup';
import type { GenieEnd } from './genie-history';

const CONV = 'conv-1';
const PROFILE = 'fake-west';
const MOG = '00000000-0000-4000-8000-00000000000a';
const OP = '00000000-0000-4000-8000-00000000f001';

function fakeBridge(events: AskEvent[]) {
  const reqs: AskRequest[] = [];
  const bridge: Pick<GenieBridge, 'submit'> = {
    submit(req) {
      reqs.push(req);
      return {
        opId: req.opId,
        events: (async function* () {
          for (const e of events) yield e;
        })(),
      };
    },
  };
  return { bridge, reqs };
}

function deps(over: Partial<GenieFollowupDeps> = {}): GenieFollowupDeps & {
  statuses: ThreadStatus[];
  ends: GenieEnd[];
} {
  const statuses: ThreadStatus[] = [];
  const ends: GenieEnd[] = [];
  return {
    genie: fakeBridge([{ t: 'uncertain' }]).bridge,
    findMogId: async () => MOG,
    sendStatus: (s) => void statuses.push(s),
    onEnd: (x) => void ends.push(x),
    newOpId: () => OP,
    statuses,
    ends,
    ...over,
  };
}

describe('genie-followup (#145)', () => {
  it("[#145] sends one genie_ask on the thread's exact conversationId and profile, with the resolved mogId", async () => {
    const b = fakeBridge([{ t: 'uncertain' }]);
    const findMogId = vi.fn(async () => MOG);
    const f = createGenieFollowup(deps({ genie: b.bridge, findMogId }));
    await f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' });
    expect(findMogId).toHaveBeenCalledWith(CONV);
    expect(b.reqs).toEqual([
      {
        opId: OP,
        mogId: MOG,
        binding: { kind: 'genie_one_mcp', profile: PROFILE },
        question: 'How many are there?',
        conversationId: CONV,
        tags: { app: 'bottle', mog: MOG, op: 'genie' },
      },
    ]);
  });

  it('[#145] shows ASKING_GENIE at the send, then a fixed stage line, same as the card', async () => {
    const b = fakeBridge([
      { t: 'accepted', conversationId: CONV, responseId: 'r1' },
      { t: 'progress', steps: null, elapsedMs: 1_000 },
      { t: 'progress', steps: ['Running SQL: SELECT fake'], elapsedMs: 2_000 },
      {
        t: 'answer',
        md: '**152** catalogs.',
        incomplete: false,
        keyNumbers: [{ text: '152', label: 'catalogs' }],
        deepLink: '',
        queryItems: [],
      },
    ]);
    const d = deps({ genie: b.bridge });
    const f = createGenieFollowup(d);
    await f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' });
    expect(d.statuses.map((s) => ({ phase: s.phase, text: s.text }))).toEqual([
      { phase: 'working', text: ASKING_GENIE },
      { phase: 'working', text: 'Running the query' },
      { phase: 'answered', text: '' },
    ]);
    expect(d.statuses.every((s) => s.conversationId === CONV && s.requestId === OP)).toBe(true);
  });

  it('[#145] saves the terminal event through onEnd, with the accepted ids and the resolved mogId', async () => {
    const answer: AskEvent = {
      t: 'answer',
      md: '**152** catalogs.',
      incomplete: false,
      keyNumbers: [{ text: '152' }],
      deepLink: 'https://example.cloud.databricks.test/one/chat/threads/conv-1?o=1',
      queryItems: [],
    };
    const b = fakeBridge([{ t: 'accepted', conversationId: CONV, responseId: 'r9' }, answer]);
    const d = deps({ genie: b.bridge });
    const f = createGenieFollowup(d);
    await f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' });
    expect(d.ends).toEqual([
      {
        opId: OP,
        mogId: MOG,
        profile: PROFILE,
        question: 'How many are there?',
        conversationId: CONV,
        responseId: 'r9',
        end: answer,
      },
    ]);
  });

  it('[#145] a failure shows the failed phase and still saves the terminal event', async () => {
    const failed: AskEvent = {
      t: 'failed',
      code: 'GENIE_FAILED',
      userText: 'Genie could not answer this. Try again.',
      retry: 'user',
    };
    const d = deps({ genie: fakeBridge([failed]).bridge });
    const f = createGenieFollowup(d);
    await f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' });
    expect(d.statuses.at(-1)?.phase).toBe('failed');
    expect(d.ends).toHaveLength(1);
    expect(d.ends[0]?.end).toEqual(failed);
  });

  it('[#145] a blank or sanitized-away question is never sent', async () => {
    const b = fakeBridge([{ t: 'uncertain' }]);
    const d = deps({ genie: b.bridge });
    const f = createGenieFollowup(d);
    await f.send({ conversationId: CONV, profile: PROFILE, question: '   ' });
    expect(b.reqs).toHaveLength(0);
    expect(d.statuses).toEqual([
      { conversationId: CONV, requestId: '', phase: 'failed', text: 'Not sent.' },
    ]);
    expect(d.ends).toEqual([]);
  });

  it('[G-30a] a question over the cap is cut, like the card chip ask', async () => {
    const b = fakeBridge([{ t: 'uncertain' }]);
    const d = deps({ genie: b.bridge });
    const f = createGenieFollowup(d);
    const long = 'x'.repeat(1_200);
    await f.send({ conversationId: CONV, profile: PROFILE, question: long });
    expect(b.reqs[0]?.question.length).toBeLessThanOrEqual(1_000);
  });

  it('[#145] a thread with no resolvable mogId (a stale or deleted thread) sends nothing', async () => {
    const b = fakeBridge([{ t: 'uncertain' }]);
    const d = deps({ genie: b.bridge, findMogId: async () => undefined });
    const f = createGenieFollowup(d);
    await f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' });
    expect(b.reqs).toHaveLength(0);
    expect(d.statuses.at(-1)?.phase).toBe('failed');
    expect(d.ends).toEqual([]);
  });

  it('[#145] a throw from submit never rejects send, and shows failed', async () => {
    const d = deps({
      genie: {
        submit: () => {
          throw new Error('fake submit refusal');
        },
      },
    });
    const f = createGenieFollowup(d);
    await expect(
      f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' }),
    ).resolves.toBeUndefined();
    expect(d.statuses.at(-1)?.phase).toBe('failed');
  });

  it('[#145] a throw in onEnd never rejects send', async () => {
    const answer: AskEvent = {
      t: 'answer',
      md: '**152**.',
      incomplete: false,
      keyNumbers: [],
      deepLink: '',
      queryItems: [],
    };
    const d = deps({
      genie: fakeBridge([answer]).bridge,
      onEnd: () => {
        throw new Error('fake history fault');
      },
    });
    const f = createGenieFollowup(d);
    await expect(
      f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' }),
    ).resolves.toBeUndefined();
  });

  it('[#145] a stream that breaks ends as uncertain, and still shows a terminal status', async () => {
    const d = deps({
      genie: {
        submit: (req) => ({
          opId: req.opId,
          events: (async function* (): AsyncGenerator<AskEvent> {
            yield { t: 'progress', steps: null, elapsedMs: 1_000 };
            throw new Error('fake client fault');
          })(),
        }),
      },
    });
    const f = createGenieFollowup(d);
    await f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' });
    expect(d.ends.map((x) => x.end)).toEqual([{ t: 'uncertain' }]);
    expect(d.statuses.at(-1)?.phase).toBe('failed');
  });
});
