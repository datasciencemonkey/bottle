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
  type HubRecording,
  type HubRecordingDetail,
  type HubRecordings,
} from '../shared/recording-view';
import { BrainBus } from './bus';
import { core } from './test-core';
import {
  ipcRecordingApi,
  turnsOf,
  wireRecording,
  type RecDetail,
  type RecJob,
  type RecSettings,
  type RecState,
  type RecSummary,
  type RecordingApi,
} from './recording-wiring';

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

function setup(start: Partial<RecSettings> = {}) {
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

  it('[REC-H8] a silent system audio state shows the System Audio fix-it notice', async () => {
    const t = setup({ enabled: true, consentAck: true });
    t.click({ action: 'recording.ready' });
    await flush();
    t.fake.pushState({ phase: 'recording', endsAtMs: 10, systemAudio: 'silent_or_denied' });
    await flush();
    expect(t.last<HubRecording>(HUB_RECORDING_EVENT)?.error).toEqual(
      RECORDING_ERRORS.system_audio_denied,
    );
    t.click({ action: 'recording.system_audio' });
    await flush();
    expect(t.fake.api.openSystemAudioSettings).toHaveBeenCalledTimes(1);
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
    const api = ipcRecordingApi(new BrainBus(), async () => undefined);
    await expect(api.start()).rejects.toEqual({ error: 'no_profile' });
    await expect(api.list()).rejects.toEqual({ error: 'no_profile' });
    expect(core.invoke).not.toHaveBeenCalled();
    expect(RECORDING_ERRORS.no_profile.text).toBe(
      'Set a home profile on a MOG first, then try again.',
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
