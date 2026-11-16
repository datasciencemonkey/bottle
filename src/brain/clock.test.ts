import { createSimClock, createSystemClock } from './clock';

describe('monotonic clock (#229)', () => {
  it('[AT-22] the sim clock starts at the given time and moves only on advance', () => {
    const c = createSimClock(1_000);
    expect(c.now()).toBe(1_000);
    expect(c.now()).toBe(1_000);
    c.advance(55 * 60_000);
    expect(c.now()).toBe(1_000 + 55 * 60_000);
  });

  it('[AT-22] the sim clock starts at 0 by default', () => {
    expect(createSimClock().now()).toBe(0);
  });

  it('[AT-22] the sim clock rejects a negative or non-finite step, so it cannot go back', () => {
    const c = createSimClock();
    expect(() => c.advance(-1)).toThrow(RangeError);
    expect(() => c.advance(Number.NaN)).toThrow(RangeError);
    expect(() => c.advance(Number.POSITIVE_INFINITY)).toThrow(RangeError);
    expect(c.now()).toBe(0);
  });

  it('[AT-22] the system clock never goes back', () => {
    const c = createSystemClock();
    const a = c.now();
    const b = c.now();
    expect(b).toBeGreaterThanOrEqual(a);
    expect(a).toBeGreaterThanOrEqual(0);
  });
});
