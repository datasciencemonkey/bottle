/** ADR 0101 (#552, CAP-36, CAP-37): the brain side of the OS permission notice. */
import type { PermissionKind, PermissionState } from '../shared/ipc';
import {
  PERMISSION_ALLOW_TARGET,
  PERMISSION_CHECK_TARGET,
  PERMISSION_TEXT,
  PERMISSION_VIEW_ID,
  type PermissionModel,
} from '../shared/permission-view';
import type { Mock } from 'vitest';
import { BrainBus } from './bus';
import { wirePermissions, type PermissionWiringDeps } from './permission-wiring';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const MOG = '0b0e8a52-6f7e-4a57-9a43-2f3b6f8f4c11';
const OTHER = '6f1d2b4e-0c55-4c1e-8f4f-5a7d0f4b2a90';
const flush = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};

function input(bus: BrainBus, from: string, mogId: string, kind: string, payload: unknown) {
  bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
}
const hubClick = (bus: BrainBus, payload: unknown) =>
  input(bus, 'hub', PERMISSION_VIEW_ID, 'click', payload);
const mogReady = (bus: BrainBus, mogId = MOG) =>
  input(bus, `mog-${mogId}`, mogId, 'click', { action: 'view.ready' });

interface Stack {
  add: Mock<(b: { id: string; text: string; completed?: boolean }) => void>;
  setControls: Mock<
    (
      owner: string,
      controls: { id: string; label: string }[],
      onClick?: (id: string) => void,
    ) => void
  >;
  clearControls: Mock<(owner: string) => void>;
}

function setup(start: Partial<Record<PermissionKind, PermissionState>> = {}) {
  const os: Record<PermissionKind, PermissionState> = {
    input_monitoring: 'granted',
    accessibility: 'granted',
    ...start,
  };
  const stacks = new Map<string, Stack>();
  const hub: PermissionModel[] = [];
  const deps = {
    status: vi.fn(async (p: PermissionKind) => os[p]),
    request: vi.fn(async (p: PermissionKind) => os[p]),
    sendHub: vi.fn(async (m: PermissionModel) => void hub.push(m)),
    stackFor: (mogId: string) => {
      let s = stacks.get(mogId);
      if (!s)
        stacks.set(mogId, (s = { add: vi.fn(), setControls: vi.fn(), clearControls: vi.fn() }));
      return s;
    },
    showCard: vi.fn(async () => undefined),
    hideCard: vi.fn(async () => undefined),
  } satisfies PermissionWiringDeps;
  const bus = new BrainBus();
  wirePermissions(bus, deps);
  const lastControl = (mogId = MOG) => {
    const calls = stacks.get(mogId)?.setControls.mock.calls ?? [];
    return calls[calls.length - 1] as [
      string,
      { id: string; label: string }[],
      (id: string) => void,
    ];
  };
  return { os, bus, deps, hub, stacks, lastControl };
}

function stack(t: { stacks: Map<string, Stack> }, mogId = MOG): Stack {
  const s = t.stacks.get(mogId);
  if (!s) throw new Error('no card stack');
  return s;
}

describe('OS permission notice (ADR 0101, #552)', () => {
  it('[CAP-36] at launch the hub gets the missing permissions with no prompt', async () => {
    const t = setup({ input_monitoring: 'unknown', accessibility: 'unknown' });
    await flush();
    expect(t.hub.at(-1)).toEqual({
      needs: [
        { permission: 'input_monitoring', action: 'allow' },
        { permission: 'accessibility', action: 'allow' },
      ],
    });
    expect(t.deps.request).not.toHaveBeenCalled();
  });

  it('[CAP-36] a denied permission offers Settings, because the OS shows no new prompt', async () => {
    const t = setup({ input_monitoring: 'denied' });
    await flush();
    expect(t.hub.at(-1)).toEqual({
      needs: [{ permission: 'input_monitoring', action: 'settings' }],
    });
  });

  it('with every permission granted, or a failed read, nothing shows', async () => {
    const t = setup();
    t.deps.status.mockImplementation(async (p: PermissionKind) => {
      if (p === 'accessibility') throw new Error('host_failed');
      return 'granted';
    });
    hubClick(t.bus, { target: PERMISSION_CHECK_TARGET });
    mogReady(t.bus);
    await flush();
    expect(t.hub.at(-1)).toEqual({ needs: [] });
    expect(t.deps.showCard).not.toHaveBeenCalled();
  });

  it('[CAP-36] the hub open asks again and gets the model again', async () => {
    const t = setup({ accessibility: 'unknown' });
    await flush();
    const n = t.hub.length;
    hubClick(t.bus, { target: PERMISSION_CHECK_TARGET });
    await flush();
    expect(t.hub.length).toBe(n + 1);
    expect(t.hub.at(-1)).toEqual({ needs: [{ permission: 'accessibility', action: 'allow' }] });
  });

  it('[CAP-36] [CAP-37] only a hub click on Allow asks the OS, and only for that permission', async () => {
    const t = setup({ input_monitoring: 'unknown', accessibility: 'unknown' });
    await flush();
    // Input from another window or with a bad payload asks nothing.
    input(t.bus, `card-${MOG}`, PERMISSION_VIEW_ID, 'click', {
      target: PERMISSION_ALLOW_TARGET,
      permission: 'input_monitoring',
    });
    hubClick(t.bus, { target: PERMISSION_ALLOW_TARGET, permission: 'camera' });
    hubClick(t.bus, { target: PERMISSION_ALLOW_TARGET });
    input(t.bus, 'hub', PERMISSION_VIEW_ID, 'hover', {
      target: PERMISSION_ALLOW_TARGET,
      permission: 'input_monitoring',
    });
    await flush();
    expect(t.deps.request).not.toHaveBeenCalled();
    hubClick(t.bus, { target: PERMISSION_ALLOW_TARGET, permission: 'accessibility' });
    await flush();
    expect(t.deps.request.mock.calls).toEqual([['accessibility']]);
    // The prompt showed once: the next click opens Settings.
    expect(t.hub.at(-1)).toEqual({
      needs: [
        { permission: 'input_monitoring', action: 'allow' },
        { permission: 'accessibility', action: 'settings' },
      ],
    });
  });

  it('[CAP-36] a failed request keeps Allow, because the host showed no prompt', async () => {
    const t = setup({ input_monitoring: 'unknown' });
    await flush();
    t.deps.request.mockImplementation(async () => {
      throw new Error('host_failed');
    });
    hubClick(t.bus, { target: PERMISSION_ALLOW_TARGET, permission: 'input_monitoring' });
    await flush();
    expect(t.hub.at(-1)).toEqual({ needs: [{ permission: 'input_monitoring', action: 'allow' }] });
  });

  it('[CAP-37] a grant takes the notice away', async () => {
    const t = setup({ accessibility: 'unknown' });
    await flush();
    t.deps.request.mockImplementation(async (p: PermissionKind) => (t.os[p] = 'granted'));
    hubClick(t.bus, { target: PERMISSION_ALLOW_TARGET, permission: 'accessibility' });
    await flush();
    expect(t.hub.at(-1)).toEqual({ needs: [] });
  });

  it('[CAP-36] the first MOG that opens shows the pop-up with one control; the others do not', async () => {
    const t = setup({ input_monitoring: 'unknown', accessibility: 'unknown' });
    mogReady(t.bus);
    mogReady(t.bus, OTHER);
    await flush();
    const s = stack(t);
    expect(s.add).toHaveBeenLastCalledWith({
      id: 'perm:input_monitoring',
      text: PERMISSION_TEXT.input_monitoring.need,
      completed: true,
    });
    const [owner, controls] = t.lastControl();
    expect(owner).toBe('perm:input_monitoring');
    expect(controls).toEqual([{ id: 'perm:input_monitoring', label: 'Allow' }]);
    expect(t.deps.showCard.mock.calls).toEqual([[MOG]]);
    expect(t.stacks.has(OTHER)).toBe(false);
    expect(t.deps.request).not.toHaveBeenCalled();
  });

  it('[CAP-36] a click on the pop-up control asks the OS; then it says to reopen Bottle', async () => {
    const t = setup({ input_monitoring: 'unknown' });
    mogReady(t.bus);
    await flush();
    t.lastControl()[2]('perm:input_monitoring');
    // A second click while the first one waits asks nothing more.
    t.lastControl()[2]('perm:input_monitoring');
    await flush();
    expect(t.deps.request.mock.calls).toEqual([['input_monitoring']]);
    expect(stack(t).add).toHaveBeenLastCalledWith({
      id: 'perm:input_monitoring',
      text: PERMISSION_TEXT.input_monitoring.settings,
      completed: true,
    });
    expect(t.lastControl()[1]).toEqual([{ id: 'perm:input_monitoring', label: 'Open Settings' }]);
    expect(PERMISSION_TEXT.input_monitoring.settings).toMatch(/reopen Bottle/);
  });

  it('[CAP-37] after Input Monitoring, the pop-up moves to Accessibility, and a grant hides it', async () => {
    const t = setup({ input_monitoring: 'unknown', accessibility: 'unknown' });
    mogReady(t.bus);
    await flush();
    t.deps.request.mockImplementation(async (p: PermissionKind) => (t.os[p] = 'granted'));
    t.lastControl()[2]('perm:input_monitoring');
    await flush();
    const s = stack(t);
    expect(s.clearControls).toHaveBeenCalledWith('perm:input_monitoring');
    expect(s.add).toHaveBeenLastCalledWith({
      id: 'perm:accessibility',
      text: PERMISSION_TEXT.accessibility.need,
      completed: true,
    });
    t.lastControl()[2]('perm:accessibility');
    await flush();
    expect(s.clearControls).toHaveBeenCalledWith('perm:accessibility');
    expect(t.deps.hideCard).toHaveBeenCalledWith(MOG);
    expect(t.hub.at(-1)).toEqual({ needs: [] });
  });

  it('the texts name the pane and say when to reopen Bottle', () => {
    expect(PERMISSION_TEXT.input_monitoring.need).toBe(
      'Bottle needs Input Monitoring to hear the talk key',
    );
    expect(PERMISSION_TEXT.accessibility.need).toBe('Bottle needs Accessibility to paste');
    expect(PERMISSION_TEXT.input_monitoring.hub).toMatch(
      /Privacy & Security > Input Monitoring.*reopen Bottle/,
    );
    expect(PERMISSION_TEXT.accessibility.hub).toMatch(/Privacy & Security > Accessibility/);
  });
});
