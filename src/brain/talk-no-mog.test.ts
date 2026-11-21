import type { CaptureState } from '../shared/interfaces';
import type { HubNoticeLine } from '../shared/hub-mogs';
import { fakeWindows, memoryConfigs, sequentialIds } from '../test-support/mog-doubles';
import { BrainBus } from './bus';
import {
  createCaptureLifecycle,
  ipcCaptureControl,
  wireCaptureLifecycle,
} from './capture-lifecycle';
import { createMogManager, type MogManager } from './components/mog-manager';
import { NO_MOG_TEXT, wireTalkNoMog } from './talk-no-mog';
import { core } from './test-core';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const id = (n: number) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;

const flush = async () => {
  for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0));
};

/** The brain parts that a talk-key press goes through: T1, T12 (the gate) and the lifecycle. */
async function rig(opts: { mogs?: string[]; lastUsed?: string; accepted?: boolean } = {}) {
  core.reset();
  const bus = new BrainBus();
  const windows = fakeWindows();
  const manager: MogManager = createMogManager({
    configs: memoryConfigs(),
    windows,
    newId: sequentialIds(),
  });
  for (const name of opts.mogs ?? []) await manager.create({ name, home: 'p' });
  windows.calls.length = 0;
  const starts: unknown[] = [];
  const blocked: unknown[] = [];
  let lease = 0;
  const control = ipcCaptureControl({
    consent: {
      noticeState: async () =>
        ({ accepted: opts.accepted ?? true }) as Awaited<
          ReturnType<Parameters<typeof ipcCaptureControl>[0]['consent']['noticeState']>
        >,
    },
    onBlocked: (s) => void blocked.push(s),
    invoke: async (_name, args) => {
      starts.push(args);
      const state: CaptureState = {
        owner: args.mogId,
        mode: args.mode,
        leaseId: String(++lease),
        recording: true,
        paused: false,
        othersTrack: 'off',
      };
      return state;
    },
  });
  const lc = createCaptureLifecycle({
    control,
    transcribe: async () => '',
    onUtterance: () => undefined,
  });
  wireCaptureLifecycle(bus, lc, () => 0);
  const notices: (HubNoticeLine | undefined)[] = [];
  const order: string[] = [];
  wireTalkNoMog(bus, {
    showHub: () => void order.push('showHub'),
    manager,
    lastUsed: () => opts.lastUsed,
    openWindow: (mogId) => windows.openMog(mogId),
    notify: (n) => {
      order.push('notify');
      notices.push(n);
    },
  });
  const press = async (mogId = '', ...phases: string[]) => {
    for (const phase of phases.length ? phases : ['down', 'up'])
      bus.dispatch({ event: 'hotkey:ptt', payload: { mogId, phase } });
    await flush();
  };
  const stops = () =>
    core.invoke.mock.calls
      .filter(([cmd]) => cmd === 'capture_stop')
      .map(([, a]) => (a as { args: unknown }).args);
  return { bus, manager, windows, starts, blocked, notices, order, press, stops };
}

describe('talk key with no running MOG (#562)', () => {
  it('[MB-06a] a press with no running MOG starts the last-used MOG and captures into it', async () => {
    const r = await rig({ mogs: ['Ops', 'Sales'], lastUsed: id(2) });
    await r.press();
    // The hub Start path: `window_mog_open` for the last-used MOG, and T1 marks it running.
    expect(r.windows.calls).toEqual([['open', id(2)]]);
    expect(r.manager.runState(id(2))).toBe('running');
    expect(r.manager.runState(id(1))).toBe('stopped');
    // The press captures into it, through the T12 gate, and the release ends the lease.
    expect(r.starts).toEqual([{ mogId: id(2), mode: 'ptt' }]);
    expect(r.stops()).toEqual([{ mogId: id(2), leaseId: '1' }]);
    // The hub list is sent again (the MOG now shows as running), with no error text.
    expect(r.notices).toEqual([undefined]);
    // A start with no notice does not pull the hub up.
    expect(r.order).toEqual(['notify']);
  });

  it('[MB-06a] with no last-used MOG, a press starts the first MOG of the hub list', async () => {
    const r = await rig({ mogs: ['Sales', 'Ops'] });
    await r.press();
    // The hub list is sorted by name: Ops (id 2) is first.
    expect(r.windows.calls).toEqual([['open', id(2)]]);
    expect(r.starts).toEqual([{ mogId: id(2), mode: 'ptt' }]);
  });

  it('[MB-06a] a last-used MOG that was deleted falls back to the first MOG of the list', async () => {
    const r = await rig({ mogs: ['Ops'], lastUsed: id(9) });
    await r.press();
    expect(r.windows.calls).toEqual([['open', id(1)]]);
    expect(r.starts).toEqual([{ mogId: id(1), mode: 'ptt' }]);
  });

  it('[MB-06a] with no MOG at all, a press shows the hub toast and starts no capture', async () => {
    const r = await rig();
    await r.press();
    expect(r.notices).toEqual([{ kind: 'info', text: NO_MOG_TEXT }]);
    // #568: the hub window shows first (it may be closed), then the notice goes to it.
    expect(r.order).toEqual(['showHub', 'notify']);
    expect(NO_MOG_TEXT).toBe('Create a MOG to start');
    expect(r.windows.calls).toEqual([]);
    expect(r.starts).toEqual([]);
    expect(r.stops()).toEqual([]);
  });

  it('[#568] a hub that fails to show still gets the notice, and a throwing showHub is safe', async () => {
    const r = await rig();
    // A second wiring on a fresh bus whose showHub rejects.
    const bus = new BrainBus();
    const seen: (HubNoticeLine | undefined)[] = [];
    wireTalkNoMog(bus, {
      manager: r.manager,
      lastUsed: () => undefined,
      openWindow: async () => undefined,
      showHub: () => Promise.reject(new Error('no hub')),
      notify: (n) => void seen.push(n),
    });
    bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: '', phase: 'down' } });
    await flush();
    expect(seen).toEqual([{ kind: 'info', text: NO_MOG_TEXT }]);
  });

  it('[MB-06a] a running MOG: the press is the same as today', async () => {
    const r = await rig({ mogs: ['Ops', 'Sales'], lastUsed: id(2) });
    await r.manager.start(id(1));
    r.windows.calls.length = 0;
    await r.press(id(1));
    expect(r.windows.calls).toEqual([]);
    expect(r.notices).toEqual([]);
    expect(r.starts).toEqual([{ mogId: id(1), mode: 'ptt' }]);
    expect(r.stops()).toEqual([{ mogId: id(1), leaseId: '1' }]);
  });

  it('[MB-06a] [SEC-32a] the T12 consent gate still applies to the started MOG', async () => {
    const r = await rig({ mogs: ['Ops'], accepted: false });
    await r.press();
    // The MOG starts (the key press is the user action), but no capture starts without consent.
    expect(r.windows.calls).toEqual([['open', id(1)]]);
    expect(r.starts).toEqual([]);
    expect(r.blocked).toHaveLength(1);
  });

  it('[MB-06a] [CAP-14a] the up, cancel and double tap of the press go to the started MOG', async () => {
    const r = await rig({ mogs: ['Ops'] });
    // A tap, then the double tap: hands-free in the started MOG (CAP-14a).
    await r.press('', 'down', 'cancel', 'double_tap');
    expect(r.windows.calls).toEqual([['open', id(1)]]);
    expect(r.starts).toEqual([
      { mogId: id(1), mode: 'ptt' },
      { mogId: id(1), mode: 'hands_free' },
    ]);
  });

  it('[MB-06a] [RTR-17a] an ⌥G key-up of the press keeps ask: true for the started MOG', async () => {
    const r = await rig({ mogs: ['Ops'] });
    const seen: unknown[] = [];
    r.bus.on('hotkey:ptt', (p) => void seen.push(p));
    r.bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: '', phase: 'down', shift: true } });
    r.bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: '', phase: 'up', ask: true } });
    await flush();
    const relayed = seen.filter((p) => (p as { mogId: string }).mogId === id(1));
    expect(relayed).toEqual([
      { mogId: id(1), phase: 'down' },
      { mogId: id(1), phase: 'up', ask: true },
    ]);
  });

  it('[MB-06a] a MOG that T1 thinks is running but whose window is gone opens again before capture (#419)', async () => {
    const r = await rig({ mogs: ['Ops'] });
    await r.manager.start(id(1));
    // The window closed outside T1 (#419): T1 still says running.
    r.windows.open.delete(id(1));
    r.windows.calls.length = 0;
    await r.press();
    expect(r.manager.runState(id(1))).toBe('running');
    expect(r.windows.calls).toEqual([['open', id(1)]]);
    expect(r.windows.open.has(id(1))).toBe(true);
    expect(r.starts).toEqual([{ mogId: id(1), mode: 'ptt' }]);
  });

  it('[MB-06a] a refused start shows its hub text and starts no capture', async () => {
    const r = await rig({ mogs: ['Ops'] });
    r.windows.openMog = async () => Promise.reject({ error: 'host_failed' });
    await r.press();
    expect(r.notices).toEqual([{ kind: 'error', text: 'Bottle could not start Ops.' }]);
    expect(r.order).toEqual(['showHub', 'notify']);
    expect(r.starts).toEqual([]);
  });

  it('[MB-06a] nothing starts without a key press: only a down starts a MOG', async () => {
    const r = await rig({ mogs: ['Ops'] });
    await r.press('', 'up', 'cancel', 'double_tap');
    expect(r.windows.calls).toEqual([]);
    expect(r.starts).toEqual([]);
    expect(r.notices).toEqual([]);
  });
});
