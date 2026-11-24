/**
 * Slice 2a: the brain side of the hub Agent panel (plan Amendment 2 A2, "Agent panel").
 *
 * The brain keeps the content of the panel in memory (`agent-run.ts`) and sends a `HubAgentView`
 * to the hub window only (ARC-5). The hub sends `agent.ready` when it opens, and one action for
 * each click: Stop, remove a waiting question, New session, open Settings. Only the hub window,
 * with the hub scope, can run an action. The panel holds no content on disk: a new session, quit
 * or Wipe clears it (the client does that on `agent:session`).
 * "Open in Agent" on the card calls `open`: the hub shows its Agent area through a one-time
 * `info` notice, the same way a workspace notice shows Settings.
 */
import { emitTo } from '@tauri-apps/api/event';
import { asAgentAction, HUB_AGENT_EVENT, HUB_SCOPE, type HubAgentView } from '../shared/hub-agent';
import type { GenieCodeStatus } from '../shared/ipc';
import type { AgentClient } from './agent-run';
import { onViewInput, type BrainBus } from './bus';

/** The line that goes with a card "Open in Agent": it also moves the hub to its Agent area. */
export const OPEN_NOTICE = 'Showing your Genie Code session.';

export interface HubAgentWiringDeps {
  client: Pick<AgentClient, 'panel' | 'stop' | 'newSession' | 'refreshSession'>;
  /** The last known Genie Code status (`workspaces.genieCode`). */
  genieCode(): GenieCodeStatus | undefined;
  /** Send the panel to the hub window only. */
  send(view: HubAgentView): Promise<unknown>;
  /** Show the hub on Settings → Workspace. */
  openSettings(): void;
}

export interface HubAgentWiring {
  /** Send the panel as it is now. Never rejects. */
  push(): Promise<void>;
  /** Move the hub to its Agent area (after the hub window is shown). */
  open(): Promise<void>;
}

export function wireHubAgent(bus: BrainBus, deps: HubAgentWiringDeps): HubAgentWiring {
  let pendingOpen = false;
  const build = (notice?: HubAgentView['notice']): HubAgentView | undefined => {
    const genieCode = deps.genieCode();
    if (!genieCode) return undefined;
    return { ...deps.client.panel(), genieCode, ...(notice ? { notice } : {}) };
  };
  const push = async (notice?: HubAgentView['notice']): Promise<void> => {
    const v = build(notice);
    if (!v) return;
    try {
      await deps.send(v);
    } catch {
      // No hub is open. The hub asks again with `agent.ready` when it opens.
    }
  };
  onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.mogId !== HUB_SCOPE || input.kind !== 'click') return;
    const a = asAgentAction(input.payload);
    if (!a) return;
    switch (a.action) {
      case 'agent.ready': {
        const notice = pendingOpen ? ({ kind: 'info', text: OPEN_NOTICE } as const) : undefined;
        pendingOpen = false;
        return void deps.client.refreshSession().then(() => push(notice));
      }
      case 'agent.stop':
      case 'agent.remove_queued':
        return void deps.client.stop(a.turnId).then(() => push());
      case 'agent.new_session':
        return void deps.client.newSession().then(() => push());
      case 'agent.open_settings':
        return deps.openSettings();
    }
  });
  return {
    push: () => push(),
    async open() {
      // A hub that is not listening yet gets the notice with its `agent.ready`.
      pendingOpen = true;
      await push({ kind: 'info', text: OPEN_NOTICE });
    },
  };
}

/** Send the panel to the hub window. */
export const sendHubAgent = (v: HubAgentView): Promise<unknown> =>
  emitTo('hub', HUB_AGENT_EVENT, v);
