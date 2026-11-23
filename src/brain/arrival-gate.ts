/**
 * UX slice 1 (director plan 2.1): the arrival gate. While the user holds the talk key, or hands-free
 * listening runs, an Ask Genie arrival must not open the card or move a row. The arrival waits.
 * It applies 1,200 ms after the paste of the hold resolves, so the pasted text lands first.
 *
 * - The hold starts at a `hotkey:ptt` `down` (or when hands-free starts) and ends at `up` or
 *   `cancel` (or when hands-free ends). A new hold before the delay ends holds the arrivals again.
 * - After a hold that can paste, the gate waits for `pasteResolved`. After a hold with no paste
 *   (an ⌥G ask, a cancel) it waits 1,200 ms after the key-up. A paste that never resolves in
 *   5,000 ms after the key-up does not hold the arrivals longer.
 * - The held functions run in the order they came in.
 *
 * Slice 2a lane T1 must reuse this gate and not build its own. The clock and the scheduler are
 * injected: tests move a simulated clock.
 */
import type { MonotonicClock } from './clock';
import type { PttEvent } from './ptt';

/** Run `fn` at monotonic time `atMs`. Returns a cancel function (the `card-stack.ts` seam). */
export type GateScheduler = (atMs: number, fn: () => void) => () => void;

/** The delay between the end of the paste (or the key-up) and the arrivals. */
export const ARRIVAL_DELAY_MS = 1_200;
/** The arrivals never wait longer than this after the key-up for a paste that does not come. */
export const ARRIVAL_CAP_MS = 5_000;

export interface ArrivalGateDeps {
  clock: MonotonicClock;
  schedule: GateScheduler;
}

export interface ArrivalGate {
  /** A `hotkey:ptt` item. Only `down`, `up` and `cancel` count. */
  onPtt(e: Pick<PttEvent, 'phase' | 'ask'>): void;
  /** Hands-free listening starts (`true`) or ends (`false`). */
  setHandsFree(on: boolean): void;
  /** One utterance ended its route. `pasted` is true when `insert_text` inserted the text. */
  pasteResolved(pasted: boolean): void;
  /** True while a hold runs or the delay after it has not ended. */
  isHeld(): boolean;
  /** Run `fn` now when nothing holds, or later, in order, when the hold and its delay end. */
  whenClear(fn: () => void): void;
}

export function createArrivalGate(deps: ArrivalGateDeps): ArrivalGate {
  let pttHeld = false;
  let handsFree = false;
  /** After a hold: the key-up time, and whether a paste is still expected. */
  let settling: { upAt: number; expectPaste: boolean } | undefined;
  let cancelTimers: Array<() => void> = [];
  let queue: Array<() => void> = [];

  const clearTimers = () => {
    for (const c of cancelTimers) c();
    cancelTimers = [];
  };
  const at = (atMs: number, fn: () => void) => cancelTimers.push(deps.schedule(atMs, fn));

  const flush = () => {
    clearTimers();
    settling = undefined;
    const run = queue;
    queue = [];
    for (const fn of run) {
      try {
        fn();
      } catch {
        // One held arrival that throws never stops the next one.
      }
    }
  };

  const holding = () => pttHeld || handsFree;

  const begin = () => {
    // A new hold holds the arrivals again and drops the timers of the last one.
    clearTimers();
    settling = undefined;
  };

  const end = (expectPaste: boolean) => {
    if (holding()) return;
    const upAt = deps.clock.now();
    clearTimers();
    settling = { upAt, expectPaste };
    if (expectPaste) at(upAt + ARRIVAL_CAP_MS, flush);
    else at(upAt + ARRIVAL_DELAY_MS, flush);
  };

  return {
    onPtt(e) {
      if (e.phase === 'down') {
        if (!pttHeld) begin();
        pttHeld = true;
      } else if ((e.phase === 'up' || e.phase === 'cancel') && pttHeld) {
        pttHeld = false;
        end(e.phase === 'up' && e.ask !== true);
      }
    },
    setHandsFree(on) {
      if (on === handsFree) return;
      handsFree = on;
      if (on) begin();
      else end(true);
    },
    pasteResolved(pasted) {
      if (!settling?.expectPaste) return;
      const now = deps.clock.now();
      clearTimers();
      settling.expectPaste = false;
      // A paste is followed by 1,200 ms. No paste gives the same wait after the key-up.
      at(pasted ? now + ARRIVAL_DELAY_MS : Math.max(now, settling.upAt + ARRIVAL_DELAY_MS), flush);
    },
    isHeld: () => holding() || settling !== undefined,
    whenClear(fn) {
      if (holding() || settling !== undefined) queue.push(fn);
      else fn();
    },
  };
}
