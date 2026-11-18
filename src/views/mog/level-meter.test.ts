/**
 * UX-06a / UX-06b (AT-14): the MOG view computes `level` with Table UX-LEVEL, and only for the
 * MOG that owns the open mic. Other MOGs show 0.
 */
import { LEVEL, LevelMeter, levelTarget, type MicLevelFrame } from './level-meter';

const ME = '00000000-0000-4000-8000-000000000001';
const OTHER = '00000000-0000-4000-8000-000000000002';
const f = (t: number, dbfs: number, owner: string | null = ME): MicLevelFrame => ({
  owner,
  dbfs,
  t,
});

/** Push frames every `step` ms from `t0` to `t1` and return the last shown level. */
function run(m: LevelMeter, t0: number, t1: number, dbfs: number, step = 5): number[] {
  const shown: number[] = [];
  for (let t = t0; t <= t1; t += step) {
    const v = m.push(f(t, dbfs));
    if (v !== undefined) shown.push(v);
  }
  return shown;
}

describe('UX-06a: level = clamp((dBFS + 60) / 50, 0, 1)', () => {
  it('[UX-06a] [AT-14] the table values are the ones of Table UX-LEVEL', () => {
    expect(LEVEL).toEqual({ floorDb: -60, rangeDb: 50, attackMs: 30, releaseMs: 150, maxHz: 30 });
  });

  it('[UX-06a] [AT-14] maps dBFS to the target level with the formula and clamps it', () => {
    expect(levelTarget(-60)).toBe(0);
    expect(levelTarget(-35)).toBeCloseTo(0.5, 10);
    expect(levelTarget(-10)).toBe(1);
    expect(levelTarget(-80)).toBe(0);
    expect(levelTarget(0)).toBe(1);
    expect(levelTarget(-Infinity)).toBe(0);
    expect(levelTarget(Number.NaN)).toBe(0);
    expect(levelTarget(Infinity)).toBe(1);
  });

  it('[UX-06a] [AT-14] rises with a 30 ms attack time constant', () => {
    const m = new LevelMeter(ME);
    m.push(f(0, -60));
    // One step of 30 ms at the attack constant reaches 1 - 1/e of the target.
    expect(m.push(f(30, -10))).toBeCloseTo(1 - Math.exp(-1), 6);
  });

  it('[UX-06a] [AT-14] falls with a 150 ms release time constant', () => {
    const m = new LevelMeter(ME);
    m.push(f(0, -60));
    run(m, 5, 1000, -10);
    const start = m.value;
    expect(start).toBeCloseTo(1, 3);
    expect(m.push(f(1150, -60))).toBeCloseTo(start * Math.exp(-1), 6);
  });

  it('[UX-06a] [AT-14] the level trace matches the formula over a step up and down', () => {
    const m = new LevelMeter(ME);
    m.push(f(0, -60));
    let model = 0;
    let last = 0;
    for (let t = 10; t <= 600; t += 10) {
      const target = t <= 300 ? levelTarget(-20) : 0;
      const tau = target > model ? 30 : 150;
      model += (target - model) * (1 - Math.exp(-10 / tau));
      const v = m.push(f(t, t <= 300 ? -20 : -90));
      if (v !== undefined) last = v;
      expect(m.value).toBeCloseTo(model, 9);
    }
    expect(last).toBeGreaterThan(0);
  });

  it('[UX-06a] [AT-14] shows a new level at most 30 times per second', () => {
    const m = new LevelMeter(ME);
    const times: number[] = [];
    for (let t = 0; t <= 1000; t += 2) {
      const v = m.push(f(t, t % 100 < 50 ? -10 : -60));
      if (v !== undefined) times.push(t);
    }
    expect(times.length).toBeLessThanOrEqual(31);
    expect(times.length).toBeGreaterThan(20);
    times.slice(1).forEach((t, i) => expect(t - (times[i] ?? 0)).toBeGreaterThanOrEqual(1000 / 30));
  });

  it('[UX-06a] [UX-09a] shows nothing new while the level does not change', () => {
    const m = new LevelMeter(ME);
    expect(run(m, 0, 2000, -60)).toEqual([]);
  });
});

describe('UX-06b: level = 0 when the MOG does not own an open mic', () => {
  it('[UX-06b] [AT-14] frames owned by another MOG never move this level', () => {
    const m = new LevelMeter(ME);
    for (let t = 0; t <= 500; t += 10) expect(m.push(f(t, -10, OTHER))).toBeUndefined();
    expect(m.value).toBe(0);
  });

  it('[UX-06b] [AT-14] drops to 0 at once when the mic closes or moves to another MOG', () => {
    for (const owner of [null, OTHER]) {
      const m = new LevelMeter(ME);
      run(m, 0, 500, -10);
      expect(m.value).toBeGreaterThan(0.9);
      expect(m.push(f(505, -10, owner))).toBe(0);
      expect(m.value).toBe(0);
      expect(m.push(f(510, -10, owner))).toBeUndefined();
    }
  });

  it('[UX-06b] [AT-14] after the mic comes back, the meter starts again from 0', () => {
    const m = new LevelMeter(ME);
    run(m, 0, 500, -10);
    m.push(f(505, -10, null));
    m.push(f(2000, -10));
    expect(m.push(f(2030, -10))).toBeCloseTo(1 - Math.exp(-1), 6);
  });

  // The MOG view's default source (the `mog.level` frames the brain sends, CAP-16a) is tested
  // in MogView.test.tsx: `[UX-06a] [UX-06b] [CAP-16a] with no levels prop ...`.
});
