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

import { workspaceList } from '../../test-support/workspace-list';

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
  overflowChips: { id: string; label: string }[];
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

  it('[RTR-15c] [RTR-01b] with a typed home and no router model on the host, a dictation is inserted, or kept for Copy when the insert fails', async () => {
    // The demo setup: the hub MOG form has a home; the host serves no profiles_list (#17) and
    // the model call fails. The real router step must give the heuristic, not `context`.
    // #551: insertion is on. A working insert gets the text; a failed one keeps it for Copy.
    let insert: 'ok' | 'fails' = 'ok';
    core.invoke.mockImplementation(async (cmd) => {
      if (cmd === 'config_read')
        return { json: { name: 'Finance', binding: { home: 'fake-home' } } };
      if (cmd === 'workspace_list') return workspaceList('fake-home');
      if (cmd === 'insert_text' && insert === 'ok') return { result: 'inserted' };
      return Promise.reject({ error: 'not_implemented' });
    });
    const d = handlerDeps();
    const input = await d.routerInput(MOG);
    expect(input.profile).toBe('fake-home');
    const dictate = () =>
      routeUtterance(
        { mogId: MOG, text: SENTENCE, forceAsk: false, insertionOn: d.insertionOn(MOG), input },
        d.route,
      );
    expect(await dictate()).toMatchObject({ route: 'chip', dictated: { route: 'inserted' } });
    const sent = core.invoke.mock.calls.find(([c]) => c === 'insert_text');
    expect((sent?.[1] as { args: { text: string } }).args.text).toBe(SENTENCE);
    insert = 'fails';
    // No text loss: a failed insert gives the text back for the Copy card (#498).
    expect(await dictate()).toMatchObject({
      route: 'chip',
      dictated: { route: 'insert_failed', reason: 'host_failed', text: SENTENCE },
    });
    const q = await d.route.route('What was revenue?', MOG, input);
    expect(q.intent).toBe('question');
    expect(callsTo(core.invoke.mock.calls, ['agent_start', 'agent_send', 'mcp_call'])).toEqual([]);
  });

  it('[#557] [RTR-15c] [B1] [W-07d] a no-home question gets the Ask Genie chip; a press lists the profiles and sends nothing', async () => {
    core.invoke.mockImplementation(async (cmd) => {
      if (cmd === 'config_read') return Promise.reject({ error: 'config_not_found' });
      // AC-1.10: no home. One listed profile is still never picked for the user.
      if (cmd === 'workspace_list') return workspaceList(null);
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
    brain?.talkTarget.viewReady(MOG);
    const d = handlerDeps();
    d.onAsk(
      {
        route: 'chip',
        decision: {
          ...(await d.route.route('What was revenue?', MOG, await d.routerInput(MOG))),
        },
        warning: undefined,
      },
      { utteranceId: 'utt-pick-1', mogId: MOG },
    );
    await settle();
    const last = () => cards().at(-1);
    const all = () => [...(last()?.chips ?? []), ...(last()?.overflowChips ?? [])];
    // No pick card any more: the dictated text gets one Ask Genie chip.
    expect(all().map((c) => c.label)).not.toContain('Pick a profile');
    const ask = all().find((c) => c.id === 'ask_genie:utt-pick-1');
    expect(ask?.label).toBe('Ask Genie');
    expect(callsTo(core.invoke.mock.calls, ['agent_start', 'agent_send', 'mcp_call'])).toEqual([]);
    // The card view's press arrives as `view:input` on the brain Channel.
    brain?.bus.dispatch({
      event: 'view:input',
      payload: {
        from: `card-${MOG}`,
        input: { mogId: MOG, kind: 'chip_click', payload: { chipId: ask?.id } },
      },
    });
    await settle();
    expect(all().map((c) => c.label)).toContain('fake-a');
    // Listing is not picking: nothing is saved or sent before the user clicks a profile.
    expect(core.invoke.mock.calls.map(([c]) => c)).not.toContain('config_write');
    expect(callsTo(core.invoke.mock.calls, ['agent_start', 'agent_send', 'mcp_call'])).toEqual([]);
  });

  it('[RTR-15c] [B1] with a listed home profile, an Ask Genie click sends the question to Genie One on that home profile (#573)', async () => {
    core.invoke.mockImplementation(async (cmd) => {
      if (cmd === 'config_read')
        return { json: { name: 'Finance', binding: { home: 'fake-home' } } };
      if (cmd === 'workspace_list') return workspaceList('fake-home');
      if (cmd === 'profiles_list')
        return [
          {
            name: 'fake-home',
            host: 'https://h.example.test',
            authType: 'databricks-cli',
            supported: true,
          },
        ];
      if (cmd === 'model_invoke') return Promise.reject({ error: 'offline' });
      return {};
    });
    // The MOG window is open (its view sent `ready`), so the gate may hand the question over.
    brain?.talkTarget.viewReady(MOG);
    const d = handlerDeps();
    const input = await d.routerInput(MOG);
    const decision = await d.route.route('What was revenue?', MOG, input);
    d.onAsk(
      { route: 'chip', decision, warning: undefined },
      { utteranceId: 'utt-ask-1', mogId: MOG },
    );
    await settle();
    // Nothing goes out before the click (B1).
    expect(callsTo(core.invoke.mock.calls, ['agent_send', 'mcp_call'])).toEqual([]);
    const last = cards().at(-1);
    // The card stack of the MOG is shared across these tests, so the chip may be in the overflow.
    const ask = [...(last?.chips ?? []), ...(last?.overflowChips ?? [])].find(
      (c) => c.label === 'Ask Genie',
    );
    expect(ask).toBeDefined();
    brain?.bus.dispatch({
      event: 'view:input',
      payload: {
        from: `card-${MOG}`,
        input: { mogId: MOG, kind: 'chip_click', payload: { chipId: ask?.id } },
      },
    });
    await settle();
    // #573 (ADR 0097): the press goes straight to Genie One MCP on the home profile; no agent.
    const sent = core.invoke.mock.calls.find(([c]) => c === 'mcp_call');
    expect(sent?.[1]).toMatchObject({ args: { profile: 'fake-home', tool: 'genie_ask' } });
    expect(callsTo(core.invoke.mock.calls, ['agent_start', 'agent_send'])).toEqual([]);
  });
});
