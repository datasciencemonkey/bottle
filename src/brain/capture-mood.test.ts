/**
 * #551: the MOG mood follows `capture:state`, and a refused capture start shows its reason on
 * the MOG (UX-01a, UX-05a, UX-05b, UX-07, MB-08a, CAP-01a, CAP-17b).
 */
import type { CaptureState } from '../shared/interfaces';
import { createSimScheduler } from '../test-support/sim-scheduler';
import { ERROR_MS, MOOD_HOLD_MS } from './components/mood-mapper';
import {
  REFUSAL_REASONS,
  REFUSAL_TEXT,
  createCaptureMood,
  refusalReason,
  type RefusalReason,
} from './capture-mood';
import { createMoodPublisher, type PublishedViewState } from './mood-publisher';

const A = '00000000-0000-4000-8000-0000000000a1';
const B = '00000000-0000-4000-8000-0000000000b2';

const idle: CaptureState = { recording: false, paused: false, othersTrack: 'off' };
const ptt = (owner: string): CaptureState => ({
  owner,
  mode: 'ptt',
  recording: true,
  paused: false,
  othersTrack: 'off',
});

function setup() {
  const s = createSimScheduler(1000);
  const sent: PublishedViewState[] = [];
  const publisher = createMoodPublisher({
    clock: s.clock,
    send: (v) => void sent.push(v),
    scheduler: s,
  });
  const cm = createCaptureMood({ publisher, clock: s.clock, scheduler: s });
  const last = (mogId: string) => sent.filter((v) => v.mogId === mogId).at(-1);
  return { s, sent, cm, last };
}

describe('capture mood: capture:state to the MOG mood (#551)', () => {
  it('[UX-01a] [UX-05b] the capture owner shows listening in the same task as capture:state', () => {
    const { s, cm, last } = setup();
    cm.onState(ptt(A));
    expect(last(A)?.mood).toBe('listening');
    expect(s.clock.now()).toBe(1000);
  });

  it('[UX-07] only the owner shows listening; another MOG it has seen keeps its mood', async () => {
    const { s, cm, last } = setup();
    cm.onState(ptt(B));
    cm.onState(idle);
    await s.runUntil(1000 + MOOD_HOLD_MS);
    cm.onState(ptt(A));
    expect(last(A)?.mood).toBe('listening');
    expect(last(B)?.mood).not.toBe('listening');
  });

  it('[UX-01a] [UX-05a] the owner goes back to idle when the capture stops, after the 400 ms hold', async () => {
    const { s, cm, last } = setup();
    cm.onState(ptt(A));
    s.clock.advance(100);
    cm.onState(idle);
    expect(last(A)?.mood).toBe('listening');
    await s.runUntil(1000 + MOOD_HOLD_MS);
    expect(last(A)?.mood).toBe('idle');
  });
});

describe('capture mood: a refused start is never silent (#551)', () => {
  it('[MB-08a] [CAP-01a] a refused start shows error, attention and the reason on that MOG', () => {
    const { cm, last } = setup();
    cm.refused(A, { error: 'mic_busy' });
    const v = last(A);
    expect(v?.mood).toBe('error');
    expect(v?.overlays.attention).toBe(true);
    expect(v?.badges).toEqual([
      {
        kind: 'capture_refused',
        text: REFUSAL_TEXT.mic_busy.text,
        label: REFUSAL_TEXT.mic_busy.label,
        hoverOnly: false,
      },
    ]);
    expect(last(B)).toBeUndefined();
  });

  it('[MB-08a] the reason clears with the 3 s error window', async () => {
    const { s, cm, last } = setup();
    cm.refused(A, { error: 'consent_required' });
    expect(last(A)?.badges?.[0]?.text).toBe(REFUSAL_TEXT.consent_required.text);
    await s.runUntil(1000 + ERROR_MS);
    expect(last(A)?.mood).toBe('idle');
    expect(last(A)?.badges).toBeUndefined();
  });

  it('[MB-08a] [CAP-01a] every refusal reason has a short fixed text and label', () => {
    for (const r of REFUSAL_REASONS) {
      const t = REFUSAL_TEXT[r];
      expect(t.text.length).toBeGreaterThan(0);
      expect(t.text.length).toBeLessThanOrEqual(32);
      expect(t.label.length).toBeGreaterThan(0);
    }
  });

  it('[MB-08a] [CAP-01a] maps a rejection to a fixed reason; anything unknown is host_failed', () => {
    const cases: [unknown, RefusalReason][] = [
      [{ error: 'consent_required' }, 'consent_required'],
      [{ error: 'consent_storage_failed' }, 'consent_storage_failed'],
      [{ error: 'mic_busy' }, 'mic_busy'],
      [{ error: 'host_failed' }, 'host_failed'],
      [{ error: 'invalid_mode' }, 'host_failed'],
      [new Error('secret words'), 'host_failed'],
      [undefined, 'host_failed'],
    ];
    for (const [err, want] of cases) expect(refusalReason(err)).toBe(want);
  });

  it('[CAP-17b] a refused mic open names the mic fault that capture:state reports', () => {
    for (const mic of ['permission_denied', 'no_device'] as const) {
      const { cm, last } = setup();
      cm.onState({ ...idle, mic });
      cm.refused(A, { error: 'host_failed' });
      expect(last(A)?.badges?.[0]?.text).toBe(REFUSAL_TEXT[mic].text);
    }
    // The fault can also come just after the refusal.
    const { cm, last } = setup();
    cm.refused(A, { error: 'host_failed' });
    cm.onState({ ...idle, mic: 'no_device' });
    expect(last(A)?.badges?.[0]?.text).toBe(REFUSAL_TEXT.no_device.text);
  });

  it('[UX-01a] a granted capture after a refusal shows listening and drops the reason', () => {
    const { cm, last } = setup();
    cm.refused(A, { error: 'mic_busy' });
    cm.onState(ptt(A));
    expect(last(A)?.mood).toBe('listening');
    expect(last(A)?.badges).toBeUndefined();
  });
});
