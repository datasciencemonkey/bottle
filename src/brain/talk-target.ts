/**
 * T1 talk-key target in the brain (#73, MB-06a, MB-06b, UX-28, ADR 0058).
 *
 * The brain keeps the open MOGs in the order of their last interaction. The target is the
 * first one. Rust keeps the same list (`PttTarget` in `src-tauri/src/hotkey_host.rs`) and
 * latches the target at key-down. The brain calls `mog_set_active` on each interaction, so the
 * Rust list follows the brain.
 *
 * Rules:
 * - The first `view.ready` of a MOG view makes that MOG open and the target. A reload sends
 *   `view.ready` again; it is not an interaction (#305 review note A).
 * - A click, poke or drag end on a MOG, and any input from its card except hover, make that
 *   MOG the target. A card never proves that its MOG window is open (#305 review note B).
 * - When the target MOG closes, the target moves to the most recently interacted open MOG.
 * - A `hotkey:ptt` down for an open MOG makes it the target (Rust latched it) and shows the
 *   toast with its name for 1.2 s (MB-06b). The brain ignores a MOG it does not know as open.
 *
 * TODO(#439): `mog_set_active` answers `not_implemented` until the contract routes it; the
 * brain ignores the refusal. The event name and the ready signal move into the contract.
 * TODO(#83): a drag on the macOS native panel reaches the brain only when #83 relays drag_end.
 * TODO(#62): a chord use (CAP-46a, v1) calls `interacted` for the chord MOG.
 */
import { emitTo } from '@tauri-apps/api/event';
import type { MogId } from '../shared/interfaces';
import { call, EVENTS } from '../shared/ipc';
import {
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
  /** Tell Rust the new target (`mog_set_active`). A refusal is ignored. */
  setActive: (mogId: MogId) => Promise<unknown>;
  /** Run `fn` after `ms`; the result cancels it. */
  after: (ms: number, fn: () => void) => () => void;
  /** The display name of a MOG. A failure or `undefined` shows "MOG". */
  nameOf?: (mogId: MogId) => Promise<string | undefined>;
}

export interface TalkTarget {
  /** A MOG view mounted (first open or reload). */
  viewReady(mogId: MogId): void;
  /** The user interacted with an open MOG. False if the MOG is not open. */
  interacted(mogId: MogId): boolean;
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
  const setActive = (id: MogId) => {
    void deps.setActive(id).catch(() => undefined);
  };
  /** Move `id` to the front, and send the marker to the old and the new target. */
  const front = (id: MogId) => {
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
        setActive(id);
        lookUpName(id);
      }
      send(id);
    },
    interacted(id) {
      if (!isOpen(id)) return false;
      const moved = current() !== id;
      front(id);
      if (moved) send(id);
      setActive(id);
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
 * `ready` view_emit kind. TODO(#26): MogManager closes a MOG with `window_mog_close` and then
 * calls `closed`. A close that the brain did not start reaches only Rust today; the next
 * `hotkey:ptt` down moves the brain to the MOG that Rust latched.
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
      target.interacted(input.mogId);
    }
  });
  const offPtt = bus.on(EVENTS['hotkey.ptt'].name, (p) => {
    if (!isObj(p) || typeof p['mogId'] !== 'string' || p['phase'] !== 'down') return;
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
