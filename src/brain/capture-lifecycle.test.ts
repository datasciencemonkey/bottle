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

/**
 * A typed double of R2 behind the IPC (the host route to CaptureCore is #425, ADR 0055).
 * It keeps R2's one-lease rule (`CaptureCore::start`, ARC-F9): a start while another start is
 * pending gets `mic_busy`, and so does any start while a lease is live, except a PTT over a
 * hands-free lease. A released PTT stays live in the 120 ms post-roll until `endPostRoll()`.
 */
function rig(opts: { holdStart?: boolean; config?: CaptureLifecycleDeps['config'] } = {}) {
  const calls: Call[] = [];
  const refused: Call[] = [];
  let next = 0;
  const pending: (() => void)[] = [];
  let starting = 0;
  const live = new Map<string, CaptureState['mode']>();
  const postRoll = new Set<string>();
  const control: CaptureControl = {
    start: (args) => {
      calls.push(['start', args]);
      const modes = [...live.values()];
      const free =
        starting === 0 &&
        (modes.length === 0 ||
          (modes.length === 1 && modes[0] === 'hands_free' && args.mode === 'ptt'));
      if (!free) {
        refused.push(['start', args]);
        return Promise.reject(new Error('mic_busy'));
      }
      const leaseId = String(++next);
      const state: CaptureState = {
        owner: args.mogId,
        mode: args.mode,
        leaseId,
        recording: true,
        paused: false,
        othersTrack: 'off',
      };
      if (!opts.holdStart) {
        live.set(leaseId, args.mode);
        return Promise.resolve(state);
      }
      starting++;
      return new Promise((resolve) =>
        pending.push(() => {
          starting--;
          live.set(leaseId, args.mode);
          resolve(state);
        }),
      );
    },
    release: async (args) => {
      calls.push(['release', args]);
      if (live.has(args.leaseId)) postRoll.add(args.leaseId);
    },
    discard: async (args) => {
      calls.push(['discard', args]);
      live.delete(args.leaseId);
    },
    stopHandsFree: async (args) => {
      calls.push(['stopHandsFree', args]);
      live.delete(args.leaseId);
    },
  };
  const endPostRoll = () => {
    for (const id of postRoll) live.delete(id);
    postRoll.clear();
  };
  const refusals: { mogId: string; mode: string; error?: unknown }[] = [];
  const toggles: { mogId: string; liveAtToggle: number }[] = [];
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
    onRefused: (e) => void refusals.push(e),
    onRecordToggle: (e) => void toggles.push({ mogId: e.mogId, liveAtToggle: live.size }),
    ...(opts.config ? { config: opts.config } : {}),
  };
  const lc = createCaptureLifecycle(deps);
  const flush = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  };
  const ptt = (phase: PttEvent['phase'], at: number, mogId = A, ask = false, record = false) => {
    const e = {
      mogId,
      phase,
      at,
      ...(ask ? { ask: true as const } : {}),
      ...(record ? { record: true as const } : {}),
    };
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
  return {
    lc,
    calls,
    refused,
    refusals,
    toggles,
    live,
    endPostRoll,
    utterances,
    transcribed,
    texts,
    ptt,
    seg,
    flush,
    resolveStarts,
  };
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

  it('[CAP-12b] [RTR-17a] the utterance of a kept PTT goes to onUtterance with forceAsk from the ⌥G key-up', async () => {
    const r = rig();
    r.ptt('down', 0);
    await r.flush();
    r.ptt('up', 900, A, true);
    await r.lc.onSegment(r.seg('1'));
    expect(r.utterances).toEqual([
      { mogId: A, text: 'hello world', forceAsk: true, segment: r.seg('1'), mode: 'ptt' },
    ]);
  });

  it('[RTR-17a] an ⌥G key-up before the grant still marks the utterance as an ask', async () => {
    const r = rig({ holdStart: true });
    r.ptt('down', 0);
    r.ptt('up', 900, A, true);
    await r.resolveStarts();
    await r.lc.onSegment(r.seg('1'));
    expect(r.utterances.map((u) => u.forceAsk)).toEqual([true]);
  });

  it('[RTR-17a] a plain key-up is dictation: forceAsk is false', async () => {
    const r = rig();
    r.ptt('down', 0);
    await r.flush();
    r.ptt('up', 900);
    await r.lc.onSegment(r.seg('1'));
    expect(r.utterances.map((u) => u.forceAsk)).toEqual([false]);
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

describe('capture lifecycle: R2 one-lease rule (#14)', () => {
  it('[CAP-14a] [AT-03] a double-tap whose second tap ends before its grant starts hands-free after that discard', async () => {
    // The mic opens more slowly than a tap (NFR-02 allows 150 ms): R2 must get the discard of
    // the second tap before the hands-free start, or it answers mic_busy.
    const r = rig({ holdStart: true });
    r.ptt('down', 0);
    await r.resolveStarts();
    r.ptt('cancel', 100);
    r.ptt('down', 200);
    r.ptt('cancel', 300);
    r.ptt('double_tap', 300);
    await r.flush();
    await r.resolveStarts();
    await r.resolveStarts();
    expect(r.refused).toEqual([]);
    expect(names(r.calls)).toEqual(['start', 'discard', 'start', 'discard', 'start']);
    expect(r.calls.at(-1)).toEqual(['start', { mogId: A, mode: 'hands_free' }]);
    expect([...r.live.values()]).toEqual(['hands_free']);
  });

  it('[CAP-14a] a key-down while the double-tap waits for the discard cancels the hands-free start', async () => {
    const r = rig({ holdStart: true });
    r.ptt('down', 0);
    await r.resolveStarts();
    r.ptt('cancel', 100);
    r.ptt('down', 200);
    r.ptt('cancel', 300);
    r.ptt('double_tap', 300);
    // A third press: R1 starts a new pair (CAP-14a), so hands-free does not start.
    r.ptt('down', 350);
    await r.resolveStarts();
    await r.resolveStarts();
    expect(r.calls.filter((c) => c[1].mode === 'hands_free')).toEqual([]);
  });

  it('[CAP-12b] a key-down in the post-roll gets mic_busy; the brain surfaces it and its key-up sends nothing', async () => {
    const r = rig();
    r.ptt('down', 0);
    await r.flush();
    r.ptt('up', 1000);
    r.ptt('down', 1050);
    await r.flush();
    expect(r.refused).toEqual([['start', { mogId: A, mode: 'ptt' }]]);
    expect(r.refusals).toEqual([{ mogId: A, mode: 'ptt', error: new Error('mic_busy') }]);
    r.ptt('up', 2000);
    await r.flush();
    expect(names(r.calls)).toEqual(['start', 'release', 'start']);
  });

  it('[CAP-14b] a tap in the post-roll of a PTT over hands-free still stops hands-free', async () => {
    const r = rig();
    r.lc.click(A);
    await r.flush();
    r.ptt('down', 10_000);
    await r.flush();
    r.ptt('up', 12_000);
    r.ptt('down', 12_050);
    await r.flush();
    r.ptt('cancel', 12_150);
    expect(r.calls.at(-1)).toEqual(['stopHandsFree', { mogId: A, leaseId: '1', by: 'tap' }]);
  });

  it('[CAP-14b] [CAP-44c] TODO(#444) pins the gap: the brain times the tap, so a cancel under 250 ms stops hands-free', async () => {
    // R1's `HotkeyOutput::Tap` does not reach the brain (#444). A CAP-44c cancel under 250 ms
    // looks like a tap and stops hands-free; a cancel 250 ms or more after its key-down does not.
    const r = rig();
    r.lc.click(A);
    await r.flush();
    r.ptt('down', 10_000);
    await r.flush();
    r.ptt('cancel', 10_000 + TAP_MAX_HOLD_MS);
    expect(names(r.calls)).toEqual(['start', 'start', 'discard']);
    r.ptt('down', 20_000);
    await r.flush();
    r.ptt('cancel', 20_000 + TAP_MAX_HOLD_MS - 1);
    expect(r.calls.at(-1)).toEqual(['stopHandsFree', { mogId: A, leaseId: '1', by: 'tap' }]);
  });
});

describe('capture lifecycle: segments (#14)', () => {
  it('[CAP-12b] [CAP-14c] only a final segment goes to STT, and onUtterance fires once for it', async () => {
    const r = rig();
    r.lc.click(A);
    await r.flush();
    const partial = { ...r.seg('1', 's1'), final: false };
    await r.lc.onSegment(partial);
    expect(r.transcribed).toEqual([]);
    await r.lc.onSegment(r.seg('1', 's1'));
    await r.lc.onSegment(r.seg('1', 's1'));
    await r.lc.onSegment(r.seg('1', 's2'));
    expect(r.utterances.map((u) => u.segment.segmentId)).toEqual(['s1', 's2']);
  });
});

describe('capture lifecycle: the ⌥R record toggle (ADR 0103)', () => {
  it('[ADR-0103] an ⌥R key-up drops the PTT audio and toggles the recording once the mic is free', async () => {
    const r = rig();
    r.ptt('down', 0);
    await r.flush();
    r.ptt('up', 900, A, false, true);
    // No dictation: the lease is discarded, never released, and nothing reaches STT.
    expect(r.calls).toEqual([
      ['start', { mogId: A, mode: 'ptt' }],
      ['discard', { mogId: A, leaseId: '1' }],
    ]);
    await r.flush();
    await r.lc.onSegment(r.seg('1'));
    expect(r.transcribed).toEqual([]);
    expect(r.utterances).toEqual([]);
    // The toggle comes after the discard settled, so the recording can take the mic.
    expect(r.toggles).toEqual([{ mogId: A, liveAtToggle: 0 }]);
  });

  it('[ADR-0103] an ⌥R key-up before the grant toggles after the discard', async () => {
    const r = rig({ holdStart: true });
    r.ptt('down', 0);
    r.ptt('up', 900, A, false, true);
    await r.flush();
    expect(r.toggles).toEqual([]);
    await r.resolveStarts();
    await r.flush();
    expect(names(r.calls)).toEqual(['start', 'discard']);
    expect(r.toggles).toEqual([{ mogId: A, liveAtToggle: 0 }]);
  });

  it('[ADR-0103] a refused hold (a recording runs) still toggles: ⌥R stops the recording', async () => {
    const r = rig();
    // A recording holds the mic, so R2 refuses the PTT start.
    r.live.set('rec', 'session');
    r.ptt('down', 0);
    await r.flush();
    expect(r.refusals).toHaveLength(1);
    r.ptt('up', 900, A, false, true);
    await r.flush();
    expect(names(r.calls)).toEqual(['start']);
    expect(r.toggles.map((t) => t.mogId)).toEqual([A]);
    expect(r.utterances).toEqual([]);
  });

  it('[ADR-0103] a plain key-up, an ⌥G key-up and a cancel never toggle', async () => {
    const r = rig();
    r.ptt('down', 0);
    await r.flush();
    r.ptt('up', 900);
    r.endPostRoll();
    r.ptt('down', 2000);
    await r.flush();
    r.ptt('up', 2900, A, true);
    r.endPostRoll();
    r.ptt('down', 4000);
    await r.flush();
    r.ptt('cancel', 4500);
    await r.flush();
    expect(r.toggles).toEqual([]);
  });

  it('[ADR-0103] an ⌥R key-up is no tap: it never stops hands-free', async () => {
    const r = rig();
    r.lc.click(A);
    await r.flush();
    r.ptt('down', 0);
    await r.flush();
    r.ptt('up', 300, A, false, true);
    await r.flush();
    expect(names(r.calls)).not.toContain('stopHandsFree');
    expect(r.toggles).toHaveLength(1);
  });
});

describe('capture lifecycle wiring (#14)', () => {
  it('[ADR-0103] the hotkey:ptt up with record: true reaches the lifecycle as a record toggle', async () => {
    const bus = new BrainBus();
    const r = rig();
    wireCaptureLifecycle(bus, r.lc, () => 5);
    await bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: A, phase: 'down' } });
    await r.flush();
    await bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: A, phase: 'up', record: true } });
    await r.flush();
    expect(r.toggles.map((t) => t.mogId)).toEqual([A]);
  });

  it('[RTR-17a] the hotkey:ptt up with ask: true reaches the lifecycle as a forced ask', async () => {
    const bus = new BrainBus();
    const r = rig();
    wireCaptureLifecycle(bus, r.lc, () => 5);
    await bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: A, phase: 'down' } });
    await r.flush();
    await bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: A, phase: 'up', ask: true } });
    await r.lc.onSegment(r.seg('1'));
    r.endPostRoll();
    await bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: A, phase: 'down', shift: true } });
    await r.flush();
    await bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: A, phase: 'up' } });
    await r.lc.onSegment(r.seg('2'));
    expect(r.utterances.map((u) => u.forceAsk)).toEqual([true, false]);
  });

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

  it('[CAP-14b] Esc in the card of the hands-free MOG (view:input cancel) stops it and drops its audio', async () => {
    const bus = new BrainBus();
    const r = rig();
    wireCaptureLifecycle(bus, r.lc, () => 5);
    r.lc.click(A);
    await r.flush();
    // Esc in the card of another MOG does not stop A.
    await bus.dispatch({
      event: 'view:input',
      payload: { from: `card-${B}`, input: { mogId: B, kind: 'cancel' } },
    });
    expect(names(r.calls)).toEqual(['start']);
    await bus.dispatch({
      event: 'view:input',
      payload: { from: `card-${A}`, input: { mogId: A, kind: 'cancel' } },
    });
    expect(r.calls.at(-1)).toEqual(['stopHandsFree', { mogId: A, leaseId: '1', by: 'escape' }]);
    await r.lc.onSegment(r.seg('1'));
    expect(r.transcribed).toEqual([]);
  });
});
