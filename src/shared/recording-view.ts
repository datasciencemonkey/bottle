/**
 * The hub side of call recording (ADR 0103, refs #65): names, view models, fixed texts and
 * shape checks. No state lives here. This is not a contract file (like `hub-conversations.ts`):
 * the Tauri commands and core events stay in `src/shared/ipc.ts`.
 *
 * Ownership (ARC-2): Rust owns capture, files, network and the job. The brain calls the
 * `recording_*` commands, owns every rule and text, and sends these models to the hub window
 * only. The hub view renders them and sends clicks with `view_emit` (id `recording`). The view
 * never builds or opens a URL: "Open in Databricks" asks the brain, and Bottle opens the link.
 */

/** The brain-to-hub event with the settings section and the Record control. */
export const HUB_RECORDING_EVENT = 'hub:recording';
/** The brain-to-hub event with the Recordings list. */
export const HUB_RECORDINGS_EVENT = 'hub:recordings';
/** The brain-to-hub event with one recording (notes and transcript). */
export const HUB_RECORDING_DETAIL_EVENT = 'hub:recording.detail';

/** The `mogId` of the recording `view_emit`. It cannot be a MOG ID (those are UUIDs). */
export const RECORDING_VIEW_ID = 'recording';

/** The preset choices of the recording length, in minutes (UPDATE 1). Default 60. */
export const LENGTH_CHOICES = [15, 30, 45, 60, 90] as const;
export const DEFAULT_LENGTH_MIN = 60;
/** A custom length is a whole number of minutes in this range. */
export const LENGTH_MIN = 5;
export const LENGTH_MAX = 120;

/** The model that transcribes the audio (ADR 0103 update 4, ADR 0123). */
export const DEFAULT_TRANSCRIBE_ENDPOINT = 'system.ai.gemini-3-8-flash';
/** The model that writes the notes from the transcript. */
export const DEFAULT_NOTES_ENDPOINT = 'system.ai.claude-sonnet-5-5';
/** The longest instructions text Bottle keeps. */
export const INSTRUCTIONS_MAX = 8000;

/** The default preset prompt (ADR 0103). */
export const DEFAULT_INSTRUCTIONS = `You get the transcript of a call. Speakers are labeled Speaker 1, Speaker 2, and so on.
Write short notes in Markdown with these sections:
## Summary
Three to five sentences.
## Decisions
A bullet list. Write "None" if there are no decisions.
## Action items
One sub-heading per speaker. Under each, a bullet list of that speaker's action items, with an owner and a due date if the call names one.
Use only facts from the transcript.`;

export type RecordingPlatform = 'mac' | 'windows';

/** The fixed texts of the hub. Upstream text never shows. */
export const RECORDING_TEXT = {
  consent: "Recording a call may need everyone's consent. Let people know.",
  binding: {
    mac: 'Start or stop: hold Right Option, tap R',
    windows: 'Start or stop: hold Right Alt, tap R',
  },
  myFilesMissing:
    "My Files isn't available on this workspace. Pick a volume, or a catalog and schema.",
  badVolume: 'Use a volume path like /Volumes/catalog/schema/volume.',
  badSchema: 'Use a catalog and schema like main.default.',
  badEndpoint: 'Use a model id: letters, digits, dots, dashes and underscores.',
  badLength: `Use a length from ${LENGTH_MIN} to ${LENGTH_MAX} minutes.`,
  volumeCreateFailed:
    "Bottle couldn't create the volume. Check that you can create volumes in this schema.",
  storageCheckFailed: "Bottle couldn't check My Files. Check your network, then try again.",
  settingsFailed: "Bottle couldn't save this setting. Try again.",
  turnOn: 'Turn on call recording in Settings to record.',
  /** The banner title of Recordings when recordings cannot upload (storage is unusable). */
  cantUpload: "Recordings can't upload: choose where to save them",
  chooseWhere: 'Choose where to save',
  /** My Files does not resolve and no volume is set (the probe of ADR 0103 decision 7). */
  noMyFiles: 'No My Files on this workspace. Choose a volume in Settings.',
  /** Shown after a recording starts while storage is unusable. The audio stays local. */
  localOnlyRow: 'Saved on this device only for now.',
  keepsLocal: 'Until then, Bottle keeps the audio on this device.',
} as const;

/** The file names of a finished recording, in its folder (Databricks opens the folder). */
export function recordingFileNames(name: string): { transcript: string; notes: string } {
  return { transcript: `${name}.transcript.md`, notes: `${name}.notes.md` };
}

/** What `recordings.open` opens. The core builds the link; the view never passes a URL. */
export type RecordingOpenWhat = 'folder' | 'transcript' | 'notes';

/** What a notice button does. `settings` is handled in the hub; the brain handles the rest. */
export type RecordingActionTarget = 'settings' | 'system_audio' | 'retry' | 'open';

/** One plain-words problem with one action (ADR 0103). */
export interface RecordingNotice {
  text: string;
  action?: { label: string; target: RecordingActionTarget };
}

/** The error kinds of the core and the job, and their plain words. */
export const RECORDING_ERRORS = {
  transcribe_endpoint_missing: {
    text: "The transcribe model isn't available. Pick another model in Settings.",
    action: { label: 'Open Settings', target: 'settings' },
  },
  notes_endpoint_missing: {
    text: "The notes model isn't available. Pick another model in Settings.",
    action: { label: 'Open Settings', target: 'settings' },
  },
  volume_permission: {
    text: "Bottle can't write to this volume. Pick another volume in Settings.",
    action: { label: 'Open Settings', target: 'settings' },
  },
  too_long: {
    text: 'This recording is too long to transcribe.',
    action: { label: 'Open in Databricks', target: 'open' },
  },
  system_audio_denied: {
    text: "Bottle can't hear the other side of the call. Allow System Audio Recording.",
    action: { label: 'Open System Settings', target: 'system_audio' },
  },
  upload_failed: {
    text: "Bottle couldn't upload this recording. It is still on this device.",
    action: { label: 'Retry', target: 'retry' },
  },
  no_storage: {
    text: 'Pick where Bottle saves recordings in Settings.',
    action: { label: 'Open Settings', target: 'settings' },
  },
  recording_off: {
    text: RECORDING_TEXT.turnOn,
    action: { label: 'Open Settings', target: 'settings' },
  },
  consent_required: {
    text: RECORDING_TEXT.turnOn,
    action: { label: 'Open Settings', target: 'settings' },
  },
  no_profile: {
    text: 'Choose a home workspace in Settings → Workspace first, then try again.',
    action: { label: 'Open Settings', target: 'settings' },
  },
  recording_busy: { text: 'A recording is already running.' },
  mic_busy: { text: 'The mic is in use. Finish the dictation, then record.' },
  audio_unreadable: {
    text: "Bottle couldn't read the audio of this recording.",
    action: { label: 'Open in Databricks', target: 'open' },
  },
  rate_limited: {
    text: 'The model is busy right now. Try again in a minute.',
    action: { label: 'Retry', target: 'retry' },
  },
  network: {
    text: "Bottle can't reach Databricks. Check your network, then try again.",
    action: { label: 'Retry', target: 'retry' },
  },
  auth: {
    text: 'Your Databricks sign-in ended. Sign in again, then try again.',
    action: { label: 'Retry', target: 'retry' },
  },
  failed: {
    text: "Bottle couldn't finish this recording.",
    action: { label: 'Retry', target: 'retry' },
  },
} as const satisfies Record<string, RecordingNotice>;
export type RecordingErrorKind = keyof typeof RECORDING_ERRORS;

/** The notice of an error kind. An unknown kind reads as `failed`, never as upstream text. */
export function recordingNotice(kind: string | undefined): RecordingNotice {
  return (
    (RECORDING_ERRORS as Record<string, RecordingNotice>)[kind ?? ''] ?? RECORDING_ERRORS.failed
  );
}

/** The settings the section shows. */
export interface RecordingSettingsView {
  enabled: boolean;
  consentAck: boolean;
  lengthMin: number;
  transcribeEndpoint: string;
  notesEndpoint: string;
  instructions: string;
}

/** Where recordings go. `unavailable`: My Files did not resolve and no volume is set. */
export type StorageView =
  | { status: 'checking' }
  | { status: 'my_files'; path: string }
  | { status: 'volume'; path: string }
  | { status: 'unavailable'; error?: string }
  | { status: 'creating' };

export type RecordingPhase = 'idle' | 'starting' | 'recording' | 'stopping';

export interface RecordingControlView {
  phase: RecordingPhase;
  /** Epoch ms when the recording stops by itself (`ends_at_ms`). */
  endsAtMs?: number;
  /** Epoch ms when it started. */
  startedAtMs?: number;
  /** The system audio of the running recording; `mic_only` on a system without a tap. */
  systemAudio?: 'ok' | 'silent_or_denied' | 'mic_only';
}

/** The `hub:recording` payload. `settings` is missing until the brain reads them. */
export interface HubRecording {
  platform: RecordingPlatform;
  settings?: RecordingSettingsView;
  storage: StorageView;
  control: RecordingControlView;
  /** A problem with the Record control (a refused start). */
  error?: RecordingNotice;
  /** A setting the brain refused or could not save. */
  settingsError?: string;
}

export type RecordingStatus =
  'recording' | 'uploading' | 'transcribing' | 'writing_notes' | 'done' | 'failed';
const STATUSES: readonly RecordingStatus[] = [
  'recording',
  'uploading',
  'transcribing',
  'writing_notes',
  'done',
  'failed',
];

/** One row of the Recordings list. */
export interface RecordingRow {
  id: string;
  name: string;
  /** Epoch ms. */
  startedAt: number;
  durationS: number;
  status: RecordingStatus;
  /** Set when `status` is `failed`. */
  error?: RecordingNotice;
  /** The audio is in the volume, so "Open in volume" can show. */
  canOpen: boolean;
  /** Parts in the volume, and parts so far (upload in parts during the call). */
  partsUploaded?: number;
  partsTotal?: number;
  /** The audio is only on this device ("Not uploaded", with Upload). */
  onDeviceOnly?: true;
  /** A recording or upload that retries in the background until the network is back. */
  waitingNetwork?: true;
  /** Not uploaded, or failed at the upload: Upload all sends it. */
  needsUpload?: true;
  /** Its place in the Upload all batch that runs or just ran. */
  batch?: BatchState;
}

export type BatchState = 'queued' | 'running' | 'done' | 'failed';
const BATCH_STATES: readonly BatchState[] = ['queued', 'running', 'done', 'failed'];

/** Recordings cannot upload now. The banner shows `reason` under `RECORDING_TEXT.cantUpload`. */
export interface StorageProblem {
  reason: string;
  /** False when Upload all cannot work yet (no storage at all). */
  canUpload: boolean;
}

/** Upload all: the one action that sends every waiting recording, one at a time. */
export interface UploadAllView {
  running: boolean;
  /** Finished so far (done or failed). */
  done: number;
  total: number;
  failed: number;
}

export interface HubRecordings {
  rows: RecordingRow[];
  storageProblem?: StorageProblem;
  /** Rows that Upload all would send (`needsUpload`), or the batch that runs now. */
  uploadAll?: UploadAllView;
  unavailable?: true;
  /** Why the list could not be read, when the brain knows a plain reason. */
  error?: RecordingNotice;
}

/** One speaker turn: consecutive segments of one speaker, merged. */
export interface RecordingTurn {
  speaker: string;
  /** Seconds from the start of the recording. */
  startS: number;
  text: string;
}

/** The `hub:recording.detail` payload. */
export interface HubRecordingDetail {
  id: string;
  status: RecordingStatus;
  /** The notes markdown; missing until the job writes them. */
  notesMd?: string;
  /** The transcript markdown, for a recording whose turns could not be read. */
  transcriptMd?: string;
  turns: RecordingTurn[];
  error?: RecordingNotice;
  /** True when the brain could not read the files. */
  unavailable?: true;
}

/** The hub actions, sent as the `view_emit` payload. */
export type RecordingAction =
  | { action: 'recording.ready' }
  | { action: 'recording.enable'; on: boolean }
  | { action: 'recording.consent_ack' }
  | { action: 'recording.length'; minutes: number }
  | { action: 'recording.endpoint'; which: 'transcribe' | 'notes'; endpoint: string }
  | { action: 'recording.instructions'; text: string }
  | { action: 'recording.storage_probe' }
  | { action: 'recording.storage_volume'; path: string }
  | { action: 'recording.volume_create'; catalog: string; schema: string }
  | { action: 'recording.start'; instructions?: string }
  | { action: 'recording.stop' }
  | { action: 'recording.system_audio' }
  | { action: 'recordings.ready' }
  | { action: 'recordings.detail'; id: string }
  | { action: 'recordings.retry'; id: string }
  | { action: 'recordings.open'; id: string; target?: RecordingOpenWhat }
  | { action: 'recordings.upload_all' };

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const isStr = (v: unknown): v is string => typeof v === 'string';
const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const isId = (v: unknown): v is string => isStr(v) && v.length > 0 && v.length <= 128;

/** Shape check of a hub action. Unknown actions and fields are dropped. */
export function asRecordingAction(p: unknown): RecordingAction | undefined {
  if (!isObj(p) || !isStr(p['action'])) return undefined;
  const a = p['action'];
  switch (a) {
    case 'recording.ready':
    case 'recording.consent_ack':
    case 'recording.storage_probe':
    case 'recording.stop':
    case 'recording.system_audio':
    case 'recordings.ready':
    case 'recordings.upload_all':
      return { action: a };
    case 'recording.enable':
      return typeof p['on'] === 'boolean' ? { action: a, on: p['on'] } : undefined;
    case 'recording.length':
      return isNum(p['minutes']) ? { action: a, minutes: p['minutes'] } : undefined;
    case 'recording.endpoint':
      return (p['which'] === 'transcribe' || p['which'] === 'notes') && isStr(p['endpoint'])
        ? { action: a, which: p['which'], endpoint: p['endpoint'] }
        : undefined;
    case 'recording.instructions':
      return isStr(p['text']) ? { action: a, text: p['text'] } : undefined;
    case 'recording.storage_volume':
      return isStr(p['path']) ? { action: a, path: p['path'] } : undefined;
    case 'recording.volume_create':
      return isStr(p['catalog']) && isStr(p['schema'])
        ? { action: a, catalog: p['catalog'], schema: p['schema'] }
        : undefined;
    case 'recording.start':
      if (p['instructions'] === undefined) return { action: a };
      return isStr(p['instructions']) ? { action: a, instructions: p['instructions'] } : undefined;
    case 'recordings.detail':
    case 'recordings.retry':
      return isId(p['id']) ? { action: a, id: p['id'] } : undefined;
    case 'recordings.open': {
      if (!isId(p['id'])) return undefined;
      const t = p['target'];
      if (t === undefined) return { action: a, id: p['id'] };
      return t === 'folder' || t === 'transcript' || t === 'notes'
        ? { action: a, id: p['id'], target: t }
        : undefined;
    }
    default:
      return undefined;
  }
}

/** A `/Volumes/<catalog>/<schema>/<volume>` path, with a strict charset. */
export const VOLUME_PATH =
  /^\/Volumes\/[A-Za-z0-9_-]{1,255}\/[A-Za-z0-9_-]{1,255}\/[A-Za-z0-9_-]{1,255}\/?$/;
/** A `catalog.schema` pair. */
export const CATALOG_SCHEMA = /^([A-Za-z0-9_-]{1,255})\.([A-Za-z0-9_-]{1,255})$/;
/** A model id for the AI Gateway chat route, such as `system.ai.gemini-3-8-flash` (ADR 0123). */
export const ENDPOINT_NAME = /^[A-Za-z0-9_.-]{1,128}$/;

function isNotice(v: unknown): v is RecordingNotice {
  if (!isObj(v) || !isStr(v['text'])) return false;
  const act = v['action'];
  return (
    act === undefined ||
    (isObj(act) &&
      isStr(act['label']) &&
      ['settings', 'system_audio', 'retry', 'open'].includes(act['target'] as string))
  );
}
const optNotice = (v: unknown) => v === undefined || isNotice(v);

function isStorage(v: unknown): v is StorageView {
  if (!isObj(v)) return false;
  switch (v['status']) {
    case 'checking':
    case 'creating':
      return true;
    case 'my_files':
      return isStr(v['path']);
    case 'volume':
      return isStr(v['path']);
    case 'unavailable':
      return v['error'] === undefined || isStr(v['error']);
    default:
      return false;
  }
}

function isSettings(v: unknown): v is RecordingSettingsView {
  return (
    isObj(v) &&
    typeof v['enabled'] === 'boolean' &&
    typeof v['consentAck'] === 'boolean' &&
    isNum(v['lengthMin']) &&
    isStr(v['transcribeEndpoint']) &&
    isStr(v['notesEndpoint']) &&
    isStr(v['instructions'])
  );
}

function isControl(v: unknown): v is RecordingControlView {
  return (
    isObj(v) &&
    ['idle', 'starting', 'recording', 'stopping'].includes(v['phase'] as string) &&
    (v['endsAtMs'] === undefined || isNum(v['endsAtMs'])) &&
    (v['startedAtMs'] === undefined || isNum(v['startedAtMs'])) &&
    (v['systemAudio'] === undefined ||
      ['ok', 'silent_or_denied', 'mic_only'].includes(v['systemAudio'] as string))
  );
}

/** Shape check of a `hub:recording` payload. */
export function asHubRecording(p: unknown): HubRecording | undefined {
  if (!isObj(p) || (p['platform'] !== 'mac' && p['platform'] !== 'windows')) return undefined;
  if (p['settings'] !== undefined && !isSettings(p['settings'])) return undefined;
  if (!isStorage(p['storage']) || !isControl(p['control'])) return undefined;
  if (!optNotice(p['error'])) return undefined;
  if (p['settingsError'] !== undefined && !isStr(p['settingsError'])) return undefined;
  return p as unknown as HubRecording;
}

const isStatus = (v: unknown): v is RecordingStatus => STATUSES.includes(v as RecordingStatus);

function isRow(v: unknown): v is RecordingRow {
  return (
    isObj(v) &&
    isId(v['id']) &&
    isStr(v['name']) &&
    isNum(v['startedAt']) &&
    isNum(v['durationS']) &&
    isStatus(v['status']) &&
    optNotice(v['error']) &&
    typeof v['canOpen'] === 'boolean' &&
    (v['partsUploaded'] === undefined || isNum(v['partsUploaded'])) &&
    (v['partsTotal'] === undefined || isNum(v['partsTotal'])) &&
    (v['onDeviceOnly'] === undefined || v['onDeviceOnly'] === true) &&
    (v['waitingNetwork'] === undefined || v['waitingNetwork'] === true) &&
    (v['needsUpload'] === undefined || v['needsUpload'] === true) &&
    (v['batch'] === undefined || BATCH_STATES.includes(v['batch'] as BatchState))
  );
}

/** Shape check of a `hub:recordings` payload. */
export function asHubRecordings(p: unknown): HubRecordings | undefined {
  if (!isObj(p) || !Array.isArray(p['rows']) || !p['rows'].every(isRow)) return undefined;
  if (p['unavailable'] !== undefined && p['unavailable'] !== true) return undefined;
  if (!optNotice(p['error'])) return undefined;
  const sp = p['storageProblem'];
  if (
    sp !== undefined &&
    !(isObj(sp) && isStr(sp['reason']) && typeof sp['canUpload'] === 'boolean')
  )
    return undefined;
  const ua = p['uploadAll'];
  if (
    ua !== undefined &&
    !(
      isObj(ua) &&
      typeof ua['running'] === 'boolean' &&
      isNum(ua['done']) &&
      isNum(ua['total']) &&
      isNum(ua['failed'])
    )
  )
    return undefined;
  return p as unknown as HubRecordings;
}

/** Shape check of a `hub:recording.detail` payload. */
export function asHubRecordingDetail(p: unknown): HubRecordingDetail | undefined {
  if (!isObj(p) || !isId(p['id']) || !isStatus(p['status'])) return undefined;
  if (p['notesMd'] !== undefined && !isStr(p['notesMd'])) return undefined;
  if (p['transcriptMd'] !== undefined && !isStr(p['transcriptMd'])) return undefined;
  const t = p['turns'];
  if (
    !Array.isArray(t) ||
    !t.every((i) => isObj(i) && isStr(i['speaker']) && isNum(i['startS']) && isStr(i['text']))
  )
    return undefined;
  if (!optNotice(p['error'])) return undefined;
  if (p['unavailable'] !== undefined && p['unavailable'] !== true) return undefined;
  return p as unknown as HubRecordingDetail;
}

/** `[01:23]` or `[1:02:03]`: a time stamp from the start of the recording. */
export function stampOf(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${String(m).padStart(2, '0')}:${ss}`;
}

/** "42 min", "1 h 5 min", "Under 1 min". */
export function lengthOf(seconds: number): string {
  const m = Math.round(Math.max(0, seconds) / 60);
  if (m < 1) return 'Under 1 min';
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  const r = m % 60;
  return r === 0 ? `${h} h` : `${h} h ${r} min`;
}

/** The time left as `41:52` or `1:02:10`; never below zero. */
export function timeLeftOf(endsAtMs: number, nowMs: number): string {
  const s = Math.max(0, Math.ceil((endsAtMs - nowMs) / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

/** The label of a status in the list. */
export const STATUS_LABEL: Readonly<Record<RecordingStatus, string>> = {
  recording: 'Recording',
  uploading: 'Uploading',
  transcribing: 'Transcribing',
  writing_notes: 'Writing notes',
  done: 'Done',
  failed: 'Failed',
};

/** The status words of a row, with the part count while it uploads. */
export function statusLabelOf(
  row: Pick<
    RecordingRow,
    'status' | 'partsUploaded' | 'partsTotal' | 'onDeviceOnly' | 'waitingNetwork'
  >,
): string {
  if (row.onDeviceOnly) return 'Not uploaded';
  if (row.waitingNetwork) return 'Waiting for network';
  if (row.status === 'uploading' && row.partsTotal !== undefined && row.partsTotal > 0)
    return `Uploading ${Math.min(row.partsUploaded ?? 0, row.partsTotal)}/${row.partsTotal}`;
  return STATUS_LABEL[row.status];
}

/** The one action of a row that has none in its error: Upload (device only) or Retry (failed). */
export function rowActionOf(row: {
  status: RecordingStatus;
  onDeviceOnly?: true;
}): 'upload' | 'retry' | undefined {
  if (row.onDeviceOnly) return 'upload';
  return row.status === 'failed' ? 'retry' : undefined;
}
