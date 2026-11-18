/**
 * The brain's IPC with the core and the views (ADR 0003, decision 3).
 *
 * Core-to-brain items arrive only on the private Channel that `brain_subscribe` registers,
 * as `{ event, payload }`. They never travel as Tauri events. The brain sends to a view with
 * `emitTo(<window label>, ...)`, never with a global emit.
 */
import { Channel } from '@tauri-apps/api/core';
import { emitTo } from '@tauri-apps/api/event';
import type { MogId } from '../shared/interfaces';
import {
  call,
  EVENTS,
  VIEW_EMIT_KINDS,
  type BrainMessage,
  type CardUpdate,
  type EventName,
  type HubTextNotice,
  type MicLevelFrame,
  type MogViewState,
  type ViewEmitKind,
} from '../shared/ipc';

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

/** The contract payload of each brain-to-view event (#285, ADR 0040). */
export interface ViewPayloads {
  'mog.view_state': MogViewState;
  'card.update': CardUpdate;
  'mog.level': MicLevelFrame;
}

/** Send a brain-to-view event to one window (`hub`, `mog-<id>` or `card-<id>`). */
export function emitToView<K extends keyof ViewPayloads>(
  label: string,
  key: K,
  payload: ViewPayloads[K],
): Promise<void> {
  return emitTo(label, EVENTS[key].name, payload);
}

/** A MOG ID that is safe in a `mog-<id>` / `card-<id>` label (same rule as `WindowKind` in Rust). */
const LABEL_ID = /^[A-Za-z0-9-]{1,64}$/;

export function viewLabel(kind: 'mog' | 'card', mogId: MogId): string {
  if (!LABEL_ID.test(mogId)) throw new TypeError(`MOG ID is not a valid window label`);
  return `${kind}-${mogId}`;
}

/**
 * Send `mog.view_state` to the MOG window of `mogId` and to the hub, which lists every MOG
 * with its mood (ARC-5, ADR 0008 decision 4).
 */
export async function sendMogViewState(mogId: MogId, state: MogViewState): Promise<void> {
  const label = viewLabel('mog', mogId);
  await emitToView(label, 'mog.view_state', state);
  await emitToView('hub', 'mog.view_state', state);
}

/** Send `card.update` to the card window of `mogId` only (ARC-5). */
export async function sendCardUpdate(mogId: MogId, update: CardUpdate): Promise<void> {
  return emitToView(viewLabel('card', mogId), 'card.update', update);
}

/**
 * Send one `mog.level` frame to the MOG window of `mogId` only (UX-06a, ADR 0040). `mogId` is
 * the mic owner, or, for the `owner: null` frame on mic close, the MOG that owned it (UX-06b).
 */
export async function sendMicLevel(mogId: MogId, frame: MicLevelFrame): Promise<void> {
  return emitToView(viewLabel('mog', mogId), 'mog.level', frame);
}

/**
 * Send an unshown dictation to the hub only (#509, ADR 0091), on the hub's private Channel.
 * False when the core refuses (for example no hub listens yet): the caller keeps the text.
 * Nothing is logged: `text` is user content.
 */
export async function sendHubNotice(mogId: MogId, text: string): Promise<boolean> {
  const notice: HubTextNotice = { kind: 'dictation_unshown', mogId, text };
  try {
    await call('hub_notice_send', notice);
    return true;
  } catch {
    return false;
  }
}

/** One `view_emit` call, as the brain gets it on the Channel (`view:input`). */
export interface ViewInput {
  /** The window label of the sender: `hub`, `mog-<id>` or `card-<id>`. */
  from: string;
  mogId: MogId;
  kind: ViewEmitKind;
  payload: unknown;
}

const VIEW_KINDS: ReadonlySet<string> = new Set<ViewEmitKind>(VIEW_EMIT_KINDS);

/** Parse a `view:input` payload. The core checks the same rules; the brain checks again. */
export function asViewInput(p: unknown): ViewInput | undefined {
  if (typeof p !== 'object' || p === null) return undefined;
  const { from, input } = p as { from?: unknown; input?: unknown };
  if (typeof from !== 'string' || typeof input !== 'object' || input === null) return undefined;
  const { mogId, kind, payload } = input as { mogId?: unknown; kind?: unknown; payload?: unknown };
  if (typeof mogId !== 'string' || typeof kind !== 'string' || !VIEW_KINDS.has(kind))
    return undefined;
  // Only views send view_emit. A MOG or card view speaks only for its own MOG.
  if (from !== 'hub' && from !== `mog-${mogId}` && from !== `card-${mogId}`) return undefined;
  // #507: `ready` has no payload. The core refuses one; the brain drops it too.
  if (kind === 'ready' && 'payload' in input) return undefined;
  return { from, mogId, kind: kind as ViewEmitKind, payload };
}

/** Register a handler for view input (`view_emit` relayed as `view:input`). */
export function onViewInput(bus: BrainBus, handler: (input: ViewInput) => void): () => void {
  return bus.on(EVENTS['view.input'].name, (p) => {
    const input = asViewInput(p);
    if (input) handler(input);
  });
}
