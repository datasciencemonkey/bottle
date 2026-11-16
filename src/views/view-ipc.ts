/**
 * The only IPC the thin views use (ARC-2): brain-to-view events in, `view_emit` out.
 * Views render what the brain sends and hold no domain logic.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect } from 'react';
import type { Mood, MogId } from '../shared/interfaces';
import {
  call,
  EVENTS,
  type CardSection,
  type CardUpdate,
  type DomainPickerView,
  type MicLevelFrame,
  type MogViewState,
  type PagesChecklistView,
  type ViewEmitKind,
} from '../shared/ipc';

// The payload types are in the contract (#285, ADR 0040). The views keep the shape checks.
export type {
  CardBlurbView,
  CardSection,
  CardUpdate,
  DomainPickerView,
  MicLevelFrame,
  MogBadgeView,
  MogViewState,
  PagesChecklistView,
} from '../shared/ipc';

/** The brain-to-view events a view can listen to. */
export type ViewEventKey = 'mog.view_state' | 'card.update' | 'mog.level';

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

const isStrs = (v: unknown): v is string[] =>
  Array.isArray(v) && v.every((s) => typeof s === 'string');
const optStr = (v: unknown) => v === undefined || typeof v === 'string';

/** Shape check of the section list: a known ID with the fields the view reads. */
function isSections(v: unknown): v is CardSection[] {
  return (
    Array.isArray(v) &&
    v.every((s) => {
      if (!isObj(s)) return false;
      switch (s['id']) {
        case 'answer':
          return typeof s['md'] === 'string';
        case 'keyNumbers':
          return isStrs(s['items']);
        case 'sql':
          return typeof s['sql'] === 'string';
        case 'tablePreview':
          return (
            isStrs(s['columns']) &&
            Array.isArray(s['rows']) &&
            s['rows'].every(isStrs) &&
            typeof s['moreRows'] === 'number'
          );
        case 'sentToGenie':
        case 'dataFlow':
          return typeof s['text'] === 'string';
        case 'resend':
          return true;
        case 'copy':
          return (
            Array.isArray(s['formats']) &&
            s['formats'].every((f) => f === 'markdown' || f === 'csv')
          );
        case 'footer':
          return optStr(s['sttBadge']) && optStr(s['deepLink']);
        default:
          return false;
      }
    })
  );
}

function isDomainPicker(v: unknown): v is DomainPickerView {
  if (!isObj(v) || (v['scope'] !== 'home' && v['scope'] !== 'draft')) return false;
  const rid = v['reviewId'];
  if (v['scope'] === 'draft' && !(typeof rid === 'string' && rid !== '')) return false;
  const cur = v['current'];
  if (cur !== undefined && !(isObj(cur) && typeof cur['id'] === 'string' && optStr(cur['name'])))
    return false;
  if (v['options'] !== undefined && !isItems(v['options'], ['id', 'name'])) return false;
  return optStr(rid) && optStr(v['paste']) && optStr(v['error']);
}

const isChecklist = (v: unknown): v is PagesChecklistView =>
  isObj(v) &&
  typeof v['text'] === 'string' &&
  Array.isArray(v['items']) &&
  v['items'].every((i) => typeof i === 'string');

/** Shape check only: drop a payload the view cannot render. */
export function asMogViewState(p: unknown): MogViewState | undefined {
  if (!isObj(p) || typeof p['mogId'] !== 'string' || !isObj(p['overlays'])) return undefined;
  if (!MOODS.includes(p['mood'] as Mood)) return undefined;
  const b = p['badges'];
  if (
    b !== undefined &&
    !(
      isItems(b, ['kind', 'text', 'label']) &&
      (b as Record<string, unknown>[]).every(
        (i) =>
          typeof i['hoverOnly'] === 'boolean' &&
          (i['detail'] === undefined || typeof i['detail'] === 'string') &&
          (i['action'] === undefined || typeof i['action'] === 'string'),
      )
    )
  )
    return undefined;
  if (p['homeDomain'] !== undefined && !isDomainPicker(p['homeDomain'])) return undefined;
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
  const oc = p['overflowChips'];
  if (oc !== undefined && !isItems(oc, ['id', 'label'])) return undefined;
  if (p['sections'] !== undefined && !isSections(p['sections'])) return undefined;
  const stt = p['sttBadge'];
  if (stt !== undefined && typeof stt !== 'string') return undefined;
  if (p['pageDomain'] !== undefined && !isDomainPicker(p['pageDomain'])) return undefined;
  if (p['pagesChecklist'] !== undefined && !isChecklist(p['pagesChecklist'])) return undefined;
  return p as unknown as CardUpdate;
}

/** Shape check of a `mog.level` frame: an owner (or `null`) and two finite numbers. */
export function asMicLevelFrame(p: unknown): MicLevelFrame | undefined {
  if (!isObj(p)) return undefined;
  const { owner, dbfs, t } = p;
  if (owner !== null && typeof owner !== 'string') return undefined;
  if (typeof dbfs !== 'number' || typeof t !== 'number' || !Number.isFinite(t)) return undefined;
  return { owner, dbfs, t };
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
export function useViewEvent(key: ViewEventKey, onPayload: (payload: unknown) => void): void {
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

/**
 * The mic frames the brain sends to this MOG window as `mog.level` (UX-06a, ADR 0040). It
 * listens only to events sent to its own label and drops a frame with a bad shape.
 */
export function viewMicLevels(): {
  subscribe(onFrame: (frame: MicLevelFrame) => void): () => void;
} {
  return {
    subscribe(onFrame) {
      let off: (() => void) | undefined;
      let gone = false;
      const target = getCurrentWindow().label;
      void listen<unknown>(
        EVENTS['mog.level'].name,
        (e) => {
          const frame = asMicLevelFrame(e.payload);
          if (frame) onFrame(frame);
        },
        { target },
      ).then((u) => {
        if (gone) u();
        else off = u;
      });
      return () => {
        gone = true;
        off?.();
      };
    },
  };
}
