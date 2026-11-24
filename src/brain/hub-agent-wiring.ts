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
import type { WipeDialogDeps } from './wipe-wiring';

/** The line that goes with a card "Open in Agent": it also moves the hub to its Agent area. */
export const OPEN_NOTICE = 'Showing your Genie Code session.';

/** A Wipe of any scope ends the agent session (`wipe.rs`): the panel content goes with it. */
export function clearingAgentPanel(
  flow: WipeDialogDeps['flow'],
  clear: () => void,
): WipeDialogDeps['flow'] {
  return {
    dialog: (scope, alsoGenie) => flow.dialog(scope, alsoGenie),
    async confirm(scope, alsoGenie) {
      const done = await flow.confirm(scope, alsoGenie);
      clear();
      return done;
    },
  };
}

const PLACEHOLDER: GenieCodeStatus = {
  binary: 'not_installed',
  pinned: '',
  agentProfile: null,
  agentProfileState: 'no_home',
};

export interface HubAgentWiringDeps {
  client: Pick<AgentClient, 'panel' | 'stop' | 'newSession' | 'refreshSession'>;
  /** The last known Genie Code status (`workspaces.genieCode`). */
  genieCode(): GenieCodeStatus | undefined;
  /** Send the panel to the hub window only. */
  send(view: HubAgentView): Promise<unknown>;
  /** Show the hub on Settings → Workspace. */
  openSettings(): void;
  now?(): number;
}

/** How long a hub that is not listening yet may still get the open notice. */
export const OPEN_PENDING_MS = 10_000;

export interface HubAgentWiring {
  /** Send the panel as it is now. Never rejects. */
  push(): Promise<void>;
  /** Move the hub to its Agent area (after the hub window is shown). */
  open(): Promise<void>;
}

export function wireHubAgent(bus: BrainBus, deps: HubAgentWiringDeps): HubAgentWiring {
  /** A hub that is not listening yet gets the open notice with its `agent.ready`, for 10 s. */
  let pendingOpenUntil = 0;
  const now = deps.now ?? Date.now;
  const build = (notice?: HubAgentView['notice']): HubAgentView => {
    // L5: with no status yet (a read that failed) the panel still draws, with a placeholder.
    const genieCode = deps.genieCode() ?? PLACEHOLDER;
    return { ...deps.client.panel(), genieCode, ...(notice ? { notice } : {}) };
  };
  const push = async (notice?: HubAgentView['notice']): Promise<void> => {
    try {
      await deps.send(build(notice));
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
        const notice =
          now() < pendingOpenUntil ? ({ kind: 'info', text: OPEN_NOTICE } as const) : undefined;
        pendingOpenUntil = 0;
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
      pendingOpenUntil = now() + OPEN_PENDING_MS;
      // A push with no notice first makes the next notice new to a hub that listens already.
      await push();
      await push({ kind: 'info', text: OPEN_NOTICE });
    },
  };
}

/** Send the panel to the hub window. */
export const sendHubAgent = (v: HubAgentView): Promise<unknown> =>
  emitTo('hub', HUB_AGENT_EVENT, v);
