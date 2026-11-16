/**
 * The card stack controller of one MOG (UX-11a..UX-12, ADR 0028). It owns a `BlurbStack`, sends
 * `card.update` after each change, and keeps one timer armed at the next collapse time.
 *
 * TODO(#285): the brain's `view_emit` handler routes the card's `hover` and `click` input to
 * `input()`, and the `card.update` payload type moves to the contract.
 * TODO(#77): the settings come from `mog.card.maxBlurbs` and `mog.card.autoHideSec` in the store.
 * TODO(#86, #89): blurb templates and chips; this controller sends no chips yet.
 */
import type { MogId } from '../shared/interfaces';
import type { ViewEmitKind } from '../shared/ipc';
import { sendCardUpdate } from './bus';
import {
  BlurbStack,
  decodeCardInput,
  type BlurbInput,
  type CardBlurb,
  type StackSettings,
} from './blurb-stack';
import type { MonotonicClock } from './clock';

/** The `card.update` payload this controller sends (the view-local `CardUpdate` mirrors it). */
export interface CardStackUpdate {
  mogId: MogId;
  blurbs: CardBlurb[];
  historyCount: number;
  chips: { id: string; label: string }[];
}

/** Run `fn` at monotonic time `atMs`. Returns a cancel function. */
export type Scheduler = (atMs: number, fn: () => void) => () => void;

export interface CardStackDeps {
  clock: MonotonicClock;
  schedule: Scheduler;
  send(update: CardStackUpdate): void | Promise<void>;
  /** The "N more in History" link. TODO(#103): open the hub History for this MOG. */
  openHistory(mogId: MogId): void;
}

export interface CardStack {
  add(b: BlurbInput): void;
  complete(blurbId: string): void;
  /** Card input from `view_emit` (`hover`, `click`). Malformed input is dropped. */
  input(kind: ViewEmitKind, payload: unknown): void;
  dispose(): void;
}

/** A `Scheduler` on `setTimeout` for the production clock. */
export function timeoutScheduler(clock: MonotonicClock): Scheduler {
  return (at, fn) => {
    const h = setTimeout(fn, Math.max(0, at - clock.now()));
    return () => clearTimeout(h);
  };
}

/** Production deps: the system clock, `setTimeout` and `sendCardUpdate` to `card-<id>`. */
export function cardStackDeps(mogId: MogId, clock: MonotonicClock): CardStackDeps {
  return {
    clock,
    schedule: timeoutScheduler(clock),
    send: (u) => sendCardUpdate(mogId, u),
    openHistory: () => undefined, // TODO(#103): open the hub History view.
  };
}

export function createCardStack(
  mogId: MogId,
  deps: CardStackDeps,
  settings?: Partial<StackSettings>,
): CardStack {
  const stack = new BlurbStack(settings);
  let cancel: (() => void) | undefined;
  let armedAt: number | undefined;

  const publish = () => {
    const v = stack.view();
    void deps.send({ mogId, blurbs: v.blurbs, historyCount: v.historyCount, chips: [] });
  };

  const arm = () => {
    const at = stack.nextDeadline();
    if (at === armedAt) return;
    cancel?.();
    cancel = undefined;
    armedAt = at;
    if (at !== undefined) cancel = deps.schedule(at, onTimer);
  };

  function onTimer() {
    cancel = undefined;
    armedAt = undefined;
    if (stack.tick(deps.clock.now()).length) publish();
    arm();
  }

  const changed = () => {
    publish();
    arm();
  };

  return {
    add(b) {
      stack.add(b, deps.clock.now());
      changed();
    },
    complete(blurbId) {
      if (stack.complete(blurbId, deps.clock.now())) changed();
    },
    input(kind, payload) {
      const i = decodeCardInput(kind, payload);
      if (!i) return;
      const now = deps.clock.now();
      if (i.t === 'open_history') return deps.openHistory(mogId);
      if (i.t === 'hover') {
        stack.hover(i.entryId, i.via, i.on, now);
        return arm(); // hover changes no visible state
      }
      const pinned = stack.view().blurbs.find((b) => b.id === i.entryId)?.pinned;
      if (pinned ? stack.unpin(i.entryId, now) : stack.pin(i.entryId)) changed();
    },
    dispose() {
      cancel?.();
      cancel = undefined;
      armedAt = undefined;
    },
  };
}
