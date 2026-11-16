/**
 * The view latency rows (#13, ADR 0031): UX-05b (key-down to the `listening` frame) and UX-36
 * (a pipeline step event in the brain to its frame in the view). The rules match the Rust
 * benchmark (ADR 0016): nearest-rank percentiles, 20 samples or more, and a sample with no end
 * mark counts as over the limit. Both the brain and the views use this file.
 */
import type { ViewTrace } from './ipc';

/** A time row needs this many samples; below it the p95 is the maximum and has no meaning. */
export const MIN_TIME_SAMPLES = 20;

/** UX-05b: p95 < 100 ms (strict). UX-36: p95 <= 100 ms. */
export const VIEW_LATENCY_ROWS = {
  'UX-05b': { limitMs: 100, strict: true },
  'UX-36': { limitMs: 100, strict: false },
} as const;

export type ViewLatencyRow = keyof typeof VIEW_LATENCY_ROWS;

export interface RowResult {
  row: ViewLatencyRow;
  n: number;
  p50?: number;
  p95?: number;
  verdict: 'pass' | 'fail' | 'too_few_samples';
}

/** The nearest-rank percentile `p` (0..100) of `samples`, or undefined when there are none. */
export function nearestRank(samples: readonly number[], p: number): number | undefined {
  if (samples.length === 0) return undefined;
  const sorted = [...samples].sort((a, b) => a - b);
  const rank = Math.max(1, Math.ceil((p / 100) * sorted.length));
  return sorted[rank - 1];
}

/** Check one row. `undefined` is a sample whose end mark never came: it counts as Infinity. */
export function checkRow(row: ViewLatencyRow, samples: readonly (number | undefined)[]): RowResult {
  const values = samples.map((v) => v ?? Infinity);
  const n = values.length;
  const p50 = nearestRank(values, 50);
  const p95 = nearestRank(values, 95);
  const base = {
    row,
    n,
    ...(p50 === undefined ? {} : { p50 }),
    ...(p95 === undefined ? {} : { p95 }),
  };
  if (n < MIN_TIME_SAMPLES || p95 === undefined) return { ...base, verdict: 'too_few_samples' };
  const { limitMs, strict } = VIEW_LATENCY_ROWS[row];
  const within = strict ? p95 < limitMs : p95 <= limitMs;
  return { ...base, verdict: within ? 'pass' : 'fail' };
}

/**
 * The time base the brain and the view windows share: `performance.timeOrigin + now()`, in ms.
 * Each window has its own `performance.now()` origin, so a span across windows needs this.
 */
export function epochNow(): number {
  return performance.timeOrigin + performance.now();
}

/**
 * The optional trace of one `mog.view_state` (only with `VITE_BOTTLE_LATENCY_TRACE=1`). It has
 * a sequence number and times only: no text, transcript or token (SEC-16b). The type is in the
 * contract (`src/shared/ipc.ts`, #285).
 */
export type { ViewTrace };

const TRACE_KEYS: ReadonlySet<string> = new Set(['seq', 'eventAt', 'sentAt', 'keyDownAt']);

/** A trace with exactly the known number fields, or undefined. */
export function asViewTrace(v: unknown): ViewTrace | undefined {
  if (typeof v !== 'object' || v === null) return undefined;
  const o = v as Record<string, unknown>;
  if (!Object.keys(o).every((k) => TRACE_KEYS.has(k))) return undefined;
  const num = (x: unknown) => typeof x === 'number' && Number.isFinite(x);
  if (!num(o['seq']) || !num(o['eventAt']) || !num(o['sentAt'])) return undefined;
  if (o['keyDownAt'] !== undefined && !num(o['keyDownAt'])) return undefined;
  return o as unknown as ViewTrace;
}
