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
 * - A resolve counts only for the lease that is settling: a late paste of an older hold never
 *   releases the arrivals of a newer one. A hold that never ends has two safety nets: the state
 *   of its lease (`recording: false`) and `MAX_HOLD_MS`.
 *
 * Slice 2a lane T1 must reuse this gate and not build its own. The clock and the scheduler are
 * injected: tests move a simulated clock.
 */
import type { MonotonicClock } from './clock';
import type { SegmentRef } from '../shared/interfaces';
import type { Transcribed } from './capture-lifecycle';
import type { PttEvent } from './ptt';

/** Run `fn` at monotonic time `atMs`. Returns a cancel function (the `card-stack.ts` seam). */
export type GateScheduler = (atMs: number, fn: () => void) => () => void;

/** The delay between the end of the paste (or the key-up) and the arrivals. */
export const ARRIVAL_DELAY_MS = 1_200;
/** The arrivals never wait longer than this after the key-up for a paste that does not come. */
export const ARRIVAL_CAP_MS = 5_000;
/** One talk-key hold never holds arrivals longer than this (a lost key-up). */
export const MAX_HOLD_MS = 120_000;

/** The segment of an utterance: its ID and the lease that made it. */
export interface GateSegment {
  segmentId: string;
  leaseId?: string | undefined;
}

export interface ArrivalGateDeps {
  clock: MonotonicClock;
  schedule: GateScheduler;
  /** A held function threw. The caller logs a fixed code; the gate never reads the content. */
  onFault?: (() => void) | undefined;
}

/** The part of one `capture:state` item that the gate reads. */
export interface GateCaptureState {
  recording?: boolean | undefined;
  mode?: string | undefined;
  leaseId?: string | undefined;
}

export interface ArrivalGate {
  /** A `hotkey:ptt` item. Only `down`, `up` and `cancel` count. */
  onPtt(e: Pick<PttEvent, 'phase' | 'ask'>): void;
  /** One `capture:state` item: it names the lease of a hold, and ends a hold whose key-up was lost. */
  onCaptureState(s: GateCaptureState): void;
  /** A final segment came from STT capture: one resolve is due for it. */
  segmentFinal(seg: GateSegment): void;
  /**
   * One utterance ended its route (or STT gave no text). `pasted` is true when `insert_text`
   * inserted the text. With no `seg` the resolve is for the current hold, whatever lease.
   */
  pasteResolved(pasted: boolean, seg?: GateSegment): void;
  /** The start of the hold was refused: no paste follows. */
  refused(): void;
  /** True while a hold runs or the delay after it has not ended. */
  isHeld(): boolean;
  /** Run `fn` now when nothing holds, or later, in order, when the hold and its delay end. */
  whenClear(fn: () => void): void;
}

const MAX_PENDING = 64;

export function createArrivalGate(deps: ArrivalGateDeps): ArrivalGate {
  let pttHeld = false;
  let handsFree = false;
  /** The lease of the hold that runs now (learned from `capture:state`). */
  let pttLease: string | undefined;
  let hfLease: string | undefined;
  /** The hold started and its start was refused. */
  let refusedHold = false;
  /** After a hold: the key-up time, the lease, and whether a paste is still expected. */
  let settling: { upAt: number; lease: string | undefined; expectPaste: boolean } | undefined;
  let cancelTimers: Array<() => void> = [];
  let holdCap: (() => void) | undefined;
  let queue: Array<() => void> = [];
  /** Final segments that wait for their one resolve: segment ID to lease. */
  const pending = new Map<string, string | undefined>();

  const clearTimers = () => {
    for (const c of cancelTimers) c();
    cancelTimers = [];
  };
  const at = (atMs: number, fn: () => void) => cancelTimers.push(deps.schedule(atMs, fn));
  const clearHoldCap = () => {
    holdCap?.();
    holdCap = undefined;
  };

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
        deps.onFault?.();
      }
    }
  };

  const holding = () => pttHeld || handsFree;

  const begin = (capHold: boolean) => {
    // A new hold holds the arrivals again and drops the timers of the last one.
    clearTimers();
    settling = undefined;
    refusedHold = false;
    clearHoldCap();
    if (!capHold) return;
    // A talk-key hold that never ends (a lost key-up) is ended here.
    holdCap = deps.schedule(deps.clock.now() + MAX_HOLD_MS, () => {
      holdCap = undefined;
      if (!pttHeld) return;
      pttHeld = false;
      end(true, pttLease);
    });
  };

  function end(expectPaste: boolean, lease: string | undefined) {
    if (holding()) return;
    clearHoldCap();
    const upAt = deps.clock.now();
    clearTimers();
    const expect = expectPaste && !refusedHold;
    settling = { upAt, lease, expectPaste: expect };
    if (expect) at(upAt + ARRIVAL_CAP_MS, flush);
    else at(upAt + ARRIVAL_DELAY_MS, flush);
  }

  const leaseCounts = (seg: GateSegment | undefined) =>
    !seg?.leaseId || !settling?.lease || seg.leaseId === settling.lease;

  function resolve(pasted: boolean, seg?: GateSegment) {
    if (seg) pending.delete(seg.segmentId);
    if (!settling?.expectPaste || !leaseCounts(seg)) return;
    const now = deps.clock.now();
    clearTimers();
    settling.expectPaste = false;
    // A paste is followed by 1,200 ms. No paste gives the same wait after the key-up.
    at(pasted ? now + ARRIVAL_DELAY_MS : Math.max(now, settling.upAt + ARRIVAL_DELAY_MS), flush);
  }

  return {
    onPtt(e) {
      if (e.phase === 'down') {
        if (!pttHeld) {
          begin(true);
          pttLease = undefined;
        }
        pttHeld = true;
      } else if ((e.phase === 'up' || e.phase === 'cancel') && pttHeld) {
        pttHeld = false;
        end(e.phase === 'up' && e.ask !== true, pttLease);
      }
    },
    onCaptureState(s) {
      const hf = s.recording === true && s.mode === 'hands_free';
      if (hf !== handsFree) {
        handsFree = hf;
        if (hf) {
          begin(false);
          hfLease = s.leaseId;
        } else {
          // Hands-free ended. With no segment of its lease in flight, no paste follows.
          const open = [...pending.values()].some((l) => hfLease === undefined || l === hfLease);
          end(open, hfLease);
        }
        return;
      }
      if (s.recording === true && s.mode === 'ptt' && pttHeld && pttLease === undefined)
        pttLease = s.leaseId;
      // A hold whose lease stopped with no key-up: the hold ends as an up that expects a paste.
      if (s.recording !== true && pttHeld && pttLease !== undefined && s.leaseId === pttLease) {
        pttHeld = false;
        end(true, pttLease);
      }
    },
    segmentFinal(seg) {
      if (pending.has(seg.segmentId)) return;
      if (pending.size >= MAX_PENDING) pending.delete(pending.keys().next().value as string);
      pending.set(seg.segmentId, seg.leaseId);
      // A segment after the end of the hold, inside its delay: a paste follows after all.
      if (settling && !settling.expectPaste && leaseCounts(seg)) {
        clearTimers();
        settling.expectPaste = true;
        at(Math.max(deps.clock.now(), settling.upAt + ARRIVAL_CAP_MS), flush);
      }
    },
    pasteResolved: resolve,
    refused() {
      if (holding()) refusedHold = true;
      else if (settling) resolve(false);
    },
    isHeld: () => holding() || settling !== undefined,
    whenClear(fn) {
      if (holding() || settling !== undefined) queue.push(fn);
      else fn();
    },
  };
}

/**
 * Wrap the STT step of the capture lifecycle. A segment with no text, or an STT step that
 * throws, never reaches the utterance route, so no paste follows: tell the gate at once.
 */
export function noPasteOnEmpty(
  transcribe: (seg: SegmentRef) => Promise<Transcribed>,
  noPaste: (seg: GateSegment) => void,
): (seg: SegmentRef) => Promise<Transcribed> {
  return async (seg) => {
    let done: Transcribed;
    try {
      done = await transcribe(seg);
    } catch (e) {
      noPaste(seg);
      throw e;
    }
    if ((typeof done === 'string' ? done : done.text).trim() === '') noPaste(seg);
    return done;
  };
}
