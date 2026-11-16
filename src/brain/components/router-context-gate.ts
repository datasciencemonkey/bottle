/**
 * T6 Router step + ContextGate (4.3, hidden TS brain window).
 *
 * Responsibilities: Command grammar, strict json_schema router step (first step of the main
 * agent; the fallback classifier without it), `dictation` intent, hop and split hints,
 * `directed_mog_id`. ContextGate binds approval to one payload, target, conversation and
 * attempt.
 */
import type { AskRequest, MogId, RouterDecision } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';

export interface RouterContextGate {
  route(text: string, mogId: MogId): Promise<RouterDecision>;
  /** Called only on a user action. Binds the approval to this payload and attempt. */
  approve(req: AskRequest, attempt: number): { approvalId: string };
  verify(approvalId: string, req: AskRequest, attempt: number): boolean;
}

const C = 'RouterContextGate';

export class RouterContextGateStub implements RouterContextGate {
  async route(_text: string, _mogId: MogId): Promise<RouterDecision> {
    throw new NotImplementedError(C, 'route', 57); // TODO(#57)
  }
  approve(_req: AskRequest, _attempt: number): { approvalId: string } {
    throw new NotImplementedError(C, 'approve', 152); // TODO(#152)
  }
  verify(_approvalId: string, _req: AskRequest, _attempt: number): boolean {
    throw new NotImplementedError(C, 'verify', 152); // TODO(#152)
  }
}

export function createRouterContextGate(): RouterContextGate {
  return new RouterContextGateStub();
}
