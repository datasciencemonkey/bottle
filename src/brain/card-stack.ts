/**
 * The card stack controller of one MOG (UX-11a..UX-12, ADR 0028). It owns a `BlurbStack`, sends
 * `card.update` after each change, and keeps one timer armed at the next collapse time.
 *
 * The `card.update` payload is the contract `CardUpdate` (#285).
 * TODO(#374): the brain's `view_emit` handler routes the card's `hover` and `click` input to
 * `input()`.
 * TODO(#77): the settings come from `mog.card.maxBlurbs` and `mog.card.autoHideSec` in the store.
 * TODO(#86): blurb templates. Chips and expanded sections: #89, ADR 0043.
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
import { ChipRow, type ChipAction, type ChipInput, type ChipView } from './card-chips';
import { buildSections, type ExpandedSection, type SectionInput } from './card-sections';
import type { MonotonicClock } from './clock';

/** The `card.update` payload this controller sends (the view-local `CardUpdate` mirrors it). */
export interface CardStackUpdate {
  mogId: MogId;
  blurbs: CardBlurb[];
  historyCount: number;
  /** The chip row: 4 chips or less (UX-20a). */
  chips: ChipView[];
  /** The chips after the 4th: the view shows them in the expanded card (UX-20a). */
  overflowChips: ChipView[];
  /** The expanded card sections in the UX-EXP order (UX-18). Absent while the card is small. */
  sections?: ExpandedSection[];
  /** The STT badge of the card footer (UX-38), from `sttBadgeText` (ADR 0029). */
  sttBadge?: string;
}

/** Run `fn` at monotonic time `atMs`. Returns a cancel function. */
export type Scheduler = (atMs: number, fn: () => void) => () => void;

export interface CardStackDeps {
  clock: MonotonicClock;
  schedule: Scheduler;
  send(update: CardStackUpdate): void | Promise<void>;
  /** The "N more in History" link. TODO(#103): open the hub History for this MOG. */
  openHistory(mogId: MogId): void;
  /** Runs the action of a chip after a click on it (UX-20c). TODO(#285): route to the agent. */
  onChip?(action: ChipAction): void;
}

export interface CardStack {
  add(b: BlurbInput): void;
  complete(blurbId: string): void;
  /** Card input from `view_emit` (`hover`, `click`). Malformed input is dropped. */
  input(kind: ViewEmitKind, payload: unknown): void;
  /** UX-38: set the footer STT badge. TODO(#52): call it with the provider of each Transcript. */
  setSttBadge(text: string | undefined): void;
  /** Offer a chip (Appendix E). The row keeps 4 or less, the rest go to the expanded card. */
  offerChip(c: ChipInput): void;
  /** Remove a chip, or all chips of a blurb or request that ended. */
  removeChip(id: string): void;
  removeChipsOf(owner: string): void;
  /** UX-18: set the content of the expanded card, or `undefined` to show the small card. */
  setExpanded(input: SectionInput | undefined): void;
  /** Send the current state again, for a card view that starts to listen late (#498). */
  republish(): void;
  dispose(): void;
}

/** The longest `setTimeout` delay. A longer one fires at once, so the delay is capped. */
const MAX_TIMEOUT_MS = 2 ** 31 - 1;

/**
 * A `Scheduler` on `setTimeout` for the production clock. A capped timer fires early; the
 * controller then finds nothing due and arms again for the rest of the time.
 */
export function timeoutScheduler(clock: MonotonicClock): Scheduler {
  return (at, fn) => {
    const h = setTimeout(fn, Math.min(MAX_TIMEOUT_MS, Math.max(0, at - clock.now())));
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

const earliest = (a: number | undefined, b: number | undefined) =>
  a === undefined ? b : b === undefined ? a : Math.min(a, b);

function isChipClick(p: unknown): string | undefined {
  const id = typeof p === 'object' && p !== null ? (p as { chipId?: unknown }).chipId : undefined;
  return typeof id === 'string' ? id : undefined;
}

export function createCardStack(
  mogId: MogId,
  deps: CardStackDeps,
  settings?: Partial<StackSettings>,
): CardStack {
  const stack = new BlurbStack(settings);
  let cancel: (() => void) | undefined;
  let armedAt: number | undefined;
  let sttBadge: string | undefined;
  let expanded: SectionInput | undefined;
  const row = new ChipRow();

  const publish = () => {
    const v = stack.view();
    const c = row.view();
    const u: CardStackUpdate = {
      mogId,
      blurbs: v.blurbs,
      historyCount: v.historyCount,
      chips: c.chips,
      overflowChips: c.overflow,
    };
    if (sttBadge !== undefined) u.sttBadge = sttBadge;
    if (expanded)
      u.sections = buildSections(sttBadge === undefined ? expanded : { sttBadge, ...expanded });
    void deps.send(u);
  };

  const arm = () => {
    const at = earliest(stack.nextDeadline(), row.nextDeadline());
    if (at === armedAt) return;
    cancel?.();
    cancel = undefined;
    armedAt = at;
    if (at !== undefined) cancel = deps.schedule(at, onTimer);
  };

  function onTimer() {
    cancel = undefined;
    armedAt = undefined;
    const now = deps.clock.now();
    const collapsed = stack.tick(now).length;
    if (row.tick(now).length || collapsed) publish();
    arm();
  }

  const changed = () => {
    publish();
    arm();
  };

  return {
    add(b) {
      if (stack.add(b, deps.clock.now())) changed();
    },
    complete(blurbId) {
      if (stack.complete(blurbId, deps.clock.now())) changed();
    },
    input(kind, payload) {
      if (kind === 'chip_click') {
        // UX-20c: a chip acts only on a click. An unknown or expired chip does nothing.
        const id = isChipClick(payload);
        const action = id === undefined ? undefined : row.click(id);
        if (action) {
          deps.onChip?.(action);
          changed();
        }
        return;
      }
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
    setSttBadge(text) {
      if (text === sttBadge) return;
      sttBadge = text;
      publish();
    },
    offerChip(c) {
      if (row.offer(c, deps.clock.now())) changed();
    },
    removeChip(id) {
      if (row.remove(id)) changed();
    },
    removeChipsOf(owner) {
      if (row.removeOwner(owner).length) changed();
    },
    setExpanded(input) {
      expanded = input;
      publish();
    },
    republish: () => publish(),
    dispose() {
      cancel?.();
      cancel = undefined;
      armedAt = undefined;
    },
  };
}
