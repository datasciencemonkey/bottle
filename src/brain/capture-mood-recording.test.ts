/**
 * Call recording on the MOG (ADR 0103, refs #65): the REC overlay while a call recording runs,
 * "Recording in progress" on a talk-key hold that the core refuses, and the answer-ready moment
 * when the notes are ready. All through the one mood publisher.
 */
import type { CaptureState } from '../shared/interfaces';
import { createSimScheduler } from '../test-support/sim-scheduler';
import { ERROR_MS, SUCCESS_MS } from './components/mood-mapper';
import { RECORDING_IN_PROGRESS, REFUSAL_TEXT, createCaptureMood } from './capture-mood';
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
  return { s, cm, last };
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

  it('[REC-M2] a refused talk-key hold during a recording says "Recording in progress"', () => {
    const { cm, last } = setup();
    cm.callRecording(true, [A]);
    cm.refused(A, { error: 'mic_busy' });
    expect(last(A)?.badges?.[0]).toMatchObject({
      kind: 'capture_refused',
      ...RECORDING_IN_PROGRESS,
    });
    expect(RECORDING_IN_PROGRESS.text).toBe('Recording in progress');
  });

  it('[REC-M2] without a recording, mic_busy keeps its own text', () => {
    const { cm, last } = setup();
    cm.refused(A, { error: 'mic_busy' });
    expect(last(A)?.badges?.[0]).toMatchObject(REFUSAL_TEXT.mic_busy);
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
