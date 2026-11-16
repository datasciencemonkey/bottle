import type { CaptureState } from '../shared/interfaces';
import { createSimScheduler } from '../test-support/sim-scheduler';
import { MOOD_HOLD_MS, type MoodSources, type RequestView } from './components/mood-mapper';
import { createMoodPublisher, type PublishedViewState } from './mood-publisher';

const MOG = '00000000-0000-4000-8000-000000000001';
const closed: CaptureState = { recording: false, paused: false, othersTrack: 'off' };
const ptt: CaptureState = {
  owner: MOG,
  mode: 'ptt',
  recording: true,
  paused: false,
  othersTrack: 'off',
};
const src = (capture: CaptureState, requests: RequestView[] = []): MoodSources => ({
  mog: { lifecycle: 'ready', since: 0 },
  capture,
  requests,
});

function setup(trace = false) {
  const s = createSimScheduler(1000);
  const sent: PublishedViewState[] = [];
  const pub = createMoodPublisher({
    clock: s.clock,
    send: (v) => void sent.push(v),
    scheduler: s,
    ...(trace ? { traceNow: s.clock.now } : {}),
  });
  return { s, sent, pub };
}

describe('mood publisher: mood to mog:view_state (#13, ADR 0031)', () => {
  it('[UX-05b] sends listening in the same task as the capture change, with no timer', () => {
    const { s, sent, pub } = setup();
    pub.update(MOG, src(closed));
    pub.update(MOG, src(ptt));
    expect(sent.map((v) => v.mood)).toEqual(['idle', 'listening']);
    expect(s.clock.now()).toBe(1000);
  });

  it('[UX-05b] listening is not held back by a mood that just started (ADR 0018)', () => {
    const { s, sent, pub } = setup();
    pub.update(MOG, src(closed, [{ phase: 'transcribing', at: 1000 }]));
    expect(sent.at(-1)?.mood).toBe('processing');
    s.clock.advance(50);
    pub.update(MOG, src(ptt));
    expect(sent.at(-1)?.mood).toBe('listening');
  });

  it('[UX-36] [NFR-01c] key-up shows processing at once, even 300 ms into the listening hold', () => {
    const { s, sent, pub } = setup();
    pub.update(MOG, src(ptt));
    s.clock.advance(300);
    pub.update(MOG, src(closed, [{ phase: 'transcribing', at: s.clock.now() }]));
    expect(sent.map((v) => v.mood)).toEqual(['listening', 'processing']);
  });

  it('[UX-36] a step inside a UX-05a hold renders when the hold ends, with its first event time', async () => {
    const { s, sent, pub } = setup(true);
    pub.update(MOG, src(closed, [{ phase: 'inserted', at: 1000 }]));
    expect(sent.at(-1)?.mood).toBe('success');
    s.clock.advance(100);
    pub.update(MOG, src(closed, [{ phase: 'failed', at: 1100 }]), { eventAt: 1100 });
    expect(sent).toHaveLength(1);
    await s.runUntil(1000 + MOOD_HOLD_MS);
    expect(sent.map((v) => v.mood)).toEqual(['success', 'error']);
    // The trace keeps the time the event reached the brain, so the probe sees the hold delay.
    expect(sent[1]?.trace).toMatchObject({ eventAt: 1100, sentAt: 1000 + MOOD_HOLD_MS });
  });

  it('[UX-36] a newer update replaces the pending recheck', async () => {
    const { s, sent, pub } = setup();
    pub.update(MOG, src(closed, [{ phase: 'inserted', at: 1000 }]));
    s.clock.advance(100);
    pub.update(MOG, src(closed, [{ phase: 'failed', at: 1100 }]));
    s.clock.advance(100);
    pub.update(MOG, src(ptt)); // key-down wins at once and drops the held error
    await s.runUntil(2000);
    expect(sent.map((v) => v.mood)).toEqual(['success', 'listening']);
  });

  it('[UX-09a] [UX-36] the same mood and overlays are not sent twice', () => {
    const { s, sent, pub } = setup();
    pub.update(MOG, src(ptt));
    s.clock.advance(10);
    pub.update(MOG, src(ptt));
    expect(sent).toHaveLength(1);
    pub.update(MOG, { ...src(ptt), mog: { lifecycle: 'offline', since: 0 } });
    expect(sent).toHaveLength(2);
    expect(sent[1]?.overlays.offline).toBe(true);
  });

  it('[UX-36] the trace is off by default and has only numbers when on', () => {
    const off = setup();
    off.pub.update(MOG, src(ptt));
    expect(off.sent[0]).not.toHaveProperty('trace');
    const on = setup(true);
    on.pub.update(MOG, src(ptt), { eventAt: 990, keyDownAt: 980 });
    on.s.clock.advance(5);
    on.pub.update(MOG, src(closed, [{ phase: 'transcribing', at: 1005 }]));
    expect(on.sent[0]?.trace).toEqual({ seq: 1, eventAt: 990, keyDownAt: 980, sentAt: 1000 });
    expect(on.sent[1]?.trace).toEqual({ seq: 2, eventAt: 1005, sentAt: 1005 });
  });

  it('[UX-36] MOGs do not share a hold', () => {
    const { sent, pub } = setup();
    const other = '00000000-0000-4000-8000-000000000002';
    pub.update(MOG, src(closed, [{ phase: 'transcribing', at: 1000 }]));
    pub.update(other, { ...src(closed), capture: { ...ptt, owner: other } });
    expect(sent.map((v) => [v.mogId, v.mood])).toEqual([
      [MOG, 'processing'],
      [other, 'listening'],
    ]);
  });
});
