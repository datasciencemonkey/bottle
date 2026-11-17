/**
 * The router dispatch step (#144; RTR-11a, RTR-18a, RTR-18b, ADR in docs/adr).
 *
 * Red stage: the types only. The green commit adds the behaviour.
 */
import type {
  MemoryHit,
  MemoryService,
  MogId,
  ProfileName,
  ProfileService,
  RouterDecision,
} from '../../shared/interfaces';
import type { RouterProfile } from './router-step';

export type HopCommand = 'switch_workspace_once' | 'switch_workspace_session' | 'reset_workspace';
export type HopScope = 'question' | 'session';
export type HopRefusal =
  | 'no_argument'
  | 'profile_unknown'
  | 'host_not_allowed'
  | 'resolver_failed'
  | 'split_not_supported';
export type GateOutcome = 'pass' | 'augment' | 'error' | 'timeout';

export interface HopProposal {
  command: HopCommand;
  profile: ProfileName;
  host: string;
  scope: HopScope;
  requiresUserAction: true;
}

export type DispatchOutcome =
  | { kind: 'hop'; proposal: HopProposal }
  | {
      kind: 'choose_workspace';
      command: HopCommand | 'none';
      scope: HopScope;
      candidates: ProfileName[];
      requiresUserAction: true;
    }
  | { kind: 'connect_workspace'; requiresUserAction: true }
  | { kind: 'hop_refused'; reason: HopRefusal }
  | { kind: 'memory_answer'; hits: MemoryHit[]; tier: 'local' | 'remote' }
  | {
      kind: 'preview';
      decision: RouterDecision;
      question: string;
      target: ProfileName;
      scope: 'current' | 'question';
      recall: MemoryHit[];
      gate?: GateOutcome;
    }
  | { kind: 'none'; reason: 'injection' | 'not_agent' | 'other_command' };

export interface DispatchContext {
  mogId: MogId;
  home: ProfileName;
  currentTarget: ProfileName;
  profiles: RouterProfile[];
  allowedWorkspaceHosts?: string[];
  workspaceOf(profile: ProfileName): { workspaceId: string; spaceId?: string };
}

export interface DispatchDeps {
  profiles: Pick<ProfileService, 'resolveTarget'>;
  memory: Pick<MemoryService, 'gate'>;
  wait?: (ms: number) => Promise<void>;
}

export async function dispatchDecision(
  _d: RouterDecision,
  _ctx: DispatchContext,
  _deps: DispatchDeps,
): Promise<DispatchOutcome> {
  throw new Error('router dispatch: red stage');
}
