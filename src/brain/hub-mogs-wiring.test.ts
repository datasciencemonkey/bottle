import { core } from './test-core';
import { fakeWindows, memoryConfigs, sequentialIds } from '../test-support/mog-doubles';
import { BrainBus } from './bus';
import { createMogManager, type MogManager } from './components/mog-manager';
import { sendHubMogs, sendHubSnapshot, wireHubMogs } from './hub-mogs-wiring';
import type { HubMogList } from '../shared/hub-mogs';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const id = (n: number) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;

let bus: BrainBus;
let m: MogManager;
let sent: HubMogList[];
const send = vi.fn(async (l: HubMogList) => {
  sent.push(l);
});

function input(payload: unknown, from = 'hub', mogId = 'hub', kind = 'click'): void {
  bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
}

const last = () => sent.at(-1);

beforeEach(() => {
  core.reset();
  bus = new BrainBus();
  m = createMogManager({
    configs: memoryConfigs(),
    windows: fakeWindows(),
    newId: sequentialIds(),
    maxRunningMogs: 1,
  });
  sent = [];
  send.mockClear();
});

describe('hub MOG wiring (#26)', () => {
  it('[MB-01] [MB-06a] sendHubSnapshot sends the list with a notice outside a hub action (#562)', async () => {
    await m.create({ name: 'Ops', home: 'p' });
    await sendHubSnapshot(m, { kind: 'info', text: 'Create a MOG to start' }, send);
    expect(last()).toEqual({
      mogs: [
        {
          mogId: id(1),
          name: 'Ops',
          home: 'p',
          persona: '',
          running: false,
          colorVariant: 'coral',
        },
      ],
      unreadable: [],
      runningCount: 0,
      maxRunning: 1,
      notice: { kind: 'info', text: 'Create a MOG to start' },
    });
    // No hub is open: the send fails quietly.
    await expect(
      sendHubSnapshot(m, undefined, async () => Promise.reject(new Error('x'))),
    ).resolves.toBe(false);
  });

  it('[MB-01] sends the list to the hub once at start and again on mogs.ready', async () => {
    wireHubMogs(bus, m, send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(last()).toEqual({ mogs: [], unreadable: [], runningCount: 0, maxRunning: 1 });
    input({ action: 'mogs.ready' });
    await vi.waitFor(() => expect(sent).toHaveLength(2));
  });

  it('[MB-01] runs create, edit, duplicate, start, stop and delete from the hub', async () => {
    wireHubMogs(bus, m, send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    const step = async (payload: unknown) => {
      const n = sent.length;
      input(payload);
      await vi.waitFor(() => expect(sent.length).toBeGreaterThan(n));
      return last();
    };
    expect((await step({ action: 'mog.create', name: 'Sales', home: 'p' }))?.mogs).toEqual([
      {
        mogId: id(1),
        name: 'Sales',
        home: 'p',
        persona: '',
        running: false,
        colorVariant: 'coral',
      },
    ]);
    await step({ action: 'mog.edit', mogId: id(1), name: 'Ops', home: 'q', persona: 'Hi' });
    expect(last()?.mogs[0]).toMatchObject({ name: 'Ops', home: 'q', persona: 'Hi' });
    expect((await step({ action: 'mog.duplicate', mogId: id(1) }))?.mogs).toHaveLength(2);
    expect((await step({ action: 'mog.start', mogId: id(1) }))?.runningCount).toBe(1);
    expect((await step({ action: 'mog.stop', mogId: id(1) }))?.runningCount).toBe(0);
    // #ui-4: the row swatch picker.
    const recoloured = await step({ action: 'mog.set_color', mogId: id(1), colorVariant: 'teal' });
    expect(recoloured?.mogs.find((r) => r.mogId === id(1))?.colorVariant).toBe('teal');
    const del = await step({ action: 'mog.delete', mogId: id(2), confirmed: true });
    expect(del?.mogs.map((r) => r.name)).toEqual(['Ops']);
    expect(del?.notice).toEqual({
      kind: 'info',
      text: 'Deleted Ops copy. Its memory and history stay until you use Wipe in Settings.',
    });
  });

  it('[MB-03b] a start over the cap sends the exact cap text as an error notice', async () => {
    wireHubMogs(bus, m, send);
    await m.create({ name: 'A', home: 'p' });
    await m.create({ name: 'B', home: 'p' });
    await m.start(id(1));
    const n = sent.length;
    input({ action: 'mog.start', mogId: id(2) });
    await vi.waitFor(() => expect(sent.length).toBeGreaterThan(n));
    expect(last()?.notice).toEqual({
      kind: 'error',
      text: '1 MOG is running. Stop it to start B.',
    });
    expect(last()?.runningCount).toBe(1);
  });

  it('[MB-01] [ARC-5] ignores MOG actions from a MOG or card window, other kinds and bad payloads', async () => {
    wireHubMogs(bus, m, send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    const create = { action: 'mog.create', name: 'X', home: 'p' };
    input(create, `mog-${id(1)}`, id(1));
    input(create, 'hub', 'hub', 'poked');
    input(create, 'hub', id(1));
    input({ action: 'mog.delete', mogId: id(1) });
    input({ action: 'nope' });
    await new Promise((r) => setTimeout(r, 20));
    expect(sent).toHaveLength(1);
    expect((await m.list()).mogs).toEqual([]);
  });

  it('[MB-01] a list failure sends an empty list with a fixed error text', async () => {
    const broken = createMogManager({
      configs: { ...memoryConfigs(), list: () => Promise.reject({ error: 'not_implemented' }) },
      windows: fakeWindows(),
    });
    wireHubMogs(bus, broken, send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(last()).toEqual({
      mogs: [],
      unreadable: [],
      runningCount: 0,
      maxRunning: 8,
      notice: { kind: 'error', text: 'Bottle cannot read the MOG list.' },
    });
  });

  it('[ARC-5] sendHubMogs emits hub:mogs to the hub window only', async () => {
    const l: HubMogList = { mogs: [], unreadable: [], runningCount: 0, maxRunning: 8 };
    await sendHubMogs(l);
    expect(core.emitTo.mock.calls).toEqual([['hub', 'hub:mogs', l]]);
  });

  it('[MB-01] the returned function stops the wiring', async () => {
    const off = wireHubMogs(bus, m, send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    off();
    input({ action: 'mogs.ready' });
    await new Promise((r) => setTimeout(r, 20));
    expect(sent).toHaveLength(1);
  });

  it('[ui-4] [#603] mog.edit and mog.set_color tell onIdentityChanged, so the open MOG and card can pick it up', async () => {
    const changed: string[] = [];
    wireHubMogs(bus, m, send, (mogId) => changed.push(mogId));
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    input({ action: 'mog.create', name: 'Ops', home: 'p' });
    await vi.waitFor(() => expect(sent).toHaveLength(2));
    input({ action: 'mog.edit', mogId: id(1), name: 'Ops EU', home: 'p', persona: '' });
    await vi.waitFor(() => expect(changed).toEqual([id(1)]));
    input({ action: 'mog.set_color', mogId: id(1), colorVariant: 'teal' });
    await vi.waitFor(() => expect(changed).toEqual([id(1), id(1)]));
    // Actions with no identity change never call it.
    input({ action: 'mog.stop', mogId: id(1) });
    await new Promise((r) => setTimeout(r, 20));
    expect(changed).toEqual([id(1), id(1)]);
  });

  it('[ui-4] [#603] openEdit ("Change home workspace…") asks the hub to open one MOG\'s Edit form, once', async () => {
    const wired = wireHubMogs(bus, m, send);
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    wired.openEdit(id(1));
    await vi.waitFor(() => expect(sent).toHaveLength(2));
    expect(last()?.openEdit).toEqual({ mogId: id(1), token: 1 });
    // A later, unrelated push (mogs.ready) never repeats the ask: it was consumed.
    input({ action: 'mogs.ready' });
    await vi.waitFor(() => expect(sent).toHaveLength(3));
    expect(last()?.openEdit).toBeUndefined();
  });
});
