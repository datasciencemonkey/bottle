/**
 * The mic level stream in the brain (CAP-16a, CAP-16b, #23, ADR 0067).
 *
 * R2 sends one RMS level in dBFS at 30 Hz or less on the `capture_subscribe` `onLevel`
 * Channel, only while a capture records (`mog_core::audio_device::level`). The brain sends
 * each level only to the target MOG as `mog.level` (ADR 0040): the `directedTo` MOG of a
 * directed PTT in a session, else the `owner`. Other MOGs get no frame, so their meter stays
 * at 0 (CAP-16a, UX-06b).
 *
 * - When the target changes or the capture stops or pauses, the old target gets one frame with
 *   `owner: null`, and its meter drops to 0 (UX-06b).
 * - With no target, a level is dropped: no level goes out after the stop (CAP-16b), even if a
 *   late Channel message arrives.
 *
 * The brain start-up calls `subscribeLevels` with `wireLevelStream`. The host routes
 * `capture_subscribe` to `CaptureCore` and sends the levels on the `onLevel` Channel (#476).
 *
 * TODO(#475): a level carries no capture ID, so a level that arrives after the next capture's
 * `capture:state` goes to the next target. The contract change binds each level to its capture.
 */
import { Channel } from '@tauri-apps/api/core';
import type { CaptureState, MogId } from '../shared/interfaces';
import { EVENTS, call, type MicLevelFrame } from '../shared/ipc';
import type { BrainBus } from './bus';

/**
 * The dBFS value of the `owner: null` frame: the R2 silence floor (`LEVEL_FLOOR_DBFS`). It is
 * finite because JSON has no `-Infinity`.
 */
export const SILENT_DBFS = -100;

/** Where a frame goes: `sendMicLevel` of `bus.ts` in the app. */
export type SendLevel = (mogId: MogId, frame: MicLevelFrame) => void | Promise<void>;

/** The MOG that gets the level: the directed MOG, else the owner; none when not recording. */
export function levelTargetOf(state: CaptureState): MogId | null {
  if (!state.recording || state.paused) return null;
  return state.directedTo ?? state.owner ?? null;
}

export class LevelRouter {
  readonly #send: SendLevel;
  readonly #now: () => number;
  #target: MogId | null = null;

  constructor(send: SendLevel, now: () => number) {
    this.#send = send;
    this.#now = now;
  }

  /** The MOG that gets the level now, or null. */
  get target(): MogId | null {
    return this.#target;
  }

  /** One `capture:state` item. */
  onState(state: CaptureState): void {
    const next = levelTargetOf(state);
    const prev = this.#target;
    if (next === prev) return;
    this.#target = next;
    if (prev !== null) this.#deliver(prev, { owner: null, dbfs: SILENT_DBFS, t: this.#now() });
  }

  /**
   * One `onLevel` Channel message: a dBFS number. Anything else is dropped.
   * TODO(#475): the message names no capture, so it goes to the current target.
   */
  onLevel(dbfs: unknown): void {
    const to = this.#target;
    if (to === null || typeof dbfs !== 'number' || !Number.isFinite(dbfs)) return;
    this.#deliver(to, { owner: to, dbfs, t: this.#now() });
  }

  #deliver(to: MogId, frame: MicLevelFrame): void {
    try {
      void Promise.resolve(this.#send(to, frame)).catch(() => undefined);
    } catch {
      // A failed send drops one frame; the next frame tries again.
    }
  }
}

/** Feed the router from `capture:state` on the brain Channel. Returns `off`. */
export function wireLevelStream(bus: BrainBus, router: LevelRouter): () => void {
  return bus.on(EVENTS['capture.state'].name, (p) => {
    if (typeof p === 'object' && p !== null) router.onState(p as CaptureState);
  });
}

/**
 * Register the `onLevel` Channel with `capture_subscribe`. Resolves false when the core
 * refuses (the reason is not logged). `onPartial` has no reader yet: TODO(#514).
 */
export async function subscribeLevels(router: LevelRouter): Promise<boolean> {
  const onLevel = new Channel<number>();
  onLevel.onmessage = (dbfs) => router.onLevel(dbfs);
  const onPartial = new Channel<string>();
  try {
    await call('capture_subscribe', { onLevel, onPartial });
    return true;
  } catch {
    return false;
  }
}
