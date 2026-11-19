/**
 * genie-followup (#145, #103, #584): sending a question from the hub thread composer is a true
 * Genie follow-up. It calls T8 `GenieBridge.submit` with the thread's exact `conversationId`, on
 * the MOG and the profile `findThread` resolves from the stored exchanges (ARC-2: never from
 * `FollowupInput.profile`, the view's own copy, which the composer never carries a `mogId`
 * alongside anyway) — a conversation stays in its workspace. It streams the same words the card
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
    findThread: async () => ({ mogId: MOG, profile: PROFILE }),
    sendStatus: (s) => void statuses.push(s),
    onEnd: (x) => void ends.push(x),
    newOpId: () => OP,
    statuses,
    ends,
    ...over,
  };
}

describe('genie-followup (#145)', () => {
  it("[#145] sends one genie_ask on the thread's exact conversationId, with the resolved mogId and profile", async () => {
    const b = fakeBridge([{ t: 'uncertain' }]);
    const findThread = vi.fn(async () => ({ mogId: MOG, profile: PROFILE }));
    const f = createGenieFollowup(deps({ genie: b.bridge, findThread }));
    await f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' });
    expect(findThread).toHaveBeenCalledWith(CONV);
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

  it('[#145] [ARC-2] a mismatched view profile never moves the conversation: the resolved one from findThread wins', async () => {
    const b = fakeBridge([{ t: 'uncertain' }]);
    const f = createGenieFollowup(
      deps({ genie: b.bridge, findThread: async () => ({ mogId: MOG, profile: 'fake-east' }) }),
    );
    // The view sends its own (stale or mismatched) copy of the profile.
    await f.send({ conversationId: CONV, profile: 'fake-west', question: 'How many are there?' });
    expect(b.reqs[0]?.binding).toEqual({ kind: 'genie_one_mcp', profile: 'fake-east' });
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
    const d = deps({ genie: b.bridge, findThread: async () => undefined });
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

  it('[#145] a failure carries the fixed genie-errors text and the question, so the composer can retry', async () => {
    const failed: AskEvent = {
      t: 'failed',
      code: 'AUTH_EXPIRED',
      userText: 'Sign in to fake.example.com to ask Genie.',
      retry: 'relogin',
    };
    const d = deps({ genie: fakeBridge([failed]).bridge });
    const f = createGenieFollowup(d);
    await f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' });
    expect(d.statuses.at(-1)).toEqual({
      conversationId: CONV,
      requestId: OP,
      phase: 'failed',
      text: 'Sign in to fake.example.com to ask Genie.',
      question: 'How many are there?',
    });
  });

  it('[#145] an uncertain end shows the fixed ASK_UNCERTAIN text, with the question', async () => {
    const d = deps({ genie: fakeBridge([{ t: 'uncertain' }]).bridge });
    const f = createGenieFollowup(d);
    await f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' });
    expect(d.statuses.at(-1)).toMatchObject({
      phase: 'failed',
      text: 'I do not know if Genie got your question.',
      question: 'How many are there?',
    });
  });

  it('[#145] a stale thread is refused with the question, so Try again can send it again', async () => {
    const d = deps({ findThread: async () => undefined });
    const f = createGenieFollowup(d);
    await f.send({ conversationId: CONV, profile: PROFILE, question: 'How many are there?' });
    expect(d.statuses).toEqual([
      {
        conversationId: CONV,
        requestId: '',
        phase: 'failed',
        text: 'Not sent.',
        question: 'How many are there?',
      },
    ]);
  });

  describe('access (the disabled state)', () => {
    it('[#145] a profile that needs a sign-in closes the composer, with the host from the store', async () => {
      const profileStatus = vi.fn(async () => ({ state: 'login_required' }));
      const f = createGenieFollowup(
        deps({
          findThread: async () => ({ mogId: MOG, profile: 'fake-east', host: 'fake.example.com' }),
          profileStatus,
        }),
      );
      await expect(f.access(CONV)).resolves.toEqual({
        conversationId: CONV,
        state: 'sign_in',
        text: 'Sign in to fake.example.com to ask Genie.',
        busy: false,
      });
      // The profile comes from the store, not from the view.
      expect(profileStatus).toHaveBeenCalledWith('fake-east');
    });

    it('[#145] a valid profile is ready', async () => {
      const f = createGenieFollowup(deps({ profileStatus: async () => ({ state: 'valid' }) }));
      await expect(f.access(CONV)).resolves.toMatchObject({ state: 'ready' });
    });

    it('[#145] a status that cannot be read, or a stale thread, never blocks the composer', async () => {
      const broken = createGenieFollowup(
        deps({ profileStatus: () => Promise.reject(new Error('fake')) }),
      );
      await expect(broken.access(CONV)).resolves.toMatchObject({ state: 'ready' });
      const stale = createGenieFollowup(
        deps({ findThread: async () => undefined, profileStatus: async () => ({ state: 'x' }) }),
      );
      await expect(stale.access(CONV)).resolves.toMatchObject({ state: 'ready' });
    });
  });

  describe('one request at a time for each thread', () => {
    function gated() {
      let release!: () => void;
      const gate = new Promise<void>((r) => (release = r));
      const reqs: AskRequest[] = [];
      const bridge: Pick<GenieBridge, 'submit'> = {
        submit(req) {
          reqs.push(req);
          return {
            opId: req.opId,
            events: (async function* (): AsyncGenerator<AskEvent> {
              await gate;
              yield { t: 'uncertain' };
            })(),
          };
        },
      };
      return { bridge, reqs, release };
    }

    it('[#145] a second send while one runs sends nothing, and says Genie is still answering', async () => {
      const g = gated();
      const d = deps({ genie: g.bridge });
      const f = createGenieFollowup(d);
      const first = f.send({ conversationId: CONV, profile: PROFILE, question: 'First?' });
      await vi.waitFor(() => expect(g.reqs).toHaveLength(1));
      await f.send({ conversationId: CONV, profile: PROFILE, question: 'Second?' });
      expect(g.reqs).toHaveLength(1);
      expect(d.statuses.at(-1)).toMatchObject({
        phase: 'working',
        text: 'Genie is still answering your last question here.',
      });
      // The thread says it is busy, for a composer that comes back meanwhile.
      await expect(f.access(CONV)).resolves.toMatchObject({ busy: true });
      g.release();
      await first;
      await expect(f.access(CONV)).resolves.toMatchObject({ busy: false });
      // And it takes the next one.
      await f.send({ conversationId: CONV, profile: PROFILE, question: 'Third?' });
      expect(g.reqs).toHaveLength(2);
    });

    it('[#145] another thread is not blocked', async () => {
      const g = gated();
      const f = createGenieFollowup(deps({ genie: g.bridge }));
      void f.send({ conversationId: CONV, profile: PROFILE, question: 'First?' });
      await vi.waitFor(() => expect(g.reqs).toHaveLength(1));
      void f.send({ conversationId: 'conv-2', profile: PROFILE, question: 'Other?' });
      await vi.waitFor(() => expect(g.reqs).toHaveLength(2));
      g.release();
    });
  });
});
