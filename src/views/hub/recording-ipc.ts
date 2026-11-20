/**
 * The hub's call recording IPC (ADR 0103): brain events in, `view_emit` out (ARC-2). Each hook
 * listens only to events sent to this window (ARC-5) and asks the brain once it listens.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';
import {
  asHubRecording,
  HUB_RECORDING_EVENT,
  RECORDING_VIEW_ID,
  type HubRecording,
  type RecordingAction,
} from '../../shared/recording-view';
import { emitView } from '../view-ipc';

/** Send one recording action to the brain. A refused call is dropped. */
export function sendRecording(action: RecordingAction): void {
  void emitView(RECORDING_VIEW_ID, 'click', action).catch(() => undefined);
}

/**
 * Listen to `name` while mounted; `onListening` runs once the listener is on. `onPayload` and
 * `onListening` must be stable.
 */
export function useHubEvent(
  name: string,
  onPayload: (payload: unknown) => void,
  onListening?: () => void,
): void {
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    const target = getCurrentWindow().label;
    void listen<unknown>(name, (e) => onPayload(e.payload), { target }).then((u) => {
      if (gone) u();
      else {
        off = u;
        onListening?.();
      }
    });
    return () => {
      gone = true;
      off?.();
    };
  }, [name, onPayload, onListening]);
}

const askModel = () => sendRecording({ action: 'recording.ready' });

/** The settings and Record control model, or `undefined` before the brain answers. */
export function useHubRecording(): HubRecording | undefined {
  const [model, setModel] = useState<HubRecording | undefined>(undefined);
  const [onPayload] = useState(() => (p: unknown) => {
    // A bad model is ignored: the last good one stays.
    const m = asHubRecording(p);
    if (m) setModel(m);
  });
  useHubEvent(HUB_RECORDING_EVENT, onPayload, askModel);
  return model;
}

/** The epoch clock, ticking once a second while `on`. */
export function useNow(on: boolean, now: () => number = Date.now): number {
  const [t, setT] = useState(now);
  useEffect(() => {
    if (!on) return;
    const id = setInterval(() => setT(now()), 1000);
    return () => clearInterval(id);
  }, [on, now]);
  return on ? t : now();
}
