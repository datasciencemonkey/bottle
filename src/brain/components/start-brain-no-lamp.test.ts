/**
 * startBrain, ADR 0117: a talk-key press with no lamp sends its notice with the workspace list
 * (`hub:workspaces`), the event that the hub listens to. All values are plainly fake.
 */
import { core } from '../test-core';
import { workspaceList } from '../../test-support/workspace-list';
import { NO_MOG_TEXT } from '../talk-no-mog';
import { startBrain, type Brain } from './index';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);
vi.mock('../notice-wiring', () => ({ wireNotice: vi.fn() }));
vi.mock('../hub-window', async (orig) => ({
  ...(await orig<typeof import('../hub-window')>()),
  showHub: vi.fn(async () => undefined),
  hubExists: vi.fn(async () => true),
}));
vi.mock('../level-stream', async (orig) => ({
  ...(await orig<typeof import('../level-stream')>()),
  wireLevelStream: vi.fn(() => () => undefined),
  subscribeLevels: vi.fn(async () => true),
}));

let brain: Brain | undefined;
const settle = async () => {
  for (let i = 0; i < 15; i++) await new Promise((r) => setTimeout(r, 0));
};
const workspaceSends = () =>
  core.emitTo.mock.calls.filter(([t, e]) => t === 'hub' && e === 'hub:workspaces');

describe('startBrain: the talk key with no lamp (ADR 0117)', () => {
  beforeAll(() => {
    brain = startBrain();
  });
  beforeEach(() => {
    core.reset();
    core.invoke.mockImplementation(async (cmd) => {
      if (cmd === 'workspace_list') return workspaceList(null, [{ profile: 'cli-profile' }]);
      if (cmd === 'config_list') return { items: [], unreadable: [] };
      return {};
    });
  });

  it('the notice goes on hub:workspaces and not on the retired hub:mogs', async () => {
    brain?.bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: '', phase: 'down' } });
    await settle();
    const texts = workspaceSends().map(
      ([, , l]) => (l as { notice?: { text: string } }).notice?.text,
    );
    expect(texts).toContain(NO_MOG_TEXT);
    expect(core.emitTo.mock.calls.some(([, e]) => e === 'hub:mogs')).toBe(false);
  });
});
