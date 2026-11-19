/**
 * The local store notice in the hub (thin; MEM-08b, ADR 0105).
 *
 * The hub asks the brain with `view_emit` when it opens. On macOS the brain answers `explain`:
 * the hub shows why macOS will ask for the password, then tells the brain that the line shows,
 * so the key read (and the macOS prompt) comes after it. `off`: the user refused the key, so
 * history is off, with Try again.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';
import {
  asStoreModel,
  STORE_CHECK_TARGET,
  STORE_EVENT,
  STORE_READY_TARGET,
  STORE_RETRY_TARGET,
  STORE_TEXT,
  STORE_VIEW_ID,
  type StoreModel,
} from '../../shared/store-view';
import { emitView } from '../view-ipc';

const ask = (target: string) =>
  void emitView(STORE_VIEW_ID, 'click', { target }).catch(() => undefined);

export function StoreNotice() {
  const [state, setState] = useState<StoreModel['state']>('hidden');
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<unknown>(
      STORE_EVENT,
      (e) => {
        const m = asStoreModel(e.payload);
        if (m) setState(m.state);
      },
      { target: getCurrentWindow().label },
    ).then((u) => {
      if (gone) u();
      else off = u;
    });
    ask(STORE_CHECK_TARGET);
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  // The line shows now: the brain may read the key.
  useEffect(() => {
    if (state === 'explain') ask(STORE_READY_TARGET);
  }, [state]);

  if (state === 'hidden') return null;
  return (
    <p data-view="store" role="status" className="hub-status hub-store">
      {state === 'explain' ? (
        STORE_TEXT.explain
      ) : (
        <>
          {STORE_TEXT.off}{' '}
          <button type="button" onClick={() => ask(STORE_RETRY_TARGET)}>
            {STORE_TEXT.retry}
          </button>
        </>
      )}
    </p>
  );
}
