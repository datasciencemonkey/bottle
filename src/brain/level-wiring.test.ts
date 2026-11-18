/**
 * #476 (CAP-16a, CAP-16b): the started brain registers `onLevel` with the routed
 * `capture_subscribe`, and a level on that Channel reaches only the target MOG view.
 */
import { EVENTS } from '../shared/ipc';
import { core, type FakeChannel } from './test-core';
import { startBrain } from './components';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const A = '00000000-0000-4000-8000-0000000000a1';

const state = (recording: boolean) => ({
  owner: recording ? A : null,
  mode: recording ? 'ptt' : null,
  recording,
  paused: false,
  othersTrack: 'off',
});

describe('the brain level wiring (#476)', () => {
  it('[CAP-16a] [CAP-16b] [AT-14] startBrain subscribes onLevel and a level reaches only the target MOG', () => {
    const brain = startBrain();
    const sub = core.invoke.mock.calls.find(([cmd]) => cmd === 'capture_subscribe');
    expect(sub).toBeDefined();
    const { onLevel } = (sub?.[1] as { args: { onLevel: FakeChannel } }).args;
    expect(core.channels).toContain(onLevel);

    brain.bus.dispatch({ event: EVENTS['capture.state'].name, payload: state(true) });
    onLevel.onmessage(-21.5);
    const levels = () => core.emitTo.mock.calls.filter(([, ev]) => ev === EVENTS['mog.level'].name);
    expect(levels()).toHaveLength(1);
    const [target, , frame] = levels()[0] ?? [];
    expect(target).toBe(`mog-${A}`);
    expect(frame).toMatchObject({ owner: A, dbfs: -21.5 });

    // CAP-16b: the stop sends one `owner: null` frame, and a late level is dropped.
    brain.bus.dispatch({ event: EVENTS['capture.state'].name, payload: state(false) });
    onLevel.onmessage(-21.5);
    expect(levels()).toHaveLength(2);
    expect(levels()[1]?.[2]).toMatchObject({ owner: null });
  });
});
