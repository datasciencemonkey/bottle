/**
 * The router dispatch step (#144; RTR-11a, RTR-18a, RTR-18b, ADR 0064).
 *
 * It turns one router decision into one outcome for the brain:
 * - A hop command (`switch_workspace_once`, `switch_workspace_session`, `reset_workspace`) goes to
 *   `ProfileService.resolveTarget` and becomes a hop PROPOSAL (RTR-11a). The user confirms it. This
 *   step has no method that switches a workspace, and it never changes the MOG target.
 * - Every hop path fails closed: an unlisted profile, a host outside `AllowedWorkspaceHosts`
 *   (RTR-20b, checked again on the resolver result), a resolver error or a split result (v1) is
 *   refused. Ambiguity always asks the user (W-11b), even when one allowed candidate is left.
 * - A `memory_question` calls the RecallGate first, before any Genie call (RTR-18a). If the gate
 *   does not answer (pass, augment, error or no answer in 300 ms), the utterance continues as a
 *   `question` at the F1 preview step (RTR-18b). The preview gate still needs a user action.
 * - Injection-flagged decisions, dictation (UC1 bypasses the agent), context and noise do nothing.
 *
 * Outcomes hold profile names, hosts and memory hits only, never a token.
 * TODO(#164): the real `resolveTarget` (W-11a..c). TODO(#200): the RecallGate timing.
 * The ask gate (#142) takes the #59 `preview` route. TODO(#200): the `preview` outcome of this
 * step (with its recall and target) goes to the ask gate. TODO(#165): hop execution.
 */
import type {
  MemoryHit,
  MemoryService,
  MogId,
  ProfileName,
  ProfileService,
  RouterDecision,
} from '../../shared/interfaces';
import { hostAllowed, type RouterProfile } from './router-step';

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

/** The RecallGate budget (MEM-16a). */
export const GATE_BUDGET_MS = 300;

const HOP_SCOPE: Record<Exclude<HopCommand, 'reset_workspace'>, HopScope> = {
  switch_workspace_once: 'question',
  switch_workspace_session: 'session',
};

type Resolved = { ok: true; profile: RouterProfile } | { ok: false; outcome: DispatchOutcome };

/** The listed profile `name` on an allowed host, or the refusal (RTR-20b, fail closed). */
function checkProfile(name: ProfileName, ctx: DispatchContext): Resolved {
  const p = ctx.profiles.find((x) => x.name === name);
  if (p === undefined)
    return { ok: false, outcome: { kind: 'hop_refused', reason: 'profile_unknown' } };
  if (!hostAllowed(p.host, ctx.allowedWorkspaceHosts))
    return { ok: false, outcome: { kind: 'hop_refused', reason: 'host_not_allowed' } };
  return { ok: true, profile: p };
}

/** Resolves one hint through ProfileService. Anything but one allowed profile is not a target. */
async function resolveHint(
  hint: string,
  command: HopCommand | 'none',
  scope: HopScope,
  ctx: DispatchContext,
  deps: DispatchDeps,
): Promise<Resolved> {
  let t: Awaited<ReturnType<ProfileService['resolveTarget']>>;
  try {
    t = await deps.profiles.resolveTarget([hint], ctx.mogId);
  } catch {
    return { ok: false, outcome: { kind: 'hop_refused', reason: 'resolver_failed' } };
  }
  switch (t.kind) {
    case 'one':
      return checkProfile(t.profile, ctx);
    case 'ambiguous': {
      const candidates = t.candidates.filter((c) => checkProfile(c, ctx).ok);
      if (candidates.length === 0)
        return { ok: false, outcome: { kind: 'hop_refused', reason: 'host_not_allowed' } };
      return {
        ok: false,
        outcome: { kind: 'choose_workspace', command, scope, candidates, requiresUserAction: true },
      };
    }
    case 'none':
      return { ok: false, outcome: { kind: 'connect_workspace', requiresUserAction: true } };
    default:
      // TODO(#146): split routing is v1.
      return { ok: false, outcome: { kind: 'hop_refused', reason: 'split_not_supported' } };
  }
}

async function dispatchHop(
  d: RouterDecision,
  command: HopCommand,
  ctx: DispatchContext,
  deps: DispatchDeps,
): Promise<DispatchOutcome> {
  if (command === 'reset_workspace') {
    const home = checkProfile(ctx.home, ctx);
    if (!home.ok) return home.outcome;
    return proposal(command, home.profile, 'session');
  }
  const arg = d.command_arg.trim();
  if (arg === '') return { kind: 'hop_refused', reason: 'no_argument' };
  const scope = HOP_SCOPE[command];
  const r = await resolveHint(arg, command, scope, ctx, deps);
  return r.ok ? proposal(command, r.profile, scope) : r.outcome;
}

function proposal(command: HopCommand, p: RouterProfile, scope: HopScope): DispatchOutcome {
  return {
    kind: 'hop',
    proposal: { command, profile: p.name, host: p.host, scope, requiresUserAction: true },
  };
}

type Gate = Awaited<ReturnType<MemoryService['gate']>>;

/** Calls the RecallGate within its budget (MEM-16a). An error or no answer in time is not an answer. */
async function callGate(
  d: RouterDecision,
  target: ProfileName,
  ctx: DispatchContext,
  deps: DispatchDeps,
): Promise<Gate | 'error' | 'timeout'> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const wait =
    deps.wait ??
    ((ms: number) =>
      new Promise<void>((r) => {
        timer = setTimeout(r, ms);
      }));
  const gate = (async () =>
    deps.memory.gate(
      {
        mogId: ctx.mogId,
        text: d.cleaned_text,
        intent: 'memory_question',
        target: ctx.workspaceOf(target),
      },
      GATE_BUDGET_MS,
    ))().catch(() => 'error' as const);
  try {
    return await Promise.race([gate, wait(GATE_BUDGET_MS).then(() => 'timeout' as const)]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

export async function dispatchDecision(
  d: RouterDecision,
  ctx: DispatchContext,
  deps: DispatchDeps,
): Promise<DispatchOutcome> {
  if (d.injection_suspected) return { kind: 'none', reason: 'injection' };
  if (d.intent === 'command') {
    if (
      d.command === 'switch_workspace_once' ||
      d.command === 'switch_workspace_session' ||
      d.command === 'reset_workspace'
    )
      return dispatchHop(d, d.command, ctx, deps);
    // TODO(#143): the command grammar handles the other commands.
    return { kind: 'none', reason: 'other_command' };
  }
  if (d.intent !== 'question' && d.intent !== 'memory_question')
    return { kind: 'none', reason: 'not_agent' };

  // A hint applies to this one question only. It never changes the MOG target.
  let target = ctx.currentTarget;
  let scope: 'current' | 'question' = 'current';
  if (d.target_profile_hint !== '') {
    const r = await resolveHint(d.target_profile_hint, 'none', 'question', ctx, deps);
    if (!r.ok) return r.outcome;
    target = r.profile.name;
    scope = 'question';
  }

  let recall: MemoryHit[] = [];
  let gate: GateOutcome | undefined;
  if (d.intent === 'memory_question') {
    const g = await callGate(d, target, ctx, deps);
    if (g === 'error' || g === 'timeout') gate = g;
    else if (g.verdict === 'answer_from_memory' && g.hits.length > 0)
      return { kind: 'memory_answer', hits: g.hits, tier: g.tier };
    else if (g.verdict === 'augment') {
      gate = 'augment';
      recall = g.hits;
    } else gate = 'pass';
  }

  const question = d.genie_question.trim() !== '' ? d.genie_question : d.cleaned_text;
  return {
    kind: 'preview',
    decision: { ...d, intent: 'question' },
    question,
    target,
    scope,
    recall,
    ...(gate === undefined ? {} : { gate }),
  };
}
