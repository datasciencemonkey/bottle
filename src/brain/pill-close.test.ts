/**
 * The close (x) of the Genie status pill, on the brain side: a close of the WORKING pill only
 * hides the card window. The request runs on, its blurb stays, and the answer opens the card
 * again (owner decision). The talk-key arrival gate still applies. A close of the READY pill is a
 * real close. Real card stack and sim clock; the Genie bridge and the window host are doubles.
 */
import type { AgentTurn, AskEvent } from '../shared/interfaces';
import { ANSWER_READY } from '../shared/genie-status';
import { createSimScheduler } from '../test-support/sim-scheduler';
import { routerDecision } from '../test-support/model-invoke';
import { MOG, PROFILE } from '../test-support/poll-actor-rig';
import { BrainBus } from './bus';
import { createCardStack, type CardStackUpdate } from './card-stack';
import { createGenieDirectSend } from './genie-direct';
import { wireGenieCard } from './genie-card-wiring';

const ANSWER: AskEvent = {
  t: 'answer',
  md: 'Churn was 4.2%.',
  incomplete: false,
  keyNumbers: [{ text: '4.2%' }],
  deepLink: 'https://acme.cloud.databricks.test/one/chat/threads/c1?o=1',
  queryItems: [],
};

const turn = (): AgentTurn => ({
  mogId: MOG as AgentTurn['mogId'],
  question: 'What was churn?',
  decision: routerDecision({ genie_question: 'What was churn?' }),
  recall: [],
  target: PROFILE,
});

function rig(held = false) {
  const sim = createSimScheduler();
  const bus = new BrainBus();
  const updates: CardStackUpdate[] = [];
  const log: string[] = [];
  const stack = createCardStack(MOG, {
    clock: sim.clock,
    schedule: sim.at,
    send: (u) => void updates.push(u),
    openHistory: () => undefined,
  });
  let isHeld = held;
  const waiting: Array<() => void> = [];
  const events: Array<(e: AskEvent) => void> = [];
  let n = 0;
  const send = createGenieDirectSend({
    genie: {
      submit: (req) => {
        const queue: AskEvent[] = [];
        let wake: (() => void) | undefined;
        events.push((e) => {
          queue.push(e);
          wake?.();
        });
        return {
          opId: req.opId,
          events: (async function* () {
            for (;;) {
              while (queue.length === 0) await new Promise<void>((r) => (wake = r));
              yield queue.shift() as AskEvent;
            }
          })(),
        };
      },
    },
    cardFor: () => stack,
    newOpId: () => `00000000-0000-4000-8000-00000000000${++n}`,
    showCard: () => void log.push('show'),
    hostOf: async () => 'https://acme.cloud.databricks.test',
    arrivals: {
      isHeld: () => isHeld,
      whenClear: (fn) => (isHeld ? void waiting.push(fn) : fn()),
    },
  });
  wireGenieCard(bus, {
    hideCard: () => void log.push('hide'),
    clearAnswer: () => {
      log.push('clear');
      stack.setExpanded(undefined);
    },
    forget: (_m, id) => {
      log.push('forget');
      stack.add({ id: `genie:${id}`, text: '', completed: true });
    },
    sendToCard: () => undefined,
    lastOpId: () => `00000000-0000-4000-8000-00000000000${n}`,
    openConversations: () => undefined,
    closeMog: () => undefined,
    changeHome: () => undefined,
    isWorking: (m, id) => send.isWorking(m as never, id),
    closePending: (m, id) => send.close(m, id),
  });
  const closeClick = (n0: number, extra: object = {}) =>
    bus.dispatch({
      event: 'view:input',
      payload: {
        from: `card-${MOG}`,
        input: {
          mogId: MOG,
          kind: 'click',
          payload: {
            target: 'card_done',
            requestId: `00000000-0000-4000-8000-00000000000${n0}`,
            ...extra,
          },
        },
      },
    });
  const last = () => updates[updates.length - 1];
  return {
    sim,
    stack,
    log,
    send,
    events,
    closeClick,
    last,
    release() {
      isHeld = false;
      for (const fn of waiting.splice(0)) fn();
    },
  };
}
const flush = () => new Promise((r) => setTimeout(r, 0));

describe('close of the Genie pill (owner decision: pop back with the answer)', () => {
  it('a close while working only hides; an answer after 60 s shows the card with the answer', async () => {
    const r = rig();
    const done = r.send(turn());
    await flush();
    r.closeClick(1);
    // Hide only: no clear, no forget. The working blurb is intact.
    expect(r.log).toEqual(['hide']);
    expect(r.last()?.blurbs[0]).toMatchObject({
      id: expect.stringContaining('genie:'),
      completed: false,
    });
    await r.sim.runUntil(r.sim.clock.now() + 90_000);
    r.events[0]?.(ANSWER);
    await done;
    expect(r.log).toEqual(['hide', 'show']);
    expect(r.last()?.blurbs[0]).toMatchObject({ text: 'Churn was 4.2%.', completed: true });
    expect(r.last()?.sections).toBeDefined();
  });

  it('a close while working keeps the newer pop-up in the stack', async () => {
    const r = rig();
    const done = r.send(turn());
    await flush();
    r.stack.add({ id: 'd1', text: 'Quick update', completed: true });
    r.closeClick(1);
    expect(r.log).toEqual(['hide']);
    // The pop-up and the working Genie blurb are both still in the stack.
    expect(r.last()?.blurbs.some((b) => b.id === 'd1')).toBe(true);
    expect(r.last()?.blurbs.some((b) => b.id.startsWith('genie:') && !b.completed)).toBe(true);
    r.events[0]?.(ANSWER);
    await done;
  });

  it('after a close, an answer in a hold shows nothing until the key-up, then the card', async () => {
    const r = rig(true);
    const done = r.send(turn());
    await flush();
    r.closeClick(1);
    r.events[0]?.(ANSWER);
    await flush();
    // The pill says "Answer ready"; the card is not shown and no blurb has the answer.
    expect(r.log).toEqual(['hide']);
    expect(r.last()?.blurbs[0]).toMatchObject({ text: ANSWER_READY, completed: false });
    r.release();
    await done;
    expect(r.log).toEqual(['hide', 'show']);
    expect(r.last()?.blurbs[0]).toMatchObject({ text: 'Churn was 4.2%.', completed: true });
  });

  it('a close of the ready pill is a real close: the card stays closed when the hold ends', async () => {
    const r = rig(true);
    const done = r.send(turn());
    await flush();
    r.events[0]?.(ANSWER);
    await flush();
    expect(r.send.isWorking(MOG as never, '00000000-0000-4000-8000-000000000001')).toBe(false);
    r.closeClick(1);
    expect(r.log).toEqual(['hide', 'clear', 'forget']);
    r.release();
    await done;
    expect(r.log).not.toContain('show');
    expect(r.last()?.blurbs[0]).toMatchObject({ text: '', completed: true });
  });

  it('a stacked ready pill closed by the view only marks the request closed', async () => {
    const r = rig(true);
    const done = r.send(turn());
    await flush();
    r.events[0]?.(ANSWER);
    await flush();
    r.closeClick(1, { pillOnly: true });
    expect(r.log).toEqual([]);
    r.release();
    await done;
    expect(r.log).not.toContain('show');
  });
  it('a late answer of request 1 shows no card after the pill of request 2 was closed', async () => {
    const r = rig();
    const d1 = r.send(turn());
    const d2 = r.send(turn());
    await flush();
    r.closeClick(2);
    expect(r.log).toEqual(['hide']);
    r.events[0]?.(ANSWER);
    await d1;
    // No empty or foreign card window pops up; the blurb of request 1 still ends.
    expect(r.log).toEqual(['hide']);
    expect(r.last()?.blurbs.some((b) => b.completed && b.text === 'Churn was 4.2%.')).toBe(true);
    r.events[1]?.(ANSWER);
    await d2;
    expect(r.log).toEqual(['hide', 'show']);
  });

  it('a request whose submit throws never takes the card from the one that works', async () => {
    const r = rig();
    const sends = createGenieDirectSend({
      genie: {
        submit: () => {
          throw new Error('not sent');
        },
      },
      cardFor: () => r.stack,
      newOpId: () => 'op-x',
    });
    const d1 = r.send(turn());
    await flush();
    expect(() => sends(turn())).toThrow('not sent');
    r.events[0]?.(ANSWER);
    await d1;
    // Request 1 is still the newest sent request, so its sections reach the card.
    expect(r.last()?.sections).toBeDefined();
  });

  it('a close of the ready pill takes its early sections away when the hold ends', async () => {
    const r = rig(true);
    const done = r.send(turn());
    await flush();
    r.events[0]?.(ANSWER);
    await flush();
    expect(r.last()?.sections).toBeDefined();
    r.closeClick(1, { pillOnly: true });
    r.release();
    await done;
    expect(r.last()?.sections).toBeUndefined();
  });

  it('the working state is per MOG: another MOG cannot close or hide a request of this one', async () => {
    const r = rig(true);
    const done = r.send(turn());
    await flush();
    const op = '00000000-0000-4000-8000-000000000001';
    expect(r.send.isWorking(MOG as never, op)).toBe(true);
    expect(r.send.isWorking('other-mog' as never, op)).toBe(false);
    r.send.close('other-mog' as never, op);
    r.events[0]?.(ANSWER);
    await flush();
    r.release();
    await done;
    // The foreign close marked nothing: the card shows as usual.
    expect(r.log).toEqual(['show']);
  });

  it('a working close that lands after the answer applied still only hides', async () => {
    const r = rig();
    const done = r.send(turn());
    await flush();
    r.events[0]?.(ANSWER);
    await done;
    expect(r.log).toEqual(['show']);
    r.closeClick(1, { phase: 'working' });
    // Hide only: no clear, no forget.
    expect(r.log).toEqual(['show', 'hide']);
  });
});
