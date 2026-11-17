/**
 * The MOG level meter (UX-06a, UX-06b, Table UX-LEVEL, ADR 0027). Pure: the caller pushes mic
 * frames with their time, and the meter says when the mascot must show a new level.
 *
 * - `level = clamp((dBFS + 60) / 50, 0, 1)`.
 * - A one-pole smoother: attack 30 ms when the level rises, release 150 ms when it falls.
 * - It shows a new level at most 30 times per second, and only when the level changes, so an
 *   idle mascot gets no writes (UX-09a).
 * - Only frames whose owner is this MOG move the level. Any other frame sets 0 at once (UX-06b).
 */
import type { MogId } from '../../shared/interfaces';
import type { MicLevelFrame } from '../../shared/ipc';

/** One mic level sample: the contract `mog.level` payload (#285, ADR 0040). */
export type { MicLevelFrame } from '../../shared/ipc';

/** Table UX-LEVEL. */
export const LEVEL = {
  floorDb: -60,
  rangeDb: 50,
  attackMs: 30,
  releaseMs: 150,
  maxHz: 30,
} as const;

const MIN_GAP_MS = 1000 / LEVEL.maxHz;
/** A change smaller than this is not worth a frame. */
const EPSILON = 1e-4;

/**
 * Where the MOG view gets mic frames. When the mic closes or moves to another MOG, the source
 * must send one frame with the new owner (`null` when closed): the meter sets 0 only on such a
 * frame (UX-06b), so a source that just stops would leave the last level on screen.
 * The MOG window uses `viewMicLevels()` (`view-ipc.ts`), which reads the `mog.level` event.
 * The brain feeds `sendMicLevel` from the CaptureService level stream (`capture_subscribe`
 * `onLevel`) and sends the `owner: null` frame when the mic closes (`src/brain/level-stream.ts`,
 * #23). TODO(#425): the host routes `capture_subscribe`.
 */
export interface MicLevelSource {
  subscribe(onFrame: (frame: MicLevelFrame) => void): () => void;
}

/** No mic frames: the level stays 0. The default until #4 lands. */
export const NO_MIC_LEVELS: MicLevelSource = { subscribe: () => () => undefined };

/** The target level for one dBFS reading. Silence (-Infinity) and NaN give 0. */
export function levelTarget(dbfs: number): number {
  if (Number.isNaN(dbfs)) return 0;
  return Math.min(1, Math.max(0, (dbfs - LEVEL.floorDb) / LEVEL.rangeDb));
}

export class LevelMeter {
  readonly #mogId: MogId;
  #value = 0;
  #lastT: number | undefined;
  #shown = 0;
  #shownAt = -Infinity;

  constructor(mogId: MogId) {
    this.#mogId = mogId;
  }

  /** The smoothed level now (it may not be shown yet). */
  get value(): number {
    return this.#value;
  }

  /** Feed one frame. Returns the level to show, or undefined when nothing must change. */
  push(frame: MicLevelFrame): number | undefined {
    if (frame.owner !== this.#mogId) {
      this.#value = 0;
      this.#lastT = undefined;
      return this.#show(0, frame.t, true);
    }
    const target = levelTarget(frame.dbfs);
    const dt = this.#lastT === undefined ? 0 : Math.max(0, frame.t - this.#lastT);
    this.#lastT = frame.t;
    const tau = target > this.#value ? LEVEL.attackMs : LEVEL.releaseMs;
    this.#value += (target - this.#value) * (1 - Math.exp(-dt / tau));
    return this.#show(this.#value, frame.t, false);
  }

  #show(v: number, t: number, now: boolean): number | undefined {
    if (Math.abs(v - this.#shown) < EPSILON && !(v === 0 && this.#shown !== 0)) return undefined;
    if (!now && t - this.#shownAt < MIN_GAP_MS) return undefined;
    this.#shown = v;
    this.#shownAt = t;
    return v;
  }
}
