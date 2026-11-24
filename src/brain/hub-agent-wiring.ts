/**
 * Slice 2a: the brain side of the agent view in hub Conversations (plan Amendment 2 A2).
 *
 * The brain keeps the content of the panel in memory (`agent-run.ts`) and sends a `HubAgentView`
 * to the hub window only (ARC-5). The hub sends `agent.ready` when it opens, and one action for
 * each click: Stop, remove a waiting question, New session, open Settings. Only the hub window,
 * with the hub scope, can run an action. The view in memory is what the live session holds: a new
 * session, quit or Wipe clears it (the client does that on `agent:session`). The turns also live
 * in Conversations (ADR 0121): `threads` links each turn to its thread. There is no Agent area:
 * "Open in Conversations" on the card sends `hub:conversations.open` (`hub-conversations-wiring`).
 */
import { emitTo } from '@tauri-apps/api/event';
import { asAgentAction, HUB_AGENT_EVENT, HUB_SCOPE, type HubAgentView } from '../shared/hub-agent';
import type { GenieCodeStatus } from '../shared/ipc';
import type { AgentClient } from './agent-run';
import { onViewInput, type BrainBus } from './bus';
import type { WipeDialogDeps } from './wipe-wiring';

/** A Wipe of any scope ends the agent session (`wipe.rs`): the panel content goes with it. */
export function clearingAgentPanel(
  flow: WipeDialogDeps['flow'],
  clear: () => void,
  /** Fences the writes before the Wipe runs. Returns the lift for a Wipe that failed. */
  fence?: () => () => void,
): WipeDialogDeps['flow'] {
  return {
    dialog: (scope, alsoGenie) => flow.dialog(scope, alsoGenie),
    async confirm(scope, alsoGenie) {
      // The wipe is slow (core and remote deletes): no turn row may land in the meantime.
      const lift = fence?.();
      let done;
      try {
        done = await flow.confirm(scope, alsoGenie);
      } catch (e) {
        lift?.();
        throw e;
      }
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
}

export interface HubAgentWiring {
  /** Send the panel as it is now. Never rejects. */
  push(): Promise<void>;
}

export function wireHubAgent(bus: BrainBus, deps: HubAgentWiringDeps): HubAgentWiring {
  const build = (): HubAgentView => {
    // L5: with no status yet (a read that failed) the panel still draws, with a placeholder.
    const genieCode = deps.genieCode() ?? PLACEHOLDER;
    return { ...deps.client.panel(), genieCode };
  };
  const push = async (): Promise<void> => {
    try {
      await deps.send(build());
    } catch {
      // No hub is open. The hub asks again with `agent.ready` when it opens.
    }
  };
  onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.mogId !== HUB_SCOPE || input.kind !== 'click') return;
    const a = asAgentAction(input.payload);
    if (!a) return;
    switch (a.action) {
      case 'agent.ready':
        return void deps.client.refreshSession().then(() => push());
      case 'agent.stop':
      case 'agent.remove_queued':
        return void deps.client.stop(a.turnId).then(() => push());
      case 'agent.new_session':
        return void deps.client.newSession().then(() => push());
      case 'agent.open_settings':
        return deps.openSettings();
    }
  });
  return { push: () => push() };
}

/** Send the panel to the hub window. */
export const sendHubAgent = (v: HubAgentView): Promise<unknown> =>
  emitTo('hub', HUB_AGENT_EVENT, v);
