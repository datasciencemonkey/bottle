/**
 * The brain's IPC with the core and the views (ADR 0003, decision 3).
 *
 * Core-to-brain items arrive only on the private Channel that `brain_subscribe` registers,
 * as `{ event, payload }`. They never travel as Tauri events. The brain sends to a view with
 * `emitTo(<window label>, ...)`, never with a global emit.
 */
import { Channel } from '@tauri-apps/api/core';
import { emitTo } from '@tauri-apps/api/event';
import { call, EVENTS, type BrainMessage, type EventName } from '../shared/ipc';

type Handler = (payload: unknown) => void;

const CORE_TO_BRAIN: ReadonlySet<string> = new Set(
  Object.values(EVENTS)
    .filter((e) => e.direction === 'core_to_brain')
    .map((e) => e.name),
);

/** Routes Channel messages to the brain components by event name. */
export class BrainBus {
  readonly #handlers = new Map<EventName, Set<Handler>>();

  on(event: EventName, handler: Handler): () => void {
    const set = this.#handlers.get(event) ?? new Set<Handler>();
    set.add(handler);
    this.#handlers.set(event, set);
    return () => set.delete(handler);
  }

  /** Deliver one message. False for a malformed message or a name that is not core-to-brain. */
  dispatch(msg: unknown): boolean {
    if (typeof msg !== 'object' || msg === null) return false;
    const { event, payload } = msg as Partial<BrainMessage>;
    if (typeof event !== 'string' || !CORE_TO_BRAIN.has(event)) return false;
    for (const h of this.#handlers.get(event) ?? []) h(payload);
    return true;
  }
}

/** Register the brain Channel. Resolves false when the core refuses (the reason is not logged). */
export async function subscribeBrain(bus: BrainBus): Promise<boolean> {
  const onEvent = new Channel<BrainMessage>();
  onEvent.onmessage = (m) => void bus.dispatch(m);
  try {
    await call('brain_subscribe', { onEvent });
    return true;
  } catch {
    return false;
  }
}

/** Send a brain-to-view event to one window (`hub`, `mog-<id>` or `card-<id>`). */
export function emitToView(
  label: string,
  key: 'mog.view_state' | 'card.update',
  payload: unknown,
): Promise<void> {
  return emitTo(label, EVENTS[key].name, payload);
}
