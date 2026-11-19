/**
 * The hub side of the store gate (MEM-08b, ADR 0105). The hub asks for the model when it opens
 * (`store.check`), says when it shows the reason (`store.ready`), and sends Try again
 * (`store.retry`). A talk-key press also lets the store open, for a run with no hub.
 */
import { emitTo } from '@tauri-apps/api/event';
import { EVENTS } from '../shared/ipc';
import {
  STORE_CHECK_TARGET,
  STORE_EVENT,
  STORE_READY_TARGET,
  STORE_RETRY_TARGET,
  STORE_VIEW_ID,
  type StoreModel,
} from '../shared/store-view';
import { onViewInput, type BrainBus } from './bus';
import type { StoreGate } from './store-gate';

export interface StoreWiringDeps {
  /** macOS asks for the password: the hub shows the reason first. */
  explain: boolean;
  /** Send the model to the hub only. */
  send?: (m: StoreModel) => Promise<unknown>;
  /** After Try again: read the store again (the hub notice state). */
  onRetry: () => void;
}

/** True only in the macOS app: a Mac user agent inside Tauri. */
export function isTauriMac(
  nav: { userAgent?: string } | undefined = globalThis.navigator,
  g: object = globalThis,
): boolean {
  return /Macintosh/.test(nav?.userAgent ?? '') && '__TAURI_INTERNALS__' in g;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

export function wireStoreGate(bus: BrainBus, gate: StoreGate, deps: StoreWiringDeps): () => void {
  const send = deps.send ?? ((m: StoreModel) => emitTo('hub', STORE_EVENT, m));
  const publish = (m: StoreModel) => void send(m).catch(() => undefined);
  gate.arm({ explain: deps.explain, onChange: publish });
  const offInput = onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.kind !== 'click' || input.mogId !== STORE_VIEW_ID) return;
    if (!isObj(input.payload)) return;
    const target = input.payload['target'];
    if (target === STORE_CHECK_TARGET) publish(gate.view());
    else if (target === STORE_READY_TARGET) gate.allow();
    else if (target === STORE_RETRY_TARGET) {
      gate.retry();
      deps.onRetry();
    }
  });
  const offPtt = bus.on(EVENTS['hotkey.ptt'].name, (p) => {
    if (isObj(p) && p['phase'] === 'down') gate.allow();
  });
  return () => {
    offInput();
    offPtt();
  };
}
