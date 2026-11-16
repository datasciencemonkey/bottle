/**
 * T6 Router step + ContextGate (4.3, hidden TS brain window).
 *
 * Responsibilities: Command grammar, strict json_schema router step (first step of the main
 * agent; the fallback classifier without it), `dictation` intent, hop and split hints,
 * `directed_mog_id`. ContextGate binds approval to one payload, target, conversation and
 * attempt.
 *
 * `route` runs the router step of `./router-step` (#57). It only returns a proposal: it never
 * calls Genie, the agent, capture or a workspace switch. TODO(#143): the command grammar runs
 * before it and the fallback classifier and offline heuristic decide without it.
 */
import type { AskRequest, MogId, RouterDecision } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';
import {
  ipcModelInvoker,
  runRouterStep,
  type RouterInput,
  type RouterStepDeps,
} from './router-step';

export interface RouterContextGate {
  route(text: string, mogId: MogId, input: RouterInput): Promise<RouterDecision>;
  /** Called only on a user action. Binds the approval to this payload and attempt. */
  approve(req: AskRequest, attempt: number): { approvalId: string };
  verify(approvalId: string, req: AskRequest, attempt: number): boolean;
}

const C = 'RouterContextGate';

export class RouterContextGateImpl implements RouterContextGate {
  private readonly deps: RouterStepDeps;

  constructor(deps: Partial<RouterStepDeps> = {}) {
    this.deps = { ...deps, invoke: deps.invoke ?? ipcModelInvoker };
  }

  route(text: string, mogId: MogId, input: RouterInput): Promise<RouterDecision> {
    return runRouterStep(text, mogId, input, this.deps);
  }
  approve(_req: AskRequest, _attempt: number): { approvalId: string } {
    throw new NotImplementedError(C, 'approve', 152); // TODO(#152)
  }
  verify(_approvalId: string, _req: AskRequest, _attempt: number): boolean {
    throw new NotImplementedError(C, 'verify', 152); // TODO(#152)
  }
}

export function createRouterContextGate(deps: Partial<RouterStepDeps> = {}): RouterContextGate {
  return new RouterContextGateImpl(deps);
}
