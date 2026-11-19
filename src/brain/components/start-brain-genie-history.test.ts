/**
 * startBrain, #40 (AT-09, ADR 0061, ADR 0097): an Ask Genie request that ends saves one local
 * exchange row through `mem_local_exec`: `mem.mog.upsert` first, then `mem.exchange.upsert`.
 * With no OS key store the host answers `host_failed` (#576): the card still shows the outcome,
 * and the log holds only a fixed code. On main the host R5 is a stub (#16), so the request ends
 * in the fixed failure text. All values are plainly fake.
 */
import type { MockInstance } from 'vitest';
import { core } from '../test-core';
import { routerDecision } from '../../test-support/model-invoke';
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

const MOG = '00000000-0000-4000-8000-0000000000a9';
const HOME = 'fake-home';
const HOST = 'example.cloud.databricks.test';

let deps: UtteranceHandlerDeps | undefined;
let brain: Brain | undefined;

type CardPayload = { blurbs: { text: string }[] };
const lastBlurbs = () =>
  core.emitTo.mock.calls
    .filter(([target, event]) => target === `card-${MOG}` && event === 'card:update')
    .map(([, , payload]) => payload as CardPayload)
    .at(-1)
    ?.blurbs.map((b) => b.text) ?? [];
const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
type Exec = { stmtId: string; params: Record<string, unknown> };
const execs = () =>
  core.invoke.mock.calls
    .filter(([c]) => c === 'mem_local_exec')
    .map(([, a]) => (a as { args: Exec }).args);
const asks = () =>
  core.invoke.mock.calls.filter(
    ([c, a]) => c === 'mcp_call' && (a as { args: { tool: string } }).args.tool === 'genie_ask',
  );

/** The host as on main: a home profile, R5 stubbed (#16), and the given local store. */
function host(store: (a: Exec) => Promise<unknown>) {
  core.invoke.mockImplementation(async (cmd, args) => {
    if (cmd === 'config_read') return { json: { name: 'Fake Finance', binding: { home: HOME } } };
    if (cmd === 'profiles_list')
      return [{ name: HOME, host: HOST, authType: 'databricks-cli', supported: true }];
    if (cmd === 'mcp_call') return Promise.reject({ error: 'not_implemented' });
    if (cmd === 'mem_local_exec') return store((args as { args: Exec }).args);
    return {};
  });
}

async function ask(question: string, utteranceId: string) {
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
  await settle();
  brain?.bus.dispatch({
    event: 'view:input',
    payload: {
      from: `card-${MOG}`,
      input: { mogId: MOG, kind: 'chip_click', payload: { chipId: `ask_genie:${utteranceId}` } },
    },
  });
  await settle();
}

describe('startBrain saves each Ask Genie exchange locally (#40)', () => {
  let info: MockInstance<typeof console.info>;
  beforeAll(() => {
    brain = startBrain();
    deps = vi.mocked(createUtteranceHandler).mock.calls[0]?.[0];
  });
  beforeEach(() => {
    core.reset();
    info = vi.spyOn(console, 'info').mockImplementation(() => undefined);
  });
  afterEach(() => info.mockRestore());

  it('[AT-09] [ADR 0061] an ended request writes the MOG row, then one exchange with the exact text', async () => {
    host(async () => ({ rows: [{ changes: 1 }] }));
    brain?.talkTarget.viewReady(MOG);
    await ask('What were fake EMEA bookings?', 'utt-hist-1');
    expect(asks()).toHaveLength(1);
    expect(execs().map((e) => e.stmtId)).toEqual(['mem.mog.upsert', 'mem.exchange.upsert']);
    expect(execs()[0]?.params).toMatchObject({ mogId: MOG, displayName: 'Fake Finance' });
    expect(execs()[1]?.params).toMatchObject({
      mogId: MOG,
      profileName: HOME,
      workspaceHost: HOST,
      questionUser: 'What were fake EMEA bookings?',
      status: 'failed',
    });
    // One request is one exchange write; nothing more comes later.
    await new Promise((r) => setTimeout(r, 50));
    expect(execs().filter((e) => e.stmtId === 'mem.exchange.upsert')).toHaveLength(1);
  });

  it('[AT-09] [SEC-16c] a host_failed store still shows the outcome and logs only a code', async () => {
    host(() => Promise.reject({ error: 'host_failed' }));
    brain?.talkTarget.viewReady(MOG);
    await ask('What were fake APAC bookings?', 'utt-hist-2');
    expect(lastBlurbs().some((t) => /\nCode [A-Z_]+$/.test(t))).toBe(true);
    const lines = info.mock.calls.map((c) => String(c[0]));
    expect(lines).toContain('event=memory.write ok=0');
    expect(lines.join('\n')).not.toContain('APAC');
  });

  it('[AT-09] [B1] no exchange write for a question that was never sent', async () => {
    host(async () => ({ rows: [{ changes: 1 }] }));
    brain?.talkTarget.viewReady(MOG);
    if (!deps) throw new Error('startBrain made no utterance handler');
    await deps.routerInput(MOG);
    deps.onAsk(
      {
        route: 'chip',
        decision: routerDecision({ genie_question: 'Fake unsent?', cleaned_text: 'Fake unsent?' }),
        warning: undefined,
      },
      { utteranceId: 'utt-hist-3', mogId: MOG },
    );
    await settle();
    expect(asks()).toEqual([]);
    expect(execs()).toEqual([]);
  });
});
