/**
 * UX slice 1 (director plan 2.1): the arrival gate, on a simulated clock and scheduler.
 */
import { createSimClock } from './clock';
import {
  ARRIVAL_CAP_MS,
  ARRIVAL_DELAY_MS,
  MAX_HOLD_MS,
  createArrivalGate,
  noPasteOnEmpty,
  type GateScheduler,
} from './arrival-gate';

const HF_ON = { recording: true, mode: 'hands_free', leaseId: 'hf-1' };
const HF_OFF = { recording: false, mode: 'hands_free', leaseId: 'hf-1' };

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
  let faults = 0;
  const gate = createArrivalGate({ clock, schedule, onFault: () => void faults++ });
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
  return { gate, tick, log, arrival, faults: () => faults };
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
    r.gate.onCaptureState(HF_ON);
    r.arrival('a');
    r.tick(60_000);
    expect(r.log).toEqual([]);
    r.gate.onCaptureState(HF_OFF);
    r.gate.pasteResolved(true);
    r.tick(ARRIVAL_DELAY_MS);
    expect(r.log).toEqual(['a']);
  });

  it('[AC-1.8] a talk-key hold during hands-free keeps the hold until both end', () => {
    const r = rig();
    r.gate.onCaptureState(HF_ON);
    r.gate.onPtt({ phase: 'down' });
    r.arrival('a');
    r.gate.onPtt({ phase: 'up', ask: true });
    r.tick(10_000);
    expect(r.log).toEqual([]);
    r.gate.onCaptureState(HF_OFF);
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
    // The fault is reported once, with no content.
    expect(r.faults()).toBe(1);
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

  describe('review fixes', () => {
    const PTT = (leaseId: string) => ({ recording: true, mode: 'ptt', leaseId });
    const seg = (segmentId: string, leaseId: string) => ({ segmentId, leaseId });

    it('[AC-1.4] a late resolve of an older lease never frees the arrivals of a newer hold', () => {
      const r = rig();
      // Hold A: its route is still pending when hold B comes and ends.
      r.gate.onPtt({ phase: 'down' });
      r.gate.onCaptureState(PTT('lease-a'));
      r.gate.onPtt({ phase: 'up' });
      r.gate.onPtt({ phase: 'down' });
      r.gate.onCaptureState(PTT('lease-b'));
      r.arrival('b');
      r.gate.onPtt({ phase: 'up' });
      r.tick(1_000);
      r.gate.pasteResolved(true, seg('s-a', 'lease-a'));
      r.tick(ARRIVAL_DELAY_MS + 100);
      expect(r.log).toEqual([]);
      // B resolves: 1,200 ms later the arrival applies.
      r.gate.pasteResolved(true, seg('s-b', 'lease-b'));
      r.tick(ARRIVAL_DELAY_MS);
      expect(r.log).toEqual(['b']);
    });

    it('[AC-1.6] hold B that never resolves ends at its own 5,000 ms cap', () => {
      const r = rig();
      r.gate.onPtt({ phase: 'down' });
      r.gate.onCaptureState(PTT('lease-a'));
      r.gate.onPtt({ phase: 'up' });
      r.gate.onPtt({ phase: 'down' });
      r.gate.onCaptureState(PTT('lease-b'));
      r.arrival('b');
      r.gate.onPtt({ phase: 'up' });
      r.gate.pasteResolved(true, seg('s-a', 'lease-a'));
      r.tick(ARRIVAL_CAP_MS - 1);
      expect(r.log).toEqual([]);
      r.tick(1);
      expect(r.log).toEqual(['b']);
    });

    it('a lost key-up: the stop of the lease ends the hold as an up that expects a paste', () => {
      const r = rig();
      r.gate.onPtt({ phase: 'down' });
      r.gate.onCaptureState(PTT('lease-a'));
      r.arrival('a');
      r.tick(30_000);
      expect(r.log).toEqual([]);
      r.gate.onCaptureState({ recording: false, mode: 'ptt', leaseId: 'lease-a' });
      expect(r.gate.isHeld()).toBe(true);
      r.gate.pasteResolved(true, seg('s-a', 'lease-a'));
      r.tick(ARRIVAL_DELAY_MS);
      expect(r.log).toEqual(['a']);
      // The key-up that comes late changes nothing.
      r.gate.onPtt({ phase: 'up' });
      expect(r.gate.isHeld()).toBe(false);
    });

    it('a lost key-up: a state of another lease does not end the hold', () => {
      const r = rig();
      r.gate.onPtt({ phase: 'down' });
      r.gate.onCaptureState(PTT('lease-a'));
      r.arrival('a');
      r.gate.onCaptureState({ recording: false, mode: 'ptt', leaseId: 'lease-other' });
      r.tick(10_000);
      expect(r.log).toEqual([]);
    });

    it('a hold with no key-up and no state ends at MAX_HOLD_MS, then at its 5,000 ms cap', () => {
      const r = rig();
      r.gate.onPtt({ phase: 'down' });
      r.arrival('a');
      r.tick(MAX_HOLD_MS - 1);
      expect(r.log).toEqual([]);
      r.tick(1);
      expect(r.gate.isHeld()).toBe(true);
      r.tick(ARRIVAL_CAP_MS);
      expect(r.log).toEqual(['a']);
    });

    it('the hold cap does not cut hands-free listening', () => {
      const r = rig();
      r.gate.onCaptureState(HF_ON);
      r.arrival('a');
      r.tick(MAX_HOLD_MS * 2);
      expect(r.log).toEqual([]);
    });

    it('[AC-1.5] hands-free that ends with no segment in flight applies 1,200 ms after the end', () => {
      const r = rig();
      r.gate.onCaptureState(HF_ON);
      r.gate.segmentFinal(seg('s1', 'hf-1'));
      r.gate.pasteResolved(true, seg('s1', 'hf-1'));
      r.arrival('a');
      r.tick(5_000);
      r.gate.onCaptureState(HF_OFF);
      r.tick(ARRIVAL_DELAY_MS - 1);
      expect(r.log).toEqual([]);
      r.tick(1);
      expect(r.log).toEqual(['a']);
    });

    it('[AC-1.4] hands-free that ends with a segment in flight waits for its paste', () => {
      const r = rig();
      r.gate.onCaptureState(HF_ON);
      r.gate.segmentFinal(seg('s1', 'hf-1'));
      r.arrival('a');
      r.gate.onCaptureState(HF_OFF);
      r.tick(3_000);
      expect(r.log).toEqual([]);
      r.gate.pasteResolved(true, seg('s1', 'hf-1'));
      r.tick(ARRIVAL_DELAY_MS);
      expect(r.log).toEqual(['a']);
    });

    it('a final segment inside the delay after hands-free waits for its paste after all', () => {
      const r = rig();
      r.gate.onCaptureState(HF_ON);
      r.arrival('a');
      r.gate.onCaptureState(HF_OFF);
      r.tick(500);
      r.gate.segmentFinal(seg('s-late', 'hf-1'));
      r.tick(ARRIVAL_DELAY_MS * 2);
      expect(r.log).toEqual([]);
      r.gate.pasteResolved(true, seg('s-late', 'hf-1'));
      r.tick(ARRIVAL_DELAY_MS);
      expect(r.log).toEqual(['a']);
    });

    it('[AC-1.5] a refused start: the key-up applies after 1,200 ms, also when it came before', () => {
      const r = rig();
      r.gate.onPtt({ phase: 'down' });
      r.arrival('a');
      r.gate.refused();
      r.gate.onPtt({ phase: 'up' });
      r.tick(ARRIVAL_DELAY_MS);
      expect(r.log).toEqual(['a']);
      // A refusal that comes after the key-up.
      r.gate.onPtt({ phase: 'down' });
      r.arrival('b');
      r.gate.onPtt({ phase: 'up' });
      r.tick(500);
      r.gate.refused();
      r.tick(ARRIVAL_DELAY_MS - 500);
      expect(r.log).toEqual(['a', 'b']);
    });

    it('[AC-1.5] noPasteOnEmpty tells the gate when STT gives no text or throws', async () => {
      const told: string[] = [];
      const wrap = (t: () => Promise<string | { text: string }>) =>
        noPasteOnEmpty(t, (s) => void told.push(s.segmentId));
      const segment = { segmentId: 'x' } as Parameters<ReturnType<typeof wrap>>[0];
      await wrap(async () => '  ')(segment);
      await wrap(async () => ({ text: '' }))(segment);
      await expect(wrap(async () => Promise.reject(new Error('fake')))(segment)).rejects.toThrow();
      await wrap(async () => 'hello')(segment);
      expect(told).toEqual(['x', 'x', 'x']);
    });
  });
});
