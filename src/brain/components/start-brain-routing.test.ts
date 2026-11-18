/**
 * startBrain routes every utterance (#163): the router input comes from the MOG's home profile,
 * and with no home profile the RTR-15c offline heuristic decides. No model call, no agent and
 * no Genie take part in a dictation with no profile (RTR-01c).
 */
import { core } from '../test-core';
import { callsTo } from '../../test-support/cloud-calls';
import { createUtteranceHandler, type UtteranceHandlerDeps } from '../utterance-wiring';
import { NO_HOME } from '../offline-intent';
import { routeUtterance } from '../utterance-route';
import { startBrain, type Brain } from './index';

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
let brain: Brain | undefined;
function handlerDeps(): UtteranceHandlerDeps {
  if (!deps) throw new Error('startBrain made no utterance handler');
  return deps;
}

type CardPayload = {
  mogId: string;
  blurbs: { text: string }[];
  chips: { id: string; label: string }[];
};
const cards = () =>
  core.emitTo.mock.calls
    .filter(([target, event]) => target === `card-${MOG}` && event === 'card:update')
    .map(([, , payload]) => payload as CardPayload);
const settle = () => new Promise((r) => setTimeout(r, 0));

describe('startBrain utterance routing (#163)', () => {
  beforeAll(() => {
    brain = startBrain();
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

  it('[RTR-15c] [RTR-01b] with a typed home and no router model on the host, a dictation still routes to Copy', async () => {
    // The demo setup: the hub MOG form has a home; the host serves no profiles_list (#17) and
    // the model call fails. The real router step must give the heuristic, not `context`.
    core.invoke.mockImplementation(async (cmd) =>
      cmd === 'config_read'
        ? { json: { name: 'Finance', binding: { home: 'fake-home' } } }
        : Promise.reject({ error: 'not_implemented' }),
    );
    const d = handlerDeps();
    const input = await d.routerInput(MOG);
    expect(input.profile).toBe('fake-home');
    const r = await routeUtterance(
      { mogId: MOG, text: SENTENCE, forceAsk: false, insertionOn: d.insertionOn(MOG), input },
      d.route,
    );
    expect(r).toEqual({ route: 'copy', text: SENTENCE });
    const q = await d.route.route('What was revenue?', MOG, input);
    expect(q.intent).toBe('question');
    expect(callsTo(core.invoke.mock.calls, ['agent_start', 'agent_send', 'mcp_call'])).toEqual([]);
  });

  it('[RTR-15c] [B1] [W-07d] a no-home question shows the pick card; a card chip click lists the profiles', async () => {
    core.invoke.mockImplementation(async (cmd) => {
      if (cmd === 'config_read') return Promise.reject({ error: 'config_not_found' });
      if (cmd === 'profiles_list')
        return [
          {
            name: 'fake-a',
            host: 'https://a.example.test',
            authType: 'databricks-cli',
            supported: true,
          },
        ];
      return {};
    });
    const d = handlerDeps();
    const owner = 'utt-pick-1';
    await d.routerInput(MOG);
    d.onAsk(
      {
        route: 'chip',
        decision: {
          ...(await d.route.route('What was revenue?', MOG, await d.routerInput(MOG))),
        },
        warning: undefined,
      },
      { utteranceId: owner, mogId: MOG },
    );
    await settle();
    const last = () => cards().at(-1);
    expect(
      last()
        ?.blurbs.map((b) => b.text)
        .join('\n'),
    ).toContain('What was revenue?');
    expect(last()?.chips.map((c) => c.label)).toEqual(['Pick a profile', 'Insert as text', 'Copy']);
    expect(callsTo(core.invoke.mock.calls, ['agent_start', 'agent_send', 'mcp_call'])).toEqual([]);
    // The card view's click arrives as `view:input` on the brain Channel.
    const pickId = last()?.chips.find((c) => c.label === 'Pick a profile')?.id;
    brain?.bus.dispatch({
      event: 'view:input',
      payload: {
        from: `card-${MOG}`,
        input: { mogId: MOG, kind: 'chip_click', payload: { chipId: pickId } },
      },
    });
    await settle();
    expect(last()?.chips.map((c) => c.label)).toContain('fake-a');
    // Listing is not picking: nothing is saved before the user clicks a profile.
    expect(core.invoke.mock.calls.map(([c]) => c)).not.toContain('config_write');
  });
});
