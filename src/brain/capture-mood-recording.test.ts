/**
 * Call recording on the MOG (ADR 0103, refs #65): the REC overlay while a call recording runs and
 * the answer-ready moment when the notes are ready, through the one mood publisher. The
 * "Recording in progress" refusal text is #594's (`recording_in_progress`).
 */
import type { CaptureState } from '../shared/interfaces';
import { createSimScheduler } from '../test-support/sim-scheduler';
import { ERROR_MS, SUCCESS_MS } from './components/mood-mapper';
import { createCaptureMood } from './capture-mood';
import { createMoodPublisher, type PublishedViewState } from './mood-publisher';

const A = '00000000-0000-4000-8000-0000000000a1';
const B = '00000000-0000-4000-8000-0000000000b2';
const idle: CaptureState = { recording: false, paused: false, othersTrack: 'off' };

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
  return { s, cm, last, sent };
}

describe('capture mood: call recording (ADR 0103)', () => {
  it('[REC-M1] every MOG it names shows REC while a call recording runs, then not', () => {
    const { cm, last } = setup();
    cm.callRecording(true, [A, B]);
    expect(last(A)?.overlays.recording).toBe(true);
    expect(last(B)?.overlays.recording).toBe(true);
    expect(last(A)?.mood).toBe('idle');
    cm.onState(idle);
    expect(last(A)?.overlays.recording).toBe(true);
    cm.callRecording(false);
    expect(last(A)?.overlays.recording).toBe(false);
    expect(last(B)?.overlays.recording).toBe(false);
  });

  it('[REC-M2] the recording capture owner bottle-recording never shows as a MOG', () => {
    const { cm, sent } = setup();
    cm.onState({
      owner: 'bottle-recording',
      mode: 'session',
      recording: true,
      paused: false,
      othersTrack: 'off',
    });
    cm.callRecording(true, [A]);
    expect(sent.some((v) => v.mogId === 'bottle-recording')).toBe(false);
  });

  it('[REC-M3] ready notes show the answer-ready moment (success) on every known MOG', async () => {
    const { s, cm, last } = setup();
    cm.callRecording(true, [A]);
    cm.callRecording(false);
    s.clock.advance(ERROR_MS);
    cm.recordingReady();
    expect(last(A)?.mood).toBe('success');
    await s.runUntil(s.clock.now() + SUCCESS_MS + 500);
    cm.onState(idle);
    expect(last(A)?.mood).toBe('idle');
  });
});
