import { core } from './test-core';
import { fakeWindows, memoryConfigs, sequentialIds } from '../test-support/mog-doubles';
import { BrainBus } from './bus';
import { createMogManager, type MogManager } from './components/mog-manager';
import {
  sendHubWorkspaces,
  wireHubWorkspaces,
  type HubWorkspacesDeps,
} from './hub-workspaces-wiring';
import type { HubWorkspaceList } from '../shared/hub-workspaces';
import type { Profile } from '../shared/interfaces';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const CLI = {
  path: '/usr/local/bin/databricks',
  version: '1.25.0',
  found: true,
  plaintextStorage: false,
};

let bus: BrainBus;
let m: MogManager;
let sent: HubWorkspaceList[];
let profiles: Profile[];
const send = vi.fn(async (l: HubWorkspaceList) => {
  sent.push(l);
});

function input(payload: unknown, from = 'hub', mogId = 'hub', kind = 'click'): void {
  bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
}

const last = () => sent.at(-1);

function deps(over: Partial<HubWorkspacesDeps> = {}): HubWorkspacesDeps {
  return {
    manager: m,
    listProfiles: async () => profiles,
    cliStatus: async () => CLI,
    signInState: () => 'unknown',
    ...over,
  };
}

beforeEach(() => {
  core.reset();
  bus = new BrainBus();
  m = createMogManager({
    configs: memoryConfigs(),
    windows: fakeWindows(),
    newId: sequentialIds(),
  });
  profiles = [
    {
      name: 'DEFAULT',
      host: 'https://a.cloud.databricks.com',
      authType: 'databricks-cli',
      supported: true,
    },
    {
      name: 'legacy-pat',
      host: 'https://b.cloud.databricks.com',
      authType: 'basic',
      supported: false,
    },
  ];
  sent = [];
  send.mockClear();
});

describe('hub Workspaces wiring (#560)', () => {
  it('[#560] sends the list once at start, with sign-in state and the reason of an unsupported profile', async () => {
    wireHubWorkspaces(
      bus,
      deps({ signInState: (p) => (p === 'DEFAULT' ? 'signed_in' : 'unknown') }),
      send,
    );
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(last()).toEqual({
      profiles: [
        {
          name: 'DEFAULT',
          host: 'https://a.cloud.databricks.com',
          authType: 'databricks-cli',
          supported: true,
          signIn: 'signed_in',
          homeOf: [],
        },
        {
          name: 'legacy-pat',
          host: 'https://b.cloud.databricks.com',
          authType: 'basic',
          supported: false,
          reason:
            'This profile does not use Databricks sign-in in the browser. Sign in with a bottle profile.',
          homeOf: [],
        },
      ],
      cli: CLI,
    });
  });

  it('[#560] lists the MOGs whose home is each profile under "Home of"', async () => {
    await m.create({ name: 'Sales', home: 'DEFAULT' });
    await m.create({ name: 'Ops', home: 'DEFAULT' });
    await m.create({ name: 'Finance', home: 'legacy-pat' });
    wireHubWorkspaces(bus, deps(), send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    const byName = new Map(last()?.profiles.map((p) => [p.name, p.homeOf]));
    // MogManager.list sorts MOGs by name (#26).
    expect(byName.get('DEFAULT')).toEqual(['Ops', 'Sales']);
    expect(byName.get('legacy-pat')).toEqual(['Finance']);
  });

  it('[#560] workspaces.ready asks again without refresh; workspaces.refresh asks with refresh', async () => {
    const listProfiles = vi.fn(async () => profiles);
    wireHubWorkspaces(bus, deps({ listProfiles }), send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(listProfiles).toHaveBeenLastCalledWith({ refresh: false });
    input({ action: 'workspaces.ready' });
    await vi.waitFor(() => expect(sent).toHaveLength(2));
    expect(listProfiles).toHaveBeenLastCalledWith({ refresh: false });
    input({ action: 'workspaces.refresh' });
    await vi.waitFor(() => expect(sent).toHaveLength(3));
    expect(listProfiles).toHaveBeenLastCalledWith({ refresh: true });
  });

  it('[#560] a MOG change (create, edit, delete) sends the list again so "Home of" stays correct', async () => {
    wireHubWorkspaces(bus, deps(), send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    await m.create({ name: 'Sales', home: 'DEFAULT' });
    await vi.waitFor(() => expect(sent).toHaveLength(2));
    expect(last()?.profiles.find((p) => p.name === 'DEFAULT')?.homeOf).toEqual(['Sales']);
  });

  it('[#560] a profile list failure sends an empty list with a fixed error text, and the CLI info still shows', async () => {
    wireHubWorkspaces(bus, deps({ listProfiles: () => Promise.reject(new Error('x')) }), send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(last()).toEqual({
      profiles: [],
      cli: CLI,
      notice: { kind: 'error', text: 'Bottle cannot list your Databricks CLI profiles.' },
    });
  });

  it('[#560] a cli_status failure still lists the profiles, with an unfound CLI', async () => {
    wireHubWorkspaces(bus, deps({ cliStatus: () => Promise.reject(new Error('x')) }), send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(last()?.cli).toEqual({ path: '', version: '', found: false, plaintextStorage: false });
    expect(last()?.profiles).toHaveLength(2);
  });

  it('[#560] [ARC-5] ignores actions from a MOG window, other kinds, mog.* actions and bad payloads', async () => {
    wireHubWorkspaces(bus, deps(), send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    input({ action: 'workspaces.refresh' }, 'mog-x', 'x');
    input({ action: 'workspaces.refresh' }, 'hub', 'hub', 'poked');
    input({ action: 'mog.create', name: 'X', home: 'p' });
    input({ action: 'nope' });
    await new Promise((r) => setTimeout(r, 20));
    expect(sent).toHaveLength(1);
  });

  it('[ARC-5] sendHubWorkspaces emits hub:workspaces to the hub window only', async () => {
    const l: HubWorkspaceList = { profiles: [], cli: CLI };
    await sendHubWorkspaces(l);
    expect(core.emitTo.mock.calls).toEqual([['hub', 'hub:workspaces', l]]);
  });

  it('[#560] the returned function stops the wiring, including the MOG change subscription', async () => {
    const off = wireHubWorkspaces(bus, deps(), send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    off();
    input({ action: 'workspaces.ready' });
    await m.create({ name: 'X', home: 'DEFAULT' });
    await new Promise((r) => setTimeout(r, 20));
    expect(sent).toHaveLength(1);
  });
});
