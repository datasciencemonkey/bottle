/** #16: the brain side of the hub CLI settings (W-02a, W-02c, W-02d, ARC-5). Values are fake. */
import { CLI_SETTINGS_EVENT, CLI_SETTINGS_TARGET, CLI_VIEW_ID } from '../shared/cli-view';
import { BrainBus } from './bus';
import { wireCliSettings } from './cli-wiring';
import { core } from './test-core';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const PATH = '/usr/local/bin/databricks';

function click(bus: BrainBus, from = 'hub', mogId = CLI_VIEW_ID, target = CLI_SETTINGS_TARGET) {
  bus.dispatch({
    event: 'view:input',
    payload: { from, input: { mogId, kind: 'click', payload: { target } } },
  });
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('CLI settings wiring (#16)', () => {
  beforeEach(() => core.reset());

  it('[W-02a] [W-02c] a hub ask sends the CLI path and version to the hub only', async () => {
    const bus = new BrainBus();
    wireCliSettings(bus, {
      cliInfo: async () => ({ path: PATH, version: '1.20.0', plaintextStorage: false }),
    });
    click(bus);
    await flush();
    expect(core.emitTo).toHaveBeenCalledTimes(1);
    expect(core.emitTo).toHaveBeenCalledWith('hub', CLI_SETTINGS_EVENT, {
      found: true,
      path: PATH,
      version: '1.20.0',
      belowRecommended: true,
      warnings: [],
    });
  });

  it('[W-02a] a click from another view, id or target does nothing', async () => {
    const bus = new BrainBus();
    const cliInfo = vi.fn(async () => ({ path: PATH, version: '1.25.0', plaintextStorage: false }));
    wireCliSettings(bus, { cliInfo });
    click(bus, 'mog-0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01', '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01');
    click(bus, 'hub', 'wipe');
    click(bus, 'hub', CLI_VIEW_ID, 'wipe_settings');
    await flush();
    expect(cliInfo).not.toHaveBeenCalled();
    expect(core.emitTo).not.toHaveBeenCalled();
  });

  it('[W-02a] when cli_status is refused the hub gets an unknown status, with no error text', async () => {
    const bus = new BrainBus();
    wireCliSettings(bus, {
      cliInfo: async () => {
        throw new Error('not_implemented');
      },
    });
    click(bus);
    await flush();
    expect(core.emitTo).toHaveBeenCalledWith('hub', CLI_SETTINGS_EVENT, { unknown: true });
  });
});
