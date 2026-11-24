/**
 * The hub's copy of the orchestrator view (ADR 0122). Thin view (ARC-2, ARC-4): it keeps the
 * `HubOrchestratorView` that the brain sends on `hub:orchestrator` and sends `orchestrator.ready`
 * once the listener is on. Settings → Workspace draws the controls from it, and the agent thread
 * composer reads its state.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';
import {
  asHubOrchestratorView,
  HUB_ORCHESTRATOR_EVENT,
  HUB_SCOPE,
  type HubOrchestratorView,
  type OrchestratorAction,
} from '../../shared/hub-orchestrator';
import { askBrain } from '../view-ipc';

/** The view the brain sends to this window, or undefined before the brain answers (ARC-5). */
export function useHubOrchestrator(): HubOrchestratorView | undefined {
  const [view, setView] = useState<HubOrchestratorView | undefined>(undefined);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<unknown>(
      HUB_ORCHESTRATOR_EVENT,
      (e) => {
        const v = asHubOrchestratorView(e.payload);
        if (v) setView(v);
      },
      { target: getCurrentWindow().label },
    ).then((u) => {
      if (gone) u();
      else {
        off = u;
        askBrain(HUB_SCOPE, 'click', { action: 'orchestrator.ready' } satisfies OrchestratorAction);
      }
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  return view;
}
