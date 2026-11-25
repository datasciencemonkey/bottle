/**
 * startBrain, slice 2a: the real wiring of the agent client, the card and the direct path. A
 * question goes to the orchestrator when it is ready (ADR 0122); the fallback and the notes must
 * show on the card, behind the arrival gate (H4, M6).
 */
import { core } from '../test-core';
import { routerDecision } from '../../test-support/model-invoke';
import { workspaceList } from '../../test-support/workspace-list';
import { createUtteranceHandler, type UtteranceHandlerDeps } from '../utterance-wiring';
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

const MOG = '00000000-0000-4000-8000-0000000005a4';
const HOME = 'fake-home';
const T1 = 'a'.repeat(32);
const TN = 'e'.repeat(32);
const Q = 'Compare churn in fake-home and fake-fin';

let deps: UtteranceHandlerDeps | undefined;
let brain: Brain | undefined;

type CardPayload = {
  blurbs: { id: string; text: string }[];
  chips: { id: string; label: string }[];
  agentTurn?: { turnId: string; state: string; notice?: string };
};
const cards = () =>
  core.emitTo.mock.calls
    .filter(([target, event]) => target === `card-${MOG}` && event === 'card:update')
    .map(([, , payload]) => payload as CardPayload);
const settle = async () => {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
};
const commands = () => core.invoke.mock.calls.map(([c]) => c as string);
const genieAsks = () =>
  core.invoke.mock.calls.filter(
    ([c, a]) => c === 'mcp_call' && (a as { args: { tool: string } }).args.tool === 'genie_ask',
  );

function host(over: Record<string, () => unknown> = {}) {
  core.invoke.mockImplementation(async (cmd) => {
    if (over[cmd as string]) return over[cmd as string]?.();
    if (cmd === 'config_read') return { json: { name: 'Finance', binding: { home: HOME } } };
    if (cmd === 'workspace_list')
      return workspaceList(HOME, [{ profile: HOME }, { profile: 'fake-fin', available: true }]);
    if (cmd === 'orchestrator_status')
      return { state: 'ready', model: 'system.ai.claude-haiku-5-5' };
    if (cmd === 'orchestrator_models')
      return {
        models: [
          {
            id: 'system.ai.claude-haiku-5-5',
            label: 'Claude Haiku 5.5',
            tested: true,
            tools: true,
          },
        ],
        selected: 'system.ai.claude-haiku-5-5',
        defaultModel: 'system.ai.claude-haiku-5-5',
      };
    if (cmd === 'agent_turn_submit')
      return {
        turnId: T1,
        sessionId: 'd'.repeat(32),
        position: 0,
        newSession: true,
        budget: 10,
        catalog: [
          { workspaceId: 'ws-a', profile: HOME, name: HOME, isHome: true, isAgentWorkspace: true },
        ],
      };
    if (cmd === 'agent_session_state')
      return { state: 'busy', agentProfile: HOME, turns: 1, queued: 0, tokens: 0 };
    if (cmd === 'mcp_call') return Promise.reject({ error: 'not_implemented' });
    return {};
  });
}

/** The hub opens: the brain reads the workspace list and the orchestrator status (the routing cache). */
async function prime() {
  brain?.talkTarget.viewReady(MOG);
  brain?.bus.dispatch({
    event: 'view:input',
    payload: {
      from: 'hub',
      input: { mogId: 'hub', kind: 'click', payload: { action: 'workspaces.ready' } },
    },
  });
  brain?.bus.dispatch({
    event: 'view:input',
    payload: {
      from: 'hub',
      input: { mogId: 'hub', kind: 'click', payload: { action: 'orchestrator.ready' } },
    },
  });
  await settle();
}

async function ask(question: string, id: string) {
  if (!deps) throw new Error('no utterance handler');
  await deps.routerInput(MOG);
  deps.onAsk(
    {
      route: 'ask',
      question,
      decision: routerDecision({ genie_question: question, cleaned_text: question }),
    },
    { utteranceId: id, mogId: MOG },
  );
}
const turnEvent = (payload: unknown) => brain?.bus.dispatch({ event: 'agent:turn', payload });

describe('startBrain: the agent, the card and the direct fallback (slice 2a)', () => {
  beforeAll(() => {
    brain = startBrain();
    deps = vi.mocked(createUtteranceHandler).mock.calls[0]?.[0];
  });
  beforeEach(() => core.reset());
  afterEach(() => vi.useRealTimers());

  it('[AC-2a.2] with the orchestrator ready a question goes to the agent, and the card shows the turn', async () => {
    host();
    await prime();
    await ask(Q, 'u-agent-1');
    await settle();
    expect(commands().filter((c) => c === 'agent_turn_submit')).toHaveLength(1);
    expect(genieAsks()).toEqual([]);
    expect(cards().at(-1)?.agentTurn).toMatchObject({ turnId: T1, state: 'planning' });
    expect(
      cards()
        .at(-1)
        ?.chips.map((c) => c.id),
    ).toContain(`agent-stop:${T1}`);
  });

  it('[H4] [AC-2a.11] a fallback takes the card: the direct request shows, with no agent turn on it', async () => {
    host();
    await prime();
    await ask(Q, 'u-agent-2');
    await settle();
    turnEvent({ kind: 'ended', turnId: T1, outcome: 'timeout', calls: 0 });
    await settle();
    expect(genieAsks()).toHaveLength(1);
    const last = cards().at(-1);
    expect(last?.agentTurn).toBeUndefined();
    expect(
      last?.blurbs.some((b) => b.id.startsWith('genie:') || /\nCode [A-Z_]+$/.test(b.text)),
    ).toBe(true);
    // A late event of the agent turn never covers the direct card again.
    turnEvent({ kind: 'slow', turnId: T1 });
    await settle();
    expect(cards().at(-1)?.agentTurn).toBeUndefined();
  });

  it('[R-6] with the orchestrator not reachable a question goes direct to home, with no agent turn', async () => {
    host({ orchestrator_status: () => ({ state: 'unreachable', model: 'system.ai.x' }) });
    await prime();
    await ask(Q, 'u-agent-direct');
    await settle();
    expect(commands().filter((c) => c === 'agent_turn_submit')).toEqual([]);
    expect(genieAsks()).toHaveLength(1);
  });

  it('[H1] a New session that ends a waiting turn asks home never', async () => {
    host();
    await prime();
    await ask(Q, 'u-agent-3');
    await settle();
    turnEvent({ kind: 'ended', turnId: T1, outcome: 'session_ended', calls: 0 });
    await settle();
    expect(genieAsks()).toEqual([]);
  });

  it('[H4] [M6] a queue-full note shows on the card, and not during a hold', async () => {
    host({ agent_turn_submit: () => Promise.reject({ error: 'agent_queue_full' }) });
    await prime();
    await ask(Q, 'u-agent-4'); // route ask: no hold
    await settle();
    expect(
      cards()
        .at(-1)
        ?.blurbs.some((b) => b.text.includes('3 questions waiting')),
    ).toBe(true);

    const noteIds = () =>
      new Set(
        cards()
          .flatMap((c) => c.blurbs.map((b) => b.id))
          .filter((id) => id.startsWith('agent-note:')),
      );
    const old = noteIds();
    const fresh = () => [...noteIds()].filter((id) => !old.has(id));
    vi.useFakeTimers();
    brain?.bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: MOG, phase: 'down' } });
    await ask(Q, 'u-agent-5');
    await vi.advanceTimersByTimeAsync(100);
    expect(fresh()).toEqual([]);
    brain?.bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: MOG, phase: 'cancel' } });
    await vi.advanceTimersByTimeAsync(1_199);
    expect(fresh()).toEqual([]);
    await vi.advanceTimersByTimeAsync(1);
    expect(fresh()).toHaveLength(1);
  });

  it('[N1] a queue-full note on a live turn leaves the turn on the card, and its answer shows', async () => {
    let n = 0;
    host({
      agent_turn_submit: () => {
        if (++n === 1)
          return {
            turnId: TN,
            sessionId: 'd'.repeat(32),
            position: 0,
            newSession: true,
            budget: 10,
            catalog: [
              {
                workspaceId: 'ws-a',
                profile: HOME,
                name: HOME,
                isHome: true,
                isAgentWorkspace: true,
              },
            ],
          };
        return Promise.reject({ error: 'agent_queue_full' });
      },
    });
    await prime();
    await ask(Q, 'u-agent-6');
    await settle();
    await ask(`${Q} again`, 'u-agent-7');
    await settle();
    // The note is one line inside the agent card, and the live turn is still on it.
    expect(cards().at(-1)?.agentTurn).toMatchObject({ turnId: TN });
    expect(cards().some((c) => c.agentTurn?.notice?.includes('3 questions waiting'))).toBe(true);
    turnEvent({ kind: 'ended', turnId: TN, outcome: 'answered', calls: 0 });
    await settle();
    expect(cards().at(-1)?.agentTurn).toMatchObject({ turnId: TN, state: 'done' });
  });
});
