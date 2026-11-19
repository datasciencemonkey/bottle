/**
 * startBrain, #573 (ADR 0097): an Ask Genie press sends the exact sanitized and capped text
 * straight to Genie One MCP (`mcp_call` `genie_ask`) on the MOG's home profile. The main agent
 * takes no part: no `agent_*` call is ever made. Nothing is sent without a press (B1), and a
 * failed send is not sent again (G-15b). On main the host R5 is a stub (#16), so a real run
 * gets `not_implemented`: the card then shows the fixed failure text.
 */
import { core } from '../test-core';
import { routerDecision } from '../../test-support/model-invoke';
import { createUtteranceHandler, type UtteranceHandlerDeps } from '../utterance-wiring';
import { workingLine } from '../genie-direct';
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

const MOG = '00000000-0000-4000-8000-0000000005a3';
const HOME = 'fake-home';

let deps: UtteranceHandlerDeps | undefined;
let brain: Brain | undefined;

type CardPayload = { blurbs: { text: string }[]; chips: { id: string; label: string }[] };
const cards = () =>
  core.emitTo.mock.calls
    .filter(([target, event]) => target === `card-${MOG}` && event === 'card:update')
    .map(([, , payload]) => payload as CardPayload);
const settle = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};
const commands = () => core.invoke.mock.calls.map(([c]) => c as string);
const genieAsks = () =>
  core.invoke.mock.calls.filter(
    ([c, a]) => c === 'mcp_call' && (a as { args: { tool: string } }).args.tool === 'genie_ask',
  );

/** The host as on main: a home profile, and an `mcp_call` that R5 cannot run yet (#16). */
function hostWithStubR5() {
  core.invoke.mockImplementation(async (cmd) => {
    if (cmd === 'config_read') return { json: { name: 'Finance', binding: { home: HOME } } };
    if (cmd === 'mcp_call') return Promise.reject({ error: 'not_implemented' });
    return {};
  });
}

/** The utterance hook's ask: the router input first (it learns the home profile), then the chip. */
async function offer(question: string, utteranceId: string) {
  if (!deps) throw new Error('startBrain made no utterance handler');
  await deps.routerInput(MOG);
  deps.onAsk(
    {
      route: 'chip',
      decision: routerDecision({ genie_question: question, cleaned_text: question }),
      warning: undefined,
    },
    { utteranceId, mogId: MOG },
  );
}

function press(chipId: string) {
  brain?.bus.dispatch({
    event: 'view:input',
    payload: {
      from: `card-${MOG}`,
      input: { mogId: MOG, kind: 'chip_click', payload: { chipId } },
    },
  });
}

describe('startBrain Ask Genie goes straight to Genie One MCP (#573, ADR 0097)', () => {
  beforeAll(() => {
    brain = startBrain();
    deps = vi.mocked(createUtteranceHandler).mock.calls[0]?.[0];
  });
  beforeEach(() => core.reset());

  it('[B1] [G-47] [G-30a] [#573] nothing sends before the press; one press is one genie_ask with the sanitized text, and no agent call', async () => {
    hostWithStubR5();
    brain?.talkTarget.viewReady(MOG);
    await offer('What was\u200b [EMEA](https://evil.example.test) <b>revenue</b>?', 'utt-direct-1');
    await settle();
    // B1: the chip shows, and nothing leaves the device before the press.
    expect(
      cards()
        .at(-1)
        ?.chips.map((c) => c.label),
    ).toContain('Ask Genie');
    expect(commands().filter((c) => c === 'mcp_call' || c.startsWith('agent_'))).toEqual([]);

    press('ask_genie:utt-direct-1');
    await settle();
    expect(genieAsks()).toHaveLength(1);
    expect(genieAsks()[0]?.[1]).toMatchObject({
      args: {
        profile: HOME,
        server: 'genie_one',
        tool: 'genie_ask',
        args: { question: 'What was EMEA revenue?' },
        tags: { app: 'bottle', mog: MOG, op: 'genie' },
      },
    });
    // ADR 0097: no main agent and no Agent SDK on this path.
    expect(commands().filter((c) => c.startsWith('agent_'))).toEqual([]);

    // A second press on the same (now gone) chip sends nothing more; no retry runs by itself.
    press('ask_genie:utt-direct-1');
    await settle();
    expect(genieAsks()).toHaveLength(1);
  });

  it('[G-14] [#573] a failed mcp_call (R5 stub, #16) shows the fixed failure text and is not sent again', async () => {
    hostWithStubR5();
    brain?.talkTarget.viewReady(MOG);
    await offer('What were EMEA bookings last week?', 'utt-direct-2');
    await settle();
    press('ask_genie:utt-direct-2');
    await settle();
    const texts = cards().flatMap((u) => u.blurbs.map((b) => b.text));
    // The working line shows first (G-44), then the failure replaces it.
    expect(texts).toContain(workingLine(HOME));
    const last =
      cards()
        .at(-1)
        ?.blurbs.map((b) => b.text) ?? [];
    expect(last.some((t) => /\nCode [A-Z_]+$/.test(t))).toBe(true);
    expect(last.join('\n')).not.toContain('not_implemented');
    expect(last).not.toContain(workingLine(HOME));
    await new Promise((r) => setTimeout(r, 50));
    expect(genieAsks()).toHaveLength(1);
    expect(commands().filter((c) => c.startsWith('agent_'))).toEqual([]);
  });
});
