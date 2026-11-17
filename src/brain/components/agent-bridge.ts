/**
 * T15 AgentBridge (4.3, hidden TS brain window). Implements the 4.7 `AgentBridge`.
 *
 * Responsibilities: Main agent turn input (cleaned question, router decision, recall hits), the
 * UI tool handlers, the guardrail checks of RTR-29a..e and the fallback classifier route (B4).
 *
 * `start` and `send` (#112) go to the Rust R16 AgentHost through `agent_start` and `agent_send`.
 * The core runs one shared harness runtime with one session per MOG (U11, RTR-21a) and keeps
 * every token on its side of the secret boundary (ARC-7). A rejected `send` means the session is
 * unavailable (ARC-F14); the caller routes with the fallback classifier (RTR-15b).
 */
import type {
  AgentBridge,
  AgentTurn,
  BottleTool,
  HarnessKind,
  MogId,
} from '../../shared/interfaces';
import { call, type JsonObject } from '../../shared/ipc';
import { NotImplementedError } from '../errors';

const C = 'AgentBridge';

export class CoreAgentBridge implements AgentBridge {
  async start(mogId: MogId, h: HarnessKind): Promise<void> {
    await call('agent_start', { mogId, harness: h });
  }
  async send(t: AgentTurn): Promise<void> {
    await call('agent_send', { mogId: t.mogId, turn: t as unknown as JsonObject });
  }
  async onTool(_call: { callId: string; tool: BottleTool; args: unknown }): Promise<unknown> {
    throw new NotImplementedError(C, 'onTool', 141); // TODO(#141)
  }
}

export function createAgentBridge(): AgentBridge {
  return new CoreAgentBridge();
}
