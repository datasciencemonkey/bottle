/**
 * UX slice 1 (director plan 2.1): the arrival gate, on a simulated clock and scheduler.
 */
import { createSimClock } from './clock';
import {
  ARRIVAL_CAP_MS,
  ARRIVAL_DELAY_MS,
  createArrivalGate,
  type GateScheduler,
} from './arrival-gate';

/** A scheduler on the simulated clock: `tick` moves the clock and runs the timers that fall due. */
function rig() {
  const clock = createSimClock(1_000);
  let timers: Array<{ at: number; fn: () => void; live: boolean }> = [];
  const schedule: GateScheduler = (at, fn) => {
    const t = { at, fn, live: true };
    timers.push(t);
    return () => {
      t.live = false;
    };
  };
  const gate = createArrivalGate({ clock, schedule });
  const tick = (ms: number) => {
    const to = clock.now() + ms;
    for (;;) {
      const due = timers.filter((t) => t.live && t.at <= to).sort((a, b) => a.at - b.at)[0];
      if (!due) break;
      due.live = false;
      if (due.at > clock.now()) clock.advance(due.at - clock.now());
      due.fn();
    }
    if (to > clock.now()) clock.advance(to - clock.now());
    timers = timers.filter((t) => t.live);
  };
  const log: string[] = [];
  const arrival = (name: string) => gate.whenClear(() => log.push(name));
  return { gate, tick, log, arrival };
}

describe('arrival gate (UX slice 1)', () => {
  it('[AC-1.1] runs an arrival at once when no hold runs', () => {
    const r = rig();
    r.arrival('a');
    expect(r.log).toEqual(['a']);
    expect(r.gate.isHeld()).toBe(false);
  });

  it('[AC-1.1] holds an arrival from the down until up or cancel', () => {
    const r = rig();
    r.gate.onPtt({ phase: 'down' });
    r.arrival('a');
    r.tick(60_000);
    expect(r.log).toEqual([]);
    expect(r.gate.isHeld()).toBe(true);
    r.gate.onPtt({ phase: 'cancel' });
    r.tick(ARRIVAL_DELAY_MS - 1);
    expect(r.log).toEqual([]);
    r.tick(1);
    expect(r.log).toEqual(['a']);
  });

  it('[AC-1.4] applies 1,200 ms after the paste resolves, not after the key-up', () => {
    const r = rig();
    r.gate.onPtt({ phase: 'down' });
    r.arrival('a');
    r.gate.onPtt({ phase: 'up' });
    r.tick(3_000);
    expect(r.log).toEqual([]);
    r.gate.pasteResolved(true);
    r.tick(ARRIVAL_DELAY_MS - 1);
    expect(r.log).toEqual([]);
    r.tick(1);
    expect(r.log).toEqual(['a']);
  });

  it('[AC-1.5] applies 1,200 ms after the key-up for an ask (no paste)', () => {
    const r = rig();
    r.gate.onPtt({ phase: 'down' });
    r.arrival('a');
    r.gate.onPtt({ phase: 'up', ask: true });
    r.tick(ARRIVAL_DELAY_MS - 1);
    expect(r.log).toEqual([]);
    r.tick(1);
    expect(r.log).toEqual(['a']);
  });

  it('[AC-1.5] a refused paste applies 1,200 ms after the key-up, or at once if that has passed', () => {
    const r = rig();
    r.gate.onPtt({ phase: 'down' });
    r.arrival('a');
    r.gate.onPtt({ phase: 'up' });
    r.tick(300);
    r.gate.pasteResolved(false);
    r.tick(899);
    expect(r.log).toEqual([]);
    r.tick(1);
    expect(r.log).toEqual(['a']);
    // A refusal after the 1.2 s have passed applies with no further wait.
    r.gate.onPtt({ phase: 'down' });
    r.arrival('b');
    r.gate.onPtt({ phase: 'up' });
    r.tick(2_000);
    r.gate.pasteResolved(false);
    r.tick(0);
    expect(r.log).toEqual(['a', 'b']);
  });

  it('[AC-1.6] applies 5,000 ms after the key-up when no paste resolves', () => {
    const r = rig();
    r.gate.onPtt({ phase: 'down' });
    r.arrival('a');
    r.gate.onPtt({ phase: 'up' });
    r.tick(ARRIVAL_CAP_MS - 1);
    expect(r.log).toEqual([]);
    r.tick(1);
    expect(r.log).toEqual(['a']);
    // A paste after the cap changes nothing.
    r.gate.pasteResolved(true);
    r.tick(10_000);
    expect(r.log).toEqual(['a']);
  });

  it('[AC-1.7] a new hold before the delay ends holds the arrivals again', () => {
    const r = rig();
    r.gate.onPtt({ phase: 'down' });
    r.arrival('a');
    r.gate.onPtt({ phase: 'up' });
    r.gate.pasteResolved(true);
    r.tick(1_000);
    r.gate.onPtt({ phase: 'down' });
    r.tick(10_000);
    expect(r.log).toEqual([]);
    r.gate.onPtt({ phase: 'up', ask: true });
    r.tick(ARRIVAL_DELAY_MS);
    expect(r.log).toEqual(['a']);
  });

  it('[AC-1.8] hands-free listening holds arrivals like a hold', () => {
    const r = rig();
    r.gate.setHandsFree(true);
    r.arrival('a');
    r.tick(60_000);
    expect(r.log).toEqual([]);
    r.gate.setHandsFree(false);
    r.gate.pasteResolved(true);
    r.tick(ARRIVAL_DELAY_MS);
    expect(r.log).toEqual(['a']);
  });

  it('[AC-1.8] a talk-key hold during hands-free keeps the hold until both end', () => {
    const r = rig();
    r.gate.setHandsFree(true);
    r.gate.onPtt({ phase: 'down' });
    r.arrival('a');
    r.gate.onPtt({ phase: 'up', ask: true });
    r.tick(10_000);
    expect(r.log).toEqual([]);
    r.gate.setHandsFree(false);
    r.gate.pasteResolved(false);
    r.tick(ARRIVAL_DELAY_MS);
    expect(r.log).toEqual(['a']);
  });

  it('[AC-1.10] runs held arrivals in the order they came in', () => {
    const r = rig();
    r.gate.onPtt({ phase: 'down' });
    r.arrival('first');
    r.arrival('second');
    r.gate.onPtt({ phase: 'up', ask: true });
    r.tick(ARRIVAL_DELAY_MS);
    expect(r.log).toEqual(['first', 'second']);
  });

  it('[AC-1.11] a held arrival that throws does not stop the next one', () => {
    const r = rig();
    r.gate.onPtt({ phase: 'down' });
    r.gate.whenClear(() => {
      throw new Error('fake');
    });
    r.arrival('next');
    r.gate.onPtt({ phase: 'up', ask: true });
    r.tick(ARRIVAL_DELAY_MS);
    expect(r.log).toEqual(['next']);
  });

  it('ignores an up with no down, a repeated down and a paste with no hold', () => {
    const r = rig();
    r.gate.onPtt({ phase: 'up' });
    r.gate.pasteResolved(true);
    expect(r.gate.isHeld()).toBe(false);
    r.gate.onPtt({ phase: 'down' });
    r.gate.onPtt({ phase: 'down' });
    r.gate.onPtt({ phase: 'up', ask: true });
    r.tick(ARRIVAL_DELAY_MS);
    expect(r.gate.isHeld()).toBe(false);
  });

  it('an arrival after the delay runs at once', () => {
    const r = rig();
    r.gate.onPtt({ phase: 'down' });
    r.gate.onPtt({ phase: 'up', ask: true });
    r.tick(ARRIVAL_DELAY_MS);
    r.arrival('late');
    expect(r.log).toEqual(['late']);
  });
});
