/**
 * The hub host of the Forget dialog (#123, thin, ARC-2, ARC-5). It shows the model that the brain
 * sends to this window on `hub:forget_dialog`, and closes on `null`. TODO(#521): the event name
 * moves into the contract.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';
import {
  asForgetDialogModel,
  FORGET_DIALOG_EVENT,
  type ForgetDialogModel,
} from '../../shared/forget-view';
import { ForgetDialog } from '../ForgetDialog';

export function ForgetDialogHost() {
  const [model, setModel] = useState<ForgetDialogModel | null>(null);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    // ARC-5: only events sent to this window.
    const target = getCurrentWindow().label;
    void listen<unknown>(
      FORGET_DIALOG_EVENT,
      (e) => {
        const m = asForgetDialogModel(e.payload);
        if (m !== undefined) setModel(m);
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
  }, []);
  return model === null ? null : <ForgetDialog model={model} />;
}
