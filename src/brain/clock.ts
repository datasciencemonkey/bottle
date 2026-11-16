// Monotonic clock (#229): the TS twin of mog_core::clock. Times are milliseconds since an
// arbitrary origin. The brain uses it for poll cadence and timers; tests use the sim clock.

export interface MonotonicClock {
  /** Milliseconds since the origin of this clock. Never goes back. */
  now(): number;
}

export interface SimClock extends MonotonicClock {
  /** Move the clock forward by `ms`. A negative or non-finite step throws a RangeError. */
  advance(ms: number): void;
}

/** The production clock, on `performance.now()`. The origin is the moment of construction. */
export function createSystemClock(): MonotonicClock {
  const origin = performance.now();
  return { now: () => performance.now() - origin };
}

/** A simulated clock for tests. It moves only on `advance`. */
export function createSimClock(startMs = 0): SimClock {
  let t = startMs;
  return {
    now: () => t,
    advance(ms: number) {
      if (!Number.isFinite(ms) || ms < 0)
        throw new RangeError(`advance needs a finite step >= 0, got ${ms}`);
      t += ms;
    },
  };
}
