/**
 * The brain side of call recording in the hub (ADR 0103, refs #65). A fake `RecordingApi` only:
 * no capture, no network and no token exist here.
 */
import {
  DEFAULT_NOTES_ENDPOINT,
  DEFAULT_TRANSCRIBE_ENDPOINT,
  DEFAULT_INSTRUCTIONS,
  HUB_RECORDING_DETAIL_EVENT,
  HUB_RECORDING_EVENT,
  HUB_RECORDINGS_EVENT,
  RECORDING_ERRORS,
  RECORDING_TEXT,
  RECORDING_VIEW_ID,
  SYSTEM_AUDIO_HINT,
  SYSTEM_AUDIO_SETTINGS_OPEN_FAILED,
  type HubRecording,
  type HubRecordingDetail,
  type HubRecordings,
} from '../shared/recording-view';
import { BrainBus } from './bus';
import { core } from './test-core';
import {
  createHeldNote,
  ipcRecordingApi,
  recSettingsOf,
  settingsPatchOf,
  turnsOf,
  wireRecording,
  SYSTEM_AUDIO_HINT_MS,
  type RecDetail,
  type RecJob,
  type RecSettings,
  type RecState,
  type RecSummary,
  type RecordingApi,
} from './recording-wiring';
import { createSimScheduler, type SimScheduler } from '../test-support/sim-scheduler';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const flush = async () => {
  for (let i = 0; i < 5; i += 1) await new Promise((r) => setTimeout(r, 0));
};

const MY_FILES = '/Volumes/databricks/home/1234/someone@example.com/my_files';

function settings(over: Partial<RecSettings> = {}): RecSettings {
  return {
    enabled: false,
    consentAck: false,
    lengthMin: 60,
    transcribeEndpoint: DEFAULT_TRANSCRIBE_ENDPOINT,
    notesEndpoint: DEFAULT_NOTES_ENDPOINT,
    instructions: DEFAULT_INSTRUCTIONS,
    ...over,
  };
}

function fakeApi(start: Partial<RecSettings> = {}) {
  let s = settings(start);
  let stateCb: ((s: RecState) => void) | undefined;
  let jobCb: ((j: RecJob) => void) | undefined;
  const rows: RecSummary[] = [
    { id: 'r1', name: '2026-11-18_0900', startedAt: 1000, durationS: 1500, status: 'done' },
    {
      id: 'r2',
      name: '2026-11-19_1000',
      startedAt: 5000,
      durationS: 3600,
      status: 'failed',
      errorKind: 'transcribe_endpoint_missing',
    },
    { id: 'r3', name: '2026-11-19_1100', startedAt: 9000, durationS: 600, status: 'transcribing' },
  ];
  const api = {
    readSettings: vi.fn(async () => s),
    writeSettings: vi.fn(async (next: RecSettings) => {
      s = next;
      return s;
    }),
    markHeldNoteShown: vi.fn(async () => {
      s = { ...s, heldNoteShown: true };
    }),
    probeStorage: vi.fn(async (): Promise<{ myFiles: string | null }> => ({ myFiles: MY_FILES })),
    createVolume: vi.fn(async (c: string, sc: string) => ({
      path: `/Volumes/${c}/${sc}/bottle_audio`,
    })),
    start: vi.fn(async (_i?: string): Promise<RecState> => ({
      phase: 'recording',
      startedAtMs: 100,
      endsAtMs: 3_600_100,
    })),
    stop: vi.fn(async (): Promise<RecState> => ({ phase: 'stopping' })),
    state: vi.fn(async (): Promise<RecState> => ({ phase: 'idle' })),
    list: vi.fn(async () => rows),
    detail: vi.fn(async (id: string): Promise<RecDetail> => ({
      id,
      status: 'done',
      notesMd: '## Summary\nWe met.',
      segments: [
        { start: 0, end: 4, speaker: 'Speaker 1', text: 'Hello.' },
        { start: 4, end: 9, speaker: 'Speaker 1', text: 'Shall we start?' },
        { start: 9, end: 12, speaker: 'Speaker 2', text: 'Yes.' },
      ],
    })),
    retry: vi.fn(async (_id: string) => undefined),
    delete: vi.fn(async (_id: string) => undefined),
    open: vi.fn(async (_id: string) => undefined),
    openSystemAudioSettings: vi.fn(async () => undefined),
    onState: (cb: (s: RecState) => void) => {
      stateCb = cb;
      return () => (stateCb = undefined);
    },
    onJob: (cb: (j: RecJob) => void) => {
      jobCb = cb;
      return () => (jobCb = undefined);
    },
  } satisfies RecordingApi;
  return {
    api,
    rows,
    current: () => s,
    pushState: (st: RecState) => stateCb?.(st),
    pushJob: (j: RecJob) => jobCb?.(j),
  };
}

function setup(start: Partial<RecSettings> = {}, scheduler?: SimScheduler) {
  const bus = new BrainBus();
  const fake = fakeApi(start);
  const sent: { event: string; payload: unknown }[] = [];
  const mood = { callRecording: vi.fn(), ready: vi.fn() };
  const notify = vi.fn();
  const off = wireRecording(bus, {
    api: fake.api,
    platform: 'mac',
    send: async (event, payload) => {
      sent.push({ event, payload });
    },
    mood,
    notify,
    ...(scheduler ? { clock: scheduler.clock, schedule: scheduler.at } : {}),
  });
  const click = (payload: Record<string, unknown>, from = 'hub') =>
    bus.dispatch({
      event: 'view:input',
      payload: { from, input: { mogId: RECORDING_VIEW_ID, kind: 'click', payload } },
    });
  const last = <T>(event: string): T | undefined =>
    sent.filter((s) => s.event === event).at(-1)?.payload as T | undefined;
  return { bus, fake, sent, mood, notify, off, click, last };
}

describe('recording wiring (brain, ADR 0103)', () => {
  it('[REC-H1] on recording.ready sends the settings, the My Files storage and the state', async () => {
    const t = setup();
    t.click({ action: 'recording.ready' });
    await flush();
    const m = t.last<HubRecording>(HUB_RECORDING_EVENT);
    expect(m?.platform).toBe('mac');
    expect(m?.settings).toEqual({
      enabled: false,
      consentAck: false,
      lengthMin: 60,
      transcribeEndpoint: DEFAULT_TRANSCRIBE_ENDPOINT,
      notesEndpoint: DEFAULT_NOTES_ENDPOINT,
      instructions: DEFAULT_INSTRUCTIONS,
    });
    expect(m?.storage).toEqual({ status: 'my_files', path: MY_FILES });
    expect(m?.control).toEqual({ phase: 'idle' });
    // ADR 0103 update 4: no compute setting of any kind.
    expect(JSON.stringify(m)).not.toMatch(/warehouse|serverless|compute/i);
  });

  it('[REC-H1] ignores input from a window other than the hub', async () => {
    const t = setup();
    t.click({ action: 'recording.ready' }, 'mog-x');
    t.click({ action: 'recording.start' }, 'card-x');
    await flush();
    expect(t.sent).toEqual([]);
    expect(t.fake.api.start).not.toHaveBeenCalled();
  });

  it('[REC-H2] the knob stays off until the consent notice is acknowledged', async () => {
    const t = setup();
    t.click({ action: 'recording.ready' });
    await flush();
    t.click({ action: 'recording.enable', on: true });
    await flush();
    expect(t.fake.current().enabled).toBe(false);
    t.click({ action: 'recording.consent_ack' });
    await flush();
    t.click({ action: 'recording.enable', on: true });
    await flush();
    expect(t.fake.current()).toMatchObject({ enabled: true, consentAck: true });
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.settings?.enabled).toBe(true);
  });

  it('[REC-H2] the acknowledgement turns the knob on in one write, also with a click right after it', async () => {
    const t = setup();
    t.click({ action: 'recording.ready' });
    await flush();
    t.click({ action: 'recording.consent_ack' });
    t.click({ action: 'recording.enable', on: true });
    await flush();
    expect(t.fake.current()).toMatchObject({ enabled: true, consentAck: true });
    expect(t.fake.api.writeSettings).toHaveBeenCalledWith(
      expect.objectContaining({ consentAck: true, enabled: true }),
    );
  });

  it('[REC-H1] with no home profile, storage and the list say why in plain words', async () => {
    const t = setup();
    t.fake.api.probeStorage.mockRejectedValue({ error: 'no_profile' });
    t.fake.api.list.mockRejectedValue({ error: 'no_profile' });
    t.click({ action: 'recording.ready' });
    t.click({ action: 'recordings.ready' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.storage).toEqual({
      status: 'unavailable',
      error: RECORDING_ERRORS.no_profile.text,
    });
    expect(t.last<HubRecordings>(HUB_RECORDINGS_EVENT)).toEqual({
      rows: [],
      unavailable: true,
      error: RECORDING_ERRORS.no_profile,
    });
  });

  it('[REC-H11] a failed Retry or Open shows its plain text', async () => {
    const t = setup();
    t.fake.api.retry.mockRejectedValue({ error: 'network' });
    t.click({ action: 'recordings.retry', id: 'r2' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(RECORDING_ERRORS.network);
    t.fake.api.open.mockRejectedValue({ error: 'no_profile' });
    t.click({ action: 'recordings.open', id: 'r1' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(RECORDING_ERRORS.no_profile);
  });

  it('[REC-H3] keeps a length from 5 to 120 minutes and refuses others', async () => {
    const t = setup();
    t.click({ action: 'recording.length', minutes: 90 });
    await flush();
    expect(t.fake.current().lengthMin).toBe(90);
    t.click({ action: 'recording.length', minutes: 200 });
    t.click({ action: 'recording.length', minutes: 2.5 });
    await flush();
    expect(t.fake.current().lengthMin).toBe(90);
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.settingsError).toBe(RECORDING_TEXT.badLength);
  });

  it('[REC-H4] keeps valid transcribe and notes model names and the instructions; refuses a bad name', async () => {
    const t = setup();
    t.click({ action: 'recording.endpoint', which: 'transcribe', endpoint: 'my-transcriber' });
    t.click({ action: 'recording.endpoint', which: 'notes', endpoint: 'my-endpoint_2' });
    t.click({ action: 'recording.instructions', text: 'Only action items.' });
    await flush();
    expect(t.fake.current()).toMatchObject({
      transcribeEndpoint: 'my-transcriber',
      notesEndpoint: 'my-endpoint_2',
      instructions: 'Only action items.',
    });
    t.click({ action: 'recording.endpoint', which: 'notes', endpoint: "x'); DROP" });
    await flush();
    expect(t.fake.current().notesEndpoint).toBe('my-endpoint_2');
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.settingsError).toBe(
      RECORDING_TEXT.badEndpoint,
    );
  });

  it('[REC-H5] without My Files: an existing volume path is kept; a bad path is refused', async () => {
    const t = setup();
    t.fake.api.probeStorage.mockResolvedValue({ myFiles: null });
    t.click({ action: 'recording.ready' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.storage).toEqual({ status: 'unavailable' });
    t.click({ action: 'recording.storage_volume', path: '/Volumes/main/team/audio' });
    await flush();
    expect(t.fake.current().volumePath).toBe('/Volumes/main/team/audio');
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.storage).toEqual({
      status: 'volume',
      path: '/Volumes/main/team/audio',
    });
    t.click({ action: 'recording.storage_volume', path: '/tmp/../etc' });
    await flush();
    expect(t.fake.current().volumePath).toBe('/Volumes/main/team/audio');
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.storage).toEqual({
      status: 'unavailable',
      error: RECORDING_TEXT.badVolume,
    });
  });

  it('[REC-H5] creates bottle_audio only on the confirmed action, then keeps its path', async () => {
    const t = setup();
    t.fake.api.probeStorage.mockResolvedValue({ myFiles: null });
    t.click({ action: 'recording.ready' });
    await flush();
    expect(t.fake.api.createVolume).not.toHaveBeenCalled();
    t.click({ action: 'recording.volume_create', catalog: 'main', schema: 'bad schema' });
    await flush();
    expect(t.fake.api.createVolume).not.toHaveBeenCalled();
    t.click({ action: 'recording.volume_create', catalog: 'main', schema: 'team' });
    await flush();
    expect(t.fake.api.createVolume).toHaveBeenCalledWith('main', 'team');
    expect(t.fake.current().volumePath).toBe('/Volumes/main/team/bottle_audio');
  });

  it('[REC-H5] a failed volume create shows a plain text and changes nothing', async () => {
    const t = setup();
    t.fake.api.probeStorage.mockResolvedValue({ myFiles: null });
    t.fake.api.createVolume.mockRejectedValue({ error: 'permission_denied' });
    t.click({ action: 'recording.volume_create', catalog: 'main', schema: 'team' });
    await flush();
    expect(t.fake.current().volumePath).toBeUndefined();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.storage).toEqual({
      status: 'unavailable',
      error: RECORDING_TEXT.volumeCreateFailed,
    });
  });

  it('[REC-H7] Record sends only edited instructions, shows REC with ends_at_ms, and the MOG shows it', async () => {
    const t = setup({ enabled: true, consentAck: true });
    t.click({ action: 'recording.ready' });
    await flush();
    t.click({ action: 'recording.start', instructions: DEFAULT_INSTRUCTIONS });
    await flush();
    expect(t.fake.api.start).toHaveBeenLastCalledWith(undefined);
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.control).toEqual({
      phase: 'recording',
      startedAtMs: 100,
      endsAtMs: 3_600_100,
    });
    expect(t.mood.callRecording).toHaveBeenLastCalledWith(true);
    t.fake.pushState({ phase: 'idle' });
    t.click({ action: 'recording.start', instructions: '  Just decisions.  ' });
    await flush();
    expect(t.fake.api.start).toHaveBeenLastCalledWith('Just decisions.');
  });

  it('[REC-H7] Stop stops early; the idle state clears the MOG REC', async () => {
    const t = setup({ enabled: true, consentAck: true });
    t.click({ action: 'recording.start' });
    await flush();
    t.click({ action: 'recording.stop' });
    await flush();
    expect(t.fake.api.stop).toHaveBeenCalledTimes(1);
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.control.phase).toBe('stopping');
    t.fake.pushState({ phase: 'idle' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.control).toEqual({ phase: 'idle' });
    expect(t.mood.callRecording).toHaveBeenLastCalledWith(false);
  });

  it('[REC-H9] a slow, older list reply never overwrites a newer one', async () => {
    const t = setup({ enabled: true, consentAck: true });
    let release: (v: RecSummary[]) => void = () => undefined;
    const old = { id: 'old', name: '2026-11-24_1600', startedAt: 1, durationS: 0 };
    t.fake.api.list
      .mockImplementationOnce(
        () =>
          new Promise<RecSummary[]>((r) => {
            release = r;
          }),
      )
      .mockImplementationOnce(async () => [
        { id: 'new', name: '2026-11-24_1741', startedAt: 2, durationS: 0, status: 'uploading' },
      ]);
    t.click({ action: 'recordings.ready' });
    await flush();
    t.click({ action: 'recordings.ready' });
    await flush();
    release([{ ...old, status: 'recording' }]);
    await flush();
    const rows = t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.rows ?? [];
    expect(rows.map((r) => r.id)).toEqual(['new']);
  });

  it('[REC-H9] start and stop each read the list at once, so the new row shows with no job event', async () => {
    const t = setup({ enabled: true, consentAck: true });
    // The record key starts it (no hub click): only recording.state reaches the brain.
    // "4" of the 2026-11-26 round: the start itself now reads the list too, not only the stop.
    t.fake.pushState({
      phase: 'recording',
      startedAtMs: 100,
      endsAtMs: 3_600_100,
      name: '2026-11-24_1741',
    });
    await flush();
    expect(t.fake.api.list).toHaveBeenCalledTimes(1);
    t.fake.rows.push({
      id: '2026-11-24_1741',
      name: '2026-11-24_1741',
      startedAt: 20_000,
      durationS: 0,
      status: 'uploading',
    });
    t.fake.pushState({ phase: 'idle' });
    await flush();
    expect(t.fake.api.list).toHaveBeenCalledTimes(2);
    const rows = t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.rows ?? [];
    expect(rows[0]).toMatchObject({ id: '2026-11-24_1741', status: 'uploading' });
  });

  it('[REC-H9] a missed job event is caught up by a bounded poll while a take is in flight', async () => {
    // "A" review 2026-11-25/26: `recording.job` is not sticky and nothing else polls, so if
    // the next status-change event never reaches the brain, the row must still catch up on
    // its own instead of sitting on "Uploading…" forever.
    const s = createSimScheduler();
    const t = setup({ enabled: true, consentAck: true }, s);
    // A controlled list: only this one take, so the fixture's own in-flight row never muddies
    // the catch-up assertions below.
    t.fake.rows.length = 0;
    t.fake.rows.push({
      id: '2026-11-25_0628',
      name: '2026-11-25_0628',
      startedAt: 100,
      durationS: 19,
      status: 'uploading',
    });
    t.fake.pushState({
      phase: 'recording',
      startedAtMs: 100,
      endsAtMs: 3_600_100,
      name: '2026-11-25_0628',
    });
    await flush();
    t.fake.pushState({ phase: 'idle' });
    await flush();
    const readsAfterStop = t.fake.api.list.mock.calls.length;
    expect(t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.rows).toMatchObject([
      { id: '2026-11-25_0628', status: 'uploading' },
    ]);
    // The take finishes server-side, but no job event ever tells the brain.
    const row = t.fake.rows.find((r) => r.id === '2026-11-25_0628');
    if (row) row.status = 'done';
    await s.runUntil(10_000, { settle: true });
    await flush();
    expect(t.fake.api.list.mock.calls.length).toBeGreaterThan(readsAfterStop);
    expect(t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.rows).toMatchObject([
      { id: '2026-11-25_0628', status: 'done' },
    ]);
  });

  it('[REC-H9] the catch-up poll stops once every row is at a terminal status', async () => {
    const s = createSimScheduler();
    const t = setup({ enabled: true, consentAck: true }, s);
    t.fake.rows.length = 0;
    t.fake.rows.push({
      id: '2026-11-25_0628',
      name: '2026-11-25_0628',
      startedAt: 100,
      durationS: 19,
      status: 'done',
    });
    t.fake.pushState({
      phase: 'recording',
      startedAtMs: 100,
      endsAtMs: 3_600_100,
      name: '2026-11-25_0628',
    });
    await flush();
    t.fake.pushState({ phase: 'idle' });
    await flush();
    const reads = t.fake.api.list.mock.calls.length;
    await s.runUntil(60_000, { settle: true });
    await flush();
    // Already done at the one read the stop triggers: no catch-up poll is ever armed.
    expect(t.fake.api.list.mock.calls.length).toBe(reads);
  });

  it('[REC-H9] the live take alone never arms the catch-up poll ("6")', async () => {
    const s = createSimScheduler();
    const t = setup({ enabled: true, consentAck: true }, s);
    t.fake.rows.length = 0;
    t.fake.rows.push({
      id: '2026-11-25_0628',
      name: '2026-11-25_0628',
      startedAt: 100,
      durationS: 0,
      status: 'recording',
    });
    t.fake.pushState({
      phase: 'recording',
      startedAtMs: 100,
      endsAtMs: 3_600_100,
      name: '2026-11-25_0628',
    });
    await flush();
    const reads = t.fake.api.list.mock.calls.length;
    await s.runUntil(60_000, { settle: true });
    await flush();
    // The take still recording is expected to stay `recording` for the whole call: that
    // alone is not something stalled, so it must never by itself arm a poll.
    expect(t.fake.api.list.mock.calls.length).toBe(reads);
  });

  it('[REC-H9] a list that rejects is retried, with the backoff resetting once it recovers ("6")', async () => {
    const s = createSimScheduler();
    const t = setup({ enabled: true, consentAck: true }, s);
    // A first, successful read, so there is something to keep when the next one fails.
    t.click({ action: 'recordings.ready' });
    await flush();
    expect(t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.rows).toHaveLength(3);
    t.fake.api.list.mockRejectedValueOnce({ error: 'offline' });
    t.click({ action: 'recordings.ready' });
    await flush();
    expect(t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.unavailable).toBe(true);
    // The last known rows (the fixture's three) are kept, never blanked to `[]` ("3").
    expect(t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.rows).toHaveLength(3);
    const readsAfterFailure = t.fake.api.list.mock.calls.length;
    // The one retry ("6") fires on its own, with no further click.
    await s.runUntil(10_000, { settle: true });
    await flush();
    expect(t.fake.api.list.mock.calls.length).toBeGreaterThan(readsAfterFailure);
    expect(t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.unavailable).toBeUndefined();
  });

  it('[REC-H9] repeated list failures back off up to the cap ("6")', async () => {
    const s = createSimScheduler();
    const t = setup({ enabled: true, consentAck: true }, s);
    t.fake.api.list.mockRejectedValue({ error: 'offline' });
    t.click({ action: 'recordings.ready' });
    await flush();
    const firstRetryAt = s.clock.now();
    await s.runUntil(4_000, { settle: true });
    await flush();
    const afterFirstRetry = t.fake.api.list.mock.calls.length;
    expect(afterFirstRetry).toBeGreaterThan(1);
    // The next retry does not fire again within the base delay: it backed off past it.
    await s.runUntil(firstRetryAt + 4_000, { settle: true });
    await flush();
    expect(t.fake.api.list.mock.calls.length).toBe(afterFirstRetry);
  });

  it('[REC-D] a failed detail fetch for a live take still shows a plain fallback ("4")', async () => {
    const t = setup({ enabled: true, consentAck: true });
    t.fake.pushState({
      phase: 'recording',
      startedAtMs: 100,
      endsAtMs: 3_600_100,
      name: '2026-11-25_0628',
    });
    await flush();
    // The first `recording.job` event is when `detail` is finally fetched for this id ("4");
    // make that one fetch reject, as it genuinely can for a take whose files are not all in
    // the volume yet.
    t.fake.api.detail.mockRejectedValueOnce(new Error('not found'));
    t.fake.rows.push({
      id: '2026-11-25_0628',
      name: '2026-11-25_0628',
      startedAt: 100,
      durationS: 0,
      status: 'uploading',
    });
    t.fake.pushJob({ id: '2026-11-25_0628', status: 'uploading' });
    await flush();
    expect(t.last<HubRecordingDetail>(HUB_RECORDING_DETAIL_EVENT)).toMatchObject({
      id: '2026-11-25_0628',
      status: 'uploading',
      turns: [],
      unavailable: true,
    });
  });

  it('[REC-D] a detail row picked by the user during the call survives the stop ("5")', async () => {
    const t = setup({ enabled: true, consentAck: true });
    t.fake.pushState({
      phase: 'recording',
      startedAtMs: 100,
      endsAtMs: 3_600_100,
      name: '2026-11-25_0628',
    });
    await flush();
    expect(t.last<HubRecordingDetail>(HUB_RECORDING_DETAIL_EVENT)?.id).toBe('2026-11-25_0628');
    // The user looks at a different, unrelated recording while the call still runs.
    t.click({ action: 'recordings.detail', id: 'r1' });
    await flush();
    expect(t.fake.api.detail).toHaveBeenLastCalledWith('r1');
    expect(t.last<HubRecordingDetail>(HUB_RECORDING_DETAIL_EVENT)?.id).toBe('r1');
    const detailCallsBeforeStop = t.fake.api.detail.mock.calls.length;
    t.fake.pushState({ phase: 'idle' });
    await flush();
    // The stop must not pull the user back to the take that just finished: their own pick
    // stands, and no extra `detail` fetch runs for the recording either.
    expect(t.last<HubRecordingDetail>(HUB_RECORDING_DETAIL_EVENT)?.id).toBe('r1');
    expect(t.fake.api.detail.mock.calls.length).toBe(detailCallsBeforeStop);
  });

  it('[REC-D] a new take becomes the selected row when it starts, and stays selected when it stops', async () => {
    const t = setup({ enabled: true, consentAck: true });
    t.fake.pushState({
      phase: 'recording',
      startedAtMs: 100,
      endsAtMs: 3_600_100,
      name: '2026-11-25_0628',
    });
    await flush();
    // "4" of the 2026-11-26 round: selected at once, but no `detail` call yet for a take
    // that is still recording (nothing to show: no transcript, no notes).
    expect(t.fake.api.detail).not.toHaveBeenCalled();
    expect(t.last<HubRecordingDetail>(HUB_RECORDING_DETAIL_EVENT)).toMatchObject({
      id: '2026-11-25_0628',
      status: 'recording',
      turns: [],
    });
    t.fake.pushState({ phase: 'idle' });
    await flush();
    // At the stop, `detail` is finally fetched for real.
    expect(t.fake.api.detail).toHaveBeenLastCalledWith('2026-11-25_0628');
  });

  it('[REC-H8] a refused start shows its plain text with one action', async () => {
    const t = setup({ enabled: true, consentAck: true });
    t.fake.api.start.mockRejectedValue({ error: 'recording_busy' });
    t.click({ action: 'recording.start' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(
      RECORDING_ERRORS.recording_busy,
    );
    t.fake.api.start.mockRejectedValue({ error: 'some upstream text' });
    t.click({ action: 'recording.start' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(RECORDING_ERRORS.failed);
  });

  it('[REC-H8] a short system audio silence shows no alert at all (#593 follow-up)', async () => {
    const s = createSimScheduler();
    const t = setup({ enabled: true, consentAck: true }, s);
    t.click({ action: 'recording.ready' });
    await flush();
    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'silent_or_denied' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toBeUndefined();
    await s.runUntil(SYSTEM_AUDIO_HINT_MS - 1);
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toBeUndefined();
  });

  it('[REC-H8] a long system audio silence shows a calm status hint, not a red alert', async () => {
    const s = createSimScheduler();
    const t = setup({ enabled: true, consentAck: true }, s);
    t.click({ action: 'recording.ready' });
    await flush();
    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'silent_or_denied' });
    await flush();
    await s.runUntil(SYSTEM_AUDIO_HINT_MS);
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(SYSTEM_AUDIO_HINT);
    expect(SYSTEM_AUDIO_HINT.role).toBe('status');
    t.click({ action: 'recording.system_audio' });
    await flush();
    expect(t.fake.api.openSystemAudioSettings).toHaveBeenCalledTimes(1);
  });

  it('[REC-H8] sound before the hint cancels it; sound after it clears it', async () => {
    const s = createSimScheduler();
    const t = setup({ enabled: true, consentAck: true }, s);
    t.click({ action: 'recording.ready' });
    await flush();
    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'silent_or_denied' });
    await flush();
    await s.runUntil(SYSTEM_AUDIO_HINT_MS / 2);
    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'ok' });
    await flush();
    await s.runUntil(SYSTEM_AUDIO_HINT_MS * 2);
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toBeUndefined();

    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'silent_or_denied' });
    await flush();
    await s.runUntil(SYSTEM_AUDIO_HINT_MS * 3);
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(SYSTEM_AUDIO_HINT);

    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'ok' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toBeUndefined();
  });

  it('[REC-H8] the hint timer is dropped once the recording ends', async () => {
    const s = createSimScheduler();
    const t = setup({ enabled: true, consentAck: true }, s);
    t.click({ action: 'recording.ready' });
    await flush();
    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'silent_or_denied' });
    await flush();
    t.fake.pushState({ phase: 'idle' });
    await flush();
    await s.runUntil(SYSTEM_AUDIO_HINT_MS * 2);
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toBeUndefined();
  });

  it('[REC-H8] a real error already showing is not overwritten by the hint timer', async () => {
    const s = createSimScheduler();
    const t = setup({ enabled: true, consentAck: true }, s);
    t.click({ action: 'recording.ready' });
    await flush();
    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'silent_or_denied' });
    await flush();
    // A real error shows up before the hint timer fires.
    t.fake.api.retry.mockRejectedValue({ error: 'network' });
    t.click({ action: 'recordings.retry', id: 'r2' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(RECORDING_ERRORS.network);
    await s.runUntil(SYSTEM_AUDIO_HINT_MS);
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(RECORDING_ERRORS.network);
  });

  it('[REC-H8] the hint is not lost for good: it shows once the blocking error clears', async () => {
    const s = createSimScheduler();
    const t = setup({ enabled: true, consentAck: true }, s);
    t.click({ action: 'recording.ready' });
    await flush();
    t.click({ action: 'recording.start' });
    await flush();
    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'silent_or_denied' });
    await flush();
    t.fake.api.retry.mockRejectedValue({ error: 'network' });
    t.click({ action: 'recordings.retry', id: 'r2' });
    await flush();
    await s.runUntil(SYSTEM_AUDIO_HINT_MS);
    await flush();
    // Blocked: the unrelated real error wins over the hint (not shown), but the hint is not
    // discarded either.
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(RECORDING_ERRORS.network);

    // The recording ends; the error is untouched (it has nothing to do with system audio).
    t.fake.pushState({ phase: 'idle' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(RECORDING_ERRORS.network);
    // A later `recording.start` clears the stale error at once.
    t.click({ action: 'recording.start' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toBeUndefined();
    // Once silence lasts past the hint window again, with nothing left blocking it, the hint
    // shows: the earlier block never lost it for good.
    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'silent_or_denied' });
    await flush();
    await s.runUntil(s.clock.now() + SYSTEM_AUDIO_HINT_MS);
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(SYSTEM_AUDIO_HINT);
  });

  it('[REC-H8] sound arriving clears a failed Open System Settings notice too', async () => {
    const t = setup({ enabled: true, consentAck: true });
    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'silent_or_denied' });
    await flush();
    t.fake.api.openSystemAudioSettings.mockRejectedValue({ error: 'host_failed' });
    t.click({ action: 'recording.system_audio' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(
      SYSTEM_AUDIO_SETTINGS_OPEN_FAILED,
    );
    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'ok' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toBeUndefined();
  });

  it('[REC-H8] a real system_audio_denied error keeps its own red alert meaning', async () => {
    const t = setup({ enabled: true, consentAck: true });
    t.fake.api.retry.mockRejectedValue({ error: 'system_audio_denied' });
    t.click({ action: 'recordings.retry', id: 'r2' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(
      RECORDING_ERRORS.system_audio_denied,
    );
  });

  it('[REC-H8] a failed Open System Settings shows plain steps with no action, never nothing', async () => {
    const t = setup({ enabled: true, consentAck: true });
    t.fake.api.openSystemAudioSettings.mockRejectedValue({ error: 'host_failed' });
    t.click({ action: 'recording.system_audio' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(
      SYSTEM_AUDIO_SETTINGS_OPEN_FAILED,
    );
  });

  it('[REC-H9] the Recordings list is newest first; a failed row has its plain text', async () => {
    const t = setup();
    t.click({ action: 'recordings.ready' });
    await flush();
    const l = t.last<HubRecordings>(HUB_RECORDINGS_EVENT);
    expect(l?.rows.map((r) => r.id)).toEqual(['r3', 'r2', 'r1']);
    expect(l?.rows[1]?.error).toEqual(RECORDING_ERRORS.transcribe_endpoint_missing);
    expect(l?.rows[0]?.error).toBeUndefined();
  });

  it('[REC-H9] a row can open in Databricks only once its audio is in the volume', async () => {
    const t = setup();
    t.fake.api.list.mockResolvedValue([
      { id: 'a', name: 'a', startedAt: 6, durationS: 1, status: 'recording' },
      { id: 'b', name: 'b', startedAt: 5, durationS: 1, status: 'uploading' },
      { id: 'c', name: 'c', startedAt: 4, durationS: 1, status: 'transcribing' },
      { id: 'd', name: 'd', startedAt: 3, durationS: 1, status: 'done' },
      {
        id: 'e',
        name: 'e',
        startedAt: 2,
        durationS: 1,
        status: 'failed',
        errorKind: 'upload_failed',
        errorStage: 'uploading',
      },
      {
        id: 'f',
        name: 'f',
        startedAt: 1,
        durationS: 1,
        status: 'failed',
        errorKind: 'notes_endpoint_missing',
        errorStage: 'writing_notes',
      },
    ]);
    t.click({ action: 'recordings.ready' });
    await flush();
    const rows = t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.rows ?? [];
    expect(rows.map((r) => [r.id, r.canOpen])).toEqual([
      ['a', false],
      ['b', false],
      ['c', true],
      ['d', true],
      ['e', false],
      ['f', true],
    ]);
  });

  it('[REC-H9] a failed list read sends an empty list marked unavailable', async () => {
    const t = setup();
    t.fake.api.list.mockRejectedValue({ error: 'offline' });
    t.click({ action: 'recordings.ready' });
    await flush();
    expect(t.last<HubRecordings>(HUB_RECORDINGS_EVENT)).toEqual({ rows: [], unavailable: true });
  });

  it('[REC-H10] the detail merges the segments of one speaker into turns', async () => {
    const t = setup();
    t.click({ action: 'recordings.detail', id: 'r1' });
    await flush();
    const d = t.last<HubRecordingDetail>(HUB_RECORDING_DETAIL_EVENT);
    expect(d).toEqual({
      id: 'r1',
      status: 'done',
      notesMd: '## Summary\nWe met.',
      turns: [
        { speaker: 'Speaker 1', startS: 0, text: 'Hello. Shall we start?' },
        { speaker: 'Speaker 2', startS: 9, text: 'Yes.' },
      ],
    });
  });

  it('[REC-H10] turnsOf skips empty text and keeps order by time', () => {
    expect(
      turnsOf([
        { start: 5, end: 6, speaker: '2', text: 'B' },
        { start: 0, end: 1, speaker: '1', text: 'A' },
        { start: 1, end: 2, speaker: '1', text: '   ' },
      ]),
    ).toEqual([
      { speaker: '1', startS: 0, text: 'A' },
      { speaker: '2', startS: 5, text: 'B' },
    ]);
  });

  it('[REC-H11] Retry and Open in Databricks go to the core with the id; no URL goes to the hub', async () => {
    const t = setup();
    t.click({ action: 'recordings.retry', id: 'r2' });
    t.click({ action: 'recordings.open', id: 'r1' });
    await flush();
    expect(t.fake.api.retry).toHaveBeenCalledWith('r2');
    expect(t.fake.api.open).toHaveBeenCalledWith('r1');
    expect(JSON.stringify(t.sent)).not.toMatch(/https?:/);
  });

  it('[REC-H11] Delete goes to the core with the id, then the list is read again', async () => {
    const t = setup();
    t.click({ action: 'recordings.ready' });
    await flush();
    const before = t.fake.api.list.mock.calls.length;
    t.click({ action: 'recordings.delete', id: 'r2' });
    await flush();
    expect(t.fake.api.delete).toHaveBeenCalledWith('r2');
    expect(t.fake.api.list.mock.calls.length).toBeGreaterThan(before);
    expect(t.last<HubRecordings>(HUB_RECORDINGS_EVENT)).toBeDefined();
  });

  it('[REC-H11] a refused Delete shows its plain text and sends nothing else', async () => {
    const t = setup();
    t.fake.api.delete.mockRejectedValue({ error: 'recording_busy' });
    t.click({ action: 'recordings.delete', id: 'r2' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toBeDefined();
  });

  it('[REC-H12] a job that finishes notifies once and shows the MOG answer-ready moment', async () => {
    const t = setup();
    t.click({ action: 'recordings.ready' });
    await flush();
    const r3 = t.fake.rows.find((r) => r.id === 'r3');
    if (r3) r3.status = 'done';
    t.fake.pushJob({ id: 'r3', status: 'done' });
    await flush();
    expect(t.notify).toHaveBeenCalledTimes(1);
    expect(t.notify.mock.calls[0]?.[0]).toMatch(/notes are ready/i);
    expect(t.mood.ready).toHaveBeenCalledTimes(1);
    expect(t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.rows[0]?.status).toBe('done');
    t.fake.pushJob({ id: 'r3', status: 'done' });
    t.fake.pushJob({ id: 'r1', status: 'done' });
    await flush();
    expect(t.notify).toHaveBeenCalledTimes(1);
  });

  it('[REC-H12] a job that fails sends no notice and refreshes the open detail', async () => {
    const t = setup();
    t.click({ action: 'recordings.detail', id: 'r3' });
    await flush();
    t.fake.api.detail.mockResolvedValue({
      id: 'r3',
      status: 'failed',
      errorKind: 'notes_endpoint_missing',
      segments: [],
    });
    t.fake.pushJob({ id: 'r3', status: 'failed', errorKind: 'notes_endpoint_missing' });
    await flush();
    expect(t.notify).not.toHaveBeenCalled();
    expect(t.last<HubRecordingDetail>(HUB_RECORDING_DETAIL_EVENT)?.error).toEqual(
      RECORDING_ERRORS.notes_endpoint_missing,
    );
  });

  it("[REC-G1] the hub's first open reconciles the index; every other refresh is the fast path (owner authorization 2026-11-25)", async () => {
    const t = setup();
    t.click({ action: 'recordings.ready' });
    await flush();
    expect(t.fake.api.list).toHaveBeenLastCalledWith(true);
    // Retry's refresh, and every other read after it, is the fast path: no reconcile.
    t.click({ action: 'recordings.retry', id: 'r2' });
    await flush();
    expect(t.fake.api.list).toHaveBeenLastCalledWith(false);
  });

  it('[REC-G2] a recording reaching done or failed reconciles the index in the background', async () => {
    const t = setup();
    t.click({ action: 'recordings.ready' });
    await flush();
    const callsAfterReady = t.fake.api.list.mock.calls.length;
    expect(t.fake.api.list).toHaveBeenLastCalledWith(true);
    t.fake.pushJob({ id: 'r3', status: 'uploading', partsUploaded: 1, partsTotal: 4 });
    await flush();
    // An in-flight status update (a known row) makes no new list call at all.
    expect(t.fake.api.list.mock.calls.length).toBe(callsAfterReady);
    t.fake.pushJob({ id: 'r3', status: 'done' });
    await flush();
    expect(t.fake.api.list.mock.calls.length).toBe(callsAfterReady + 1);
    expect(t.fake.api.list).toHaveBeenLastCalledWith(true);
    // A second `done` event for the same row (not a new transition) reconciles no further.
    const calls = t.fake.api.list.mock.calls.length;
    t.fake.pushJob({ id: 'r3', status: 'done' });
    await flush();
    expect(t.fake.api.list.mock.calls.length).toBe(calls);
  });

  it('[REC-P7] each job event updates the open list: part counts, the network mark and its end, and a device-only row', async () => {
    const t = setup();
    t.click({ action: 'recordings.ready' });
    await flush();
    const row = () => t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.rows.find((r) => r.id === 'r3');
    t.fake.pushJob({ id: 'r3', status: 'uploading', partsUploaded: 1, partsTotal: 4 });
    await flush();
    expect(row()).toMatchObject({ status: 'uploading', partsUploaded: 1, partsTotal: 4 });
    expect(row()?.canOpen).toBe(true);
    expect(row()?.waitingNetwork).toBeUndefined();
    t.fake.pushJob({
      id: 'r3',
      status: 'uploading',
      errorKind: 'network',
      partsUploaded: 1,
      partsTotal: 4,
    });
    await flush();
    expect(row()?.waitingNetwork).toBe(true);
    expect(row()?.error).toBeUndefined();
    // The network is back: the event has no error, so the mark goes.
    t.fake.pushJob({ id: 'r3', status: 'uploading', partsUploaded: 2, partsTotal: 4 });
    await flush();
    expect(row()?.waitingNetwork).toBeUndefined();
    expect(row()?.partsUploaded).toBe(2);
    t.fake.pushJob({
      id: 'r3',
      status: 'failed',
      errorKind: 'upload_failed',
      errorStage: 'uploading',
      onDeviceOnly: true,
    });
    await flush();
    expect(row()).toMatchObject({ status: 'failed', onDeviceOnly: true, canOpen: false });
  });

  it('[REC-H9] a job writing notes shows that status; the model errors name the model', async () => {
    const t = setup();
    t.click({ action: 'recordings.ready' });
    await flush();
    t.fake.pushJob({ id: 'r3', status: 'writing_notes' });
    await flush();
    expect(t.last<HubRecordings>(HUB_RECORDINGS_EVENT)?.rows[0]?.status).toBe('writing_notes');
    expect(RECORDING_ERRORS.transcribe_endpoint_missing.text).toBe(
      "The transcribe model isn't available. Pick another model in Settings.",
    );
    expect(RECORDING_ERRORS.notes_endpoint_missing.text).toBe(
      "The notes model isn't available. Pick another model in Settings.",
    );
  });

  it('[REC-H1] the unsubscribe stops the wiring', async () => {
    const t = setup();
    t.off();
    t.click({ action: 'recording.ready' });
    await flush();
    expect(t.sent).toEqual([]);
  });
});

describe('ipcRecordingApi: the typed recording_* contract (ADR 0103)', () => {
  beforeEach(() => core.reset());

  const contractSettings = {
    enabled: true,
    consentAck: true,
    storage: { kind: 'volume', root: '/Volumes/main/calls/bottle_audio' },
    transcribeEndpoint: DEFAULT_TRANSCRIBE_ENDPOINT,
    notesEndpoint: DEFAULT_NOTES_ENDPOINT,
    presetInstructions: 'Write short notes in Markdown.',
    lengthMin: 45,
  };

  it('[REC-H13] maps settings both ways; a volume wins over My Files in the patch', async () => {
    core.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'recording_settings_read' || cmd === 'recording_settings_write'
        ? contractSettings
        : {},
    );
    const api = ipcRecordingApi(new BrainBus(), async () => 'work');
    const s = await api.readSettings();
    expect(s).toEqual({
      enabled: true,
      consentAck: true,
      lengthMin: 45,
      transcribeEndpoint: DEFAULT_TRANSCRIBE_ENDPOINT,
      notesEndpoint: DEFAULT_NOTES_ENDPOINT,
      instructions: 'Write short notes in Markdown.',
      volumePath: '/Volumes/main/calls/bottle_audio',
    });
    await api.writeSettings({ ...s, myFilesRoot: '/Volumes/databricks/home/1/me/my_files' });
    expect(core.invoke).toHaveBeenLastCalledWith('recording_settings_write', {
      args: {
        patch: {
          enabled: true,
          consentAck: true,
          lengthMin: 45,
          transcribeEndpoint: DEFAULT_TRANSCRIBE_ENDPOINT,
          notesEndpoint: DEFAULT_NOTES_ENDPOINT,
          presetInstructions: 'Write short notes in Markdown.',
          storage: { kind: 'volume', root: '/Volumes/main/calls/bottle_audio' },
        },
      },
    });
  });

  it('[REC-H13] maps state, list, detail and the commands with their ids', async () => {
    core.invoke.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case 'recording_state':
          return { state: 'recording', startedAtMs: 1, endsAtMs: 2, systemAudio: 'unavailable' };
        case 'recording_list':
          return [
            {
              id: 'a',
              name: 'n',
              startedAtMs: 5,
              durationS: 60,
              status: 'failed',
              errorKind: 'auth',
            },
          ];
        case 'recording_detail':
          return {
            summary: { id: 'a', name: 'n', startedAtMs: 5, durationS: 60, status: 'done' },
            instructions: 'x',
            segments: [{ start: 0, end: 1, speaker: '1', text: 'Hi.', part: 1 }],
            notesMd: '## Summary',
          };
        case 'recording_storage_probe':
          return { myFilesRoot: null };
        default:
          return {};
      }
    });
    const api = ipcRecordingApi(new BrainBus(), async () => 'work');
    expect(await api.state()).toEqual({
      phase: 'recording',
      startedAtMs: 1,
      endsAtMs: 2,
      systemAudio: 'mic_only',
    });
    expect(await api.list()).toEqual([
      { id: 'a', name: 'n', startedAt: 5, durationS: 60, status: 'failed', errorKind: 'auth' },
    ]);
    expect(await api.detail('a')).toEqual({
      id: 'a',
      status: 'done',
      notesMd: '## Summary',
      segments: [{ start: 0, end: 1, speaker: '1', text: 'Hi.' }],
    });
    expect(await api.probeStorage()).toEqual({ myFiles: null });
    await api.start();
    expect(core.invoke).toHaveBeenLastCalledWith('recording_start', {
      args: { profile: 'work' },
    });
    await api.start('Only decisions.');
    expect(core.invoke).toHaveBeenLastCalledWith('recording_start', {
      args: { profile: 'work', instructions: 'Only decisions.' },
    });
    await api.open('a');
    expect(core.invoke).toHaveBeenLastCalledWith('recording_open', {
      args: { profile: 'work', id: 'a', target: 'folder' },
    });
    await api.retry('a');
    expect(core.invoke).toHaveBeenLastCalledWith('recording_retry', {
      args: { profile: 'work', id: 'a' },
    });
    await api.delete('a');
    expect(core.invoke).toHaveBeenLastCalledWith('recording_delete', {
      args: { profile: 'work', id: 'a' },
    });
    await api.createVolume('main', 'team');
    expect(core.invoke).toHaveBeenLastCalledWith('recording_volume_create', {
      args: { profile: 'work', catalog: 'main', schema: 'team' },
    });
    const calls = core.invoke.mock.calls.map(([c, a]) => [c, (a as { args: object }).args]);
    for (const cmd of ['recording_storage_probe', 'recording_list', 'recording_detail'])
      expect(calls.find(([c]) => c === cmd)?.[1]).toMatchObject({ profile: 'work' });
    for (const cmd of ['recording_state', 'recording_stop'])
      expect(calls.find(([c]) => c === cmd)?.[1] ?? {}).not.toHaveProperty('profile');
  });

  it('[REC-H13] with no home profile, no cloud-bound command goes out and the user sees why', async () => {
    const api = ipcRecordingApi(new BrainBus(), async () => {
      throw { error: 'no_profile' };
    });
    await expect(api.start()).rejects.toEqual({ error: 'no_profile' });
    await expect(api.list()).rejects.toEqual({ error: 'no_profile' });
    expect(core.invoke).not.toHaveBeenCalled();
    expect(RECORDING_ERRORS.no_profile.text).toBe(
      'Choose a home workspace in Settings → Workspace first, then try again.',
    );
  });

  it('[REC-H13] relays recording.state and recording.job from the bus; the record key is #594s', () => {
    const bus = new BrainBus();
    const api = ipcRecordingApi(bus, async () => 'work');
    const states: unknown[] = [];
    const jobs: unknown[] = [];
    api.onState((s) => states.push(s));
    api.onJob((j) => jobs.push(j));
    bus.dispatch({ event: 'recording:state', payload: { state: 'idle', systemAudio: 'off' } });
    bus.dispatch({
      event: 'recording:job',
      payload: { id: 'a', name: 'n', startedAtMs: 5, durationS: 60, status: 'done' },
    });
    expect(states).toEqual([{ phase: 'idle' }]);
    expect(jobs).toEqual([{ id: 'a', name: 'n', startedAt: 5, durationS: 60, status: 'done' }]);
    expect('onRecordToggle' in api).toBe(false);
  });
});

describe('the held note (ADR 0118)', () => {
  const setupNote = (start: Partial<RecSettings> = {}) => {
    const fake = fakeApi(start);
    const mood = { heldNote: vi.fn() };
    const note = createHeldNote({ api: fake.api, mood });
    return { fake, mood, note };
  };

  it('[AC-6.16] the first hold in a recording shows the note and stores heldNoteShown', async () => {
    const { fake, mood, note } = setupNote();
    fake.pushState({ phase: 'recording' });
    note.held('a1');
    await flush();
    expect(mood.heldNote).toHaveBeenCalledTimes(1);
    expect(mood.heldNote).toHaveBeenCalledWith('a1');
    expect(fake.current().heldNoteShown).toBe(true);
  });

  it('[ADR-0118] the knob turned off between the read and the write stays off', async () => {
    const { fake, mood, note } = setupNote({ enabled: true });
    fake.pushState({ phase: 'recording' });
    note.held('a1');
    // The user turns the knob off while the held note reads its settings.
    await fake.api.writeSettings({ ...fake.current(), enabled: false });
    await flush();
    expect(mood.heldNote).toHaveBeenCalledTimes(1);
    expect(fake.current().enabled).toBe(false);
    expect(fake.current().heldNoteShown).toBe(true);
    // Only the flag goes to Rust: the real api sends a patch with that one field.
    core.invoke.mockClear();
    await ipcRecordingApi(new BrainBus(), async () => 'p').markHeldNoteShown();
    expect(core.invoke).toHaveBeenCalledWith('recording_settings_write', {
      args: { patch: { heldNoteShown: true } },
    });
  });

  it('[AC-6.17] a second hold, in the same or a later recording, shows no note', async () => {
    const { fake, mood, note } = setupNote();
    fake.pushState({ phase: 'recording' });
    note.held('a1');
    note.held('a1');
    await flush();
    note.held('a1');
    fake.pushState({ phase: 'idle' });
    fake.pushState({ phase: 'recording' });
    note.held('a1');
    await flush();
    expect(mood.heldNote).toHaveBeenCalledTimes(1);
  });

  it('[AC-6.17] a stored heldNoteShown keeps the note away after a restart', async () => {
    const { fake, mood, note } = setupNote({ heldNoteShown: true });
    fake.pushState({ phase: 'recording' });
    note.held('a1');
    await flush();
    expect(mood.heldNote).not.toHaveBeenCalled();
    expect(fake.api.markHeldNoteShown).not.toHaveBeenCalled();
  });

  it('a hold with no recording shows no note and keeps the flag', async () => {
    const { fake, mood, note } = setupNote();
    note.held('a1');
    fake.pushState({ phase: 'stopping' });
    note.held('a1');
    await flush();
    expect(mood.heldNote).not.toHaveBeenCalled();
    expect(fake.current().heldNoteShown).toBeUndefined();
  });

  it('a recording that is already on when the brain starts counts', async () => {
    const fake = fakeApi();
    fake.api.state.mockResolvedValue({ phase: 'recording' });
    const mood = { heldNote: vi.fn() };
    const note = createHeldNote({ api: fake.api, mood });
    await flush();
    note.held('a1');
    await flush();
    expect(mood.heldNote).toHaveBeenCalledTimes(1);
  });

  it('the settings map keeps heldNoteShown, and a stale false never resets it', () => {
    const wire = {
      enabled: true,
      consentAck: true,
      storage: null,
      transcribeEndpoint: 'a',
      notesEndpoint: 'b',
      presetInstructions: 'c',
      lengthMin: 60,
      heldNoteShown: true,
    };
    expect(recSettingsOf(wire).heldNoteShown).toBe(true);
    expect(recSettingsOf({ ...wire, heldNoteShown: false }).heldNoteShown).toBeUndefined();
    expect(settingsPatchOf(recSettingsOf(wire)).heldNoteShown).toBe(true);
    expect('heldNoteShown' in settingsPatchOf(settings())).toBe(false);
  });
});
