/**
 * #573 (ADR 0097): the Ask Genie press goes straight to T8 GenieBridge `submit` (Genie One MCP),
 * and its AskEvents show on the card of the owner MOG. No main agent takes part, and no failure
 * is sent again (G-15b).
 */
import type { AgentTurn, AskEvent, AskRequest, GenieBridge } from '../shared/interfaces';
import { scenarios } from '../test-support/genie-one-mcp';
import { MOG, PROFILE, rig } from '../test-support/poll-actor-rig';
import { routerDecision } from '../test-support/model-invoke';
import type { BlurbInput } from './blurb-stack';
import type { SectionInput } from './card-sections';
import { createGenieBridge } from './components/genie-bridge';
import { GIVE_UP_MS } from './components/poll-actor';
import { GENIE_DATA_FLOW_TEXT, createGenieDirectSend, workingLine } from './genie-direct';
import type { GenieEnd } from './genie-history';

const OP = '00000000-0000-4000-8000-00000000c573';
const Q = 'What was revenue last quarter?';
/** The audit write is out of scope here; a failed one never stops the ask. */
const noAudit = { record: () => Promise.reject(new Error('no audit in this test')) };

const turn = (over: Partial<AgentTurn> = {}): AgentTurn => ({
  mogId: MOG as AgentTurn['mogId'],
  question: Q,
  decision: routerDecision({ genie_question: Q }),
  recall: [],
  target: PROFILE,
  ...over,
});

/** A card double that logs every blurb change and the expanded content. */
function fakeCard() {
  const log: BlurbInput[] = [];
  const expanded: (SectionInput | undefined)[] = [];
  return {
    log,
    expanded,
    card: {
      add: (b: BlurbInput) => void log.push({ ...b }),
      setExpanded: (s: SectionInput | undefined) => void expanded.push(s),
    },
  };
}

/** A bridge double that records the request and plays the events. */
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

describe('genie direct send (#573, ADR 0097)', () => {
  it('[B1] [G-02] sends one Genie One MCP request with the exact question and the MOG tags', async () => {
    const c = fakeCard();
    const b = fakeBridge([{ t: 'failed', code: 'OFFLINE', userText: 'x', retry: 'none' }]);
    const send = createGenieDirectSend({
      genie: b.bridge,
      cardFor: () => c.card,
      newOpId: () => OP,
    });
    await send(turn({ question: 'What was EMEA revenue?' }));
    expect(b.reqs).toEqual([
      {
        opId: OP,
        mogId: MOG,
        binding: { kind: 'genie_one_mcp', profile: PROFILE },
        question: 'What was EMEA revenue?',
        tags: { app: 'bottle', mog: MOG, op: 'genie' },
      },
    ]);
  });

  it('[G-44] [G-17a] shows one working blurb, then the answer with its key numbers', async () => {
    const c = fakeCard();
    const b = fakeBridge([
      { t: 'accepted', conversationId: 'c1', responseId: 'r1' },
      { t: 'progress', steps: null, elapsedMs: 12_000 },
      { t: 'progress', steps: ['Reading the tables'], elapsedMs: 75_000 },
      {
        t: 'answer',
        md: 'Revenue was **$1.2M**.',
        incomplete: false,
        keyNumbers: [{ text: '$1.2M' }],
        deepLink: '',
        queryItems: [],
      },
    ]);
    const send = createGenieDirectSend({
      genie: b.bridge,
      cardFor: () => c.card,
      newOpId: () => OP,
    });
    await send(turn());
    // One blurb ID for the whole request: the working line is replaced, not stacked.
    expect(new Set(c.log.map((x) => x.id)).size).toBe(1);
    expect(c.log.map((x) => x.text)).toEqual([
      workingLine(PROFILE),
      'Working\u2026 0:12',
      'Working\u2026 1:15',
      'Revenue was **$1.2M**.',
    ]);
    expect(workingLine(PROFILE)).toBe(`Asking Genie in this workspace (profile ${PROFILE})`);
    expect(c.log.slice(0, -1).every((x) => !x.completed)).toBe(true);
    expect(c.log.at(-1)?.completed).toBe(true);
    expect(c.expanded).toEqual([
      {
        answerMd: 'Revenue was **$1.2M**.',
        keyNumbers: ['$1.2M'],
        dataFlowText: GENIE_DATA_FLOW_TEXT,
      },
    ]);
  });

  it('[G-14] [#573] a failure shows the fixed failure text and nothing is sent again', async () => {
    const r = rig({ rejectAsk: () => ({ error: 'not_implemented' }) });
    const c = fakeCard();
    const send = createGenieDirectSend({
      genie: createGenieBridge({ call: r.call, conversations: r.cm, audit: noAudit }),
      cardFor: () => c.card,
      newOpId: () => OP,
    });
    const done = send(turn());
    await r.sched.runUntil(r.sched.clock.now() + GIVE_UP_MS, { settle: true });
    await done;
    expect(r.sent.filter((s) => s.args.tool === 'genie_ask')).toHaveLength(1);
    expect(r.sent).toHaveLength(1);
    const last = c.log.at(-1);
    expect(last?.completed).toBe(true);
    expect(last?.text).toMatch(/\nCode [A-Z_]+$/);
    expect(last?.text).not.toContain('not_implemented');
    expect(c.expanded).toEqual([]);
  });

  it('[G-02] [G-07b] on the Genie One double: one genie_ask, the working line, then the answer', async () => {
    const r = rig({ responses: [scenarios.normalAnswer()] });
    const c = fakeCard();
    const send = createGenieDirectSend({
      genie: createGenieBridge({
        call: r.call,
        conversations: r.cm,
        now: () => r.sched.clock.now(),
        audit: noAudit,
      }),
      cardFor: () => c.card,
      newOpId: () => OP,
    });
    const done = send(turn());
    await r.sched.runUntil(r.sched.clock.now() + GIVE_UP_MS, { settle: true });
    await done;
    const asks = r.sent.filter((s) => s.args.tool === 'genie_ask');
    expect(asks).toHaveLength(1);
    expect(asks[0]?.args).toMatchObject({
      profile: PROFILE,
      tool: 'genie_ask',
      args: { question: Q },
    });
    expect(c.log[0]?.text).toBe(workingLine(PROFILE));
    expect(c.log.at(-1)).toMatchObject({
      text: 'Revenue last quarter was **$1.2M**.',
      completed: true,
    });
    expect(c.expanded.at(-1)).toMatchObject({ keyNumbers: ['$1.2M'] });
  });
});

describe('genie direct send: the end of each request goes to the local history (#40)', () => {
  const answer: AskEvent = {
    t: 'answer',
    md: 'Revenue was **$1.2M**.',
    incomplete: false,
    keyNumbers: [{ text: '$1.2M' }],
    deepLink: 'https://example.cloud.databricks.test/one/chat/threads/c1?o=1',
    queryItems: [],
  };

  it('[AT-09] [G-21b] calls onEnd once, after the card, with the exact question and the ids', async () => {
    const c = fakeCard();
    const b = fakeBridge([{ t: 'accepted', conversationId: 'c1', responseId: 'r1' }, answer]);
    const ends: GenieEnd[] = [];
    const send = createGenieDirectSend({
      genie: b.bridge,
      cardFor: () => c.card,
      newOpId: () => OP,
      onEnd: (x) => {
        // The card already shows the answer when the history write starts.
        expect(c.log.at(-1)?.completed).toBe(true);
        ends.push(x);
      },
    });
    await send(turn({ question: 'What was EMEA revenue?' }));
    expect(ends).toEqual([
      {
        opId: OP,
        mogId: MOG,
        profile: PROFILE,
        question: 'What was EMEA revenue?',
        conversationId: 'c1',
        responseId: 'r1',
        end: answer,
      },
    ]);
  });

  it('[AT-09] a stream that breaks ends as uncertain, once', async () => {
    const c = fakeCard();
    const ends: GenieEnd[] = [];
    const send = createGenieDirectSend({
      genie: {
        submit: (req) => ({
          opId: req.opId,
          events: (async function* (): AsyncGenerator<AskEvent> {
            yield { t: 'progress', steps: null, elapsedMs: 1000 };
            throw new Error('fake client fault');
          })(),
        }),
      },
      cardFor: () => c.card,
      newOpId: () => OP,
      onEnd: (x) => void ends.push(x),
    });
    await send(turn());
    expect(ends.map((x) => x.end)).toEqual([{ t: 'uncertain' }]);
    expect(ends[0]?.conversationId).toBeUndefined();
  });

  it('[AT-09] a throw in onEnd never changes the card or rejects the send', async () => {
    const c = fakeCard();
    const b = fakeBridge([answer]);
    const send = createGenieDirectSend({
      genie: b.bridge,
      cardFor: () => c.card,
      newOpId: () => OP,
      onEnd: () => {
        throw new Error('fake history fault');
      },
    });
    await expect(send(turn())).resolves.toBeUndefined();
    expect(c.log.at(-1)).toMatchObject({ text: 'Revenue was **$1.2M**.', completed: true });
    expect(c.log.map((x) => x.text)).toHaveLength(2);
  });

  it('[AT-09] [B1] no onEnd call for a request that was never sent', async () => {
    const ends: GenieEnd[] = [];
    const send = createGenieDirectSend({
      genie: {
        submit: () => {
          throw new Error('fake submit refusal');
        },
      },
      cardFor: () => fakeCard().card,
      newOpId: () => OP,
      onEnd: (x) => void ends.push(x),
    });
    expect(() => send(turn())).toThrow();
    await new Promise((r) => setTimeout(r, 0));
    expect(ends).toEqual([]);
  });
});
