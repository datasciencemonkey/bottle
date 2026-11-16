/**
 * The only IPC the thin views use (ARC-2): brain-to-view events in, `view_emit` out.
 * Views render what the brain sends and hold no domain logic.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect } from 'react';
import type { Mood, MogId, MoodMapper } from '../shared/interfaces';
import { call, EVENTS, type ViewEmitKind } from '../shared/ipc';

/** `mog.view_state` payload: what the brain computed for one MOG. TODO(#285): move to the contract. */
export interface MogViewState {
  mogId: MogId;
  mood: Mood;
  overlays: ReturnType<MoodMapper>['overlays'];
  level?: number;
}

/** One card slot (ADR 0028): a blurb, or a split answer with its parts in one slot (UX-11b). */
export interface CardBlurbView {
  id: string;
  text: string;
  pinned?: boolean;
  completed?: boolean;
  parts?: { id: string; text: string }[];
}

/**
 * `card.update` payload: what the card of one MOG shows. The brain orders and limits the blurbs
 * (`src/brain/blurb-stack.ts`); `historyCount` drives "N more in History" (UX-11a).
 * TODO(#285): move to the contract.
 */
export interface CardUpdate {
  mogId: MogId;
  blurbs: CardBlurbView[];
  chips: { id: string; label: string }[];
  answerMd?: string;
  historyCount?: number;
}

const MOODS: readonly Mood[] = [
  'idle',
  'listening',
  'processing',
  'answering',
  'success',
  'error',
  'sleeping',
];

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const isItems = (v: unknown, keys: string[]): boolean =>
  Array.isArray(v) && v.every((i) => isObj(i) && keys.every((k) => typeof i[k] === 'string'));

/** Shape check only: drop a payload the view cannot render. */
export function asMogViewState(p: unknown): MogViewState | undefined {
  if (!isObj(p) || typeof p['mogId'] !== 'string' || !isObj(p['overlays'])) return undefined;
  if (!MOODS.includes(p['mood'] as Mood)) return undefined;
  return p as unknown as MogViewState;
}

export function asCardUpdate(p: unknown): CardUpdate | undefined {
  if (!isObj(p) || typeof p['mogId'] !== 'string') return undefined;
  if (!isItems(p['blurbs'], ['id', 'text']) || !isItems(p['chips'], ['id', 'label']))
    return undefined;
  const parts = (p['blurbs'] as Record<string, unknown>[]).map((b) => b['parts']);
  if (!parts.every((v) => v === undefined || isItems(v, ['id', 'text']))) return undefined;
  const n = p['historyCount'];
  if (n !== undefined && !(typeof n === 'number' && Number.isInteger(n) && n >= 0))
    return undefined;
  return p as unknown as CardUpdate;
}

/** The MOG ID in a `mog-<id>` or `card-<id>` window label (ARC-3). */
export function mogIdFromLabel(label: string, kind: 'mog' | 'card'): MogId | undefined {
  const prefix = `${kind}-`;
  return label.startsWith(prefix) && label.length > prefix.length
    ? label.slice(prefix.length)
    : undefined;
}

/** The MOG ID of this window. */
export function ownMogId(kind: 'mog' | 'card'): MogId | undefined {
  return mogIdFromLabel(getCurrentWindow().label, kind);
}

/** Send user input to the brain. This is the only command a view calls. */
export function emitView(mogId: MogId, kind: ViewEmitKind, payload?: unknown): Promise<unknown> {
  return call('view_emit', payload === undefined ? { mogId, kind } : { mogId, kind, payload });
}

/** Subscribe to a brain-to-view event while the component is mounted. `onPayload` must be stable. */
export function useViewEvent(
  key: 'mog.view_state' | 'card.update',
  onPayload: (payload: unknown) => void,
): void {
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    // ARC-5: only events sent to this window (`emitTo(<own label>)`), not those of other views.
    const target = getCurrentWindow().label;
    void listen<unknown>(EVENTS[key].name, (e) => onPayload(e.payload), { target }).then((u) => {
      if (gone) u();
      else off = u;
    });
    return () => {
      gone = true;
      off?.();
    };
  }, [key, onPayload]);
}
