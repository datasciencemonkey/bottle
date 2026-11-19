/**
 * The talk-key target marker and toast of a MOG view (#73, MB-06b, UX-28, ADR 0058).
 *
 * The brain and the MOG view both read this file. It holds names and shape checks only, no
 * state. It is not a contract file.
 * TODO(#439): move `MOG_TARGET_EVENT` and `MogTargetView` into `src/shared/ipc.ts`
 * (`EVENTS['mog.target']`), and `VIEW_READY` into a `ready` view_emit kind. `VIEW_FOCUS` moves
 * with them.
 */
import type { MogId } from './interfaces';

/** The brain-to-view event that carries the target state to `mog-<id>`. TODO(#439): `EVENTS`. */
export const MOG_TARGET_EVENT = 'mog:target';

/** MB-06b: the target toast shows for 1.2 s after the talk key goes down. */
export const TARGET_TOAST_MS = 1200;

/**
 * The action of the `view_emit` kind `click` that a MOG view sends when it mounts (first open
 * or reload). It is not a user click. TODO(#439): a `ready` view_emit kind.
 */
export const VIEW_READY = 'view.ready';

/**
 * The action of the `view_emit` kind `click` that a card view sends when it loads and each time
 * its window gets focus. Rust moves the talk-key target on the same two signals (`PttTarget`),
 * so the brain moves its target on them at once. It is not a user click. TODO(#439).
 */
export const VIEW_FOCUS = 'view.focus';

/**
 * What one MOG view shows: its name, the target marker (UX-28), the toast (MB-06b) and its
 * accent colour (#ui-4, `mog.ui.variant`, `src/shared/mog-palette.ts`). The card window of the
 * same MOG gets this event too (`sendMogTarget`), so it can show the same name and colour in its
 * header, with no contract change.
 */
export interface MogTargetView {
  mogId: MogId;
  name: string;
  target: boolean;
  toast: boolean;
  /** `mog.ui.variant`, or undefined before the brain reads the config. */
  colorVariant?: string;
  /**
   * #ui-4: the home profile name (`mog.binding.home`), for the MOG hover tooltip only (cheap:
   * it rides the same config read as the name and the colour). Never a token.
   */
  home?: string;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/** Shape check of a `mog:target` payload. */
export function asMogTargetView(p: unknown): MogTargetView | undefined {
  if (!isObj(p)) return undefined;
  const { mogId, name, target, toast, colorVariant, home } = p;
  if (typeof mogId !== 'string' || typeof name !== 'string') return undefined;
  if (typeof target !== 'boolean' || typeof toast !== 'boolean') return undefined;
  if (colorVariant !== undefined && typeof colorVariant !== 'string') return undefined;
  if (home !== undefined && typeof home !== 'string') return undefined;
  return {
    mogId,
    name,
    target,
    toast,
    ...(colorVariant !== undefined ? { colorVariant } : {}),
    ...(home !== undefined ? { home } : {}),
  };
}

/**
 * The card-window ping of #ui-4: the card view sends this once it listens, so the brain resends
 * the current `MogTargetView` (name and colour) to the card, which otherwise only gets pushed
 * state (it never sends `view.ready`, so it would not get the identity if the card opened after
 * the MOG's own `view.ready`).
 */
export const CARD_IDENTITY_READY = 'card.identity_ready';

/** True for the payload of the card identity-ready signal. */
export function isCardIdentityReady(payload: unknown): boolean {
  return isObj(payload) && payload['action'] === CARD_IDENTITY_READY;
}

/** True for the payload of the card focus signal. */
export function isViewFocus(payload: unknown): boolean {
  return isObj(payload) && payload['action'] === VIEW_FOCUS;
}

/** True for the payload of the MOG view ready signal. */
export function isViewReady(payload: unknown): boolean {
  return isObj(payload) && payload['action'] === VIEW_READY;
}
