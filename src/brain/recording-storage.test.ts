/**
 * Storage check, Upload all and the warning on a local-only recording (brain, ADR 0103).
 * A fake `RecordingApi` only: no capture, no network and no token exist here.
 */
import {
  DEFAULT_INSTRUCTIONS,
  DEFAULT_NOTES_ENDPOINT,
  DEFAULT_TRANSCRIBE_ENDPOINT,
  HUB_RECORDING_EVENT,
  HUB_RECORDINGS_EVENT,
  RECORDING_TEXT,
  RECORDING_VIEW_ID,
  type HubRecording,
  type HubRecordings,
} from '../shared/recording-view';
import { BrainBus } from './bus';
import {
  needsUpload,
  wireRecording,
  type RecJob,
  type RecSettings,
  type RecState,
  type RecSummary,
  type RecordingApi,
} from './recording-wiring';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const flush = async () => {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
};
const MY_FILES = '/Volumes/databricks/home/1234/someone@example.com/my_files';

const local = (id: string, startedAt: number): RecSummary => ({
  id,
  name: id,
  startedAt,
  durationS: 30,
  status: 'failed',
  errorKind: 'upload_failed',
  errorStage: 'uploading',
  onDeviceOnly: true,
});

function setup(opts: {
  myFiles?: string | null;
  settings?: Partial<RecSettings>;
  rows?: RecSummary[];
}) {
  let stateCb: ((s: RecState) => void) | undefined;
  let s: RecSettings = {
    enabled: true,
    consentAck: true,
    lengthMin: 60,
    transcribeEndpoint: DEFAULT_TRANSCRIBE_ENDPOINT,
    notesEndpoint: DEFAULT_NOTES_ENDPOINT,
    instructions: DEFAULT_INSTRUCTIONS,
    ...opts.settings,
  };
  const rows = opts.rows ?? [];
  const api = {
    readSettings: vi.fn(async () => s),
    writeSettings: vi.fn(async (n: RecSettings) => (s = n)),
    probeStorage: vi.fn(async () => ({
      myFiles: opts.myFiles === undefined ? MY_FILES : opts.myFiles,
    })),
    createVolume: vi.fn(async () => ({ path: '/Volumes/a/b/bottle_audio' })),
    start: vi.fn(async (): Promise<RecState> => ({
      phase: 'recording',
      startedAtMs: 1,
      endsAtMs: 9,
    })),
    stop: vi.fn(async (): Promise<RecState> => ({ phase: 'idle' })),
    state: vi.fn(async (): Promise<RecState> => ({ phase: 'idle' })),
    list: vi.fn(async () => rows),
    detail: vi.fn(),
    retry: vi.fn(async (_id: string): Promise<unknown> => ({ status: 'done' })),
    open: vi.fn(async (_id: string, _t?: string) => undefined),
    openSystemAudioSettings: vi.fn(async () => undefined),
    onState: (cb: (x: RecState) => void) => {
      stateCb = cb;
      return () => undefined;
    },
    onJob: (_cb: (j: RecJob) => void) => () => undefined,
  } as unknown as RecordingApi & Record<string, ReturnType<typeof vi.fn>>;
  const bus = new BrainBus();
  const sent: { event: string; payload: unknown }[] = [];
  const notify = vi.fn();
  wireRecording(bus, {
    api,
    platform: 'mac',
    send: async (event, payload) => {
      sent.push({ event, payload });
    },
    notify,
  });
  const click = (payload: Record<string, unknown>) =>
    bus.dispatch({
      event: 'view:input',
      payload: { from: 'hub', input: { mogId: RECORDING_VIEW_ID, kind: 'click', payload } },
    });
  const lists = () =>
    sent.filter((e) => e.event === HUB_RECORDINGS_EVENT).map((e) => e.payload as HubRecordings);
  const model = () =>
    sent.filter((e) => e.event === HUB_RECORDING_EVENT).at(-1)?.payload as HubRecording | undefined;
  return { api, rows, click, lists, model, notify, push: (x: RecState) => stateCb?.(x) };
}

describe('storage check (brain)', () => {
  it('[REC-L1] no My Files and no volume is a problem, with the plain reason', async () => {
    const t = setup({ myFiles: null, rows: [local('a', 1)] });
    t.click({ action: 'recordings.ready' });
    await flush();
    const l = t.lists().at(-1);
    expect(l?.storageProblem).toEqual({ reason: RECORDING_TEXT.noMyFiles, canUpload: false });
    expect(l?.rows[0]?.error?.text).toBe(
      'No My Files on this workspace. Choose a volume in Settings.',
    );
    expect(l?.rows[0]?.error?.action?.target).toBe('settings');
  });

  it('[REC-L1] My Files that resolves, or a chosen volume, is no problem', async () => {
    const a = setup({ myFiles: MY_FILES, rows: [local('a', 1)] });
    a.click({ action: 'recordings.ready' });
    await flush();
    expect(a.lists().at(-1)?.storageProblem).toBeUndefined();
    const b = setup({ myFiles: null, settings: { volumePath: '/Volumes/c/s/v' }, rows: [] });
    b.click({ action: 'recordings.ready' });
    await flush();
    expect(b.lists().at(-1)?.storageProblem).toBeUndefined();
  });

  it('[REC-L1] a check that failed (network) is not reported as "no My Files"', async () => {
    const t = setup({ rows: [] });
    t.api.probeStorage.mockRejectedValueOnce({ error: 'offline' });
    t.click({ action: 'recordings.ready' });
    await flush();
    expect(t.lists().at(-1)?.storageProblem).toBeUndefined();
  });

  it('[REC-L1] a volume the user cannot write to is a problem that Upload all can retry', async () => {
    const t = setup({
      settings: { volumePath: '/Volumes/c/s/v' },
      rows: [{ ...local('a', 1), onDeviceOnly: undefined, errorKind: 'volume_permission' }],
    });
    t.click({ action: 'recordings.ready' });
    await flush();
    const p = t.lists().at(-1)?.storageProblem;
    expect(p?.reason).toContain("can't write to this volume");
    expect(p?.canUpload).toBe(true);
  });

  it('[REC-L1] a recording that starts with no usable storage warns that the audio stays local, for the button and for ⌥R', async () => {
    // The hub never opened: the probe has not run. A state push (the ⌥R path) probes first.
    const t = setup({ myFiles: null });
    t.push({ phase: 'recording', startedAtMs: 1, endsAtMs: 9 });
    await flush();
    const err = t.model()?.error;
    expect(err?.text).toContain('No My Files on this workspace. Choose a volume in Settings.');
    expect(err?.text).toContain('keeps the audio on this device');
    expect(err?.action).toEqual({ label: RECORDING_TEXT.chooseWhere, target: 'settings' });
    expect(t.notify).toHaveBeenCalledWith(RECORDING_TEXT.noMyFiles);

    const ok = setup({ myFiles: MY_FILES });
    ok.push({ phase: 'recording', startedAtMs: 1, endsAtMs: 9 });
    await flush();
    expect(ok.model()?.error).toBeUndefined();
  });
});

describe('Upload all (brain)', () => {
  it('[REC-L2] needsUpload is a device-only recording or one that failed at the upload', () => {
    expect(needsUpload(local('a', 1))).toBe(true);
    expect(needsUpload({ id: 'x', name: 'x', startedAt: 1, durationS: 1, status: 'done' })).toBe(
      false,
    );
    expect(
      needsUpload({
        id: 'x',
        name: 'x',
        startedAt: 1,
        durationS: 1,
        status: 'failed',
        errorKind: 'transcribe_endpoint_missing',
        errorStage: 'transcribing',
      }),
    ).toBe(false);
  });

  it('[REC-L2] retries each waiting recording one at a time, oldest first, with progress', async () => {
    const rows = [local('c', 30), local('a', 10), local('b', 20)];
    const t = setup({ rows });
    t.click({ action: 'recordings.ready' });
    await flush();
    expect(t.lists().at(-1)?.uploadAll).toEqual({ running: false, total: 3, done: 0, failed: 0 });

    const gates: (() => void)[] = [];
    let active = 0;
    let maxActive = 0;
    t.api.retry.mockImplementation(
      () =>
        new Promise((resolve) => {
          active += 1;
          maxActive = Math.max(maxActive, active);
          gates.push(() => {
            active -= 1;
            resolve({ status: 'done' });
          });
        }),
    );
    t.click({ action: 'recordings.upload_all' });
    await flush();
    expect(t.api.retry.mock.calls.map((c) => c[0])).toEqual(['a']);
    let l = t.lists().at(-1);
    expect(l?.uploadAll).toEqual({ running: true, total: 3, done: 0, failed: 0 });
    const state = (id: string) => l?.rows.find((r) => r.id === id)?.batch;
    expect([state('a'), state('b'), state('c')]).toEqual(['running', 'queued', 'queued']);

    gates.shift()?.();
    await flush();
    l = t.lists().at(-1);
    expect(t.api.retry.mock.calls.map((c) => c[0])).toEqual(['a', 'b']);
    expect(l?.uploadAll).toMatchObject({ running: true, done: 1 });
    expect([state('a'), state('b'), state('c')]).toEqual(['done', 'running', 'queued']);

    gates.shift()?.();
    await flush();
    gates.shift()?.();
    await flush();
    expect(t.api.retry.mock.calls.map((c) => c[0])).toEqual(['a', 'b', 'c']);
    expect(maxActive).toBe(1);
    expect(t.lists().at(-1)?.uploadAll?.running ?? false).toBe(false);
  });

  it('[REC-L2] a failed recording counts, and the batch goes on', async () => {
    const t = setup({ rows: [local('a', 1), local('b', 2)] });
    t.api.retry
      .mockResolvedValueOnce({ status: 'failed' })
      .mockResolvedValueOnce({ status: 'done' });
    t.click({ action: 'recordings.ready' });
    await flush();
    t.click({ action: 'recordings.upload_all' });
    await flush();
    expect(t.api.retry).toHaveBeenCalledTimes(2);
  });

  it('[REC-L2] a storage fault stops the batch and shows the reason', async () => {
    const t = setup({ rows: [local('a', 1), local('b', 2)] });
    t.api.retry.mockRejectedValueOnce({ error: 'volume_permission' });
    t.click({ action: 'recordings.ready' });
    await flush();
    t.click({ action: 'recordings.upload_all' });
    await flush();
    expect(t.api.retry).toHaveBeenCalledTimes(1);
    expect(t.model()?.error?.text).toContain("can't write to this volume");
  });

  it('[REC-L2] does nothing while storage is missing', async () => {
    const t = setup({ myFiles: null, rows: [local('a', 1)] });
    t.click({ action: 'recordings.ready' });
    await flush();
    t.click({ action: 'recordings.upload_all' });
    await flush();
    expect(t.api.retry).not.toHaveBeenCalled();
  });

  it('[REC-L3] open passes the file target to the core, and no URL', async () => {
    const t = setup({ rows: [] });
    t.click({ action: 'recordings.open', id: 'r1', target: 'transcript' });
    t.click({ action: 'recordings.open', id: 'r1', target: 'https://evil.example' });
    await flush();
    expect(t.api.open).toHaveBeenCalledTimes(1);
    expect(t.api.open).toHaveBeenCalledWith('r1', 'transcript');
  });
});
