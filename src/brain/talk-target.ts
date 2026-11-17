/**
 * T1 talk-key target in the brain (#73, MB-06a, MB-06b, UX-28, ADR 0058).
 *
 * The brain keeps the open MOGs in the order of their last interaction. The target is the
 * first one. Rust keeps the same list (`PttTarget` in `src-tauri/src/hotkey_host.rs`) and
 * latches the target at key-down. The marker must never show a MOG that the talk key does not
 * talk to (UX-28), so the brain moves its target only on signals that Rust also sees, or after
 * Rust accepts `mog_set_active`.
 *
 * Rules:
 * - The first `view.ready` of a MOG view makes that MOG open and the target (Rust sees the
 *   window open). A reload sends `view.ready` again; it is not an interaction (#305 note A).
 * - A click, poke or drag end on a MOG makes it the target only when `mog_set_active` answers
 *   ok. MOG windows never take focus (MOG-01b), so Rust sees nothing else of these.
 * - A card load or focus (`view.focus`) makes its MOG the target at once: Rust moves on the
 *   same two signals. Other card input, except hover, waits for `mog_set_active` as a MOG click
 *   does. A card never proves that its MOG window is open (#305 note B).
 * - When the target MOG closes, the target moves to the most recently interacted open MOG.
 * - A `hotkey:ptt` down for an open MOG makes it the target (Rust latched it) and shows the
 *   toast with its name for 1.2 s (MB-06b).
 *
 * TODO(#439): `mog_set_active` answers `not_implemented` until the contract routes it, so a
 * MOG click does not move the target yet. The event name and the ready signal move into the
 * contract.
 * TODO(#83): a drag on the macOS native panel reaches the brain only when #83 relays drag_end.
 * TODO(#62): a chord use (CAP-46a, v1) calls `interacted` for the chord MOG.
 */
import { emitTo } from '@tauri-apps/api/event';
import type { MogId } from '../shared/interfaces';
import { call, EVENTS } from '../shared/ipc';
import {
  isViewFocus,
  isViewReady,
  MOG_TARGET_EVENT,
  TARGET_TOAST_MS,
  type MogTargetView,
} from '../shared/mog-target';
import { onViewInput, viewLabel, type BrainBus } from './bus';

/** The name a MOG shows before (or without) its config name. */
const DEFAULT_NAME = 'MOG';

export interface TalkTargetDeps {
  /** Send one target view to `mog-<id>`. */
  send: (view: MogTargetView) => unknown;
  /** Tell Rust the new target (`mog_set_active`). A refusal leaves the target as it is. */
  setActive: (mogId: MogId) => Promise<unknown>;
  /** Run `fn` after `ms`; the result cancels it. */
  after: (ms: number, fn: () => void) => () => void;
  /** The display name of a MOG. A failure or `undefined` shows "MOG". */
  nameOf?: (mogId: MogId) => Promise<string | undefined>;
}

/** How an interaction reaches Rust: by a window signal it sees, or only by `mog_set_active`. */
export type InteractionVia = 'focus' | 'input';

export interface TalkTarget {
  /** A MOG view mounted (first open or reload). */
  viewReady(mogId: MogId): void;
  /**
   * The user interacted with an open MOG. `'focus'` is a signal that Rust sees too (a card load
   * or focus): it moves the target at once. `'input'` (the default) moves it only when
   * `mog_set_active` answers ok. False if the MOG is not open.
   */
  interacted(mogId: MogId, via?: InteractionVia): boolean;
  /** `hotkey:ptt` down for `mogId`. False if the MOG is not open. */
  pttDown(mogId: MogId): boolean;
  /** The MOG window closed. */
  closed(mogId: MogId): void;
  /** The target now. */
  current(): MogId | undefined;
  /** True while the MOG window is open. */
  isOpen(mogId: MogId): boolean;
}

export function createTalkTarget(deps: TalkTargetDeps): TalkTarget {
  /** Open MOGs, most recently interacted first. */
  const mru: MogId[] = [];
  const names = new Map<MogId, string>();
  let toastFor: MogId | undefined;
  let cancelToast: (() => void) | undefined;

  const current = () => mru[0];
  const isOpen = (id: MogId) => mru.includes(id);
  const view = (id: MogId): MogTargetView => ({
    mogId: id,
    name: names.get(id) ?? DEFAULT_NAME,
    target: current() === id,
    toast: toastFor === id && current() === id,
  });
  const send = (id: MogId) => {
    try {
      void Promise.resolve(deps.send(view(id))).catch(() => undefined);
    } catch {
      // A bad label or a closed window: nothing to show.
    }
  };
  const setActive = (id: MogId) =>
    deps.setActive(id).then(
      () => true,
      () => false,
    );
  /** Counts target moves and `mog_set_active` requests, in the order Rust applies them. */
  let version = 0;
  /** The `version` of the last move of each MOG to the front. */
  const movedAt = new Map<MogId, number>();
  /**
   * Move `id` to the front, and send the marker to the old and the new target. `stamp` is the
   * `version` at which Rust applied the move (now, by default).
   */
  const front = (id: MogId, stamp = ++version) => {
    movedAt.set(id, stamp);
    const old = current();
    const at = mru.indexOf(id);
    if (at >= 0) mru.splice(at, 1);
    mru.unshift(id);
    if (old !== undefined && old !== id) send(old);
  };
  const lookUpName = (id: MogId) => {
    if (!deps.nameOf || names.has(id)) return;
    void deps
      .nameOf(id)
      .then((n) => {
        if (typeof n !== 'string' || n.length === 0) return;
        names.set(id, n);
        if (isOpen(id)) send(id);
      })
      .catch(() => undefined);
  };

  return {
    viewReady(id) {
      if (!isOpen(id)) {
        front(id);
        void setActive(id);
        lookUpName(id);
      }
      send(id);
    },
    interacted(id, via = 'input') {
      if (!isOpen(id)) return false;
      const target = (at?: number) => {
        const moved = current() !== id;
        front(id, at);
        if (moved) send(id);
      };
      // Rust sees the card load or focus too, so the target moves at once.
      if (via === 'focus') target();
      const asked = setActive(id);
      // Rust sees nothing else: move only when Rust took the new target. Rust applied the
      // request at `mine`, so a MOG that moved to the front later stays ahead of this one.
      if (via === 'input') {
        const mine = ++version;
        void asked.then((ok) => {
          if (!ok || !isOpen(id)) return;
          const ahead = mru.filter((m) => m !== id && (movedAt.get(m) ?? 0) > mine).length;
          if (ahead === 0) return target(mine);
          mru.splice(mru.indexOf(id), 1);
          mru.splice(ahead, 0, id);
          movedAt.set(id, mine);
        });
      }
      return true;
    },
    pttDown(id) {
      if (!isOpen(id)) return false;
      front(id);
      cancelToast?.();
      toastFor = id;
      cancelToast = deps.after(TARGET_TOAST_MS, () => {
        cancelToast = undefined;
        toastFor = undefined;
        if (isOpen(id)) send(id);
      });
      send(id);
      return true;
    },
    closed(id) {
      const at = mru.indexOf(id);
      if (at < 0) return;
      const wasTarget = at === 0;
      mru.splice(at, 1);
      if (toastFor === id) {
        cancelToast?.();
        cancelToast = undefined;
        toastFor = undefined;
      }
      // Clear the marker of a view that may still be there (a close in progress).
      send(id);
      const next = current();
      if (wasTarget && next !== undefined) send(next);
    },
    current,
    isOpen,
  };
}

/** Send the target view to the MOG window of its MOG only (ARC-5). */
export async function sendMogTarget(view: MogTargetView): Promise<void> {
  await emitTo(viewLabel('mog', view.mogId), MOG_TARGET_EVENT, view);
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/** The view kinds on a MOG that count as an interaction (not hover). */
const MOG_INTERACTIONS: ReadonlySet<string> = new Set(['click', 'poked', 'drag_end']);

/**
 * Wire the tracker to view input and `hotkey:ptt`. TODO(#439): the ready signal becomes a
 * `ready` view_emit kind. TODO(#419): the brain learns MOG window closes from the host and calls
 * `closed`. Until then a close reaches only Rust; the next `hotkey:ptt` down moves the brain to
 * the MOG that Rust latched.
 */
export function wireTalkTarget(bus: BrainBus, target: TalkTarget): () => void {
  const offInput = onViewInput(bus, (input) => {
    if (input.from === `mog-${input.mogId}`) {
      if (input.kind === 'click' && isViewReady(input.payload)) {
        target.viewReady(input.mogId);
        return;
      }
      if (!MOG_INTERACTIONS.has(input.kind)) return;
      // Input from a MOG view proves that its window is open (for example after a brain restart).
      if (!target.isOpen(input.mogId)) target.viewReady(input.mogId);
      else target.interacted(input.mogId);
      return;
    }
    if (input.from === `card-${input.mogId}` && input.kind !== 'hover') {
      const focus = input.kind === 'click' && isViewFocus(input.payload);
      target.interacted(input.mogId, focus ? 'focus' : 'input');
    }
  });
  const offPtt = bus.on(EVENTS['hotkey.ptt'].name, (p) => {
    if (!isObj(p) || typeof p['mogId'] !== 'string' || p['phase'] !== 'down') return;
    // Rust latches only a MOG whose window is open, so a press for a MOG that the brain
    // missed (for example after a brain restart) is a first open.
    if (!target.isOpen(p['mogId'])) target.viewReady(p['mogId']);
    target.pttDown(p['mogId']);
  });
  return () => {
    offInput();
    offPtt();
  };
}

/** The MOG name from its config (`config_read` gives `{ id, json }`, ADR 0035). */
export async function configName(mogId: MogId): Promise<string | undefined> {
  const r = await call('config_read', { id: mogId });
  const json = isObj(r) ? r['json'] : undefined;
  const name = isObj(json) ? json['name'] : undefined;
  return typeof name === 'string' ? name : undefined;
}

/** The production tracker: `emitTo` the MOG window, `mog_set_active`, real timers. */
export function createAppTalkTarget(
  nameOf?: (mogId: MogId) => Promise<string | undefined>,
): TalkTarget {
  return createTalkTarget({
    send: sendMogTarget,
    setActive: (mogId) => call('mog_set_active', { mogId }),
    after: (ms, fn) => {
      const t = setTimeout(fn, ms);
      return () => clearTimeout(t);
    },
    ...(nameOf ? { nameOf } : {}),
  });
}
