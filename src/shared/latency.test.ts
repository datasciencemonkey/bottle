import {
  MIN_TIME_SAMPLES,
  VIEW_LATENCY_ROWS,
  asViewTrace,
  checkRow,
  epochNow,
  nearestRank,
} from './latency';

const many = (n: number, v: number) => Array.from({ length: n }, () => v);

describe('view latency rows (#13, ADR 0031)', () => {
  it('[UX-05b] [UX-36] [AT-21] the budgets are the ones of the rows', () => {
    expect(VIEW_LATENCY_ROWS['UX-05b']).toEqual({ limitMs: 100, strict: true });
    expect(VIEW_LATENCY_ROWS['UX-36']).toEqual({ limitMs: 100, strict: false });
    expect(MIN_TIME_SAMPLES).toBe(20);
  });

  it('[AT-21] the percentile is the nearest rank (ADR 0016)', () => {
    const s = Array.from({ length: 20 }, (_, i) => i + 1); // 1..20
    expect(nearestRank(s, 50)).toBe(10);
    expect(nearestRank(s, 95)).toBe(19);
    expect(nearestRank(s, 100)).toBe(20);
    expect(nearestRank([5, 1, 3], 50)).toBe(3);
    expect(nearestRank([], 95)).toBeUndefined();
  });

  it('[UX-05b] [AT-21] the listening row needs a p95 strictly under 100 ms', () => {
    expect(checkRow('UX-05b', many(20, 99.9)).verdict).toBe('pass');
    expect(checkRow('UX-05b', many(20, 100)).verdict).toBe('fail');
  });

  it('[UX-36] [AT-21] the frame row allows a p95 of exactly 100 ms', () => {
    expect(checkRow('UX-36', many(20, 100)).verdict).toBe('pass');
    expect(checkRow('UX-36', many(20, 100.1)).verdict).toBe('fail');
  });

  it('[UX-05b] [UX-36] [AT-21] fewer than 20 samples give no verdict', () => {
    const r = checkRow('UX-36', many(19, 10));
    expect(r.verdict).toBe('too_few_samples');
    expect(r.n).toBe(19);
  });

  it('[UX-05b] [UX-36] [AT-21] a sample with no end mark counts as over the limit', () => {
    // 18 fast frames and 2 that never came: the p95 is the missing frame.
    const r = checkRow('UX-36', [...many(18, 20), undefined, undefined]);
    expect(r.n).toBe(20);
    expect(r.p50).toBe(20);
    expect(r.p95).toBe(Infinity);
    expect(r.verdict).toBe('fail');
  });

  it('[UX-36] one late frame in 20 is within the p95 rule', () => {
    const r = checkRow('UX-36', [...many(19, 30), 400]);
    expect(r.p95).toBe(30);
    expect(r.verdict).toBe('pass');
  });

  it('[UX-36] the trace has only a sequence number and times', () => {
    expect(asViewTrace({ seq: 1, eventAt: 1, sentAt: 2 })).toEqual({
      seq: 1,
      eventAt: 1,
      sentAt: 2,
    });
    expect(asViewTrace({ seq: 1, eventAt: 1, sentAt: 2, keyDownAt: 0 })?.keyDownAt).toBe(0);
    expect(asViewTrace({ seq: 1, eventAt: 1, sentAt: 2, text: 'hello' })).toBeUndefined();
    expect(asViewTrace({ seq: 1, eventAt: 'x', sentAt: 2 })).toBeUndefined();
    expect(asViewTrace({ seq: 1, eventAt: Infinity, sentAt: 2 })).toBeUndefined();
    expect(asViewTrace(undefined)).toBeUndefined();
  });

  it('[UX-36] both windows share one time base', () => {
    const a = epochNow();
    expect(Math.abs(a - Date.now())).toBeLessThan(1000);
    expect(epochNow()).toBeGreaterThanOrEqual(a);
  });
});
