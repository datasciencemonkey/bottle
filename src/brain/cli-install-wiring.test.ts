/** #161: the brain side of the first-run "Install the Databricks CLI" step. Values are fake. */
import {
  CLI_INSTALL_EVENT,
  CLI_INSTALL_SOURCE,
  CLI_INSTALL_TARGET,
  CLI_INSTALL_VERSION,
  CLI_SETUP_TARGET,
  CLI_VIEW_ID,
  cliInstallFolder,
} from '../shared/cli-view';
import { BrainBus } from './bus';
import { wireCliInstall } from './cli-wiring';
import { core } from './test-core';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const BOTTLE = '/Users/someone/Library/Application Support/Bottle/bin/databricks';
const OLD = '/usr/local/bin/databricks';

function click(bus: BrainBus, target: string, from = 'hub', mogId = CLI_VIEW_ID) {
  bus.dispatch({
    event: 'view:input',
    payload: { from, input: { mogId, kind: 'click', payload: { target } } },
  });
}

const flush = () => new Promise((r) => setTimeout(r, 0));
const none = { path: '', version: '', plaintextStorage: false };

describe('CLI install wiring (#161)', () => {
  beforeEach(() => core.reset());

  it('[W-17c] [U16] with no usable CLI the hub gets the step with source, version and folder', async () => {
    const bus = new BrainBus();
    const install = vi.fn();
    wireCliInstall(bus, { cliInfo: async () => none, install, platform: 'macos' });
    click(bus, CLI_SETUP_TARGET);
    await flush();
    expect(core.emitTo).toHaveBeenCalledWith('hub', CLI_INSTALL_EVENT, {
      step: 'offer',
      source: CLI_INSTALL_SOURCE,
      version: CLI_INSTALL_VERSION,
      folder: '~/Library/Application Support/Bottle/bin',
    });
    expect(install).not.toHaveBeenCalled();
  });

  it('[W-17c] a verified CLI gives no install step', async () => {
    const bus = new BrainBus();
    wireCliInstall(bus, {
      cliInfo: async () => ({ path: OLD, version: '1.25.0', plaintextStorage: false }),
      install: vi.fn(),
      platform: 'macos',
    });
    click(bus, CLI_SETUP_TARGET);
    await flush();
    expect(core.emitTo).toHaveBeenCalledWith('hub', CLI_INSTALL_EVENT, {
      step: 'none',
      path: OLD,
      version: '1.25.0',
    });
  });

  it('[W-17b] [U16] cli_install runs only after the user selects Install in the hub', async () => {
    const bus = new BrainBus();
    const install = vi.fn(async () => ({
      path: BOTTLE,
      version: '1.26.0',
      found: true,
      plaintextStorage: false,
    }));
    wireCliInstall(bus, { cliInfo: async () => none, install, platform: 'macos' });
    click(bus, CLI_SETUP_TARGET);
    // A click from a MOG window, or another target, is not consent.
    click(bus, CLI_INSTALL_TARGET, 'mog-0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01');
    click(bus, 'cli_settings');
    await flush();
    expect(install).not.toHaveBeenCalled();
    click(bus, CLI_INSTALL_TARGET);
    await flush();
    expect(install).toHaveBeenCalledTimes(1);
    expect(core.emitTo).toHaveBeenCalledWith('hub', CLI_INSTALL_EVENT, {
      step: 'installing',
      source: CLI_INSTALL_SOURCE,
      version: CLI_INSTALL_VERSION,
      folder: '~/Library/Application Support/Bottle/bin',
    });
    // W-17f: the hub shows the path of the CLI in use (the Bottle copy).
    expect(core.emitTo).toHaveBeenLastCalledWith('hub', CLI_INSTALL_EVENT, {
      step: 'installed',
      path: BOTTLE,
      version: '1.26.0',
    });
  });

  it('[W-17d] a refused install sends a failed step with no error text', async () => {
    const bus = new BrainBus();
    wireCliInstall(bus, {
      cliInfo: async () => none,
      install: async () => {
        throw { error: 'host_failed' };
      },
      platform: 'windows',
    });
    click(bus, CLI_INSTALL_TARGET);
    await flush();
    expect(core.emitTo).toHaveBeenLastCalledWith('hub', CLI_INSTALL_EVENT, {
      step: 'failed',
      source: CLI_INSTALL_SOURCE,
      version: CLI_INSTALL_VERSION,
      folder: '%LOCALAPPDATA%\\Bottle\\bin',
    });
  });

  it('[W-17b] a second click while the install runs starts no second install', async () => {
    const bus = new BrainBus();
    let done: (v: unknown) => void = () => undefined;
    const install = vi.fn(() => new Promise((r) => (done = r)));
    wireCliInstall(bus, { cliInfo: async () => none, install, platform: 'macos' });
    click(bus, CLI_INSTALL_TARGET);
    click(bus, CLI_INSTALL_TARGET);
    await flush();
    expect(install).toHaveBeenCalledTimes(1);
    done({ path: BOTTLE, version: '1.26.0', found: true, plaintextStorage: false });
  });

  it('[W-17e] the folder text names the user-only folder of each OS', () => {
    expect(cliInstallFolder('macos')).toBe('~/Library/Application Support/Bottle/bin');
    expect(cliInstallFolder('windows')).toBe('%LOCALAPPDATA%\\Bottle\\bin');
  });
});
