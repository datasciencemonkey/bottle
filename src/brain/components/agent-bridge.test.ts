/**
 * T15 AgentBridge start/send (#112): the brain opens one agent session per MOG through
 * `agent_start` and sends each turn through `agent_send`. The Rust R16 AgentHost owns the shared
 * runtime, the secrets and the model proxy; the brain never sees a token.
 */
import type { AgentTurn, RouterDecision } from '../../shared/interfaces';
import { core } from '../test-core';
import { createAgentBridge } from './agent-bridge';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);

const MOG = '6f1c2d3e-4b5a-4c6d-8e7f-001122334455';
const turn: AgentTurn = {
  mogId: MOG,
  question: 'q',
  decision: {} as RouterDecision,
  recall: [],
  target: 'bottle-home',
};

beforeEach(() => core.reset());

describe('AgentBridge (T15)', () => {
  it('[RTR-21a] start opens the session of one MOG through agent_start', async () => {
    core.invoke.mockResolvedValueOnce({ agentSessionId: 'as-1' });
    await createAgentBridge().start(MOG, 'claude');
    expect(core.invoke).toHaveBeenCalledWith('agent_start', {
      args: { mogId: MOG, harness: 'claude' },
    });
  });

  it('[RTR-22] start passes the Codex harness kind unchanged', async () => {
    core.invoke.mockResolvedValueOnce({ agentSessionId: 'as-2' });
    await createAgentBridge().start(MOG, 'codex');
    expect(core.invoke).toHaveBeenCalledWith('agent_start', {
      args: { mogId: MOG, harness: 'codex' },
    });
  });

  it('[RTR-21a] send routes the turn to the session of its own MOG through agent_send', async () => {
    await createAgentBridge().send(turn);
    expect(core.invoke).toHaveBeenCalledWith('agent_send', {
      args: { mogId: MOG, turn },
    });
  });

  it('[ARC-F14] send rejects when the session is unavailable, so the caller can fall back', async () => {
    core.invoke.mockRejectedValueOnce({ error: 'not_implemented', command: 'agent_send' });
    await expect(createAgentBridge().send(turn)).rejects.toBeDefined();
  });
});
