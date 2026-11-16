import { createSimClock } from './clock';
import {
  createCardStack,
  timeoutScheduler,
  type CardStackUpdate,
  type Scheduler,
} from './card-stack';

const S = 1000;
const MOG = '00000000-0000-4000-8000-000000000001';

/** A scheduler on the sim clock: `run()` fires every timer that is due now. */
function simScheduler(clock: ReturnType<typeof createSimClock>) {
  let timers: { at: number; fn: () => void }[] = [];
  const scheduler: Scheduler = (at, fn) => {
    const t = { at, fn };
    timers.push(t);
    return () => {
      timers = timers.filter((x) => x !== t);
    };
  };
  return {
    scheduler,
    pending: () => timers.length,
    advance(ms: number) {
      clock.advance(ms);
      const due = timers.filter((t) => t.at <= clock.now());
      timers = timers.filter((t) => t.at > clock.now());
      due.forEach((t) => t.fn());
    },
  };
}

function setup(settings?: Parameters<typeof createCardStack>[2]) {
  const clock = createSimClock();
  const sim = simScheduler(clock);
  const sent: CardStackUpdate[] = [];
  const opened: string[] = [];
  const card = createCardStack(
    MOG,
    {
      clock,
      schedule: sim.scheduler,
      send: (u) => void sent.push(u),
      openHistory: (id) => void opened.push(id),
    },
    settings,
  );
  const last = () => sent[sent.length - 1];
  return { clock, sim, sent, opened, card, last };
}

describe('card stack controller', () => {
  it('[UX-11a] sends card.update with the stack, newest first, and the History count', () => {
    const { card, clock, last } = setup();
    for (let i = 1; i <= 4; i++) {
      card.add({ id: `b${i}`, text: `a${i}` });
      clock.advance(S);
    }
    expect(last()?.mogId).toBe(MOG);
    expect(last()?.blurbs.map((b) => b.id)).toEqual(['b4', 'b3', 'b2']);
    expect(last()?.historyCount).toBe(1);
    expect(last()?.chips).toEqual([]);
  });

  it('[UX-12] [AT-09] collapses a completed blurb after 60 s on the sim clock and sends the change', () => {
    const { card, sim, last } = setup();
    card.add({ id: 'b1', text: 'a1' });
    card.complete('b1');
    sim.advance(59 * S);
    expect(last()?.blurbs.map((b) => b.id)).toEqual(['b1']);
    sim.advance(S);
    expect(last()?.blurbs).toEqual([]);
    expect(last()?.historyCount).toBe(1);
    expect(sim.pending()).toBe(0);
  });

  it('[UX-12] hover input from the view holds the blurb and re-arms the timer on leave', () => {
    const { card, sim, last } = setup();
    card.add({ id: 'b1', text: 'a1' });
    card.complete('b1');
    card.input('hover', { blurbId: 'b1', on: true, via: 'pointer' });
    sim.advance(120 * S);
    expect(last()?.blurbs).toHaveLength(1);
    card.input('hover', { blurbId: 'b1', on: false, via: 'pointer' });
    sim.advance(59 * S);
    expect(last()?.blurbs).toHaveLength(1);
    sim.advance(S);
    expect(last()?.blurbs).toHaveLength(0);
  });

  it('[UX-11c] a pin click toggles the pin and sends the new state', () => {
    const { card, last } = setup();
    card.add({ id: 'b1', text: 'a1' });
    card.input('click', { target: 'pin', blurbId: 'b1' });
    expect(last()?.blurbs[0]?.pinned).toBe(true);
    card.input('click', { target: 'pin', blurbId: 'b1' });
    expect(last()?.blurbs[0]?.pinned).toBe(false);
  });

  it('[UX-11a] [UX-12] the History click opens History for this MOG', () => {
    const { card, opened } = setup();
    card.input('click', { target: 'history' });
    expect(opened).toEqual([MOG]);
  });

  it('[UX-11a] malformed input sends nothing', () => {
    const { card, sent } = setup();
    card.input('hover', { nope: true });
    card.input('click', 42);
    expect(sent).toEqual([]);
  });

  it('[UX-12] keeps one timer armed, and dispose clears it', () => {
    const { card, sim } = setup();
    card.add({ id: 'b1', text: 'a1', completed: true });
    card.add({ id: 'b2', text: 'a2', completed: true });
    expect(sim.pending()).toBe(1);
    card.dispose();
    expect(sim.pending()).toBe(0);
  });

  it('[UX-12] the settings reach the stack', () => {
    const { card, sim, last } = setup({ autoHideSec: 5, maxBlurbs: 2 });
    for (let i = 1; i <= 3; i++) card.add({ id: `b${i}`, text: `a${i}`, completed: true });
    expect(last()?.blurbs.map((b) => b.id)).toEqual(['b3', 'b2']);
    sim.advance(5 * S);
    expect(last()?.blurbs).toEqual([]);
    expect(last()?.historyCount).toBe(3);
  });

  it('[UX-12] the production scheduler fires at the deadline and can be cancelled', () => {
    vi.useFakeTimers();
    try {
      const clock = createSimClock(10 * S);
      const schedule = timeoutScheduler(clock);
      const fired: string[] = [];
      schedule(70 * S, () => fired.push('a'));
      const cancel = schedule(70 * S, () => fired.push('b'));
      schedule(5 * S, () => fired.push('late')); // a past deadline fires at once
      cancel();
      vi.advanceTimersByTime(0);
      expect(fired).toEqual(['late']);
      vi.advanceTimersByTime(60 * S - 1);
      expect(fired).toEqual(['late']);
      vi.advanceTimersByTime(1);
      expect(fired).toEqual(['late', 'a']);
    } finally {
      vi.useRealTimers();
    }
  });

  it('[UX-12] the production scheduler caps a long delay at the setTimeout limit', () => {
    vi.useFakeTimers();
    try {
      const spy = vi.spyOn(globalThis, 'setTimeout');
      const clock = createSimClock();
      timeoutScheduler(clock)(40 * 24 * 3600 * S, () => undefined);
      expect(spy.mock.calls[0]?.[1]).toBe(2 ** 31 - 1);
      spy.mockRestore();
    } finally {
      vi.useRealTimers();
    }
  });

  it('[UX-38] [AT-36] the STT badge goes in the card footer of every update', () => {
    const { card, last, sent } = setup();
    card.setSttBadge('STT: local');
    expect(last()?.sttBadge).toBe('STT: local');
    card.add({ id: 'b1', text: 'a1' });
    expect(last()?.sttBadge).toBe('STT: local');
    const n = sent.length;
    card.setSttBadge('STT: local');
    expect(sent).toHaveLength(n); // no change, no update
    card.setSttBadge('STT: Databricks m1');
    expect(last()?.sttBadge).toBe('STT: Databricks m1');
  });
});
