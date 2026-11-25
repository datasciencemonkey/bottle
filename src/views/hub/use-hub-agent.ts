/**
 * The hub's copy of the orchestrator agent view (slice 2a). Thin view (ARC-2, ARC-4): it keeps the
 * `HubAgentView` that the brain sends on `hub:agent` and sends `agent.ready` once the listener is
 * on. Conversations draws it inside the agent threads.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';
import {
  asHubAgentView,
  HUB_AGENT_EVENT,
  HUB_SCOPE,
  type AgentAction,
  type HubAgentView,
} from '../../shared/hub-agent';
import { askBrain } from '../view-ipc';

/**
 * The view the brain sends to this window, or undefined before the brain answers. Sends
 * `agent.ready` once the listener is on (ARC-5: only events sent to this window).
 */
export function useHubAgent(): HubAgentView | undefined {
  const [view, setView] = useState<HubAgentView | undefined>(undefined);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<unknown>(
      HUB_AGENT_EVENT,
      (e) => {
        const v = asHubAgentView(e.payload);
        if (v) setView(v);
      },
      { target: getCurrentWindow().label },
    ).then((u) => {
      if (gone) u();
      else {
        off = u;
        askBrain(HUB_SCOPE, 'click', { action: 'agent.ready' } satisfies AgentAction);
      }
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  return view;
}
