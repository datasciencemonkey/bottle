/**
 * startBrain, ADR 0113, ADR 0117: the card and the voice "remember" write to the lamp id, and
 * `memory:saved` goes to the lamp card, even when the home workspace has another data MOG.
 * (Rust mints the tickets for the lamp's MOG only.) All values are plainly fake.
 */
import { core } from '../test-core';
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

const LAMP = '00000000-0000-4000-8000-0000000000b1';
const DATA = '00000000-0000-4000-8000-0000000000b2';
const HOME = 'fake-home';

let deps: UtteranceHandlerDeps | undefined;
let brain: Brain | undefined;
const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
const execMogIds = () =>
  core.invoke.mock.calls
    .filter(([c]) => c === 'mem_local_exec')
    .map(([, a]) => (a as { args: { params: { mogId?: string } } }).args.params.mogId);
const saved = () =>
  core.emitTo.mock.calls.filter(([, event]) => event === 'memory:saved') as [
    string,
    string,
    { mogId: string },
  ][];

describe('startBrain remember goes to the lamp (ADR 0117)', () => {
  beforeAll(() => {
    brain = startBrain();
    deps = vi.mocked(createUtteranceHandler).mock.calls[0]?.[0];
  });
  beforeEach(() => {
    core.reset();
    core.invoke.mockImplementation(async (cmd) => {
      if (cmd === 'workspace_list')
        return workspaceList(HOME, [{ profile: HOME, mogIds: [DATA] }], { lampMogId: LAMP });
      if (cmd === 'mem_local_exec') return { rows: [{ changes: 1 }] };
      return {};
    });
  });

  it('a voice remember writes under the lamp id and answers the lamp card', async () => {
    await brain?.ready;
    deps?.onRemember?.(
      { route: 'remember', text: 'fake fact', ticket: 'fake-ticket' } as never,
      { utteranceId: 'u1', mogId: LAMP } as never,
    );
    await settle();
    const ids = execMogIds().filter((x) => x !== undefined);
    expect(ids.length).toBeGreaterThan(0);
    expect(ids.every((x) => x === LAMP)).toBe(true);
    expect(execMogIds()).not.toContain(DATA);
    expect(saved().map(([target, , p]) => [target, p.mogId])).toEqual(
      saved().map(() => [`card-${LAMP}`, LAMP]),
    );
    expect(saved().length).toBeGreaterThan(0);
  });

  it('a card remember answers the lamp card and never touches the data MOG', async () => {
    await brain?.ready;
    brain?.bus.dispatch({
      event: 'view:input',
      payload: {
        from: `card-${LAMP}`,
        input: {
          mogId: LAMP,
          kind: 'click',
          payload: { action: 'memory.remember' },
          actionTicket: 'fake-ticket',
        },
      },
    });
    await settle();
    expect(execMogIds()).not.toContain(DATA);
    expect(saved().length).toBeGreaterThan(0);
    expect(saved().every(([target, , p]) => target === `card-${LAMP}` && p.mogId === LAMP)).toBe(
      true,
    );
  });
});
