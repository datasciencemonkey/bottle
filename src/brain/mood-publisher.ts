/**
 * The brain step from mood sources to `mog.view_state` (#13, ADR 0031). Per MOG it runs
 * `mapMood`, then the UX-05a hold (`holdMood`), and sends the result in the same task: no
 * await and no timer stand between the event and the send (UX-05b, UX-36). A change the hold
 * defers is sent when the hold ends, on the injected scheduler.
 *
 * With `traceNow` (only when `VITE_BOTTLE_LATENCY_TRACE=1`), each state carries a `ViewTrace`
 * for the view frame probe. TODO(#14): `startBrain` builds one publisher with
 * `sendMogViewState` and `traceNow: latencyTraceClock(import.meta.env.VITE_BOTTLE_LATENCY_TRACE)`,
 * and the capture lifecycle calls `update`. The device run is #355. TODO(#39): the OperationTracker
 * calls `update` for each request phase, and a recheck runs when an error or success window ends.
 */
import type { Mood, MogId } from '../shared/interfaces';
import { epochNow, type ViewTrace } from '../shared/latency';
import type { MonotonicClock } from './clock';
import { holdMood, mapMood, type HeldMood, type MoodSources } from './components/mood-mapper';

/** What the brain sends on `mog.view_state`. TODO(#285): the contract `MogViewState`. */
export interface PublishedViewState {
  mogId: MogId;
  mood: Mood;
  overlays: ReturnType<typeof mapMood>['overlays'];
  trace?: ViewTrace;
}

/** Runs `fn` at a time on the publisher clock. Returns a cancel function. */
export interface Scheduler {
  at(atMs: number, fn: () => void): () => void;
}

/** The times of the event behind an update, on the `traceNow` clock. */
export interface UpdateCause {
  eventAt: number;
  keyDownAt?: number;
}

export interface MoodPublisherOptions {
  clock: MonotonicClock;
  send: (state: PublishedViewState) => void | Promise<void>;
  scheduler: Scheduler;
  /** The shared time base for traces (`epochNow`). Leave it out to send no trace. */
  traceNow?: (() => number) | undefined;
}

export interface MoodPublisher {
  /** The sources of `mogId` changed. `cause` defaults to now on the trace clock. */
  update(mogId: MogId, sources: MoodSources, cause?: UpdateCause): void;
}

interface MogSlot {
  sources: MoodSources;
  held?: HeldMood;
  lastKey?: string;
  cancel?: (() => void) | undefined;
  /** The cause of a mood change the hold defers. */
  pending?: UpdateCause | undefined;
}

/** The trace clock for the build flag: `epochNow` when it is `1`, else no trace. */
export function latencyTraceClock(flag: string | undefined): (() => number) | undefined {
  return flag === '1' ? epochNow : undefined;
}

export function createMoodPublisher(opts: MoodPublisherOptions): MoodPublisher {
  const { clock, send, scheduler, traceNow } = opts;
  const slots = new Map<MogId, MogSlot>();
  let seq = 0;

  const run = (mogId: MogId, slot: MogSlot, cause: UpdateCause | undefined): void => {
    slot.cancel?.();
    slot.cancel = undefined;
    const now = clock.now();
    const target = mapMood(mogId, slot.sources, now);
    const { shown, recheckAt } = holdMood(slot.held, target.mood, now);
    slot.held = shown;
    const deferred = shown.mood !== target.mood;
    // The deferred change keeps the time its first event reached the brain.
    const why = deferred ? cause : (slot.pending ?? cause);
    if (deferred) slot.pending ??= cause;
    else slot.pending = undefined;
    if (recheckAt !== undefined) {
      slot.cancel = scheduler.at(recheckAt, () => run(mogId, slot, undefined));
    }
    const key = `${shown.mood}|${JSON.stringify(target.overlays)}`;
    if (key === slot.lastKey) return;
    slot.lastKey = key;
    const state: PublishedViewState = { mogId, mood: shown.mood, overlays: target.overlays };
    if (traceNow) {
      const sentAt = traceNow();
      state.trace = { seq: ++seq, eventAt: why?.eventAt ?? sentAt, sentAt };
      if (why?.keyDownAt !== undefined) state.trace.keyDownAt = why.keyDownAt;
    }
    void send(state);
  };

  return {
    update(mogId, sources, cause) {
      const slot = slots.get(mogId) ?? { sources };
      slot.sources = sources;
      slots.set(mogId, slot);
      run(mogId, slot, cause);
    },
  };
}
