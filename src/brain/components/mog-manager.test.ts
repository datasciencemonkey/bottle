import { core } from '../test-core';
import {
  fakeWindows,
  memoryConfigs,
  sequentialIds,
  type FakeWindows,
  type MemoryConfigs,
} from '../../test-support/mog-doubles';
import { MOG_CONFIG_DEFAULTS } from './mog-config';
import {
  createMogManager,
  ipcMogConfigClient,
  ipcMogWindowClient,
  MAX_RUNNING_MOGS_DEFAULT,
  MogManagerError,
  type MogManager,
} from './mog-manager';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);

const id = (n: number) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;

let configs: MemoryConfigs;
let windows: FakeWindows;
let m: MogManager;

function manager(opts: { maxRunningMogs?: number; windows?: FakeWindows } = {}): MogManager {
  return createMogManager({
    configs,
    windows: opts.windows ?? windows,
    newId: sequentialIds(),
    ...(opts.maxRunningMogs === undefined ? {} : { maxRunningMogs: opts.maxRunningMogs }),
  });
}

async function rejection(p: Promise<unknown>): Promise<MogManagerError> {
  const e = await p.then(
    () => undefined,
    (x: unknown) => x,
  );
  expect(e).toBeInstanceOf(MogManagerError);
  return e as MogManagerError;
}

beforeEach(() => {
  configs = memoryConfigs();
  windows = fakeWindows();
  m = manager();
  core.reset();
});

describe('T1 MogManager: the six hub actions (MB-01)', () => {
  it('[MB-01] create saves a config with the Appendix F defaults and a new UUID', async () => {
    const s = await m.create({ name: 'Sales', home: 'sales-ws' });
    expect(s).toMatchObject({ mogId: id(1), name: 'Sales', runState: 'stopped' });
    expect(configs.files.get(id(1))).toEqual({
      ...MOG_CONFIG_DEFAULTS,
      name: 'Sales',
      binding: {
        home: 'sales-ws',
        backend: { kind: 'genie_one_mcp', profile: 'sales-ws' },
        genieCompanion: true,
        profileAliases: {},
      },
    });
  });

  it('[MB-01] create with no name uses the lowest free "MOG <n>" (Appendix F)', async () => {
    await m.create({ home: 'p' });
    await m.create({ name: 'MOG 3', home: 'p' });
    await m.create({ name: '  ', home: 'p' });
    const names = (await m.list()).mogs.map((s) => s.name);
    expect(names).toEqual(['MOG 1', 'MOG 2', 'MOG 3']);
  });

  it('[MB-01] create refuses a long name or no home profile and saves nothing', async () => {
    const long = await rejection(m.create({ name: 'x'.repeat(65), home: 'p' }));
    expect([long.code, long.message]).toEqual([
      'invalid_name',
      'A MOG name must have 1 to 64 characters.',
    ]);
    const home = await rejection(m.create({ name: 'A', home: ' ' }));
    expect([home.code, home.message]).toEqual([
      'invalid_home',
      'Choose a home profile for the MOG.',
    ]);
    expect(configs.files.size).toBe(0);
  });

  it('[MB-01] edit changes the name, the home profile and the persona and keeps the rest', async () => {
    await m.create({ name: 'Sales', home: 'old' });
    const before = configs.files.get(id(1));
    const s = await m.edit(id(1), { name: 'Sales EU', home: 'new', persona: 'Be brief.' });
    expect(s).toMatchObject({ name: 'Sales EU', persona: 'Be brief.' });
    expect(s.binding.home).toBe('new');
    expect(configs.files.get(id(1))).toEqual({
      ...before,
      name: 'Sales EU',
      persona: 'Be brief.',
      binding: { ...before?.binding, home: 'new', backend: { kind: 'genie_one_mcp', profile: 'new' } },
    });
  });

  it('[MB-01] edit refuses persona instructions over 2,000 characters', async () => {
    await m.create({ name: 'Sales', home: 'p' });
    const e = await rejection(m.edit(id(1), { persona: 'x'.repeat(2001) }));
    expect([e.code, e.message]).toEqual([
      'invalid_persona',
      'Persona instructions can have at most 2,000 characters.',
    ]);
    expect(configs.files.get(id(1))?.persona).toBe('');
  });

  it('[MB-01] edit of a MOG that is gone says so', async () => {
    const e = await rejection(m.edit(id(9), { name: 'X' }));
    expect([e.code, e.message]).toEqual(['not_found', 'This MOG is not in the list any more.']);
  });

  it('[MB-01] duplicate copies the config to a new ID as "<name> copy", without the chord', async () => {
    await m.create({ name: 'Sales', home: 'p' });
    const src = configs.files.get(id(1));
    if (!src) throw new Error('no config');
    configs.files.set(id(1), { ...src, hotkeys: { chord: 3 }, persona: 'P' });
    const s = await m.duplicate(id(1));
    expect(s).toMatchObject({ mogId: id(2), name: 'Sales copy', runState: 'stopped' });
    expect(configs.files.get(id(2))).toEqual({
      ...src,
      name: 'Sales copy',
      persona: 'P',
      hotkeys: {},
    });
  });

  it('[MB-01] duplicate keeps the copy name at 64 characters', async () => {
    await m.create({ name: 'n'.repeat(64), home: 'p' });
    const s = await m.duplicate(id(1));
    expect(s.name).toBe(`${'n'.repeat(59)} copy`);
    expect([...s.name]).toHaveLength(64);
  });

  it('[MB-01] start opens the mog-<id> window, stop closes it, and both are idempotent', async () => {
    await m.create({ name: 'Sales', home: 'p' });
    await m.start(id(1));
    await m.start(id(1));
    expect(m.runState(id(1))).toBe('running');
    expect(windows.calls).toEqual([['open', id(1)]]);
    await m.stop(id(1));
    await m.stop(id(1));
    expect(m.runState(id(1))).toBe('stopped');
    expect(windows.calls).toEqual([
      ['open', id(1)],
      ['close', id(1)],
    ]);
  });

  it('[MB-01] start of a MOG that is not defined opens no window', async () => {
    const e = await rejection(m.start(id(5)));
    expect(e.code).toBe('not_found');
    expect(windows.calls).toEqual([]);
  });

  it('[MB-01] a window refusal on start leaves the MOG stopped with a content-free text', async () => {
    await m.create({ name: 'Sales', home: 'p' });
    const broken = {
      ...windows,
      openMog: () => Promise.reject({ error: 'host_failed', detail: '/Users/someone' }),
    };
    const e = await rejection(manager({ windows: broken }).start(id(1)));
    expect([e.code, e.message]).toEqual(['start_failed', 'Bottle could not start Sales.']);
    expect(e.message).not.toContain('someone');
  });

  it('[MB-01] stop of a window that is gone already counts as stopped', async () => {
    await m.create({ name: 'Sales', home: 'p' });
    await m.start(id(1));
    windows.open.clear();
    await m.stop(id(1));
    expect(m.runState(id(1))).toBe('stopped');
  });

  it('[MB-01] delete needs a confirm and then removes only the config file', async () => {
    await m.create({ name: 'Sales', home: 'p' });
    const e = await rejection(m.remove(id(1), { confirmed: false as unknown as true }));
    expect(e.code).toBe('not_confirmed');
    expect(configs.files.has(id(1))).toBe(true);
    await m.remove(id(1), { confirmed: true });
    expect(configs.files.has(id(1))).toBe(false);
    expect(configs.calls.filter(([op]) => op === 'delete')).toEqual([['delete', id(1)]]);
  });

  it('[MB-01] delete of a running MOG stops it first', async () => {
    await m.create({ name: 'Sales', home: 'p' });
    await m.start(id(1));
    await m.remove(id(1), { confirmed: true });
    expect(windows.open.size).toBe(0);
    expect(m.runState(id(1))).toBe('stopped');
    expect((await m.list()).mogs).toEqual([]);
  });

  it('[MB-01] onChange tells listeners after each action, for #98 and #50', async () => {
    const seen: number[] = [];
    const off = m.onChange(() => seen.push(m.runningCount()));
    await m.create({ name: 'Sales', home: 'p' });
    await m.start(id(1));
    off();
    await m.stop(id(1));
    expect(seen).toEqual([0, 1]);
  });

  it('[MB-02] a secret refusal of the store becomes a fixed text with no value in it', async () => {
    const store = { ...configs, write: () => Promise.reject({ error: 'token_like_value' }) };
    const mm = createMogManager({ configs: store, windows, newId: sequentialIds() });
    const e = await rejection(mm.create({ name: 'Sales', home: 'p' }));
    expect([e.code, e.message]).toEqual([
      'secret_rejected',
      'A MOG config cannot hold a token, password, key or secret.',
    ]);
  });
});

describe('T1 MogManager: any number of MOGs (MB-03a)', () => {
  it('[MB-03a] [AT-01] 20 defined MOGs save and list after a restart', async () => {
    for (let i = 1; i <= 20; i++) await m.create({ name: `MOG ${i}`, home: `ws-${i}` });
    const after = await manager().list();
    expect(after.mogs).toHaveLength(20);
    expect(after.mogs.map((s) => s.name).sort()).toEqual(
      Array.from({ length: 20 }, (_, i) => `MOG ${i + 1}`).sort(),
    );
    expect(after.mogs.every((s) => s.runState === 'stopped')).toBe(true);
  });

  it('[MB-03a] list keeps unreadable configs as IDs and deletes nothing', async () => {
    await m.create({ name: 'Sales', home: 'p' });
    configs.unreadable.push('bad-file');
    const l = await m.list();
    expect(l.unreadable).toEqual(['bad-file']);
    expect(l.mogs.map((s) => s.name)).toEqual(['Sales']);
    expect(configs.calls.some(([op]) => op === 'delete')).toBe(false);
  });

  it('[MB-03a] the store limit gives a fixed text', async () => {
    configs = memoryConfigs(1);
    m = manager();
    await m.create({ name: 'A', home: 'p' });
    const e = await rejection(m.create({ name: 'B', home: 'p' }));
    expect([e.code, e.message]).toEqual([
      'limit_reached',
      'Bottle cannot keep more MOGs. Delete one to add a new MOG.',
    ]);
  });
});

describe('T1 MogManager: the running cap (MB-03b)', () => {
  it('[MB-03b] [AT-01] the 9th start shows "8 MOGs are running. Stop one to start <name>."', async () => {
    expect(MAX_RUNNING_MOGS_DEFAULT).toBe(8);
    for (let i = 1; i <= 9; i++) await m.create({ name: `M${i}`, home: 'p' });
    for (let i = 1; i <= 8; i++) await m.start(id(i));
    await m.create({ name: 'Ninth', home: 'p' });
    const e = await rejection(m.start(id(10)));
    expect([e.code, e.message]).toEqual([
      'cap_reached',
      '8 MOGs are running. Stop one to start Ninth.',
    ]);
    expect(windows.calls.filter(([op]) => op === 'open')).toHaveLength(8);
    expect(m.runningCount()).toBe(8);
    await m.stop(id(1));
    await m.start(id(10));
    expect(m.runState(id(10))).toBe('running');
  });

  it('[MB-03b] the cap follows app.maxRunningMogs and stays within the host cap of 8', async () => {
    const three = manager({ maxRunningMogs: 3 });
    expect(three.maxRunning).toBe(3);
    for (let i = 1; i <= 4; i++) await three.create({ name: `M${i}`, home: 'p' });
    for (let i = 1; i <= 3; i++) await three.start(id(i));
    const e = await rejection(three.start(id(4)));
    expect(e.message).toBe('3 MOGs are running. Stop one to start M4.');
    expect(manager({ maxRunningMogs: 12 }).maxRunning).toBe(8);
    expect(manager({ maxRunningMogs: 0 }).maxRunning).toBe(8);
    expect(manager({ maxRunningMogs: 2.5 }).maxRunning).toBe(8);
  });

  it('[MB-03b] a mog_cap_reached refusal of the Rust host gives the same text', async () => {
    const host = fakeWindows(1);
    const mm = manager({ windows: host });
    await mm.create({ name: 'A', home: 'p' });
    await mm.create({ name: 'B', home: 'p' });
    host.open.add('other');
    const e = await rejection(mm.start(id(1)));
    expect([e.code, e.message]).toEqual([
      'cap_reached',
      '8 MOGs are running. Stop one to start A.',
    ]);
    expect(mm.runState(id(1))).toBe('stopped');
  });

  it('[MB-03b] starts at the same time never pass the cap', async () => {
    const two = manager({ maxRunningMogs: 2 });
    for (let i = 1; i <= 3; i++) await two.create({ name: `M${i}`, home: 'p' });
    const results = await Promise.allSettled([1, 2, 3].map((i) => two.start(id(i))));
    expect(results.filter((r) => r.status === 'fulfilled')).toHaveLength(2);
    expect(two.runningCount()).toBe(2);
  });
});

describe('T1 MogManager IPC seams (config_* and window_mog_*)', () => {
  it('[MB-01] the config client calls config_list, config_read, config_write and config_delete', async () => {
    const cfg = { ...MOG_CONFIG_DEFAULTS, name: 'A', binding: {} };
    core.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'config_list') return { items: [{ id: 'a', name: 'A' }], unreadable: ['b'] };
      if (cmd === 'config_delete') return { deleted: true };
      return { id: 'a', json: cfg };
    });
    const c = ipcMogConfigClient();
    expect(await c.list()).toEqual({ items: [{ id: 'a', name: 'A' }], unreadable: ['b'] });
    expect(await c.read('a')).toEqual(cfg);
    expect(await c.write('a', cfg as never)).toEqual(cfg);
    expect(await c.remove('a')).toBe(true);
    expect(core.invoke.mock.calls).toEqual([
      ['config_list', { args: {} }],
      ['config_read', { args: { id: 'a' } }],
      ['config_write', { args: { id: 'a', json: cfg } }],
      ['config_delete', { args: { id: 'a' } }],
    ]);
  });

  it('[MB-01] the config client refuses a reply it cannot read', async () => {
    core.invoke.mockImplementation(async () => ({ nope: 1 }));
    const c = ipcMogConfigClient();
    await expect(c.list()).rejects.toEqual({ error: 'bad_reply' });
    await expect(c.read('a')).rejects.toEqual({ error: 'bad_reply' });
  });

  it('[MB-01] the window client calls window_mog_open and window_mog_close with the MOG ID', async () => {
    const w = ipcMogWindowClient();
    await w.openMog('a');
    await w.closeMog('a');
    expect(core.invoke.mock.calls).toEqual([
      ['window_mog_open', { args: { mogId: 'a' } }],
      ['window_mog_close', { args: { mogId: 'a' } }],
    ]);
  });
});
