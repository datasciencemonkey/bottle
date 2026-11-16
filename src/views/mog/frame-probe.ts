/**
 * The MOG view frame probe (#13, ADR 0031). For each `mog.view_state` with a trace, it notes
 * when the view applied the state and when the next frame ran (`requestAnimationFrame`): the
 * frame that draws the new mood. With the brain's trace times this gives the UX-36 span (event
 * in the brain to frame) and, for `listening`, the UX-05b span (key-down in the brain to frame).
 * Times use the shared time base (`epochNow`). Samples hold numbers and mood names only.
 * The MOG entry attaches it only with `VITE_BOTTLE_LATENCY_TRACE=1`; the device run is #355.
 */
import type { Mood } from '../../shared/interfaces';
import { asViewTrace, checkRow, epochNow, type RowResult } from '../../shared/latency';

export interface ViewFrameSample {
  seq: number;
  mood: Mood;
  eventAt: number;
  sentAt: number;
  keyDownAt?: number;
  receivedAt: number;
  frameAt: number;
}

export interface FrameProbeOptions {
  now?: () => number;
  requestFrame?: (cb: () => void) => void;
  /** The most samples kept (the newest). */
  max?: number;
}

type Pending = Omit<ViewFrameSample, 'frameAt'>;

export class FrameProbe {
  readonly #now: () => number;
  readonly #requestFrame: (cb: () => void) => void;
  readonly #max: number;
  readonly #samples: ViewFrameSample[] = [];
  readonly #pending = new Set<Pending>();

  constructor(opts: FrameProbeOptions = {}) {
    this.#now = opts.now ?? epochNow;
    this.#requestFrame = opts.requestFrame ?? ((cb) => void requestAnimationFrame(() => cb()));
    this.#max = opts.max ?? 1000;
  }

  /** The view applied `mood` to the mascot. `trace` is the payload's trace, unchecked. */
  onApplied(mood: Mood, trace: unknown): void {
    const t = asViewTrace(trace);
    if (!t) return;
    const p: Pending = {
      seq: t.seq,
      mood,
      eventAt: t.eventAt,
      sentAt: t.sentAt,
      receivedAt: this.#now(),
    };
    if (t.keyDownAt !== undefined) p.keyDownAt = t.keyDownAt;
    this.#pending.add(p);
    this.#requestFrame(() => {
      this.#pending.delete(p);
      this.#samples.push({ ...p, frameAt: this.#now() });
      if (this.#samples.length > this.#max)
        this.#samples.splice(0, this.#samples.length - this.#max);
    });
  }

  samples(): readonly ViewFrameSample[] {
    return this.#samples;
  }

  /** Both rows. A state whose frame has not run yet counts as over the limit. */
  report(): { 'UX-05b': RowResult; 'UX-36': RowResult } {
    const pending = [...this.#pending];
    const listening = (x: { mood: Mood; keyDownAt?: number }) =>
      x.mood === 'listening' && x.keyDownAt !== undefined;
    return {
      'UX-05b': checkRow('UX-05b', [
        ...this.#samples.filter(listening).map((x) => x.frameAt - (x.keyDownAt ?? NaN)),
        ...pending.filter(listening).map(() => undefined),
      ]),
      'UX-36': checkRow('UX-36', [
        ...this.#samples.map((x) => x.frameAt - x.eventAt),
        ...pending.map(() => undefined),
      ]),
    };
  }

  /** The samples and both rows as JSON, for the Web Inspector console (#355). */
  toJSON(): string {
    return JSON.stringify({ samples: this.#samples, rows: this.report() });
  }
}
