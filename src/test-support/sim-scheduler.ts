/**
 * A timer queue and a 60 Hz frame clock on the #229 sim clock, for latency tests (#13).
 * Nothing waits on wall time: `runUntil` moves the sim clock from one timer to the next.
 */
import { createSimClock, type SimClock } from '../brain/clock';

export interface SimScheduler {
  readonly clock: SimClock;
  /** Run `fn` at sim time `atMs` (now if it is in the past). Returns a cancel function. */
  at(atMs: number, fn: () => void | Promise<void>): () => void;
  /** Run `fn` `ms` after now. */
  after(ms: number, fn: () => void | Promise<void>): () => void;
  /** `requestAnimationFrame` on the sim clock: `cb` runs on the next vsync after now. */
  requestFrame(cb: () => void): void;
  /** A promise that resolves `ms` after now, or at once when `signal` aborts. */
  sleep(ms: number, signal?: AbortSignal): Promise<void>;
  /**
   * Run every timer due at or before `endMs` in time order, then set the clock to `endMs`.
   * With `settle`, it lets pending promise work finish (one real macrotask) before it picks
   * each timer, so an async loop that sets its next timer after some awaits (the PollActor,
   * #31) runs on sim time too.
   */
  runUntil(endMs: number, opts?: { settle?: boolean }): Promise<void>;
}

/** One real macrotask: every queued promise job runs first. */
const settled = () => new Promise<void>((r) => setTimeout(r, 0));

/** 60 Hz, the refresh rate of the reference displays. */
export const FRAME_MS = 1000 / 60;

export function createSimScheduler(startMs = 0, frameMs = FRAME_MS): SimScheduler {
  const clock = createSimClock(startMs);
  let next = 0;
  const timers: { t: number; n: number; fn: () => void | Promise<void> }[] = [];
  const at = (atMs: number, fn: () => void | Promise<void>) => {
    const timer = { t: Math.max(atMs, clock.now()), n: next++, fn };
    timers.push(timer);
    return () => {
      const i = timers.indexOf(timer);
      if (i >= 0) timers.splice(i, 1);
    };
  };
  return {
    clock,
    at,
    after: (ms, fn) => at(clock.now() + ms, fn),
    requestFrame(cb) {
      // The next vsync strictly after now, as a browser runs rAF callbacks.
      at((Math.floor(clock.now() / frameMs) + 1) * frameMs, cb);
    },
    sleep(ms, signal) {
      return new Promise<void>((resolve) => {
        if (signal?.aborted) return resolve();
        const onAbort = () => {
          cancel();
          resolve();
        };
        const cancel = at(clock.now() + ms, () => {
          signal?.removeEventListener('abort', onAbort);
          resolve();
        });
        signal?.addEventListener('abort', onAbort, { once: true });
      });
    },
    async runUntil(endMs, opts) {
      for (;;) {
        if (opts?.settle) await settled();
        timers.sort((a, b) => a.t - b.t || a.n - b.n);
        const first = timers[0];
        if (!first || first.t > endMs) break;
        timers.shift();
        clock.advance(first.t - clock.now());
        await first.fn();
      }
      clock.advance(Math.max(0, endMs - clock.now()));
    },
  };
}
