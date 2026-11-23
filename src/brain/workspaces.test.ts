/** The brain side of the one MOG with workspaces (ADR 0117, S1-B). All IPC is fake. */
import type { HubWorkspaceList } from '../shared/hub-workspaces';
import { HUB_WORKSPACES_EVENT } from '../shared/hub-workspaces';
import type { WorkspaceList } from '../shared/ipc';
import { MOG_PALETTE } from '../shared/mog-palette';
import { memoryConfigs, sequentialIds } from '../test-support/mog-doubles';
import { workspaceList } from '../test-support/workspace-list';
import { BrainBus } from './bus';
import type { MogConfig } from './components/mog-config';
import { core } from './test-core';
import {
  createWorkspaces,
  ipcWorkspacesDeps,
  lampOnlyManager,
  signInByHand,
  WORKSPACE_TEXT,
  wireWorkspaces,
  type WorkspacesDeps,
} from './workspaces';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const id = (n: number) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
const DAY = 24 * 60 * 60 * 1000;
const NOW = 1_800_000_000_000;

function cfg(name: string, home: string, variant: string): MogConfig {
  return {
    schemaVersion: 1,
    name,
    persona: '',
    ui: { variant, size: 'M', dozeAfterMin: 20 },
    binding: {
      home,
      backend: { kind: 'genie_one_mcp', profile: home },
      genieCompanion: true,
      profileAliases: {},
    },
  } as unknown as MogConfig;
}

function rig(
  initial: WorkspaceList,
  opts: { exchanges?: Record<string, { askedAt: number; profileName: string }[]> } = {},
) {
  let state = initial;
  const configs = memoryConfigs();
  const sent: HubWorkspaceList[] = [];
  const calls: [string, unknown][] = [];
  const changed = vi.fn();
  const rows = () => state.workspaces;
  const deps: WorkspacesDeps = {
    list: async (refresh) => {
      calls.push(['list', refresh]);
      return state;
    },
    setAvailable: async (profile, available, mogId) => {
      calls.push(['set_available', { profile, available, mogId }]);
      state = {
        ...state,
        workspaces: rows().map((r) =>
          r.profile === profile
            ? { ...r, available: available || r.isHome, mogIds: mogId ? [mogId] : r.mogIds }
            : r,
        ),
      };
      return state;
    },
    makeHome: async (profile, mogId) => {
      calls.push(['make_home', { profile, mogId }]);
      state = {
        ...state,
        home: profile,
        workspaces: rows().map((r) => ({
          ...r,
          isHome: r.profile === profile,
          available: r.profile === profile || r.available,
          mogIds: r.profile === profile && mogId ? [mogId] : r.mogIds,
        })),
      };
      return state;
    },
    settingsWrite: async (a) => {
      calls.push(['settings_write', a]);
      state = { ...state, agent: { ...state.agent, ...a } };
      return state;
    },
    reauth: async (profile) => {
      calls.push(['reauth', profile]);
    },
    configs,
    store: {
      exec: async (stmt, params) => {
        calls.push([stmt, params]);
        const mogId = (params as { mogId: string }).mogId;
        return { rows: opts.exchanges?.[mogId] ?? [] };
      },
    },
    send: async (l) => void sent.push(l),
    newId: (() => {
      const next = sequentialIds();
      for (let i = 0; i < 99; i++) next();
      return next;
    })(),
    now: () => NOW,
    onChanged: changed,
  };
  const ws = createWorkspaces(deps);
  return {
    ws,
    deps,
    configs,
    sent,
    calls,
    changed,
    last: () => sent.at(-1),
    set: (s: WorkspaceList) => (state = s),
    names: () => calls.map((c) => c[0]),
  };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('workspaces: the list and its hub view (ADR 0117)', () => {
  it('[AC-1.2] a row takes its name and colour from the config of mogIds[0]; a new row uses the profile and the next free colour', async () => {
    const list = workspaceList('work', [
      { profile: 'work', mogIds: [id(1), id(9)] },
      { profile: 'lab', mogIds: [id(2)], available: true },
      { profile: 'new-one' },
    ]);
    const r = rig(list);
    r.configs.files.set(id(1), cfg('Sales', 'work', 'coral'));
    r.configs.files.set(id(2), cfg('Lab MOG', 'lab', 'amber'));
    await r.ws.refresh();
    expect(r.last()?.rows.map((x) => [x.profile, x.name, x.colorVariant])).toEqual([
      ['work', 'Sales', 'coral'],
      ['lab', 'Lab MOG', 'amber'],
      ['new-one', 'new-one', 'emerald'],
    ]);
    expect(r.last()?.home).toBe('work');
    expect(r.last()?.homePick).toBeUndefined();
    expect(r.last()?.agent).toEqual({
      crossWorkspaceData: false,
      crossWorkspaceLocked: false,
      genieCallBudget: 10,
    });
  });

  it('[AC-1.4] a migrated list is only read: no config write or delete, no store write, so history and memory stay as they were', async () => {
    const r = rig(
      workspaceList(null, [
        { profile: 'a', mogIds: [id(1)], available: true },
        { profile: 'b', mogIds: [id(2)], available: true },
        { profile: 'c', mogIds: [id(3)], available: true },
      ]),
    );
    for (const [n, p] of [
      [1, 'a'],
      [2, 'b'],
      [3, 'c'],
    ] as const)
      r.configs.files.set(id(n), cfg(`MOG ${p}`, p, MOG_PALETTE[n]?.id ?? 'coral'));
    await r.ws.refresh({ refresh: true });
    await r.ws.act({ action: 'workspaces.ready' });
    expect(r.configs.calls.filter(([op]) => op === 'write' || op === 'remove')).toEqual([]);
    expect(r.names().filter((n) => !['list', 'mem.exchange.recent'].includes(n))).toEqual([]);
    expect([...r.configs.files.keys()]).toEqual([id(1), id(2), id(3)]);
    // The dataMogId of each profile is the old MOG: its history stays under its own id.
    expect(await r.ws.dataMogId('b')).toBe(id(2));
  });

  it('[AC-1.10] with no home and two or more candidates the hub gets the pick; nothing picks a home', async () => {
    const r = rig(
      workspaceList(null, [
        { profile: 'a', mogIds: [id(1)], available: true },
        { profile: 'b', mogIds: [id(2)], available: true },
      ]),
    );
    r.configs.files.set(id(1), cfg('A', 'a', 'coral'));
    r.configs.files.set(id(2), cfg('B', 'b', 'amber'));
    await r.ws.refresh();
    expect(r.last()?.home).toBeNull();
    expect(r.last()?.homePick?.candidates).toEqual(['a', 'b']);
    expect(r.names()).not.toContain('make_home');
    expect(r.names()).not.toContain('set_available');
    expect(await r.ws.home()).toBeUndefined();
  });

  it('[AC-1.10] the suggestion is the profile with the most exchanges in the last 30 days, and it is not applied', async () => {
    const r = rig(
      workspaceList(null, [
        { profile: 'a', mogIds: [id(1)], available: true },
        { profile: 'b', mogIds: [id(2)], available: true },
        { profile: 'c', mogIds: [id(3)], available: true },
      ]),
      {
        exchanges: {
          [id(1)]: [
            { askedAt: NOW - 40 * DAY, profileName: 'a' },
            { askedAt: NOW - 31 * DAY, profileName: 'a' },
            { askedAt: NOW - 1 * DAY, profileName: 'a' },
          ],
          [id(2)]: [
            { askedAt: NOW - 2 * DAY, profileName: 'b' },
            { askedAt: NOW - 3 * DAY, profileName: 'b' },
          ],
          [id(3)]: [],
        },
      },
    );
    await r.ws.refresh();
    expect(r.last()?.homePick?.suggested).toBe('b');
    expect(r.names()).not.toContain('make_home');
  });

  it('[AC-1.10] with no exchanges the suggestion is the first candidate; one candidate gives no pick dialog', async () => {
    const r = rig(
      workspaceList(null, [
        { profile: 'a', mogIds: [id(1)], available: true },
        { profile: 'b', mogIds: [id(2)], available: true },
      ]),
    );
    await r.ws.refresh();
    expect(r.last()?.homePick?.suggested).toBe('a');
    const one = rig(workspaceList(null, [{ profile: 'a', mogIds: [id(1)], available: true }]));
    await one.ws.refresh();
    expect(one.last()?.homePick).toBeUndefined();
    expect(one.names()).not.toContain('make_home');
  });

  it('[ARC-5] the list goes to the hub window only', async () => {
    core.reset();
    const deps = ipcWorkspacesDeps();
    await deps.send({
      rows: [],
      home: null,
      agent: { crossWorkspaceData: false, crossWorkspaceLocked: false, genieCallBudget: 10 },
    });
    expect(core.emitTo.mock.calls.map((c) => [c[0], c[1]])).toEqual([
      ['hub', HUB_WORKSPACES_EVENT],
    ]);
  });
});

describe('workspaces: hub actions (ADR 0117)', () => {
  it('[AC-1.5] a set_available on a row with no config first writes a new config, then switches it on with that mogId', async () => {
    const r = rig(
      workspaceList('home', [{ profile: 'home', mogIds: [id(1)] }, { profile: 'fresh' }]),
    );
    r.configs.files.set(id(1), cfg('Home', 'home', 'coral'));
    await r.ws.refresh();
    r.sent.length = 0;
    await r.ws.act({ action: 'workspace.set_available', profile: 'fresh', available: true });
    expect(r.configs.files.get(id(1))?.name).toBe('Home');
    const newId = [...r.configs.files.keys()].find((k) => k !== id(1)) as string;
    const c = r.configs.files.get(newId);
    expect(c?.name).toBe('fresh');
    expect(c?.binding.home).toBe('fresh');
    expect(c?.ui.variant).toBe('amber');
    expect(r.calls.find((x) => x[0] === 'set_available')?.[1]).toEqual({
      profile: 'fresh',
      available: true,
      mogId: newId,
    });
    expect(r.last()?.rows.find((x) => x.profile === 'fresh')).toMatchObject({
      available: true,
      name: 'fresh',
      colorVariant: 'amber',
    });
  });

  it('[AC-1.5] when the store refuses the switch, the new config is removed again', async () => {
    const r = rig(
      workspaceList('home', [{ profile: 'home', mogIds: [id(1)] }, { profile: 'fresh' }]),
    );
    r.configs.files.set(id(1), cfg('Home', 'home', 'coral'));
    await r.ws.refresh();
    r.deps.setAvailable = async () => Promise.reject({ error: 'workspace_storage_failed' });
    await r.ws.act({ action: 'workspace.set_available', profile: 'fresh', available: true });
    expect([...r.configs.files.keys()]).toEqual([id(1)]);
    expect(r.last()?.notice).toEqual({ kind: 'error', text: WORKSPACE_TEXT.failed });
  });

  it('[AC-1.6] home cannot be switched off: the store is not asked', async () => {
    const r = rig(workspaceList('home', [{ profile: 'home', mogIds: [id(1)] }]));
    await r.ws.refresh();
    await r.ws.act({ action: 'workspace.set_available', profile: 'home', available: false });
    expect(r.names()).not.toContain('set_available');
    expect(r.last()?.notice?.text).toBe(WORKSPACE_TEXT.home_always_available);
  });

  it('a switch on a row that has a config sends no mogId and writes no config', async () => {
    const r = rig(
      workspaceList('home', [
        { profile: 'home', mogIds: [id(1)] },
        { profile: 'b', mogIds: [id(2)] },
      ]),
    );
    await r.ws.refresh();
    await r.ws.act({ action: 'workspace.set_available', profile: 'b', available: true });
    expect(r.calls.find((x) => x[0] === 'set_available')?.[1]).toEqual({
      profile: 'b',
      available: true,
      mogId: undefined,
    });
    expect(r.configs.calls.filter(([op]) => op === 'write')).toEqual([]);
    expect(r.last()?.rows.find((x) => x.profile === 'b')?.available).toBe(true);
  });

  it('make_home calls the store with the confirmed profile, and the lamp learns the new identity', async () => {
    const r = rig(
      workspaceList('home', [
        { profile: 'home', mogIds: [id(1)] },
        { profile: 'b', mogIds: [id(2)] },
      ]),
    );
    await r.ws.refresh();
    await r.ws.act({ action: 'workspace.make_home', profile: 'b', confirmed: true });
    expect(r.names()).toContain('make_home');
    expect(r.last()?.home).toBe('b');
    expect(r.changed).toHaveBeenCalled();
  });

  it('pick_home makes the pick home; a pick when a home exists changes nothing', async () => {
    const r = rig(
      workspaceList(null, [
        { profile: 'a', mogIds: [id(1)], available: true },
        { profile: 'b', mogIds: [id(2)], available: true },
      ]),
    );
    await r.ws.refresh();
    await r.ws.act({ action: 'workspace.pick_home', profile: 'b' });
    expect(r.last()?.home).toBe('b');
    expect(r.last()?.homePick).toBeUndefined();
    const before = r.names().filter((n) => n === 'make_home').length;
    await r.ws.act({ action: 'workspace.pick_home', profile: 'a' });
    expect(r.names().filter((n) => n === 'make_home').length).toBe(before);
  });

  it('workspace.sign_in sets busyProfile while profile_reauth runs, then refreshes the list', async () => {
    const r = rig(workspaceList('home', [{ profile: 'home', mogIds: [id(1)], signIn: 'expired' }]));
    await r.ws.refresh();
    let release: () => void = () => undefined;
    r.deps.reauth = (p) => {
      r.calls.push(['reauth', p]);
      return new Promise<void>((ok) => (release = ok));
    };
    r.sent.length = 0;
    const done = r.ws.act({ action: 'workspace.sign_in', profile: 'home' });
    await flush();
    expect(r.last()?.busyProfile).toBe('home');
    r.set(workspaceList('home', [{ profile: 'home', mogIds: [id(1)], signIn: 'signed_in' }]));
    release();
    await done;
    expect(r.last()?.busyProfile).toBeUndefined();
    expect(r.last()?.rows[0]?.signIn).toBe('signed_in');
    expect(r.calls.at(-1)).toEqual(['list', true]);
  });

  it('[S1-B change] a sign-in that answers not_implemented clears busyProfile and tells the user to use the CLI', async () => {
    const r = rig(workspaceList('home', [{ profile: 'home', mogIds: [id(1)], signIn: 'expired' }]));
    await r.ws.refresh();
    r.deps.reauth = async () => Promise.reject({ error: 'not_implemented' });
    r.sent.length = 0;
    await r.ws.act({ action: 'workspace.sign_in', profile: 'home' });
    expect(r.last()?.busyProfile).toBeUndefined();
    expect(r.last()?.notice).toEqual({
      kind: 'info',
      text: 'To sign in again, run "databricks auth login --profile home" in Terminal, then reopen Settings.',
    });
    expect(signInByHand('home')).toBe(r.last()?.notice?.text);
    // The next workspaces.ready reads the new state with a refresh.
    await r.ws.act({ action: 'workspaces.ready' });
    expect(r.calls.at(-1)).toEqual(['list', true]);
  });

  it('a refused sign-in shows a fixed text, never the upstream text', async () => {
    const r = rig(workspaceList('home', [{ profile: 'home', mogIds: [id(1)], signIn: 'expired' }]));
    await r.ws.refresh();
    r.deps.reauth = async () =>
      Promise.reject({ error: 'no_grant', reason: 'secret upstream text' });
    await r.ws.act({ action: 'workspace.sign_in', profile: 'home' });
    expect(r.last()?.notice).toEqual({ kind: 'error', text: WORKSPACE_TEXT.no_grant });
    expect(r.last()?.busyProfile).toBeUndefined();
    expect(JSON.stringify(r.last())).not.toContain('secret');
  });

  it('workspace.edit writes the name and the colour to the config of mogIds[0] and tells the lamp', async () => {
    const r = rig(workspaceList('home', [{ profile: 'home', mogIds: [id(1), id(5)] }]));
    r.configs.files.set(id(1), cfg('Old', 'home', 'coral'));
    r.configs.files.set(id(5), cfg('Other', 'home', 'amber'));
    await r.ws.refresh();
    await r.ws.act({
      action: 'workspace.edit',
      profile: 'home',
      name: ' New name ',
      colorVariant: 'teal',
    });
    expect(r.configs.files.get(id(1))).toMatchObject({ name: 'New name', ui: { variant: 'teal' } });
    expect(r.configs.files.get(id(5))?.name).toBe('Other');
    expect(r.changed).toHaveBeenCalled();
    expect(r.last()?.rows[0]).toMatchObject({ name: 'New name', colorVariant: 'teal' });
  });

  it('workspace.edit refuses a colour outside the palette and a row with no config', async () => {
    const r = rig(
      workspaceList('home', [{ profile: 'home', mogIds: [id(1)] }, { profile: 'new' }]),
    );
    r.configs.files.set(id(1), cfg('Old', 'home', 'coral'));
    await r.ws.refresh();
    await r.ws.act({ action: 'workspace.edit', profile: 'home', colorVariant: '#ff0000' });
    expect(r.configs.files.get(id(1))?.ui.variant).toBe('coral');
    await r.ws.act({ action: 'workspace.edit', profile: 'new', name: 'X' });
    expect(r.last()?.notice?.text).toBe(WORKSPACE_TEXT.needs_available);
    expect(r.configs.files.size).toBe(1);
  });

  it('workspace.cross and workspace.budget write the agent settings', async () => {
    const r = rig(workspaceList('home', [{ profile: 'home', mogIds: [id(1)] }]));
    await r.ws.refresh();
    await r.ws.act({ action: 'workspace.cross', crossWorkspaceData: true });
    await r.ws.act({ action: 'workspace.budget', genieCallBudget: 20 });
    expect(r.calls.filter((c) => c[0] === 'settings_write').map((c) => c[1])).toEqual([
      { crossWorkspaceData: true },
      { genieCallBudget: 20 },
    ]);
    expect(r.last()?.agent).toMatchObject({ crossWorkspaceData: true, genieCallBudget: 20 });
  });

  it('a store failure sends a fixed error notice, never the error text', async () => {
    const r = rig(
      workspaceList('home', [
        { profile: 'home', mogIds: [id(1)] },
        { profile: 'b', mogIds: [id(2)] },
      ]),
    );
    await r.ws.refresh();
    r.deps.setAvailable = async () =>
      Promise.reject({ error: 'workspace_storage_failed', reason: '/Users/x/secret' });
    await r.ws.act({ action: 'workspace.set_available', profile: 'b', available: true });
    expect(r.last()?.notice).toEqual({ kind: 'error', text: WORKSPACE_TEXT.failed });
  });
});

describe('workspaces: the hub wiring (ADR 0117)', () => {
  function wired() {
    const r = rig(
      workspaceList('home', [
        { profile: 'home', mogIds: [id(1)] },
        { profile: 'b', mogIds: [id(2)] },
      ]),
    );
    const bus = new BrainBus();
    wireWorkspaces(bus, r.ws);
    const input = (payload: unknown, from = 'hub', mogId = 'hub', kind = 'click') =>
      bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
    return { r, input };
  }

  it('each hub action is shape-checked: a bad shape reaches the store as nothing', async () => {
    const { r, input } = wired();
    await r.ws.refresh();
    const before = r.calls.length;
    const bad: unknown[] = [
      { action: 'workspace.set_available', profile: 'b' },
      { action: 'workspace.set_available', profile: '', available: true },
      { action: 'workspace.set_available', profile: 'b', available: 'yes' },
      { action: 'workspace.make_home', profile: 'b' },
      { action: 'workspace.make_home', profile: 'b', confirmed: false },
      { action: 'workspace.pick_home' },
      { action: 'workspace.sign_in', profile: 42 },
      { action: 'workspace.edit', profile: 'b' },
      { action: 'workspace.edit', profile: 'b', name: '   ' },
      { action: 'workspace.budget', genieCallBudget: 7 },
      { action: 'workspace.cross', crossWorkspaceData: 'on' },
      { action: 'workspace.delete', profile: 'b' },
      { action: 'mog.create', name: 'x', home: 'b' },
      'workspaces.ready',
      null,
    ];
    for (const p of bad) input(p);
    await flush();
    expect(r.calls.length).toBe(before);
  });

  it('[ARC-5] only the hub window may run an action, and only as a click with the hub scope', async () => {
    const { r, input } = wired();
    await r.ws.refresh();
    const before = r.calls.length;
    const ok = { action: 'workspace.set_available', profile: 'b', available: true };
    input(ok, `mog-${id(1)}`, id(1));
    input(ok, 'hub', 'hub', 'poked');
    input(ok, 'hub', id(1));
    await flush();
    expect(r.calls.length).toBe(before);
    input(ok);
    await vi.waitFor(() => expect(r.names()).toContain('set_available'));
  });

  it('a valid action of each kind runs', async () => {
    const { r, input } = wired();
    input({ action: 'workspaces.ready' });
    await vi.waitFor(() => expect(r.sent.length).toBe(1));
    expect(r.calls[0]).toEqual(['list', true]);
    input({ action: 'workspace.make_home', profile: 'b', confirmed: true });
    await vi.waitFor(() => expect(r.names()).toContain('make_home'));
  });
});

describe('workspaces: the first Connect (ADR 0117, FLAG 5)', () => {
  it('the first Connect with no home makes that profile home', async () => {
    const r = rig(workspaceList(null, [{ profile: 'bottle-new' }]));
    await r.ws.connected('bottle-new');
    const mk = r.calls.find((c) => c[0] === 'make_home')?.[1] as {
      profile: string;
      mogId?: string;
    };
    expect(mk.profile).toBe('bottle-new');
    // The new profile has no config: it gets one first.
    expect(mk.mogId).toBeDefined();
    expect(r.configs.files.get(mk.mogId as string)?.binding.home).toBe('bottle-new');
    expect(r.last()?.home).toBe('bottle-new');
    expect(r.changed).toHaveBeenCalled();
  });

  it('a later connection stays off: the home does not change, and nothing is switched on', async () => {
    const r = rig(
      workspaceList('home', [{ profile: 'home', mogIds: [id(1)] }, { profile: 'bottle-new' }]),
    );
    await r.ws.connected('bottle-new');
    expect(r.names()).not.toContain('make_home');
    expect(r.names()).not.toContain('set_available');
    expect(r.configs.files.size).toBe(0);
    expect(r.last()?.rows.find((x) => x.profile === 'bottle-new')).toMatchObject({
      available: false,
      isHome: false,
    });
    expect(r.last()?.home).toBe('home');
  });

  it('a home that waits for the pick (two candidates) is not overridden by a Connect', async () => {
    const r = rig(
      workspaceList(null, [
        { profile: 'a', mogIds: [id(1)], available: true },
        { profile: 'b', mogIds: [id(2)], available: true },
        { profile: 'bottle-new' },
      ]),
    );
    await r.ws.connected('bottle-new');
    expect(r.names()).not.toContain('make_home');
    expect(r.last()?.home).toBeNull();
    expect(r.last()?.homePick?.candidates).toEqual(['a', 'b']);
  });

  it('two old homes, one of them missing, then Connect C: make_home is not called', async () => {
    const r = rig(
      workspaceList(null, [
        { profile: 'a', mogIds: [id(1)], available: true },
        { profile: 'b', mogIds: [id(2)], available: true, signIn: 'missing' },
        { profile: 'c' },
      ]),
    );
    await r.ws.connected('c');
    expect(r.names()).not.toContain('make_home');
    expect(r.last()?.home).toBeNull();
    // Only one old home can be the home: the hub gets a notice, not a pick of one.
    expect(r.last()?.homePick).toBeUndefined();
    expect(r.last()?.notice).toEqual({ kind: 'info', text: WORKSPACE_TEXT.choose_home });
  });

  it('connected(a) with the old homes a and b does not call make_home', async () => {
    const r = rig(
      workspaceList(null, [
        { profile: 'a', mogIds: [id(1)], available: true },
        { profile: 'b', mogIds: [id(2)], available: true },
      ]),
    );
    await r.ws.connected('a');
    expect(r.names()).not.toContain('make_home');
    expect(r.last()?.homePick?.candidates).toEqual(['a', 'b']);
  });

  it('an unsupported profile never becomes home', async () => {
    const r = rig(workspaceList(null, [{ profile: 'pat', signIn: 'unsupported' }]));
    await r.ws.connected('pat');
    expect(r.names()).not.toContain('make_home');
  });
});

describe('workspaces: a failed list (ADR 0117)', () => {
  it('a failed list keeps the lamp (lowest config id) and tells the hub with a fixed notice', async () => {
    const r = rig(workspaceList('home', [{ profile: 'home', mogIds: [id(1)] }]));
    r.deps.list = async () =>
      Promise.reject({ error: 'workspace_storage_failed', reason: '/secret' });
    r.deps.fallbackLampMogId = async () => id(3);
    expect(await r.ws.lampMogId()).toBe(id(3));
    await r.ws.refresh();
    expect(r.last()).toMatchObject({
      rows: [],
      home: null,
      notice: { kind: 'error', text: WORKSPACE_TEXT.storage_failed },
    });
    expect(JSON.stringify(r.last())).not.toContain('secret');
  });
});

describe('workspaces: Settings notice and cache (ADR 0117)', () => {
  it('showSettings sends the list with no notice, then with the plain settings notice, on each open', async () => {
    const r = rig(workspaceList('home', [{ profile: 'home', mogIds: [id(1)] }]));
    await r.ws.showSettings();
    await r.ws.showSettings();
    expect(r.sent.map((l) => l.notice?.text)).toEqual([
      undefined,
      WORKSPACE_TEXT.settings,
      undefined,
      WORKSPACE_TEXT.settings,
    ]);
    expect(WORKSPACE_TEXT.settings).toBe('Your workspaces are in Settings → Workspace.');
  });

  it('the cache is read again only after a write or on workspaces.ready', async () => {
    const r = rig(workspaceList('home', [{ profile: 'home', mogIds: [id(1)] }]));
    await r.ws.refresh();
    const reads = () => r.calls.filter((c) => c[0] === 'list').length;
    const n = reads();
    await r.ws.home();
    await r.ws.lampMogId();
    await r.ws.dataMogId('home');
    await r.ws.homeIdentity(id(1)).catch(() => undefined);
    expect(reads()).toBe(n);
    await r.ws.act({ action: 'workspaces.ready' });
    expect(reads()).toBe(n + 1);
  });
});

describe('workspaces: data MOG, lamp and identity (ADR 0117)', () => {
  it('[AC-1.11] dataMogId is mogIds[0] of the profile, so a history row goes to that MOG', async () => {
    const r = rig(
      workspaceList('home', [
        { profile: 'home', mogIds: [id(1)] },
        { profile: 'b', mogIds: [id(7), id(8)], available: true },
        { profile: 'c' },
      ]),
    );
    expect(await r.ws.dataMogId('b')).toBe(id(7));
    expect(await r.ws.dataMogId('home')).toBe(id(1));
    expect(await r.ws.dataMogId('c')).toBeUndefined();
    expect(await r.ws.dataMogId('gone')).toBeUndefined();
  });

  it('the lamp shows the name and colour of the home workspace, with its profile as the tooltip home', async () => {
    core.reset();
    core.invoke.mockImplementation(async (cmd, args) => {
      if (cmd === 'config_read') {
        const who = (args as { args: { id: string } }).args.id;
        return {
          id: who,
          json: cfg(
            who === id(2) ? 'Lab' : 'Sales',
            who === id(2) ? 'lab' : 'work',
            who === id(2) ? 'teal' : 'coral',
          ),
        };
      }
      return {};
    });
    const r = rig(
      workspaceList(
        'lab',
        [
          { profile: 'work', mogIds: [id(1)] },
          { profile: 'lab', mogIds: [id(2)] },
        ],
        { lampMogId: id(1) },
      ),
    );
    expect(await r.ws.homeIdentity(id(1))).toEqual({
      name: 'Lab',
      colorVariant: 'teal',
      home: 'lab',
    });
    expect(await r.ws.lampMogId()).toBe(id(1));
  });

  it('a home with no config shows its profile name; with no home the lamp keeps its own identity', async () => {
    core.reset();
    core.invoke.mockImplementation(async (_c, args) => ({
      id: (args as { args: { id: string } }).args.id,
      json: cfg('Lamp', 'work', 'sky'),
    }));
    const noConfig = rig(workspaceList('new', [{ profile: 'new' }], { lampMogId: id(1) }));
    expect(await noConfig.ws.homeIdentity(id(1))).toEqual({ name: 'new', home: 'new' });
    const noHome = rig(
      workspaceList(null, [{ profile: 'a', mogIds: [id(1)], available: true }], {
        lampMogId: id(1),
      }),
    );
    expect(await noHome.ws.homeIdentity(id(1))).toMatchObject({
      name: 'Lamp',
      colorVariant: 'sky',
    });
  });

  it('a failed list read gives the last known home, and none before any read', async () => {
    const r = rig(workspaceList('home', [{ profile: 'home', mogIds: [id(1)] }]));
    r.deps.list = async () => Promise.reject({ error: 'host_failed' });
    expect(await r.ws.home()).toBeUndefined();
    const ok = rig(workspaceList('home', [{ profile: 'home', mogIds: [id(1)] }]));
    expect(await ok.ws.home()).toBe('home');
    ok.deps.list = async () => Promise.reject({ error: 'host_failed' });
    expect(await ok.ws.home()).toBe('home');
  });

  it('launch opens the lamp only: the lamp-only manager lists no other MOG', async () => {
    const base = {
      list: async () => ({
        mogs: [{ mogId: id(1) }, { mogId: id(2) }, { mogId: id(3) }] as never[],
        unreadable: [],
      }),
      start: vi.fn(async () => undefined),
      runState: () => 'stopped' as const,
    };
    const m = lampOnlyManager(base, async () => id(2));
    expect((await m.list()).mogs.map((x) => x.mogId)).toEqual([id(2)]);
    expect((await lampOnlyManager(base, async () => undefined).list()).mogs).toEqual([]);
  });
});
