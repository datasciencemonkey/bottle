/**
 * The brain side of call recording in the hub (ADR 0103, refs #65; ARC-2, ARC-5).
 *
 * The thin hub view sends clicks (id `recording`); the brain owns every rule and every text:
 * - Settings: the knob turns on only after the one consent acknowledgement; the length is a whole
 *   number of minutes from 5 to 180; the notes model is a strict endpoint name; a volume path is
 *   a strict `/Volumes/c/s/v`; `bottle_audio` is created only on the user's confirm click.
 *   The core checks the knob again before any capture starts (Rust, not only this file).
 * - Record and Stop go to the core (the record key ⌥R is `record-toggle.ts`, #594); the REC state follows `recording.state` (`ends_at_ms`).
 *   While it runs, the MOGs show the REC overlay through the one mood publisher.
 * - Recordings: the list (newest first), one detail at a time with the speaker turns merged,
 *   Retry and "Open in Databricks". The core builds and opens the link; no URL reaches a view.
 * - A job that ends `done` gives one short OS notice and the MOG answer-ready moment.
 * Every failure shows a fixed plain text with one action, never upstream text.
 */
import { emitTo } from '@tauri-apps/api/event';
import {
  asRecordingAction,
  CATALOG_SCHEMA,
  DEFAULT_INSTRUCTIONS,
  ENDPOINT_NAME,
  HUB_RECORDING_DETAIL_EVENT,
  HUB_RECORDING_EVENT,
  HUB_RECORDINGS_EVENT,
  INSTRUCTIONS_MAX,
  LENGTH_MAX,
  LENGTH_MIN,
  RECORDING_TEXT,
  RECORDING_VIEW_ID,
  recordingNotice,
  VOLUME_PATH,
  type HubRecording,
  type HubRecordingDetail,
  type RecordingAction,
  type RecordingControlView,
  type RecordingNotice,
  type RecordingPlatform,
  type RecordingRow,
  type RecordingStatus,
  type RecordingTurn,
  type StorageView,
} from '../shared/recording-view';
import {
  call,
  EVENTS,
  type RecordingSettings,
  type RecordingSettingsPatch,
  type RecordingState,
  type RecordingSummary,
} from '../shared/ipc';
import type { ProfileName } from '../shared/interfaces';
import { onViewInput, type BrainBus } from './bus';

/** The recording settings as the brain reads and writes them. No secrets. */
export interface RecSettings {
  enabled: boolean;
  consentAck: boolean;
  lengthMin: number;
  transcribeEndpoint: string;
  notesEndpoint: string;
  instructions: string;
  /** The volume when My Files is not used. Missing: My Files. */
  volumePath?: string;
  /** The My Files root once the probe found it and Bottle saved it. */
  myFilesRoot?: string;
}

/** The core recording state (`recording.state`). */
export interface RecState {
  phase: 'idle' | 'starting' | 'recording' | 'stopping';
  startedAtMs?: number;
  endsAtMs?: number;
  systemAudio?: 'ok' | 'silent_or_denied' | 'mic_only';
}

/** One recording in the volume (`recording_list`). */
export interface RecSummary {
  id: string;
  name: string;
  startedAt: number;
  durationS: number;
  status: RecordingStatus;
  errorKind?: string;
}

/** One transcript segment; times in seconds, absolute across parts. */
export interface RecSegment {
  start: number;
  end: number;
  speaker: string;
  text: string;
}

/** One recording with its notes and transcript (`recording_detail`). */
export interface RecDetail {
  id: string;
  status: RecordingStatus;
  errorKind?: string;
  notesMd?: string;
  segments: RecSegment[];
}

/** A job status change (`recording.job`). */
export interface RecJob {
  id: string;
  status: RecordingStatus;
  errorKind?: string;
}

/**
 * The `recording_*` commands and events the brain uses. The real one is typed IPC
 * (`ipcRecordingApi`); tests pass a fake. A rejection carries `{ error: <code> }`.
 */
export interface RecordingApi {
  readSettings(): Promise<RecSettings>;
  writeSettings(next: RecSettings): Promise<RecSettings>;
  /** The My Files root, or `null` when it does not resolve on this workspace. */
  probeStorage(): Promise<{ myFiles: string | null }>;
  createVolume(catalog: string, schema: string): Promise<{ path: string }>;
  start(instructions?: string): Promise<RecState>;
  stop(): Promise<RecState>;
  state(): Promise<RecState>;
  list(): Promise<RecSummary[]>;
  detail(id: string): Promise<RecDetail>;
  retry(id: string): Promise<unknown>;
  open(id: string): Promise<unknown>;
  /** Open the OS privacy pane for System Audio Recording. */
  openSystemAudioSettings(): Promise<unknown>;
  onState(cb: (s: RecState) => void): () => void;
  onJob(cb: (j: RecJob) => void): () => void;
}

/** The MOG side: the REC overlay and the answer-ready moment (`capture-mood.ts`). */
export interface RecordingMood {
  callRecording(on: boolean): void;
  ready(): void;
}

export interface RecordingWireDeps {
  api: RecordingApi;
  platform: RecordingPlatform;
  /** Default: `emitTo('hub', event, payload)`. */
  send?: (event: string, payload: unknown) => Promise<unknown>;
  mood?: RecordingMood;
  /** A short OS notice. It never carries transcript or notes text. */
  notify?: (text: string) => void;
}

/** The OS notice when the notes of a recording are ready. */
export const NOTES_READY_TEXT = 'Your call notes are ready.';

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const codeOf = (e: unknown): string =>
  isObj(e) && typeof e['error'] === 'string' ? e['error'] : '';

/** Merge consecutive segments of one speaker into turns, in time order. Empty text is dropped. */
export function turnsOf(segments: readonly RecSegment[]): RecordingTurn[] {
  const out: RecordingTurn[] = [];
  for (const s of [...segments].sort((a, b) => a.start - b.start)) {
    const text = s.text.trim();
    if (text === '') continue;
    const last = out.at(-1);
    if (last && last.speaker === s.speaker) last.text = `${last.text} ${text}`;
    else out.push({ speaker: s.speaker, startS: s.start, text });
  }
  return out;
}

function controlOf(s: RecState): RecordingControlView {
  const c: RecordingControlView = { phase: s.phase };
  if (s.startedAtMs !== undefined) c.startedAtMs = s.startedAtMs;
  if (s.endsAtMs !== undefined) c.endsAtMs = s.endsAtMs;
  if (s.systemAudio !== undefined) c.systemAudio = s.systemAudio;
  return c;
}

function rowOf(r: RecSummary): RecordingRow {
  const row: RecordingRow = {
    id: r.id,
    name: r.name,
    startedAt: r.startedAt,
    durationS: r.durationS,
    status: r.status,
  };
  if (r.status === 'failed') row.error = recordingNotice(r.errorKind);
  return row;
}

/** Wire the hub recording section, control and list. Returns the unsubscribe. */
export function wireRecording(bus: BrainBus, deps: RecordingWireDeps): () => void {
  const { api, platform } = deps;
  const send = deps.send ?? ((event: string, payload: unknown) => emitTo('hub', event, payload));
  const toHub = (event: string, payload: unknown) =>
    void send(event, payload).catch(() => undefined);

  let settings: RecSettings | undefined;
  let myFiles: string | null | undefined;
  let storageError: string | undefined;
  let creating = false;
  let control: RecordingControlView = { phase: 'idle' };
  let error: RecordingNotice | undefined;
  let settingsError: string | undefined;
  let detailId: string | undefined;
  /** The last status of each recording the brain has seen, to notify once per finish. */
  const seen = new Map<string, RecordingStatus>();
  let rows: RecSummary[] = [];
  let gone = false;

  const storage = (): StorageView => {
    if (creating) return { status: 'creating' };
    if (storageError) return { status: 'unavailable', error: storageError };
    if (settings?.volumePath) return { status: 'volume', path: settings.volumePath };
    if (myFiles === undefined) return { status: 'checking' };
    return myFiles ? { status: 'my_files', path: myFiles } : { status: 'unavailable' };
  };

  const publish = () => {
    if (gone) return;
    const m: HubRecording = { platform, storage: storage(), control };
    if (settings) {
      const { enabled, consentAck, lengthMin, transcribeEndpoint, notesEndpoint, instructions } =
        settings;
      m.settings = {
        enabled,
        consentAck,
        lengthMin,
        transcribeEndpoint,
        notesEndpoint,
        instructions,
      };
    }
    if (error) m.error = error;
    if (settingsError) m.settingsError = settingsError;
    toHub(HUB_RECORDING_EVENT, m);
  };

  const publishList = () => {
    if (gone) return;
    const sorted = [...rows].sort((a, b) => b.startedAt - a.startedAt || (a.id < b.id ? 1 : -1));
    toHub(HUB_RECORDINGS_EVENT, { rows: sorted.map(rowOf) });
  };

  let recOn = false;
  const setState = (s: RecState) => {
    control = controlOf(s);
    if (s.systemAudio === 'silent_or_denied') error = recordingNotice('system_audio_denied');
    else if (error === recordingNotice('system_audio_denied')) error = undefined;
    const on = s.phase === 'recording';
    if (on !== recOn) {
      recOn = on;
      deps.mood?.callRecording(on);
    }
    publish();
  };

  async function load(): Promise<void> {
    const [s, st] = await Promise.all([
      api.readSettings().catch(() => undefined),
      api.state().catch(() => undefined),
    ]);
    if (s) settings = s;
    if (st) setState(st);
    publish();
    await probe();
  }

  async function probe(): Promise<void> {
    try {
      myFiles = (await api.probeStorage()).myFiles;
      storageError = undefined;
      // My Files is the default: save it once, so the core knows where to write.
      if (myFiles && settings && !settings.volumePath && settings.myFilesRoot !== myFiles)
        void write({ myFilesRoot: myFiles });
    } catch {
      myFiles = null;
      storageError = RECORDING_TEXT.storageCheckFailed;
    }
    publish();
  }

  /** Writes run one at a time, each on the result of the one before. */
  let writes: Promise<void> = Promise.resolve();
  const write = (patch: Partial<RecSettings>): Promise<void> =>
    (writes = writes.then(() => writeNow(patch)));

  async function writeNow(patch: Partial<RecSettings>): Promise<void> {
    const base = settings ?? (await api.readSettings().catch(() => undefined));
    if (!base) {
      settingsError = RECORDING_TEXT.settingsFailed;
      publish();
      return;
    }
    try {
      settings = await api.writeSettings({ ...base, ...patch });
      settingsError = undefined;
    } catch {
      settings = base;
      settingsError = RECORDING_TEXT.settingsFailed;
    }
    publish();
  }

  const refuse = (text: string) => {
    settingsError = text;
    publish();
  };

  async function refreshList(): Promise<void> {
    try {
      rows = await api.list();
    } catch {
      if (!gone) toHub(HUB_RECORDINGS_EVENT, { rows: [], unavailable: true });
      return;
    }
    for (const r of rows) if (!seen.has(r.id)) seen.set(r.id, r.status);
    publishList();
  }

  async function showDetail(id: string): Promise<void> {
    detailId = id;
    let d: HubRecordingDetail;
    try {
      const raw = await api.detail(id);
      d = { id: raw.id, status: raw.status, turns: turnsOf(raw.segments) };
      if (raw.notesMd !== undefined) d.notesMd = raw.notesMd;
      if (raw.status === 'failed') d.error = recordingNotice(raw.errorKind);
    } catch {
      const row = rows.find((r) => r.id === id);
      d = { id, status: row?.status ?? 'failed', turns: [], unavailable: true };
    }
    if (detailId === id && !gone) toHub(HUB_RECORDING_DETAIL_EVENT, d);
  }

  function onJob(j: RecJob): void {
    const before = seen.get(j.id);
    seen.set(j.id, j.status);
    const row = rows.find((r) => r.id === j.id);
    if (row) {
      row.status = j.status;
      if (j.errorKind !== undefined) row.errorKind = j.errorKind;
      publishList();
    } else void refreshList();
    if (j.status === 'done' && before !== 'done') {
      deps.notify?.(NOTES_READY_TEXT);
      deps.mood?.ready();
    }
    if (detailId === j.id) void showDetail(j.id);
  }

  async function start(instructions?: string): Promise<void> {
    if (control.phase !== 'idle') return;
    const text = instructions?.trim();
    const preset = (settings?.instructions ?? DEFAULT_INSTRUCTIONS).trim();
    const edited = text && text !== preset ? text.slice(0, INSTRUCTIONS_MAX) : undefined;
    error = undefined;
    control = { phase: 'starting' };
    publish();
    try {
      setState(await api.start(edited));
    } catch (e) {
      control = { phase: 'idle' };
      error = recordingNotice(codeOf(e));
      publish();
    }
  }

  async function stop(): Promise<void> {
    if (control.phase !== 'recording') return;
    control = { ...control, phase: 'stopping' };
    publish();
    try {
      setState(await api.stop());
    } catch {
      setState(await api.state().catch((): RecState => ({ phase: 'idle' })));
    }
  }

  async function createVolume(catalog: string, schema: string): Promise<void> {
    const m = CATALOG_SCHEMA.exec(`${catalog.trim()}.${schema.trim()}`);
    if (!m) {
      storageError = RECORDING_TEXT.badSchema;
      publish();
      return;
    }
    creating = true;
    publish();
    try {
      const { path } = await api.createVolume(m[1] as string, m[2] as string);
      creating = false;
      storageError = undefined;
      await write({ volumePath: path });
    } catch {
      creating = false;
      storageError = RECORDING_TEXT.volumeCreateFailed;
      publish();
    }
  }

  function handle(a: RecordingAction): void {
    switch (a.action) {
      case 'recording.ready':
        void load();
        break;
      case 'recording.consent_ack':
        void write({ consentAck: true });
        break;
      case 'recording.enable':
        // The consent notice comes first, once. The core checks the knob again on start.
        if (a.on && !(settings?.consentAck ?? false)) publish();
        else void write({ enabled: a.on });
        break;
      case 'recording.length':
        if (Number.isInteger(a.minutes) && a.minutes >= LENGTH_MIN && a.minutes <= LENGTH_MAX)
          void write({ lengthMin: a.minutes });
        else refuse(RECORDING_TEXT.badLength);
        break;
      case 'recording.endpoint': {
        const e = a.endpoint.trim();
        if (!ENDPOINT_NAME.test(e)) refuse(RECORDING_TEXT.badEndpoint);
        else if (a.which === 'transcribe') void write({ transcribeEndpoint: e });
        else void write({ notesEndpoint: e });
        break;
      }
      case 'recording.instructions': {
        const t = a.text.slice(0, INSTRUCTIONS_MAX);
        void write({ instructions: t.trim() === '' ? DEFAULT_INSTRUCTIONS : t });
        break;
      }
      case 'recording.storage_probe':
        void probe();
        break;
      case 'recording.storage_volume': {
        const p = a.path.trim();
        if (VOLUME_PATH.test(p)) {
          storageError = undefined;
          void write({ volumePath: p.replace(/\/$/, '') });
        } else {
          storageError = RECORDING_TEXT.badVolume;
          publish();
        }
        break;
      }
      case 'recording.volume_create':
        void createVolume(a.catalog, a.schema);
        break;
      case 'recording.start':
        void start(a.instructions);
        break;
      case 'recording.stop':
        void stop();
        break;
      case 'recording.system_audio':
        void api.openSystemAudioSettings().catch(() => undefined);
        break;
      case 'recordings.ready':
        void refreshList();
        break;
      case 'recordings.detail':
        void showDetail(a.id);
        break;
      case 'recordings.retry':
        void api
          .retry(a.id)
          .then(() => refreshList())
          .catch(() => undefined);
        break;
      case 'recordings.open':
        void api.open(a.id).catch(() => undefined);
        break;
    }
  }

  const offInput = onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.kind !== 'click' || input.mogId !== RECORDING_VIEW_ID) return;
    const a = asRecordingAction(input.payload);
    if (a) handle(a);
  });
  const offState = api.onState(setState);
  const offJob = api.onJob(onJob);
  return () => {
    gone = true;
    offInput();
    offState();
    offJob();
  };
}

/** The `recording.state` of the contract, as the brain reads it. */
export function recStateOf(s: RecordingState): RecState {
  const out: RecState = { phase: s.state };
  if (s.startedAtMs !== undefined) out.startedAtMs = s.startedAtMs;
  if (s.endsAtMs !== undefined) out.endsAtMs = s.endsAtMs;
  if (s.systemAudio === 'ok' || s.systemAudio === 'silent_or_denied')
    out.systemAudio = s.systemAudio;
  else if (s.systemAudio === 'unavailable') out.systemAudio = 'mic_only';
  return out;
}

function summaryOf(s: RecordingSummary): RecSummary {
  const r: RecSummary = {
    id: s.id,
    name: s.name,
    startedAt: s.startedAtMs,
    durationS: s.durationS,
    status: s.status,
  };
  if (s.errorKind !== undefined) r.errorKind = s.errorKind;
  return r;
}

/** The contract settings, as the brain reads them. */
export function recSettingsOf(s: RecordingSettings): RecSettings {
  const r: RecSettings = {
    enabled: s.enabled,
    consentAck: s.consentAck,
    lengthMin: s.lengthMin,
    transcribeEndpoint: s.transcribeEndpoint,
    notesEndpoint: s.notesEndpoint,
    instructions: s.presetInstructions,
  };
  if (s.storage?.kind === 'volume') r.volumePath = s.storage.root;
  else if (s.storage?.kind === 'my_files') r.myFilesRoot = s.storage.root;
  return r;
}

/** The brain settings as a contract patch. A volume wins over My Files. */
export function settingsPatchOf(s: RecSettings): RecordingSettingsPatch {
  const p: RecordingSettingsPatch = {
    enabled: s.enabled,
    consentAck: s.consentAck,
    lengthMin: s.lengthMin,
    transcribeEndpoint: s.transcribeEndpoint,
    notesEndpoint: s.notesEndpoint,
    presetInstructions: s.instructions,
  };
  if (s.volumePath) p.storage = { kind: 'volume', root: s.volumePath };
  else if (s.myFilesRoot) p.storage = { kind: 'my_files', root: s.myFilesRoot };
  return p;
}

/**
 * The real `RecordingApi`: the typed `recording_*` commands and the core events on `bus`.
 * `profile` is the brain's home profile (`home-profile.ts`). Every command that reaches
 * Databricks carries it; with none, that command fails `no_profile` and nothing goes out.
 */
export function ipcRecordingApi(
  bus: BrainBus,
  profile: () => Promise<ProfileName | undefined>,
): RecordingApi {
  const home = async (): Promise<ProfileName> => {
    const p = await profile().catch(() => undefined);
    if (p === undefined || p === '') throw { error: 'no_profile' };
    return p;
  };
  return {
    readSettings: async () => recSettingsOf(await call('recording_settings_read', {})),
    writeSettings: async (next) =>
      recSettingsOf(await call('recording_settings_write', { patch: settingsPatchOf(next) })),
    probeStorage: async () => ({
      myFiles: (await call('recording_storage_probe', { profile: await home() })).myFilesRoot,
    }),
    createVolume: async (catalog, schema) => ({
      path: (await call('recording_volume_create', { profile: await home(), catalog, schema }))
        .root,
    }),
    start: async (instructions) => {
      const profile = await home();
      return recStateOf(
        await call(
          'recording_start',
          instructions === undefined ? { profile } : { profile, instructions },
        ),
      );
    },
    stop: async () => recStateOf(await call('recording_stop', {})),
    state: async () => recStateOf(await call('recording_state', {})),
    list: async () => (await call('recording_list', { profile: await home() })).map(summaryOf),
    detail: async (id) => {
      const d = await call('recording_detail', { profile: await home(), id });
      const r: RecDetail = {
        id: d.summary.id,
        status: d.summary.status,
        segments: d.segments.map(({ start, end, speaker, text }) => ({
          start,
          end,
          speaker,
          text,
        })),
      };
      if (d.summary.errorKind !== undefined) r.errorKind = d.summary.errorKind;
      if (d.notesMd !== undefined) r.notesMd = d.notesMd;
      return r;
    },
    retry: async (id) => call('recording_retry', { profile: await home(), id }),
    open: async (id) => call('recording_open', { profile: await home(), id, target: 'folder' }),
    openSystemAudioSettings: () => call('recording_open_system_audio_settings', {}),
    onState: (cb) =>
      bus.on(EVENTS['recording.state'].name, (p) => {
        if (isObj(p) && typeof p['state'] === 'string')
          cb(recStateOf(p as unknown as RecordingState));
      }),
    onJob: (cb) =>
      bus.on(EVENTS['recording.job'].name, (p) => {
        if (isObj(p) && typeof p['id'] === 'string' && typeof p['status'] === 'string')
          cb(summaryOf(p as unknown as RecordingSummary));
      }),
  };
}
