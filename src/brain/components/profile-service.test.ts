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

describe('T7 login (#162)', () => {
  beforeEach(() => core.reset());

  it('[W-07c] [W-15] login calls profile_login with the host, the name and the client ID', async () => {
    core.invoke.mockImplementation(async () => ({ loginId: 'login-1' }));
    const t7 = createProfileService();
    await expect(
      t7.login('https://acme-prod.cloud.databricks.com', 'bottle-x', { clientId: 'custom-app-id' }),
    ).resolves.toEqual({ loginId: 'login-1' });
    expect(core.invoke).toHaveBeenCalledWith('profile_login', {
      args: {
        workspaceUrl: 'https://acme-prod.cloud.databricks.com',
        profile: 'bottle-x',
        clientId: 'custom-app-id',
      },
    });
    await t7.login('https://acme-prod.cloud.databricks.com', 'bottle-y');
    expect(core.invoke).toHaveBeenLastCalledWith('profile_login', {
      args: { workspaceUrl: 'https://acme-prod.cloud.databricks.com', profile: 'bottle-y' },
    });
  });

  it('[W-07b] a refused profile_login rejects with its code', async () => {
    core.invoke.mockImplementation(async () => {
      throw { error: 'host_refused' };
    });
    await expect(createProfileService().login('https://example.com', 'bottle-x')).rejects.toEqual({
      error: 'host_refused',
    });
  });
});
