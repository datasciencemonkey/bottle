/**
 * startBrain routes every utterance (#163): the router input comes from the MOG's home profile,
 * and with no home profile the RTR-15c offline heuristic decides. No model call, no agent and
 * no Genie take part in a dictation with no profile (RTR-01c).
 */
import { core } from '../test-core';
import { callsTo } from '../../test-support/cloud-calls';
import { createUtteranceHandler, type UtteranceHandlerDeps } from '../utterance-wiring';
import { NO_HOME } from '../offline-intent';
import { startBrain } from './index';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);
vi.mock('../hub-mogs-wiring', () => ({ wireHubMogs: vi.fn() }));
vi.mock('../notice-wiring', () => ({ wireNotice: vi.fn() }));
vi.mock('../level-stream', async (orig) => ({
  ...(await orig<typeof import('../level-stream')>()),
  wireLevelStream: vi.fn(() => () => undefined),
  subscribeLevels: vi.fn(async () => true),
}));
vi.mock('../utterance-wiring', async (orig) => {
  const real = await orig<typeof import('../utterance-wiring')>();
  return { ...real, createUtteranceHandler: vi.fn(real.createUtteranceHandler) };
});

const MOG = '00000000-0000-4000-8000-0000000000a1';
const SENTENCE = 'Hi Sam, the deck is attached and I will send the numbers tomorrow.';
const NO_CLOUD = ['model_invoke', 'agent_start', 'agent_send', 'mcp_call'] as const;

let deps: UtteranceHandlerDeps | undefined;
function handlerDeps(): UtteranceHandlerDeps {
  if (!deps) throw new Error('startBrain made no utterance handler');
  return deps;
}

describe('startBrain utterance routing (#163)', () => {
  beforeAll(() => {
    startBrain();
    deps = vi.mocked(createUtteranceHandler).mock.calls[0]?.[0];
  });
  beforeEach(() => core.reset());

  it('[RTR-15c] [RTR-01b] with no home profile the router input is still given', async () => {
    core.invoke.mockImplementation(async () => Promise.reject({ error: 'config_not_found' }));
    const input = await handlerDeps().routerInput(MOG);
    expect(input).toBeDefined();
    expect(input.profile).toBe(NO_HOME);
  });

  it('[RTR-15c] [RTR-01b] [RTR-01c] with no home profile a dictation routes with no model call', async () => {
    core.invoke.mockImplementation(async (cmd) =>
      cmd === 'insert_text' ? { result: 'inserted' } : Promise.reject({ error: 'not_implemented' }),
    );
    const d = handlerDeps();
    const input = await d.routerInput(MOG);
    const decision = await d.route.route(SENTENCE, MOG, input);
    expect(decision.intent).toBe('dictation');
    expect(callsTo(core.invoke.mock.calls, [...NO_CLOUD])).toEqual([]);
  });
});
