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
import type { ChipView } from './card-chips';
import type { SectionInput } from './card-sections';
import { createGenieBridge } from './components/genie-bridge';
import { GIVE_UP_MS } from './components/poll-actor';
import { ANSWER_READY, ASKING_GENIE } from '../shared/genie-status';
import {
  ASK_OTHERS_LABEL,
  CHANGE_WORKSPACE_LABEL,
  GENIE_DATA_FLOW_TEXT,
  TRY_AGAIN_LABEL,
  createGenieDirectSend,
} from './genie-direct';
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

/** A card double that logs every blurb change, the expanded content and the controls. */
function fakeCard() {
  const log: BlurbInput[] = [];
  const expanded: (SectionInput | undefined)[] = [];
  const controls = new Map<string, { list: ChipView[]; onClick?: (id: string) => void }>();
  return {
    log,
    expanded,
    controls,
    card: {
      add: (b: BlurbInput) => void log.push({ ...b }),
      setExpanded: (s: SectionInput | undefined) => void expanded.push(s),
      setControls: (owner: string, list: ChipView[], onClick?: (id: string) => void) =>
        void controls.set(owner, onClick ? { list, onClick } : { list }),
      clearControls: (owner: string) => void controls.delete(owner),
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

  it('[G-44] [G-17a] shows one status blurb, then the answer with its key numbers', async () => {
    const c = fakeCard();
    const b = fakeBridge([
      { t: 'accepted', conversationId: 'c1', responseId: 'r1' },
      { t: 'progress', steps: null, elapsedMs: 12_000 },
      { t: 'progress', steps: ['Running SQL: SELECT fake'], elapsedMs: 20_000 },
      { t: 'progress', steps: ['Running SQL: SELECT fake'], elapsedMs: 25_000 },
      { t: 'progress', steps: ['Running SQL: x', 'Query result: | a |'], elapsedMs: 75_000 },
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
    // G-17b: only fixed stage lines show, each once; the view runs the m:ss timer.
    expect(c.log.map((x) => x.text)).toEqual([
      ASKING_GENIE,
      'Running the query',
      'Reading the results',
      'Revenue was **$1.2M**.',
    ]);
    expect(c.log.slice(0, -1).every((x) => !x.completed)).toBe(true);
    expect(c.log.at(-1)?.completed).toBe(true);
    // The old answer is cleared at the send, so a new request never shows a stale answer.
    expect(c.expanded).toEqual([
      undefined,
      {
        answerMd: 'Revenue was **$1.2M**.',
        keyNumbers: ['$1.2M'],
        dataFlowText: GENIE_DATA_FLOW_TEXT,
      },
    ]);
  });

  it('[#523] the card gets the deep_link of the answer and the workspace host that it trusts', async () => {
    const c = fakeCard();
    const DEEP = 'https://example.cloud.databricks.test/one/chat/threads/c1?o=1';
    const b = fakeBridge([
      {
        t: 'answer',
        md: 'Revenue was **$1.2M**.',
        incomplete: false,
        keyNumbers: [],
        deepLink: DEEP,
        queryItems: [],
      },
    ]);
    const hosts: string[] = [];
    const send = createGenieDirectSend({
      genie: b.bridge,
      cardFor: () => c.card,
      newOpId: () => OP,
      hostOf: async (p) => {
        hosts.push(p);
        return 'https://example.cloud.databricks.test';
      },
    });
    await send(turn());
    expect(hosts).toEqual([PROFILE]);
    expect(c.expanded.at(-1)).toEqual({
      answerMd: 'Revenue was **$1.2M**.',
      keyNumbers: [],
      deepLink: DEEP,
      workspaceHost: 'https://example.cloud.databricks.test',
      dataFlowText: GENIE_DATA_FLOW_TEXT,
    });
  });

  it('[#523] a host lookup that fails still shows the answer, with no trusted host', async () => {
    const c = fakeCard();
    const b = fakeBridge([
      { t: 'answer', md: 'Fine.', incomplete: false, keyNumbers: [], deepLink: '', queryItems: [] },
    ]);
    const send = createGenieDirectSend({
      genie: b.bridge,
      cardFor: () => c.card,
      newOpId: () => OP,
      hostOf: async () => Promise.reject(new Error('profiles')),
    });
    await send(turn());
    expect(c.expanded.at(-1)).toEqual({
      answerMd: 'Fine.',
      keyNumbers: [],
      dataFlowText: GENIE_DATA_FLOW_TEXT,
    });
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
    expect(c.expanded).toEqual([undefined]);
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
    expect(c.log[0]?.text).toBe(ASKING_GENIE);
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

  it('[AT-09] [G-21b] calls onEnd once, at the terminal event (it is no arrival), with the exact question and the ids', async () => {
    const c = fakeCard();
    const b = fakeBridge([{ t: 'accepted', conversationId: 'c1', responseId: 'r1' }, answer]);
    const ends: GenieEnd[] = [];
    const send = createGenieDirectSend({
      genie: b.bridge,
      cardFor: () => c.card,
      newOpId: () => OP,
      onEnd: (x) => {
        // The history write is no arrival: it does not wait for the card update (UX slice 1).
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

describe('genie direct send: the MOG shows the work and the answer (#573)', () => {
  const answer: AskEvent = {
    t: 'answer',
    md: 'Revenue was **$1.2M**.',
    incomplete: false,
    keyNumbers: [{ text: '$1.2M' }],
    deepLink: '',
    queryItems: [],
  };
  const failed: AskEvent = {
    t: 'failed',
    code: 'GENIE_FAILED',
    userText: 'Genie could not answer this. Try again.',
    retry: 'user',
  };

  it('[UX-01a] reports working at the send, then answered; it shows the card after the answer', async () => {
    const c = fakeCard();
    const log: string[] = [];
    const send = createGenieDirectSend({
      genie: fakeBridge([{ t: 'progress', steps: null, elapsedMs: 2_000 }, answer]).bridge,
      cardFor: () => {
        log.push('card');
        return c.card;
      },
      newOpId: () => OP,
      onPhase: (mogId, phase) => void log.push(`${mogId}:${phase}`),
      showCard: (mogId) => {
        log.push(`show:${mogId}:${c.log.at(-1)?.completed === true}:${c.expanded.length}`);
        return Promise.resolve();
      },
    });
    await send(turn());
    expect(log.filter((x) => x !== 'card')).toEqual([
      `${MOG}:working`,
      // R11 places the card while it still works and already has the answer sections; then the
      // outcome goes on it in one step.
      `show:${MOG}:false:2`,
      `${MOG}:answered`,
    ]);
    expect(c.log.at(-1)?.completed).toBe(true);
  });

  it('[GL-5] [G-15b] a failure shows its fixed text with Try again; only a click sends again', async () => {
    const c = fakeCard();
    const phases: string[] = [];
    let n = 0;
    const b = fakeBridge([failed]);
    const send = createGenieDirectSend({
      genie: b.bridge,
      cardFor: () => c.card,
      newOpId: () => `op-${++n}`,
      onPhase: (_m, p) => void phases.push(p),
      showCard: () => undefined,
    });
    await send(turn());
    expect(b.reqs).toHaveLength(1);
    expect(phases).toEqual(['working', 'failed']);
    expect(c.log.at(-1)?.text.split('\n')[0]).toBe(failed.userText);
    const ctl = c.controls.get('retry:genie:op-1');
    expect(ctl?.list).toEqual([{ id: 'retry:genie:op-1', label: TRY_AGAIN_LABEL }]);
    // Nothing goes again by itself (G-15b).
    await new Promise((r) => setTimeout(r, 0));
    expect(b.reqs).toHaveLength(1);
    ctl?.onClick?.('retry:genie:op-1');
    await new Promise((r) => setTimeout(r, 0));
    expect(b.reqs.map((r) => [r.opId, r.question])).toEqual([
      ['op-1', Q],
      ['op-2', Q],
    ]);
    // The control goes with the click, so a second click sends nothing.
    expect(c.controls.has('retry:genie:op-1')).toBe(false);
  });

  it('[GL-5] a workspace failure adds a Change workspace link; it opens that MOG and keeps the request', async () => {
    const c = fakeCard();
    const changed: string[] = [];
    const b = fakeBridge([{ ...failed, code: 'PROXY_PAGE', userText: 'x' }]);
    const send = createGenieDirectSend({
      genie: b.bridge,
      cardFor: () => c.card,
      newOpId: () => 'op-1',
      showCard: () => undefined,
      changeWorkspace: (m) => void changed.push(m),
    });
    await send(turn());
    const ctl = c.controls.get('retry:genie:op-1');
    expect(ctl?.list).toEqual([
      { id: 'retry:genie:op-1', label: TRY_AGAIN_LABEL },
      { id: 'workspace:genie:op-1', label: CHANGE_WORKSPACE_LABEL },
    ]);
    ctl?.onClick?.('workspace:genie:op-1');
    await new Promise((r) => setTimeout(r, 0));
    expect(changed).toEqual([MOG]);
    // It sends nothing again and Try again stays.
    expect(b.reqs).toHaveLength(1);
    expect(c.controls.has('retry:genie:op-1')).toBe(true);
  });

  it('[GL-5] a failure that is not about the workspace has no Change workspace link', async () => {
    const c = fakeCard();
    const send = createGenieDirectSend({
      genie: fakeBridge([failed]).bridge,
      cardFor: () => c.card,
      newOpId: () => 'op-1',
      showCard: () => undefined,
      changeWorkspace: () => undefined,
    });
    await send(turn());
    expect(c.controls.get('retry:genie:op-1')?.list).toEqual([
      { id: 'retry:genie:op-1', label: TRY_AGAIN_LABEL },
    ]);
  });

  it('[G-17b] [#573] the live reply shape (fake values): stage lines, then the markdown answer', async () => {
    const r = rig({ responses: [scenarios.liveShape()] });
    const c = fakeCard();
    const phases: string[] = [];
    const send = createGenieDirectSend({
      genie: createGenieBridge({
        call: r.call,
        conversations: r.cm,
        now: () => r.sched.clock.now(),
        audit: noAudit,
      }),
      cardFor: () => c.card,
      newOpId: () => OP,
      onPhase: (_m, p) => void phases.push(p),
      showCard: () => undefined,
    });
    const done = send(turn());
    await r.sched.runUntil(r.sched.clock.now() + GIVE_UP_MS, { settle: true });
    await done;
    expect(r.sent.filter((s) => s.args.tool === 'genie_ask')).toHaveLength(1);
    expect(c.log.map((x) => x.text).slice(0, -1)).toEqual([
      ASKING_GENIE,
      'Running the query',
      'Reading the results',
      'Thinking it through',
      'Running the query',
    ]);
    // No raw step (SQL, rows, names) reaches the card.
    expect(c.log.some((x) => /SELECT|fake_catalog|\|/.test(x.text.split('\n')[0] ?? ''))).toBe(
      false,
    );
    expect(c.log.at(-1)).toMatchObject({ completed: true });
    expect(c.log.at(-1)?.text).toContain('## Tables you can access');
    expect(c.expanded.at(-1)).toMatchObject({
      answerMd: expect.stringContaining('**8,800 tables**') as unknown,
      keyNumbers: ['8,800 tables', '150 catalogs'],
    });
    expect(phases).toEqual(['working', 'answered']);
  });

  describe('UX slice 1: the arrival gate', () => {
    const ANSWER: AskEvent = {
      t: 'answer',
      md: 'Churn was 4.2%.',
      incomplete: false,
      keyNumbers: [{ text: '4.2%' }],
      deepLink: '',
      queryItems: [],
    };
    const FAILED: AskEvent = { t: 'failed', code: 'OFFLINE', userText: 'x', retry: 'none' };

    /** A gate double: `held` decides; held functions wait in `waiting` until `release`. */
    function gateDouble(held: boolean) {
      let h = held;
      const waiting: Array<() => void> = [];
      return {
        arrivals: {
          isHeld: () => h,
          whenClear: (fn: () => void) => (h ? void waiting.push(fn) : fn()),
        },
        release() {
          h = false;
          for (const fn of waiting.splice(0)) fn();
        },
        waiting,
      };
    }

    it('[AC-1.2] [AC-1.3] an answer in a hold changes only the pill to "Answer ready" (the history write still runs)', async () => {
      const c = fakeCard();
      const g = gateDouble(true);
      const phases: string[] = [];
      const shows: string[] = [];
      const ends: GenieEnd[] = [];
      const send = createGenieDirectSend({
        genie: fakeBridge([ANSWER]).bridge,
        cardFor: () => c.card,
        newOpId: () => OP,
        arrivals: g.arrivals,
        onPhase: (_m, p) => void phases.push(p),
        showCard: (m) => void shows.push(m),
        onEnd: (x) => void ends.push(x),
      });
      let done = false;
      const p = send(turn()).then(() => (done = true));
      await new Promise((r) => setTimeout(r, 0));
      expect(c.log.map((x) => x.text)).toEqual([ASKING_GENIE, ANSWER_READY]);
      expect(c.log.every((x) => !x.completed)).toBe(true);
      // Only the clearing at the send: no sections, no outcome blurb, no show, no mood phase.
      expect(c.expanded).toEqual([undefined]);
      expect(shows).toEqual([]);
      expect(phases).toEqual(['working']);
      // #40: the history write is no arrival. It runs at the terminal event.
      expect(ends).toHaveLength(1);
      expect(done).toBe(false);

      g.release();
      await p;
      expect(shows).toEqual([MOG]);
      expect(phases).toEqual(['working', 'answered']);
      expect(c.log.at(-1)).toMatchObject({ text: 'Churn was 4.2%.', completed: true });
      expect(c.expanded.at(-1)).toMatchObject({ answerMd: 'Churn was 4.2%.' });
      expect(ends).toHaveLength(1);
    });

    it('[AC-1.9] the pill of the own ask shows at once, in a hold', () => {
      const c = fakeCard();
      const g = gateDouble(true);
      const send = createGenieDirectSend({
        genie: fakeBridge([ANSWER]).bridge,
        cardFor: () => c.card,
        newOpId: () => OP,
        arrivals: g.arrivals,
      });
      void send(turn());
      expect(c.log.map((x) => x.text)).toEqual([ASKING_GENIE]);
    });

    it('[AC-1.10] two held answers apply in arrival order, and the newest shows on the card', async () => {
      const c = fakeCard();
      const g = gateDouble(true);
      const order: string[] = [];
      let n = 0;
      const send = createGenieDirectSend({
        genie: {
          submit(req) {
            const md = `answer ${++n}`;
            return {
              opId: req.opId,
              events: (async function* () {
                yield { ...ANSWER, md } as AskEvent;
              })(),
            };
          },
        },
        cardFor: () => c.card,
        newOpId: () => `op-${n + 1}`,
        arrivals: g.arrivals,
        onPhase: (_m, p, op) => void (p !== 'working' && order.push(op)),
      });
      const both = Promise.all([send(turn()), send(turn())]);
      await new Promise((r) => setTimeout(r, 0));
      expect(order).toEqual([]);
      g.release();
      await both;
      expect(order).toEqual(['op-1', 'op-2']);
      expect(c.log.at(-1)?.text).toBe('answer 2');
    });

    it('[AC-1.11] a failure waits for the gate too, and keeps the working pill', async () => {
      const c = fakeCard();
      const g = gateDouble(true);
      const phases: string[] = [];
      const send = createGenieDirectSend({
        genie: fakeBridge([FAILED]).bridge,
        cardFor: () => c.card,
        newOpId: () => OP,
        arrivals: g.arrivals,
        onPhase: (_m, p) => void phases.push(p),
        showCard: () => undefined,
      });
      const p = send(turn());
      await new Promise((r) => setTimeout(r, 0));
      expect(c.log.map((x) => x.text)).toEqual([ASKING_GENIE]);
      expect(c.controls.size).toBe(0);
      expect(phases).toEqual(['working']);
      g.release();
      await p;
      expect(phases).toEqual(['working', 'failed']);
      expect(c.controls.size).toBe(1);
    });

    it('with no hold the arrival applies at once, with no "Answer ready" pill', async () => {
      const c = fakeCard();
      const g = gateDouble(false);
      const send = createGenieDirectSend({
        genie: fakeBridge([ANSWER]).bridge,
        cardFor: () => c.card,
        newOpId: () => OP,
        arrivals: g.arrivals,
      });
      await send(turn());
      expect(c.log.map((x) => x.text)).toEqual([ASKING_GENIE, 'Churn was 4.2%.']);
    });
  });

  describe('slice 2a: the fallback notice and the "Ask other workspaces too" chip', () => {
    const answerEvent: AskEvent = {
      t: 'answer',
      md: 'Churn was 4.2%.',
      incomplete: false,
      keyNumbers: [],
      deepLink: '',
      queryItems: [],
    };

    it('[AC-2a.11] a fallback shows its one-line reason before the answer, and not in the saved end', async () => {
      const c = fakeCard();
      const ends: GenieEnd[] = [];
      const send = createGenieDirectSend({
        genie: fakeBridge([answerEvent]).bridge,
        cardFor: () => c.card,
        newOpId: () => OP,
        onEnd: (x) => void ends.push(x),
      });
      await send(turn(), { notice: 'Genie Code is not installed. Asked sales only.' });
      expect(c.expanded.at(-1)?.answerMd).toBe(
        '*Genie Code is not installed. Asked sales only.*\n\nChurn was 4.2%.',
      );
      // History keeps the answer as Genie wrote it. "Remember this" reads the saved exchange
      // (`remember-flow.ts`, `finalAnswerMd`), so neither one ever holds the notice.
      expect(ends[0]?.end).toMatchObject({ t: 'answer', md: 'Churn was 4.2%.' });
      expect(JSON.stringify(ends)).not.toContain('Genie Code is not installed');
    });

    it('[R-8] an answered request offers the chip, with the request id in the chip id', async () => {
      const c = fakeCard();
      const offered: string[] = [];
      const send = createGenieDirectSend({
        genie: fakeBridge([answerEvent]).bridge,
        cardFor: () => c.card,
        newOpId: () => OP,
        offerOthers: (_t, opId) => {
          offered.push(opId);
          return { id: `agent-more:${opId}`, label: ASK_OTHERS_LABEL };
        },
      });
      await send(turn());
      expect(offered).toEqual([OP]);
      expect([...c.controls.values()].flatMap((x) => x.list)).toEqual([
        { id: `agent-more:${OP}`, label: 'Ask other workspaces too' },
      ]);
    });

    it('[R-8] a failed request offers no chip, and the next request takes the old chip away', async () => {
      const c = fakeCard();
      const events: AskEvent[] = [answerEvent];
      const send = createGenieDirectSend({
        genie: {
          submit: (req) => ({
            opId: req.opId,
            events: (async function* () {
              for (const e of events) yield e;
            })(),
          }),
        },
        cardFor: () => c.card,
        newOpId: () => OP,
        offerOthers: (_t, opId) => ({ id: `agent-more:${opId}`, label: ASK_OTHERS_LABEL }),
      });
      await send(turn());
      expect(c.controls.size).toBe(1);
      events[0] = { t: 'failed', code: 'OFFLINE', userText: 'x', retry: 'none' };
      await send(turn());
      // The old chip left at the send; the failed card has only its Try again control.
      expect([...c.controls.keys()].every((k) => k.startsWith('retry:'))).toBe(true);
    });

    it('[R-8] with no offer (the agent cannot run) the card has no extra control', async () => {
      const c = fakeCard();
      const send = createGenieDirectSend({
        genie: fakeBridge([answerEvent]).bridge,
        cardFor: () => c.card,
        newOpId: () => OP,
        offerOthers: () => undefined,
      });
      await send(turn());
      expect(c.controls.size).toBe(0);
    });
  });
});
