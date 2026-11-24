/**
 * #551: the MOG mood follows `capture:state`, and a refused capture start shows its reason on
 * the MOG (UX-01a, UX-05a, UX-05b, UX-07, MB-08a, CAP-01a, CAP-17b).
 */
import type { CaptureState } from '../shared/interfaces';
import { createSimScheduler } from '../test-support/sim-scheduler';
import { BLURB_REVEAL_MS, ERROR_MS, MOOD_HOLD_MS, SUCCESS_MS } from './components/mood-mapper';
import {
  HELD_NOTE_MS,
  HELD_NOTE_TEXT,
  PENDING_ASK_MS,
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

  it('[ADR-0103] [ADR-0118] a hands-free start during a recording shows "Recording in progress"', () => {
    expect(refusalReason({ error: 'recording_in_progress' })).toBe('recording_in_progress');
    expect(REFUSAL_TEXT.recording_in_progress.text).toBe('Recording in progress');
    const { cm, last } = setup();
    cm.refused(A, { error: 'recording_in_progress' });
    expect(last(A)?.badges?.[0]?.text).toBe('Recording in progress');
  });

  it('[ADR-0103] an ⌥R start with no home profile shows a plain reason', () => {
    expect(refusalReason({ error: 'no_home_profile' })).toBe('no_home_profile');
    const { cm, last } = setup();
    cm.refused(A, { error: 'no_home_profile' });
    expect(last(A)?.badges?.[0]?.text).toBe(REFUSAL_TEXT.no_home_profile.text);
  });

  it('[ADR-0103, #594 review] recordingProfile refusals (no_profile) show their own reason, not host_failed', () => {
    expect(refusalReason({ error: 'no_profile' })).toBe('no_profile');
    expect(refusalReason({ error: 'profiles_differ' })).toBe('host_failed');
    const { cm, last } = setup();
    cm.refused(A, { error: 'no_profile' });
    expect(last(A)?.badges?.[0]?.text).toBe(REFUSAL_TEXT.no_profile.text);
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
  it('[MB-08a] a second refusal within 3 s keeps its own reason for its own 3 s', async () => {
    const { s, cm, last } = setup();
    cm.refused(A, { error: 'mic_busy' });
    s.clock.advance(2000);
    cm.refused(A, { error: 'host_failed' });
    await s.runUntil(1000 + ERROR_MS + 100);
    expect(last(A)?.mood).toBe('error');
    expect(last(A)?.badges?.[0]?.text).toBe(REFUSAL_TEXT.host_failed.text);
    await s.runUntil(3000 + ERROR_MS);
    expect(last(A)?.badges).toBeUndefined();
  });

  it('[CAP-12b] [MB-08a] a repeated state of the post-roll lease keeps the mic_busy reason', () => {
    const { cm, last } = setup();
    const live = { ...ptt(A), leaseId: 'L1' };
    cm.onState(live);
    cm.refused(A, { error: 'mic_busy' });
    cm.onState(live);
    expect(last(A)?.badges?.[0]?.text).toBe(REFUSAL_TEXT.mic_busy.text);
    cm.onState({ ...ptt(A), leaseId: 'L2' });
    expect(last(A)?.badges).toBeUndefined();
  });
});

describe('capture mood: an Ask Genie request on the MOG (#573)', () => {
  it('[UX-01a] working shows processing at once and keeps it until the request ends', async () => {
    const { s, cm, last } = setup();
    cm.ask(A, 'working');
    expect(last(A)?.mood).toBe('processing');
    await s.runUntil(1000 + 60_000);
    expect(last(A)?.mood).toBe('processing');
  });

  it('[UX-01a] an answer shows answering, then success, then idle', async () => {
    const { s, cm, last } = setup();
    cm.ask(A, 'working');
    s.clock.advance(5_000);
    cm.ask(A, 'answered');
    expect(last(A)?.mood).toBe('answering');
    await s.runUntil(6_000 + BLURB_REVEAL_MS);
    expect(last(A)?.mood).toBe('success');
    await s.runUntil(6_000 + BLURB_REVEAL_MS + SUCCESS_MS);
    expect(last(A)?.mood).toBe('idle');
  });

  it('[UX-01a] a failure shows error for 3 s with no attention cue, then idle', async () => {
    const { s, cm, last } = setup();
    cm.ask(A, 'working');
    s.clock.advance(5_000);
    cm.ask(A, 'failed');
    expect(last(A)?.mood).toBe('error');
    expect(last(A)?.overlays.attention).toBe(false);
    await s.runUntil(6_000 + ERROR_MS);
    expect(last(A)?.mood).toBe('idle');
  });

  it('[UX-07] the capture still wins: a new hold shows listening during a request', () => {
    const { cm, last } = setup();
    cm.ask(A, 'working');
    cm.onState(ptt(A));
    expect(last(A)?.mood).toBe('listening');
  });

  it('a new request replaces the end timers of the old one', async () => {
    const { s, cm, last } = setup();
    cm.ask(A, 'answered');
    s.clock.advance(500);
    cm.ask(A, 'working');
    await s.runUntil(1500 + BLURB_REVEAL_MS + SUCCESS_MS);
    expect(last(A)?.mood).toBe('processing');
  });
});

describe('capture mood: the ⌥G key-up (#573)', () => {
  it('[UX-01a] pending shows processing at once; the request start keeps it', async () => {
    const { s, cm, last } = setup();
    cm.ask(A, 'pending');
    expect(last(A)?.mood).toBe('processing');
    s.clock.advance(800);
    cm.ask(A, 'working');
    await s.runUntil(1000 + PENDING_ASK_MS + 1);
    expect(last(A)?.mood).toBe('processing');
  });

  it('a key-up with no request goes back to idle after 15 s', async () => {
    const { s, cm, last } = setup();
    cm.ask(A, 'pending');
    await s.runUntil(1000 + PENDING_ASK_MS);
    expect(last(A)?.mood).toBe('idle');
  });
});

describe('capture mood: the talk key during a call recording (ADR 0118)', () => {
  const kindsOf = (v: { badges?: { kind: string }[] } | undefined) => v?.badges?.map((b) => b.kind);

  it('[AC-6.15] a hold during a recording shows "Call only", and the end of the hold clears it', () => {
    const { cm, last } = setup();
    cm.callRecording(true, [A]);
    cm.hold(A, true);
    expect(last(A)?.badges).toEqual([
      expect.objectContaining({ kind: 'call_only', text: 'Call only', hoverOnly: false }),
    ]);
    cm.hold(A, false);
    expect(last(A)?.badges).toBeUndefined();
  });

  it('[AC-6.15] a hold with no recording shows no "Call only" badge', () => {
    const { cm, last } = setup();
    cm.hold(A, true);
    expect(last(A)?.badges).toBeUndefined();
    // The recording starts during the hold: the badge follows, and goes with the recording.
    cm.callRecording(true, [A]);
    expect(kindsOf(last(A))).toEqual(['call_only']);
    cm.callRecording(false);
    expect(last(A)?.badges).toBeUndefined();
  });

  it('[AC-6.15] the badge is for the MOG of the hold only', () => {
    const { cm, last } = setup();
    cm.callRecording(true, [A, B]);
    cm.hold(A, true);
    expect(kindsOf(last(A))).toEqual(['call_only']);
    expect(last(B)?.badges).toBeUndefined();
  });

  it('[AC-6.16] the held note shows for 6 s and then goes', async () => {
    const { s, cm, last } = setup();
    cm.callRecording(true, [A]);
    cm.hold(A, true);
    cm.heldNote(A);
    expect(HELD_NOTE_TEXT).toBe('Your call can still hear you.');
    expect(last(A)?.badges?.map((b) => b.text)).toEqual(['Call only', HELD_NOTE_TEXT]);
    await s.runUntil(1000 + HELD_NOTE_MS);
    expect(kindsOf(last(A))).toEqual(['call_only']);
    cm.hold(A, false);
    expect(last(A)?.badges).toBeUndefined();
  });

  it('the end of the recording clears a waiting note and its timer', async () => {
    const { s, cm, last } = setup();
    cm.callRecording(true, [A]);
    cm.heldNote(A);
    cm.callRecording(false);
    expect(last(A)?.badges).toBeUndefined();
    // A new recording starts before the old timer would have run: no stale note comes back.
    cm.callRecording(true, [A]);
    expect(last(A)?.badges).toBeUndefined();
    await s.runUntil(1000 + HELD_NOTE_MS);
    expect(last(A)?.badges).toBeUndefined();
  });

  it('a refusal and the "Call only" badge show together', () => {
    const { cm, last } = setup();
    cm.callRecording(true, [A]);
    cm.hold(A, true);
    cm.refused(A, { error: 'recording_in_progress' });
    expect(kindsOf(last(A))).toEqual(['capture_refused', 'call_only']);
  });
});
