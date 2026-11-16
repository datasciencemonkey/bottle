import type { CaptureState, SegmentRef } from '../shared/interfaces';
import { BrainBus } from './bus';
import { mapMood } from './components/mood-mapper';
import {
  TAP_MAX_HOLD_MS,
  createCaptureLifecycle,
  wireCaptureLifecycle,
  type CaptureControl,
  type CaptureLifecycleDeps,
  type Utterance,
} from './capture-lifecycle';
import type { PttEvent } from './ptt';

const A = '00000000-0000-4000-8000-00000000000a';
const B = '00000000-0000-4000-8000-00000000000b';

type Call = [string, Record<string, unknown>];

/** A typed double of R2 behind the IPC (TODO(#425): the host routes capture_* to CaptureCore). */
function rig(opts: { holdStart?: boolean; config?: CaptureLifecycleDeps['config'] } = {}) {
  const calls: Call[] = [];
  let next = 0;
  const pending: (() => void)[] = [];
  const control: CaptureControl = {
    start: (args) => {
      calls.push(['start', args]);
      const state: CaptureState = {
        owner: args.mogId,
        mode: args.mode,
        leaseId: String(++next),
        recording: true,
        paused: false,
        othersTrack: 'off',
      };
      if (!opts.holdStart) return Promise.resolve(state);
      return new Promise((resolve) => pending.push(() => resolve(state)));
    },
    release: async (args) => void calls.push(['release', args]),
    discard: async (args) => void calls.push(['discard', args]),
    stopHandsFree: async (args) => void calls.push(['stopHandsFree', args]),
  };
  const utterances: Utterance[] = [];
  const transcribed: SegmentRef[] = [];
  const texts = new Map<string, string>();
  const deps: CaptureLifecycleDeps = {
    control,
    transcribe: async (seg) => {
      transcribed.push(seg);
      return texts.get(seg.segmentId) ?? 'hello world';
    },
    onUtterance: (u) => void utterances.push(u),
    ...(opts.config ? { config: opts.config } : {}),
  };
  const lc = createCaptureLifecycle(deps);
  const flush = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  };
  const ptt = (phase: PttEvent['phase'], at: number, mogId = A, shift = false) => {
    const e = { mogId, phase, at, ...(shift ? { shift: true as const } : {}) };
    lc.onPtt(e);
  };
  const seg = (leaseId: string, segmentId = `s${leaseId}`, mogId = A): SegmentRef => ({
    segmentId,
    leaseId,
    mogId,
    track: 'you',
    tStartMs: 0,
    tEndMs: 1000,
    final: true,
  });
  const resolveStarts = async () => {
    while (pending.length) pending.shift()?.();
    await flush();
  };
  return { lc, calls, utterances, transcribed, texts, ptt, seg, flush, resolveStarts };
}

const names = (calls: Call[]) => calls.map((c) => c[0]);

describe('capture lifecycle: PTT (#14)', () => {
  it('[CAP-12a] [AT-19] key-down starts a PTT lease for the MOG in the same task', () => {
    const r = rig();
    r.ptt('down', 0);
    // No await: the start goes out in the task that got the key-down (NFR-02 budget).
    expect(r.calls).toEqual([['start', { mogId: A, mode: 'ptt' }]]);
  });

  it('[CAP-12b] [AT-19] key-up releases the lease, and R2 applies the 120 ms post-roll', async () => {
    const r = rig();
    r.ptt('down', 0);
    await r.flush();
    r.ptt('up', 1500);
    expect(r.calls).toEqual([
      ['start', { mogId: A, mode: 'ptt' }],
      ['release', { mogId: A, leaseId: '1' }],
    ]);
  });

  it('[CAP-12b] a key-up before the lease is granted releases it when the grant comes', async () => {
    const r = rig({ holdStart: true });
    r.ptt('down', 0);
    r.ptt('up', 400);
    expect(names(r.calls)).toEqual(['start']);
    await r.resolveStarts();
    expect(r.calls[1]).toEqual(['release', { mogId: A, leaseId: '1' }]);
  });

  it('[CAP-12b] [RTR-17a] the utterance of a kept PTT goes to onUtterance with forceAsk from key-down', async () => {
    const r = rig();
    r.ptt('down', 0, A, true);
    await r.flush();
    r.ptt('up', 900);
    await r.lc.onSegment(r.seg('1'));
    expect(r.utterances).toEqual([
      { mogId: A, text: 'hello world', forceAsk: true, segment: r.seg('1'), mode: 'ptt' },
    ]);
  });

  it('[CAP-12b] an empty STT result fires no utterance', async () => {
    const r = rig();
    r.ptt('down', 0);
    await r.flush();
    r.ptt('up', 900);
    r.texts.set('s1', '   ');
    await r.lc.onSegment(r.seg('1'));
    expect(r.utterances).toEqual([]);
  });

  it('[CAP-12a] a segment of a lease the lifecycle did not start is ignored', async () => {
    const r = rig();
    await r.lc.onSegment(r.seg('77'));
    expect(r.transcribed).toEqual([]);
    expect(r.utterances).toEqual([]);
  });
});

describe('capture lifecycle: taps (#14)', () => {
  it('[CAP-44b] [AT-03] a tap discards its lease at once and its audio never goes to STT', async () => {
    const r = rig();
    r.ptt('down', 0);
    await r.flush();
    r.ptt('cancel', 100);
    expect(r.calls[1]).toEqual(['discard', { mogId: A, leaseId: '1' }]);
    await r.lc.onSegment(r.seg('1'));
    expect(r.transcribed).toEqual([]);
    expect(r.utterances).toEqual([]);
  });

  it('[CAP-44b] a tap that ends before the grant discards the lease when the grant comes', async () => {
    const r = rig({ holdStart: true });
    r.ptt('down', 0);
    r.ptt('cancel', 80);
    await r.resolveStarts();
    expect(r.calls[1]).toEqual(['discard', { mogId: A, leaseId: '1' }]);
    await r.lc.onSegment(r.seg('1'));
    expect(r.transcribed).toEqual([]);
  });

  it('[CAP-44b] a CAP-44c cancel also discards the audio', async () => {
    const r = rig();
    r.ptt('down', 0);
    await r.flush();
    r.ptt('cancel', 700);
    expect(names(r.calls)).toEqual(['start', 'discard']);
  });
});

describe('capture lifecycle: hands-free (#14)', () => {
  it('[CAP-14a] [AT-03] a double-tap starts hands-free for the MOG', async () => {
    const r = rig();
    r.ptt('down', 0);
    await r.flush();
    r.ptt('cancel', 100);
    r.ptt('down', 200);
    await r.flush();
    r.ptt('cancel', 300);
    r.ptt('double_tap', 300);
    await r.flush();
    expect(r.calls.at(-1)).toEqual(['start', { mogId: A, mode: 'hands_free' }]);
  });

  it('[CAP-14a] a mascot click or Talk starts hands-free', async () => {
    const r = rig();
    r.lc.click(B);
    await r.flush();
    expect(r.calls).toEqual([['start', { mogId: B, mode: 'hands_free' }]]);
  });

  it('[CAP-14a] hands-free does not start when the MOG has it off in mog.capture.modes', async () => {
    const r = rig({ config: () => ({ modes: ['ptt'], clickTogglesListening: true }) });
    r.ptt('double_tap', 300);
    r.lc.click(A);
    await r.flush();
    expect(r.calls).toEqual([]);
  });

  it('[CAP-14a] the hands-free span shows the listening mood until it stops', () => {
    const live: CaptureState = {
      owner: A,
      mode: 'hands_free',
      leaseId: '4',
      recording: true,
      paused: false,
      othersTrack: 'off',
      startedAt: 1_800_000_000_000,
    };
    const mog = { lifecycle: 'ready', since: 0 } as const;
    for (const t of [0, 60_000, 299_999]) {
      expect(mapMood(A, { mog, capture: live, requests: [] }, t).mood).toBe('listening');
    }
    const idle: CaptureState = { recording: false, paused: false, othersTrack: 'off' };
    expect(mapMood(A, { mog, capture: idle, requests: [] }, 300_000).mood).not.toBe('listening');
  });

  async function handsFree(r: ReturnType<typeof rig>) {
    r.lc.click(A);
    await r.flush();
    r.lc.onState({
      owner: A,
      mode: 'hands_free',
      leaseId: '1',
      recording: true,
      paused: false,
      othersTrack: 'off',
    });
  }

  it('[CAP-14b] [AT-03] a tap stops hands-free in the same task and keeps its audio', async () => {
    const r = rig();
    await handsFree(r);
    r.ptt('down', 10_000);
    await r.flush();
    r.ptt('cancel', 10_000 + TAP_MAX_HOLD_MS - 1);
    // Same task as the tap: 0 ms on this side, well within 200 ms.
    expect(r.calls.slice(-2)).toEqual([
      ['discard', { mogId: A, leaseId: '2' }],
      ['stopHandsFree', { mogId: A, leaseId: '1', by: 'tap' }],
    ]);
    await r.lc.onSegment(r.seg('1'));
    expect(r.utterances.map((u) => u.mode)).toEqual(['hands_free']);
    expect(r.utterances[0]?.forceAsk).toBe(false);
  });

  it('[CAP-14b] a double-tap whose first tap stopped hands-free does not start it again', async () => {
    const r = rig();
    await handsFree(r);
    r.ptt('down', 10_000);
    await r.flush();
    r.ptt('cancel', 10_100);
    r.lc.onState({ recording: false, paused: false, othersTrack: 'off' });
    r.ptt('down', 10_200);
    await r.flush();
    r.ptt('cancel', 10_300);
    r.ptt('double_tap', 10_300);
    await r.flush();
    expect(r.calls.filter((c) => c[0] === 'start' && c[1].mode === 'hands_free')).toHaveLength(1);
  });

  it('[CAP-14b] a held PTT over hands-free does not stop hands-free', async () => {
    const r = rig();
    await handsFree(r);
    r.ptt('down', 10_000);
    await r.flush();
    r.ptt('up', 12_000);
    expect(names(r.calls)).toEqual(['start', 'start', 'release']);
  });

  it('[CAP-14b] [AT-03] Esc stops hands-free at once and drops its audio', async () => {
    const r = rig();
    await handsFree(r);
    r.lc.escape();
    expect(r.calls.at(-1)).toEqual(['stopHandsFree', { mogId: A, leaseId: '1', by: 'escape' }]);
    await r.lc.onSegment(r.seg('1'));
    expect(r.transcribed).toEqual([]);
    expect(r.utterances).toEqual([]);
  });

  it('[CAP-14b] a click on the listening MOG stops hands-free when clickTogglesListening is on', async () => {
    const r = rig();
    await handsFree(r);
    r.lc.click(A);
    expect(r.calls.at(-1)).toEqual(['stopHandsFree', { mogId: A, leaseId: '1', by: 'click' }]);
    const off = rig({
      config: () => ({ modes: ['ptt', 'hands_free'], clickTogglesListening: false }),
    });
    await handsFree(off);
    off.lc.click(A);
    expect(names(off.calls)).toEqual(['start']);
  });

  it('[CAP-14b] [CAP-14c] a stop by R2 (5 min cap or VAD end) ends the span; its audio is kept', async () => {
    const r = rig();
    await handsFree(r);
    r.lc.onState({ recording: false, paused: false, othersTrack: 'off' });
    // Nothing to stop any more: Esc and a tap do nothing.
    r.lc.escape();
    expect(names(r.calls)).toEqual(['start']);
    await r.lc.onSegment(r.seg('1'));
    expect(r.utterances).toHaveLength(1);
  });

  it('[CAP-14b] Esc before the hands-free grant stops the lease when the grant comes', async () => {
    const r = rig({ holdStart: true });
    r.lc.click(A);
    r.lc.escape();
    await r.resolveStarts();
    expect(r.calls.at(-1)).toEqual(['stopHandsFree', { mogId: A, leaseId: '1', by: 'escape' }]);
  });
});

describe('capture lifecycle wiring (#14)', () => {
  it('[CAP-12a] [CAP-14a] hotkey:ptt, view clicks and capture:segment reach the lifecycle on the bus', async () => {
    const bus = new BrainBus();
    const r = rig();
    const off = wireCaptureLifecycle(bus, r.lc, () => 5);
    await bus.dispatch({
      event: 'view:input',
      payload: { from: `mog-${B}`, input: { mogId: B, kind: 'click' } },
    });
    // A badge click is not a mascot click.
    await bus.dispatch({
      event: 'view:input',
      payload: {
        from: `mog-${B}`,
        input: { mogId: B, kind: 'click', payload: { target: 'badge' } },
      },
    });
    await bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: A, phase: 'down' } });
    await r.flush();
    expect(r.calls).toEqual([
      ['start', { mogId: B, mode: 'hands_free' }],
      ['start', { mogId: A, mode: 'ptt' }],
    ]);
    await bus.dispatch({ event: 'capture:segment', payload: r.seg('2', 's2', A) });
    await r.flush();
    expect(r.transcribed.map((s) => s.segmentId)).toEqual(['s2']);
    off();
    await bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: A, phase: 'up' } });
    expect(names(r.calls)).toEqual(['start', 'start']);
  });
});
