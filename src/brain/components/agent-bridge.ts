/**
 * T15 AgentBridge (4.3, hidden TS brain window). Implements the 4.7 `AgentBridge`.
 *
 * Responsibilities: Main agent turn input (cleaned question, router decision, recall hits), the
 * UI tool handlers, the guardrail checks of RTR-29a..e and the fallback classifier route (B4).
 */
import type {
  AgentBridge,
  AgentTurn,
  BottleTool,
  HarnessKind,
  MogId,
} from '../../shared/interfaces';
import { NotImplementedError } from '../errors';

const C = 'AgentBridge';

export class AgentBridgeStub implements AgentBridge {
  async start(_mogId: MogId, _h: HarnessKind): Promise<void> {
    throw new NotImplementedError(C, 'start', 112); // TODO(#112)
  }
  async send(_t: AgentTurn): Promise<void> {
    throw new NotImplementedError(C, 'send', 112); // TODO(#112)
  }
  async onTool(_call: { callId: string; tool: BottleTool; args: unknown }): Promise<unknown> {
    throw new NotImplementedError(C, 'onTool', 141); // TODO(#141)
  }
}

export function createAgentBridge(): AgentBridge {
  return new AgentBridgeStub();
}
