import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CaptureState, MogId } from '../shared/interfaces';
import { EVENTS, type MicLevelFrame } from '../shared/ipc';
import { BrainBus } from './bus';
import { core } from './test-core';
import {
  LevelRouter,
  SILENT_DBFS,
  levelTargetOf,
  subscribeLevels,
  wireLevelStream,
} from './level-stream';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const A: MogId = '00000000-0000-4000-8000-0000000000a1';
const B: MogId = '00000000-0000-4000-8000-0000000000b2';

const idle: CaptureState = { recording: false, paused: false, othersTrack: 'off' };
const rec = (owner: MogId, extra: Partial<CaptureState> = {}): CaptureState => ({
  owner,
  mode: 'ptt',
  leaseId: '1',
  recording: true,
  paused: false,
  othersTrack: 'off',
  ...extra,
});

function router() {
  const sent: Array<[MogId, MicLevelFrame]> = [];
  let t = 1000;
  const r = new LevelRouter(
    (mogId, frame) => {
      sent.push([mogId, frame]);
    },
    () => t,
  );
  return { r, sent, tick: (ms: number) => (t += ms) };
}

describe('level stream routing (CAP-16a, CAP-16b, ADR 0067)', () => {
  beforeEach(() => core.reset());

  it('[CAP-16a] the target is the directed MOG, else the owner, and none when not recording', () => {
    expect(levelTargetOf(rec(A))).toBe(A);
    expect(levelTargetOf(rec(A, { mode: 'session', directedTo: B }))).toBe(B);
    expect(levelTargetOf(rec(A, { paused: true }))).toBeNull();
    expect(levelTargetOf(idle)).toBeNull();
    expect(levelTargetOf({ ...idle, recording: true })).toBeNull();
  });

  it('[CAP-16a] sends each level only to the target MOG', () => {
    const { r, sent, tick } = router();
    r.onState(rec(A));
    r.onLevel(-20);
    tick(34);
    r.onLevel(-25.5);
    expect(sent).toEqual([
      [A, { owner: A, dbfs: -20, t: 1000 }],
      [A, { owner: A, dbfs: -25.5, t: 1034 }],
    ]);
    expect(sent.every(([to]) => to === A)).toBe(true);
  });

  it('[CAP-16a] other MOGs get level 0: the old target gets one owner-null frame on a move', () => {
    const { r, sent } = router();
    r.onState(rec(A));
    r.onLevel(-10);
    r.onState(rec(A, { mode: 'session', directedTo: B }));
    r.onLevel(-12);
    expect(sent).toEqual([
      [A, { owner: A, dbfs: -10, t: 1000 }],
      [A, { owner: null, dbfs: SILENT_DBFS, t: 1000 }],
      [B, { owner: B, dbfs: -12, t: 1000 }],
    ]);
  });

  it('[CAP-16a] drops a level that is not a finite number', () => {
    const { r, sent } = router();
    r.onState(rec(A));
    r.onLevel(Number.NaN);
    r.onLevel('loud' as unknown as number);
    r.onLevel(Number.POSITIVE_INFINITY);
    expect(sent).toEqual([]);
  });

  it('[CAP-16b] no level goes out after the stop, and the owner gets one owner-null frame', () => {
    const { r, sent, tick } = router();
    r.onState(rec(A));
    r.onLevel(-20);
    tick(10);
    r.onState(idle);
    r.onLevel(-20);
    r.onState(idle);
    expect(sent).toEqual([
      [A, { owner: A, dbfs: -20, t: 1000 }],
      [A, { owner: null, dbfs: SILENT_DBFS, t: 1010 }],
    ]);
    expect(r.target).toBeNull();
  });

  it('[CAP-16b] a pause stops the level stream too', () => {
    const { r, sent } = router();
    r.onState(rec(A));
    r.onState(rec(A, { paused: true }));
    r.onLevel(-20);
    expect(sent).toEqual([[A, { owner: null, dbfs: SILENT_DBFS, t: 1000 }]]);
  });

  it('[CAP-16b] no level goes out before any capture starts', () => {
    const { r, sent } = router();
    r.onLevel(-20);
    expect(sent).toEqual([]);
  });

  it('[CAP-16a] wireLevelStream reads capture:state from the brain Channel', () => {
    const { r, sent } = router();
    const bus = new BrainBus();
    const off = wireLevelStream(bus, r);
    bus.dispatch({ event: EVENTS['capture.state'].name, payload: rec(A) });
    r.onLevel(-30);
    bus.dispatch({ event: EVENTS['capture.state'].name, payload: 'not a state' });
    expect(r.target).toBe(A);
    off();
    bus.dispatch({ event: EVENTS['capture.state'].name, payload: idle });
    expect(r.target).toBe(A);
    expect(sent).toEqual([[A, { owner: A, dbfs: -30, t: 1000 }]]);
  });

  it('[CAP-16a] subscribeLevels registers the onLevel Channel with capture_subscribe', async () => {
    const { r, sent } = router();
    r.onState(rec(A));
    await expect(subscribeLevels(r)).resolves.toBe(true);
    const [cmd, args] = core.invoke.mock.calls[0] ?? [];
    expect(cmd).toBe('capture_subscribe');
    expect(args).toEqual({ args: { onLevel: core.channels[0], onPartial: core.channels[1] } });
    core.channels[0]?.onmessage(-18);
    expect(sent).toEqual([[A, { owner: A, dbfs: -18, t: 1000 }]]);
  });

  it('[CAP-16a] subscribeLevels resolves false when the core refuses', async () => {
    const { r } = router();
    core.invoke.mockRejectedValueOnce({ error: 'not_implemented' });
    await expect(subscribeLevels(r)).resolves.toBe(false);
  });
});
