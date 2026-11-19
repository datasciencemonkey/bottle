/** #16: T7 `cliInfo()` reads `cli_status` from R5 (W-02a, W-02c, W-10a). Every value is fake. */
import { createProfileService } from './profile-service';
import { core } from '../test-core';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);

describe('T7 cliInfo (#16)', () => {
  beforeEach(() => core.reset());

  it('[W-02a] [W-02c] [W-10a] cliInfo gives the path, version and plaintextStorage of cli_status', async () => {
    core.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'cli_status'
        ? {
            path: '/usr/local/bin/databricks',
            version: '1.25.0',
            found: true,
            plaintextStorage: true,
          }
        : {},
    );
    await expect(createProfileService().cliInfo()).resolves.toEqual({
      path: '/usr/local/bin/databricks',
      version: '1.25.0',
      plaintextStorage: true,
    });
    expect(core.invoke).toHaveBeenCalledWith('cli_status', { args: {} });
  });

  it('[W-02a] a refused cli_status rejects', async () => {
    core.invoke.mockImplementation(async () => {
      throw { error: 'not_implemented' };
    });
    await expect(createProfileService().cliInfo()).rejects.toEqual({ error: 'not_implemented' });
  });
});
