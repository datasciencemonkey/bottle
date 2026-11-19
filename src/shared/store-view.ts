/**
 * The local store notice in the hub (MEM-08b, ADR 0105). macOS asks for the login password when
 * an app reads its key from the keychain and the item does not trust this build yet. So the hub
 * says why before Bottle reads the key, and says so when the user refused it.
 *
 * The brain and the hub view both read this file. It holds static data and shape checks only.
 * It is not a contract file. TODO(#525): the event name moves into the contract with the other
 * hub events.
 */

/** The brain-to-hub event of the model. */
export const STORE_EVENT = 'hub:store';
/** The `view_emit` id of the hub's store notice. */
export const STORE_VIEW_ID = 'store';
/** The hub asks for the model when it opens. */
export const STORE_CHECK_TARGET = 'store.check';
/** The hub shows the reason now: the brain may read the key. */
export const STORE_READY_TARGET = 'store.ready';
/** A click on Try again. */
export const STORE_RETRY_TARGET = 'store.retry';

/** `explain`: the reason before the macOS prompt. `off`: the key was refused. */
export interface StoreModel {
  state: 'explain' | 'off' | 'hidden';
}

export const STORE_TEXT = {
  explain:
    'Bottle keeps your history encrypted on this Mac. macOS will ask for your password once so Bottle can unlock its key. Choose Always Allow.',
  off: 'History is off: Bottle could not unlock its key.',
  retry: 'Try again',
} as const;

/** Shape check of a `hub:store` payload. */
export function asStoreModel(p: unknown): StoreModel | undefined {
  if (typeof p !== 'object' || p === null) return undefined;
  const s = (p as Record<string, unknown>)['state'];
  return s === 'explain' || s === 'off' || s === 'hidden' ? { state: s } : undefined;
}
