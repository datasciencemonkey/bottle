/**
 * The routing of an ⌥G question (ADR 0122). A pure function, with no model call and no I/O. It
 * says if a question goes to the orchestrator model (the agent) or to the direct home path of
 * ADR 0097.
 *
 * - R-1: every question goes to the agent when the orchestrator model is ready. The orchestrator
 *   decides which workspaces to ask. No keyword and no workspace name picks the path.
 * - R-6: when the orchestrator model is not ready, the question takes the direct path on home,
 *   and the card says why (`fallbackNotice`).
 * - R-12: a thread follow-up (#606) stays on the direct path, with no notice.
 * - R-13: while the card of an agent turn is open, or for 10 min after the last agent answer, ⌥G
 *   goes to the session as a follow-up (`createFollowUpWindow`). UX slices 3 and 4 reuse it.
 * - R-14: a follow-up uses the open session (the window closes when the session ends).
 * - R-15: a new agent question uses the open session. Rust starts a new one if it is tainted.
 */
import type { AgentTurn, MogId } from '../shared/interfaces';
import type { OrchestratorStatus } from '../shared/ipc';
import { fallbackNotice, whyOfStatus } from './agent-card';
import type { AgentClient } from './agent-run';
import type { MonotonicClock } from './clock';
import type { DirectSendOptions } from './genie-direct';

/** R-13: the follow-up window after the last agent answer (owner default). */
export const FOLLOW_UP_WINDOW_MS = 10 * 60_000;

export interface AgentRouteInput {
  /** The last known orchestrator status. Undefined before the first read: not ready. */
  orchestrator: Pick<OrchestratorStatus, 'state'> | undefined;
  /** R-13: the follow-up window is open. */
  followUp?: boolean;
  /** R-12: the question is a follow-up in a hub thread. */
  thread?: boolean;
}

export type AgentRoute =
  | { path: 'agent'; mode: 'ask' | 'follow_up' }
  | {
      path: 'direct';
      /** R-6: the orchestrator model was not ready. The card says why. */
      reason?: 'not_ready';
    };

/** R-1. */
export function orchestratorReady(s: AgentRouteInput['orchestrator']): boolean {
  return s !== undefined && s.state === 'ready';
}

export function routeAsk(i: AgentRouteInput): AgentRoute {
  // R-12: a thread follow-up never leaves the direct path.
  if (i.thread === true) return { path: 'direct' };
  // R-6: the orchestrator model is not ready.
  if (!orchestratorReady(i.orchestrator)) return { path: 'direct', reason: 'not_ready' };
  // R-13, R-14: a follow-up goes to the open session. R-15: a new question uses it too.
  return { path: 'agent', mode: i.followUp === true ? 'follow_up' : 'ask' };
}

/**
 * The MOG of a thread reply. The hub's MOG must own the thread: a mismatch would write a turn
 * under one MOG with the root of another, so it is refused ("Not sent."). `known` is the owner
 * of the thread, if any; with no hub MOG the reply takes it.
 */
export function threadReplyMog(
  hubMogId: string | undefined,
  known: string | undefined,
): { mogId: string | undefined } | 'refused' {
  if (hubMogId !== undefined && known !== undefined && known !== hubMogId) return 'refused';
  return { mogId: hubMogId ?? known };
}

/** R-13: when the follow-up window is open. */
export interface FollowUpWindow {
  /** The card of an agent turn opened (true) or closed (false). */
  card(open: boolean): void;
  /** An agent answer landed. The 10 min start now. */
  answered(): void;
  /** The session ended: no session to follow up. */
  reset(): void;
  isOpen(): boolean;
}

export function createFollowUpWindow(clock: MonotonicClock): FollowUpWindow {
  let cardOpen = false;
  let lastAnswerAt: number | undefined;
  return {
    card: (open) => {
      cardOpen = open;
    },
    answered: () => {
      lastAnswerAt = clock.now();
    },
    reset: () => {
      cardOpen = false;
      lastAnswerAt = undefined;
    },
    isOpen: () =>
      cardOpen || (lastAnswerAt !== undefined && clock.now() - lastAnswerAt < FOLLOW_UP_WINDOW_MS),
  };
}

export interface AskRouterDeps {
  /** The last known orchestrator status (a cache: nothing is awaited). */
  context(): { orchestrator: AgentRouteInput['orchestrator'] };
  names(): { agent: string; home: string };
  followUp: Pick<FollowUpWindow, 'isOpen'>;
  agent: Pick<AgentClient, 'submit'>;
  direct(turn: AgentTurn, opts?: DirectSendOptions): Promise<void>;
  /** A direct ask takes the card: the card of an agent turn of this MOG leaves it. */
  closeAgentCard(mogId: MogId): void;
  /** Read the orchestrator status again, in the background (a stale "not reachable"). */
  refreshOrchestrator(): void;
}

export interface AskRouter {
  /** The hand-off seam of the ask gate. */
  send(turn: AgentTurn): Promise<void>;
}

export function createAskRouter(deps: AskRouterDeps): AskRouter {
  return {
    send(turn) {
      const ctx = deps.context();
      const route = routeAsk({
        orchestrator: ctx.orchestrator,
        followUp: deps.followUp.isOpen(),
      });
      if (route.path === 'agent') return deps.agent.submit(turn, route.mode);
      deps.closeAgentCard(turn.mogId);
      if (route.reason === 'not_ready') {
        // R-6: the card says why. A stale status is read again.
        deps.refreshOrchestrator();
        return deps.direct(turn, {
          notice: fallbackNotice(whyOfStatus(ctx.orchestrator), deps.names()),
        });
      }
      return deps.direct(turn);
    },
  };
}
