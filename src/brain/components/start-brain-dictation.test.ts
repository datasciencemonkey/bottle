/**
 * startBrain wires `onDone` on the utterance handler (#498): a `copy` or `insert_failed`
 * dictation reaches a visible Copy card that holds the text. No dictation text is dropped
 * silently, and the path never calls the main agent or Genie (RTR-01c).
 */
import { core } from '../test-core';
import { callsTo } from '../../test-support/cloud-calls';
import { createUtteranceHandler, type UtteranceHandlerDeps } from '../utterance-wiring';
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
const NO_AGENT_NO_GENIE = ['agent_start', 'agent_send', 'agent_tool_result', 'mcp_call'] as const;

let deps: UtteranceHandlerDeps | undefined;
/** The deps that startBrain gave the utterance handler (startBrain builds it once). */
function handlerDeps(): UtteranceHandlerDeps {
  if (!deps) throw new Error('startBrain made no utterance handler');
  return deps;
}

type CardPayload = { mogId: string; blurbs: { text: string }[]; chips: { label: string }[] };
const cardUpdates = () =>
  core.emitTo.mock.calls
    .filter(([target, event]) => target === `card-${MOG}` && event === 'card:update')
    .map(([, , payload]) => payload as CardPayload);
const settle = () => new Promise((r) => setTimeout(r, 0));

describe('startBrain dictation Copy card', () => {
  beforeAll(() => {
    startBrain();
    deps = vi.mocked(createUtteranceHandler).mock.calls[0]?.[0];
  });
  beforeEach(() => core.reset());

  it('[RTR-01c] [RTR-01b] startBrain routes utterances with an onDone sink', () => {
    // Without a sink a copy or insert_failed route drops the dictation text silently.
    expect(handlerDeps().onDone).toBeTypeOf('function');
  });

  it('[RTR-01b] [MOG-05b] a copy route reaches the visible Copy card with the text', async () => {
    handlerDeps().onDone?.({ route: 'copy', text: SENTENCE }, { utteranceId: 'u-1', mogId: MOG });
    await settle();
    const last = cardUpdates().at(-1);
    expect(last?.mogId).toBe(MOG);
    expect(last?.blurbs.map((b) => b.text)).toContain(SENTENCE);
    expect(last?.chips.map((c) => c.label)).toContain('Copy');
    expect(core.invoke).toHaveBeenCalledWith('window_card_show', { args: { mogId: MOG } });
    expect(callsTo(core.invoke.mock.calls, [...NO_AGENT_NO_GENIE])).toEqual([]);
  });

  it('[RTR-01b] [MOG-06d] an insert_failed route reaches the Copy card with the text and the reason', async () => {
    handlerDeps().onDone?.(
      { route: 'insert_failed', reason: 'secure_field', text: SENTENCE },
      { utteranceId: 'u-2', mogId: MOG },
    );
    await settle();
    const last = cardUpdates().at(-1);
    const texts = last?.blurbs.map((b) => b.text) ?? [];
    const failed = texts.find((t) => t.startsWith(SENTENCE) && t !== SENTENCE);
    expect(failed).toContain('Could not insert: the field is a password field');
    expect(last?.chips.map((c) => c.label)).toContain('Copy');
    expect(core.invoke).toHaveBeenCalledWith('window_card_show', { args: { mogId: MOG } });
    expect(callsTo(core.invoke.mock.calls, [...NO_AGENT_NO_GENIE])).toEqual([]);
  });

  it('[MOG-05b] the card view ready resends the kept Copy card state (first card not empty)', async () => {
    const brain = startBrain();
    await handlerDeps().onDone?.(
      { route: 'copy', text: SENTENCE },
      { utteranceId: 'u-3', mogId: MOG },
    );
    await settle();
    core.emitTo.mockClear();
    brain.bus.dispatch({
      event: 'view:input',
      payload: { from: `card-${MOG}`, input: { mogId: MOG, kind: 'ready' } },
    });
    await settle();
    expect(
      cardUpdates()
        .at(-1)
        ?.blurbs.map((b) => b.text),
    ).toContain(SENTENCE);
  });

  it('[MOG-05b] [MOG-06d] when the card cannot show twice, the text goes to the hub notice only', async () => {
    core.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'window_card_show') throw { code: 'closing' };
      return {};
    });
    await handlerDeps().onDone?.(
      { route: 'insert_failed', reason: 'no_focus', text: SENTENCE },
      { utteranceId: 'u-4', mogId: MOG },
    );
    await settle();
    expect(callsTo(core.invoke.mock.calls, ['window_card_show'])).toHaveLength(2);
    expect(core.invoke).toHaveBeenCalledWith('hub_notice_send', {
      args: { kind: 'dictation_unshown', mogId: MOG, text: SENTENCE },
    });
    expect(callsTo(core.invoke.mock.calls, [...NO_AGENT_NO_GENIE])).toEqual([]);
  });
});
