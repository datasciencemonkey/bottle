// @vitest-environment jsdom
/**
 * AT-21 "Latency and capacity", the view part that runs with no device (#13, ADR 0031):
 * key-down reaches the brain -> capture opens -> mood `listening` -> `mog:view_state` -> MOG view
 * -> next frame, and each later step event -> its frame. Everything runs on the #229 sim clock.
 * The hops between processes take their ADR 0031 p95 allocation; the device run (#355) measures
 * the real ones. R2 behind the capture lifecycle (#14) is a typed double here; the host route is
 * #425 (ADR 0055).
 */
import type { CaptureState } from '../shared/interfaces';
import { checkRow } from '../shared/latency';
import { BrainBus } from '../brain/bus';
import { sendMogViewState } from '../brain/bus';
import type { MoodSources, RequestView } from '../brain/components/mood-mapper';
import { createMoodPublisher } from '../brain/mood-publisher';
import { onPtt } from '../brain/ptt';
import { createSimScheduler, FRAME_MS, type SimScheduler } from '../test-support/sim-scheduler';
import { render, tauri } from '../views/test-tauri';
import type { MascotRenderer } from '../views/mog/mascot';
import { FrameProbe } from '../views/mog/frame-probe';
import { MogView } from '../views/mog/MogView';

const net = vi.hoisted(() => ({
  toView: undefined as undefined | ((label: string, name: string, p: unknown) => void),
}));

vi.mock('@tauri-apps/api/core', async () => (await import('../views/test-tauri')).coreModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../views/test-tauri')).windowModule);
vi.mock('@tauri-apps/api/event', async () => ({
  ...(await import('../views/test-tauri')).eventModule,
  // The brain's emitTo: the view gets the event after the brain-to-view hop.
  emitTo: async (label: string, name: string, payload: unknown) =>
    net.toView?.(label, name, payload),
}));

const MOG = '00000000-0000-4000-8000-000000000001';

/** ADR 0031: the p95 time each hop may take, in ms. Sum with one 60 Hz frame: 86.7 ms < 100 ms. */
const HOP = { hotkeyToBrain: 10, captureStart: 30, brainToView: 30 } as const;

const closed: CaptureState = { recording: false, paused: false, othersTrack: 'off' };
const open: CaptureState = {
  owner: MOG,
  mode: 'ptt',
  recording: true,
  paused: false,
  othersTrack: 'off',
};

const mascot = (): MascotRenderer => ({
  mount: () => undefined,
  setMood: () => undefined,
  setLevel: () => undefined,
  setAttention: () => undefined,
  setHover: () => undefined,
  poke: () => undefined,
  destroy: () => undefined,
});

/** The brain, the core doubles and one MOG view on one sim clock. */
async function rig(
  hop: { hotkeyToBrain: number; captureStart: number; brainToView: number } = HOP,
) {
  const s: SimScheduler = createSimScheduler(0);
  tauri.label = `mog-${MOG}`;
  net.toView = (label, name, p) =>
    void s.after(hop.brainToView, () => tauri.emitTo(label, name, p));
  const bus = new BrainBus();
  const pub = createMoodPublisher({
    clock: s.clock,
    scheduler: s,
    traceNow: s.clock.now,
    send: (v) => void sendMogViewState(v.mogId, v),
  });
  let capture = closed;
  let requests: RequestView[] = [];
  let keyDownAt: number | undefined;
  const sources = (): MoodSources => ({ mog: { lifecycle: 'ready', since: 0 }, capture, requests });
  // TODO(#355): on the device, R2 opens the lease of the #14 lifecycle on key-down and sends
  // capture:state.
  onPtt(bus, s.clock.now, (e) => {
    if (e.phase === 'down') keyDownAt = e.at;
    const next = e.phase === 'down' ? open : closed;
    s.after(hop.captureStart, () => void bus.dispatch({ event: 'capture:state', payload: next }));
  });
  bus.on('capture:state', (p) => {
    const was = capture.recording;
    capture = p as CaptureState;
    const t = s.clock.now();
    // TODO(#14, #39): the pipeline starts the request when the capture ends with a key-up.
    if (was && !capture.recording) requests = [{ phase: 'transcribing', at: t }];
    if (capture.recording) requests = [];
    const cause =
      capture.recording && keyDownAt !== undefined ? { eventAt: t, keyDownAt } : { eventAt: t };
    pub.update(MOG, sources(), cause);
  });
  /** A pipeline step event reaches the brain (TODO(#39): the OperationTracker sends it). */
  const step = (phase: RequestView['phase']) => {
    requests = [{ phase, at: s.clock.now() }];
    pub.update(MOG, sources(), { eventAt: s.clock.now() });
  };
  const key = (atMs: number, phase: 'down' | 'up') =>
    s.at(
      atMs + hop.hotkeyToBrain,
      () => void bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: MOG, phase } }),
    );
  const probe = new FrameProbe({ now: s.clock.now, requestFrame: (cb) => s.requestFrame(cb) });
  await render(<MogView mogId={MOG} createMascot={mascot} probe={probe} />);
  return { s, key, step, probe };
}

describe('AT-21 view latency on the sim clock (#13)', () => {
  beforeEach(() => tauri.reset());
  afterEach(() => {
    tauri.label = 'hub';
    net.toView = undefined;
  });

  it('[UX-05b] [AT-21] listening shows on screen under 100 ms after key-down, over 20 presses', async () => {
    const { s, key, probe } = await rig();
    const downs: number[] = [];
    for (let i = 0; i < 20; i++) {
      // Each press lands at another point of the 60 Hz frame.
      const t = 1000 + i * 2000 + ((i * 7) % 17);
      downs.push(t);
      key(t, 'down');
      key(t + 600, 'up');
    }
    await s.runUntil(45_000);
    const listening = probe.samples().filter((x) => x.mood === 'listening');
    expect(listening).toHaveLength(20);
    const spans = listening.map((x, i) => x.frameAt - (downs[i] ?? NaN));
    for (const d of spans) expect(d).toBeLessThan(100);
    // The sum of the allocations is the worst case: no hop, hold or timer adds time.
    expect(Math.max(...spans)).toBeLessThanOrEqual(
      HOP.hotkeyToBrain + HOP.captureStart + HOP.brainToView + FRAME_MS,
    );
    expect(checkRow('UX-05b', spans).verdict).toBe('pass');
    // The probe row (from the key-down in the brain) agrees.
    expect(probe.report()['UX-05b'].verdict).toBe('pass');
  });

  it('[UX-36] [NFR-01c] [AT-21] each pipeline step renders within 100 ms of reaching the brain', async () => {
    const { s, key, step, probe } = await rig();
    // A 300 ms press: not a tap (CAP-44a), and key-up lands inside the listening hold.
    key(1000, 'down');
    key(1300, 'up');
    s.at(2000, () => step('working'));
    s.at(2600, () => step('blurb_reveal'));
    s.at(3200, () => step('completed'));
    s.at(6000, () => step('failed'));
    await s.runUntil(10_000);
    const moods = probe.samples().map((x) => x.mood);
    expect(moods).toEqual(['listening', 'processing', 'answering', 'success', 'error']);
    const spans = probe.samples().map((x) => x.frameAt - x.eventAt);
    for (const d of spans) expect(d).toBeLessThanOrEqual(HOP.brainToView + FRAME_MS);
    for (const d of spans) expect(d).toBeLessThanOrEqual(100);
  });

  it('[UX-36] [AT-21] a step inside a UX-05a hold shows as late in the samples, not hidden', async () => {
    const { s, step, probe } = await rig();
    s.at(1000, () => step('inserted'));
    s.at(1100, () => step('failed')); // error 100 ms after success: waits for the 400 ms hold
    await s.runUntil(3000);
    const err = probe.samples().find((x) => x.mood === 'error');
    expect(err?.eventAt).toBe(1100);
    expect((err?.frameAt ?? 0) - 1100).toBeGreaterThan(300);
  });

  it('[UX-05b] [AT-21] the harness fails the row when a hop is slower than its budget', async () => {
    const { s, key, probe } = await rig({ ...HOP, captureStart: 80 });
    for (let i = 0; i < 20; i++) {
      key(1000 + i * 2000, 'down');
      key(1600 + i * 2000, 'up');
    }
    await s.runUntil(45_000);
    // Measured from the key-down in the brain: 80 + 30 + up to one frame.
    expect(probe.report()['UX-05b'].verdict).toBe('fail');
  });
});
