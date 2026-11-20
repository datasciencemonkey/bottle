/**
 * lamp-alive: pass the look vector of the core to the one MOG view it is for.
 *
 * The core sends `mog.look` `{ mogId, dx, dy }` on the brain Channel (never a pointer position,
 * `mog_core::mog_look`). The brain sends `{ dx, dy }` as `mog:look` to `mog-<id>` only (ARC-5).
 * A bad payload or an ID that is not a valid window label is dropped.
 */
import { emitTo } from '@tauri-apps/api/event';
import type { MogId } from '../shared/interfaces';
import { EVENTS } from '../shared/ipc';
import { MOG_LOOK_EVENT, asMogLook, type MogLook } from '../shared/mog-look';
import { viewLabel, type BrainBus } from './bus';

export type SendLook = (mogId: MogId, look: MogLook) => unknown;

/** The app sender: `emitTo` the one MOG window. */
export const emitLook: SendLook = (mogId, look) =>
  emitTo(viewLabel('mog', mogId), MOG_LOOK_EVENT, look).catch(() => undefined);

/** Relay each `mog.look` of the core to its MOG view. Returns the unsubscribe function. */
export function wireMogLook(bus: BrainBus, send: SendLook = emitLook): () => void {
  return bus.on(EVENTS['mog.look'].name, (p) => {
    if (typeof p !== 'object' || p === null) return;
    const mogId = (p as Record<string, unknown>)['mogId'];
    const look = asMogLook(p);
    if (typeof mogId !== 'string' || look === undefined) return;
    try {
      void send(mogId, look);
    } catch {
      // Not a valid window label: no MOG to look.
    }
  });
}
