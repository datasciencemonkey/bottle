/**
 * Slice 2a: the brain side of the agent view in hub Conversations (plan Amendment 2 A2).
 *
 * The brain keeps the content of the panel in memory (`agent-run.ts`) and sends a `HubAgentView`
 * to the hub window only (ARC-5). The hub sends `agent.ready` when it opens, and one action for
 * each click: Stop, remove a waiting question, open Settings. Only the hub window,
 * with the hub scope, can run an action. The view in memory is what the live session holds: a new
 * session, quit or Wipe clears it (the client does that on `agent:session`). The turns also live
 * in Conversations (ADR 0121): `threads` links each turn to its thread. There is no Agent area:
 * "Open in Conversations" on the card sends `hub:conversations.open` (`hub-conversations-wiring`).
 */
import { emitTo } from '@tauri-apps/api/event';
import { asAgentAction, HUB_AGENT_EVENT, HUB_SCOPE, type HubAgentView } from '../shared/hub-agent';
import type { AgentClient } from './agent-run';
import { onViewInput, type BrainBus } from './bus';
import type { WipeScope } from '../shared/wipe-view';
import type { WipeDialogDeps } from './wipe-wiring';

/**
 * A Wipe that runs longer than this lifts the fence. The local wipe has no timer of its own: a
 * hang must not block the history for good. Known trade-off (ADR 0121): a late row can survive.
 */
export const WIPE_FENCE_TIMEOUT_MS = 120_000;

/** A Wipe of any scope ends the agent session (`wipe.rs`): the panel content goes with it. */
export function clearingAgentPanel(
  flow: WipeDialogDeps['flow'],
  clear: () => void,
  /** Fences the writes before the Wipe runs. Returns the lift for a Wipe that failed. */
  fence?: () => () => void,
  /** Logs a fault (the fence timed out). */
  fault?: () => void,
  timeoutMs = WIPE_FENCE_TIMEOUT_MS,
): WipeDialogDeps['flow'] {
  // One Wipe at a time: a second confirm waits for the first. `clear()` ends every fence, which
  // is only right while there is one fence at a time.
  let tail: Promise<unknown> = Promise.resolve();
  const run = async (scope: WipeScope, alsoGenie: boolean) => {
    // The wipe is slow (core and remote deletes): no turn row may land in the meantime.
    const lift = fence?.();
    const timer = setTimeout(() => {
      lift?.();
      fault?.();
    }, timeoutMs);
    let localDone = false;
    try {
      // The fence ends for good when the local wipe worked (`clear()`), before the remote
      // deletes: a later throw must not replay rows into a wiped store, and the timer is over.
      const done = await flow.confirm(scope, alsoGenie, () => {
        localDone = true;
        clearTimeout(timer);
        clear();
      });
      // A flow that never reported the local result: clear now. Not twice: a turn that started
      // during the remote deletes keeps its row.
      if (!localDone) clear();
      return done;
    } catch (e) {
      lift?.();
      throw e;
    } finally {
      clearTimeout(timer);
    }
  };
  return {
    dialog: (scope, alsoGenie) => flow.dialog(scope, alsoGenie),
    confirm(scope, alsoGenie) {
      const p = tail.then(() => run(scope, alsoGenie));
      tail = p.catch(() => undefined);
      return p;
    },
  };
}

export interface HubAgentWiringDeps {
  client: Pick<AgentClient, 'panel' | 'stop' | 'refreshSession'>;
  /** Send the panel to the hub window only. */
  send(view: HubAgentView): Promise<unknown>;
  /** Show the hub on Settings → Workspace. */
  openSettings(): void;
  /** The owner's "little button on the Conversations copy" request: `agent.copy_markdown`. */
  copyMarkdown?(turnId: string): void;
  /** The owner's "Copy" button request (ADR 0124 amendment): `agent.copy`, formatted. */
  copy?(turnId: string): void;
}

export interface HubAgentWiring {
  /** Send the panel as it is now. Never rejects. */
  push(): Promise<void>;
}

export function wireHubAgent(bus: BrainBus, deps: HubAgentWiringDeps): HubAgentWiring {
  const build = (): HubAgentView => deps.client.panel();
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
      case 'agent.open_settings':
        return deps.openSettings();
      case 'agent.copy_markdown':
        return deps.copyMarkdown?.(a.turnId);
      case 'agent.copy':
        return deps.copy?.(a.turnId);
    }
  });
  return { push: () => push() };
}

/** Send the panel to the hub window. */
export const sendHubAgent = (v: HubAgentView): Promise<unknown> =>
  emitTo('hub', HUB_AGENT_EVENT, v);
