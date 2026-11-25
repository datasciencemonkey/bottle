/**
 * The brain side of call recording in the hub (ADR 0103, refs #65; ARC-2, ARC-5).
 *
 * The thin hub view sends clicks (id `recording`); the brain owns every rule and every text:
 * - Settings: the knob turns on only after the one consent acknowledgement; the length is a whole
 *   number of minutes from 5 to 120; the notes model is a strict endpoint name; a volume path is
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
  SYSTEM_AUDIO_HINT,
  SYSTEM_AUDIO_SETTINGS_OPEN_FAILED,
  VOLUME_PATH,
  type HubRecording,
  type HubRecordingDetail,
  type HubRecordings,
  type BatchState,
  type RecordingAction,
  type RecordingControlView,
  type RecordingOpenWhat,
  type RecordingNotice,
  type RecordingPlatform,
  type RecordingRow,
  type RecordingStatus,
  type RecordingTurn,
  type StorageProblem,
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
import { timeoutScheduler, type Scheduler } from './card-stack';
import { createSystemClock, type MonotonicClock } from './clock';

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
  /** ADR 0118: the one-time note "Your call can still hear you." was shown. Never set back. */
  heldNoteShown?: boolean;
}

/** The core recording state (`recording.state`). */
export interface RecState {
  phase: 'idle' | 'starting' | 'recording' | 'stopping';
  startedAtMs?: number;
  endsAtMs?: number;
  systemAudio?: 'ok' | 'silent_or_denied' | 'mic_only';
  /** The recording's folder name (matches `RecSummary.id`/`.name`): "A", review 2026-11-25. */
  name?: string;
}

/** One recording in the volume (`recording_list`). */
export interface RecSummary {
  id: string;
  name: string;
  startedAt: number;
  durationS: number;
  status: RecordingStatus;
  errorKind?: string;
  /** The step that failed; Retry starts again there. */
  errorStage?: RecordingStatus;
  partsUploaded?: number;
  partsTotal?: number;
  onDeviceOnly?: boolean;
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
  transcriptMd?: string;
  segments: RecSegment[];
}

/** A job status change (`recording.job`). */
export interface RecJob {
  id: string;
  status: RecordingStatus;
  errorKind?: string;
  errorStage?: RecordingStatus;
  durationS?: number;
  partsUploaded?: number;
  partsTotal?: number;
  onDeviceOnly?: boolean;
}

/**
 * The `recording_*` commands and events the brain uses. The real one is typed IPC
 * (`ipcRecordingApi`); tests pass a fake. A rejection carries `{ error: <code> }`.
 */
export interface RecordingApi {
  readSettings(): Promise<RecSettings>;
  writeSettings(next: RecSettings): Promise<RecSettings>;
  /** ADR 0118: write only `heldNoteShown: true`. It never sends the other fields. */
  markHeldNoteShown(): Promise<void>;
  /** The My Files root, or `null` when it does not resolve on this workspace. */
  probeStorage(): Promise<{ myFiles: string | null }>;
  createVolume(catalog: string, schema: string): Promise<{ path: string }>;
  start(instructions?: string): Promise<RecState>;
  stop(): Promise<RecState>;
  state(): Promise<RecState>;
  /**
   * `reconcile`: owner authorization 2026-11-25. Default (`false` or absent): the local index,
   * merged with the live overlays, with no network call. `true`: also read the volume once and
   * refresh the index, for the hub's first open and after a recording reaches `done` or
   * `failed`. Every other call omits it, so most reads make no network call at all.
   */
  list(reconcile?: boolean): Promise<RecSummary[]>;
  detail(id: string): Promise<RecDetail>;
  retry(id: string): Promise<unknown>;
  /** Delete a recording that is only on this Mac. */
  delete(id: string): Promise<unknown>;
  open(id: string, target?: RecordingOpenWhat): Promise<unknown>;
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
  /** Default: the system clock. Tests pass a `SimClock` to control the system audio hint timer. */
  clock?: MonotonicClock;
  /** Default: `timeoutScheduler(clock)`. */
  schedule?: Scheduler;
}

/** The OS notice when the notes of a recording are ready. */
export const NOTES_READY_TEXT = 'Your call notes are ready.';

/**
 * CAP-10a, ADR 0103 (#593 follow-up): how long system audio must stay silent before the calm
 * hint shows. macOS gives the same digital silence whether the permission is denied or nothing
 * is playing yet (the core's own 5 s check, `SYSTEM_SILENCE_MS`), so a short red alert on every
 * quiet moment is a false alarm; this waits much longer before saying anything.
 */
export const SYSTEM_AUDIO_HINT_MS = 20_000;

/** The refusals of `recordingProfile`: they have their own plain words. */
const PROFILE_CODES: ReadonlySet<string> = new Set(['no_profile']);

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

/** The audio is in the volume: past the upload, or failed after it. */
function inVolume(r: RecSummary): boolean {
  if (r.onDeviceOnly) return false;
  // Upload in parts: the folder exists as soon as the first part is in.
  if (r.status === 'recording' || r.status === 'uploading') return (r.partsUploaded ?? 0) > 0;
  if (r.status !== 'failed') return true;
  return (
    r.errorKind !== 'upload_failed' &&
    r.errorStage !== undefined &&
    r.errorStage !== 'recording' &&
    r.errorStage !== 'uploading'
  );
}

/** Error kinds that hit every recording alike: Upload all stops and says why. */
const STOP_KINDS = ['no_storage', 'volume_permission', 'auth', 'network'];

/** Not uploaded, or failed at the upload step: Upload all sends it. */
export function needsUpload(r: RecSummary): boolean {
  if (r.onDeviceOnly) return true;
  return (
    r.status === 'failed' &&
    (r.errorKind === 'upload_failed' ||
      r.errorStage === 'uploading' ||
      r.errorKind === 'no_storage')
  );
}

function rowOf(r: RecSummary): RecordingRow {
  const row: RecordingRow = {
    id: r.id,
    name: r.name,
    startedAt: r.startedAt,
    durationS: r.durationS,
    status: r.status,
    canOpen: inVolume(r),
  };
  // A recording that never left the device has no failure of its own: the list shows the
  // reason of the storage (when there is one), not "couldn't upload".
  if (r.status === 'failed' && !r.onDeviceOnly) row.error = recordingNotice(r.errorKind);
  if (r.partsUploaded !== undefined) row.partsUploaded = r.partsUploaded;
  if (r.partsTotal !== undefined) row.partsTotal = r.partsTotal;
  if (r.onDeviceOnly) row.onDeviceOnly = true;
  // A live recording or upload that waits for the network: not a failure, no action.
  if ((r.status === 'recording' || r.status === 'uploading') && r.errorKind === 'network')
    row.waitingNetwork = true;
  return row;
}

/** The MOG side of the one-time held note (`capture-mood.ts`). */
export interface HeldNoteMood {
  heldNote(mogId: string): void;
}

/**
 * ADR 0118: the first talk-key hold in a call recording shows the note "Your call can still hear
 * you." once, and keeps `heldNoteShown` in the recording settings so it never shows again. The
 * call app hears the user while they talk to Bottle; Bottle cannot mute the call.
 *
 * `held` takes a hold start. It does nothing outside a recording, or after the first note. The
 * flag is set in memory first, so two quick holds show one note; the write that stores it runs
 * after. A failed read leaves the flag unknown, which shows no note (fail quiet, not twice).
 */
export function createHeldNote(deps: {
  api: Pick<RecordingApi, 'readSettings' | 'markHeldNoteShown' | 'state' | 'onState'>;
  mood: HeldNoteMood;
}): { held(mogId: string): void; off(): void } {
  const { api, mood } = deps;
  let recording = false;
  let shown: boolean | undefined;
  let reading: Promise<void> | undefined;
  const off = api.onState((s) => {
    recording = s.phase === 'recording';
  });
  void api
    .state()
    .then((s) => {
      recording = recording || s.phase === 'recording';
    })
    .catch(() => undefined);
  const load = (): Promise<void> =>
    (reading ??= api
      .readSettings()
      .then((s) => {
        shown ??= s.heldNoteShown === true;
      })
      .catch(() => {
        reading = undefined;
      }));
  return {
    held(mogId) {
      if (!recording || shown === true) return;
      void load().then(async () => {
        // The recording may have ended while the settings were read.
        if (!recording || shown !== false) return;
        shown = true;
        mood.heldNote(mogId);
        try {
          // Only this flag goes out. A full copy from a stale read could turn the recording
          // knob back on after the user turned it off.
          await api.markHeldNoteShown();
        } catch {
          // The note shows again after a restart. That is better than a lost first note.
        }
      });
    },
    off,
  };
}

/** Wire the hub recording section, control and list. Returns the unsubscribe. */
export function wireRecording(bus: BrainBus, deps: RecordingWireDeps): () => void {
  const { api, platform } = deps;
  const send = deps.send ?? ((event: string, payload: unknown) => emitTo('hub', event, payload));
  const toHub = (event: string, payload: unknown) =>
    void send(event, payload).catch(() => undefined);
  const clock = deps.clock ?? createSystemClock();
  const schedule = deps.schedule ?? timeoutScheduler(clock);

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
  let listed = false;
  let gone = false;
  /** The Upload all batch that runs now, or the one that just ended. */
  let batch: { order: string[]; state: Map<string, BatchState>; running: boolean } | undefined;

  /**
   * Why recordings cannot upload now, or `undefined`. "No My Files" is the probe of ADR 0103
   * decision 7 (`MyFilesUnavailable`): no volume is set and My Files did not resolve. A failed
   * row with `volume_permission` says the chosen volume is not writable.
   */
  const storageProblem = (): StorageProblem | undefined => {
    if (!settings?.volumePath && myFiles === null && !storageError)
      return { reason: RECORDING_TEXT.noMyFiles, canUpload: false };
    if (rows.some((r) => r.status === 'failed' && r.errorKind === 'volume_permission'))
      return { reason: recordingNotice('volume_permission').text, canUpload: true };
    return undefined;
  };

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
    // A real error always wins; the calm hint only shows in its place, never over it (#593
    // follow-up). Computed fresh on every publish, so the hint reappears at once as soon as a
    // real error that was blocking it clears, with no need to catch that at the clearing site.
    const shown = error ?? (systemAudioHintDue ? SYSTEM_AUDIO_HINT : undefined);
    if (shown) m.error = shown;
    if (settingsError) m.settingsError = settingsError;
    toHub(HUB_RECORDING_EVENT, m);
  };

  /**
   * Set only while the most recent list read itself failed (review "3" of the 2026-11-26
   * round): `publishList` still shows the last rows it has, not an emptied list, with this
   * added so the hub can say the read failed without losing what was already known.
   */
  let listUnavailable = false;
  let listErrorNotice: RecordingNotice | undefined;

  const publishList = () => {
    if (gone) return;
    const sorted = [...rows].sort((a, b) => b.startedAt - a.startedAt || (a.id < b.id ? 1 : -1));
    const problem = storageProblem();
    const out: HubRecordings = {
      rows: sorted.map((r) => {
        const row = rowOf(r);
        // The banner explains why; a waiting row only says "Not uploaded".
        if (needsUpload(r)) row.needsUpload = true;
        // A call that records now, with nowhere to upload: a short local-only warning.
        if (r.status === 'recording' && problem && !problem.canUpload)
          row.error = {
            text: RECORDING_TEXT.localOnlyRow,
            action: { label: RECORDING_TEXT.chooseWhere, target: 'settings' },
          };
        const b = batch?.state.get(r.id);
        if (b) row.batch = b;
        return row;
      }),
    };
    if (problem) out.storageProblem = problem;
    if (listUnavailable) out.unavailable = true;
    if (listErrorNotice) out.error = listErrorNotice;
    if (batch) {
      const states = [...batch.state.values()];
      out.uploadAll = {
        running: batch.running,
        total: states.length,
        done: states.filter((s) => s === 'done' || s === 'failed').length,
        failed: states.filter((s) => s === 'failed').length,
      };
    } else {
      const n = rows.filter(needsUpload).length;
      if (n > 0) out.uploadAll = { running: false, total: n, done: 0, failed: 0 };
    }
    toHub(HUB_RECORDINGS_EVENT, out);
  };

  let recOn = false;
  /** Opus review round 2 (B1): one background reconcile per launch, from `recordings.ready`. */
  let reconciled = false;
  /** Opus review round 2 (L1): ids removed in this session. A late `recording.job` for one of
   * them (a straggler queued before the delete, or from an in-flight worker) is ignored
   * outright, never re-added to `rows`. Kept for the life of the brain session; never cleared. */
  const removedIds = new Set<string>();
  /** The name of the recording the device runs or just stopped (so Stop can select and read
   * its own row at once, "D" review 2026-11-25/26), or `undefined` with none known yet. */
  let currentName: string | undefined;
  /** Whether the last state read `silent_or_denied` while recording (the hint timer tracks it). */
  let systemAudioSilent = false;
  let systemAudioHintTimer: (() => void) | undefined;
  /** The one pending catch-up poll, or `undefined` with none scheduled. */
  let catchUpTimer: (() => void) | undefined;
  /**
   * How long to wait before re-reading the list while a take is still uploading,
   * transcribing or writing notes, or the list itself just failed to read ("A" review
   * 2026-11-25/26, then "6" of the 2026-11-26 round): `recording.job` is not sticky and
   * nothing else polls, so a missed or dropped event (or a transient list failure) must not
   * leave things stale for good. Doubles each time this poll itself has to fire again (3, 6,
   * 12, … up to `CATCH_UP_MAX_MS`), and resets to the base the moment a real `recording.job`
   * event arrives — a sign that updates are flowing normally again.
   */
  const CATCH_UP_BASE_MS = 3_000;
  const CATCH_UP_MAX_MS = 60_000;
  let catchUpDelayMs = CATCH_UP_BASE_MS;
  const resetCatchUpBackoff = () => {
    catchUpDelayMs = CATCH_UP_BASE_MS;
  };
  const IN_FLIGHT_STATUSES: ReadonlySet<RecordingStatus> = new Set([
    'recording',
    'uploading',
    'transcribing',
    'writing_notes',
  ]);
  /**
   * Re-read the list shortly, so a missed job event (or a slow or failed first read) still
   * catches up on its own. `force` arms it even with nothing in flight (review "6": the
   * list-failure path, which has no row statuses to check at all). The live take itself
   * (`currentName`) never alone justifies a poll: it is expected to stay `recording` for the
   * whole call, that is not something stalled. One timer at a time: each call replaces the
   * one before it.
   */
  const scheduleCatchUp = (force = false) => {
    catchUpTimer?.();
    catchUpTimer = undefined;
    const inFlight = rows.some((r) => r.id !== currentName && IN_FLIGHT_STATUSES.has(r.status));
    if (gone || !(force || inFlight)) {
      if (!force) resetCatchUpBackoff();
      return;
    }
    const delay = catchUpDelayMs;
    catchUpTimer = schedule(clock.now() + delay, () => {
      catchUpTimer = undefined;
      catchUpDelayMs = Math.min(catchUpDelayMs * 2, CATCH_UP_MAX_MS);
      void refreshList();
    });
  };
  /**
   * The silence has lasted past `SYSTEM_AUDIO_HINT_MS` (#593 follow-up). Kept separate from
   * `error`: a real error always wins in `publish`, but this stays true underneath it, so the
   * hint reappears on its own the moment that real error clears, with no lost opportunity.
   */
  let systemAudioHintDue = false;

  /** Cancel a pending hint timer, drop the due flag, and clear a shown hint or settings-open
   * failure (sound arrived, or the recording ended). Real errors are untouched. */
  const clearSystemAudioHint = () => {
    systemAudioHintTimer?.();
    systemAudioHintTimer = undefined;
    systemAudioHintDue = false;
    if (error === SYSTEM_AUDIO_HINT || error === SYSTEM_AUDIO_SETTINGS_OPEN_FAILED) {
      error = undefined;
    }
  };

  const setState = (s: RecState) => {
    control = controlOf(s);
    // CAP-10a: macOS gives the same digital silence whether System Audio Recording is denied
    // or nothing else is playing, so a short silence is never a red alert (#593 follow-up). A
    // long, unbroken silence gets a calm status hint instead, and sound clears it at once. The
    // red `system_audio_denied` notice keeps its own meaning for a real refusal elsewhere.
    const silentNow = s.systemAudio === 'silent_or_denied' && s.phase === 'recording';
    if (silentNow && !systemAudioSilent) {
      systemAudioHintTimer = schedule(clock.now() + SYSTEM_AUDIO_HINT_MS, () => {
        systemAudioHintTimer = undefined;
        systemAudioHintDue = true;
        publish();
      });
    } else if (!silentNow && systemAudioSilent) {
      clearSystemAudioHint();
    }
    systemAudioSilent = silentNow;
    const on = s.phase === 'recording';
    if (on !== recOn) {
      recOn = on;
      deps.mood?.callRecording(on);
      if (on) {
        // "D" review 2026-11-25/26, then "4" and "5" of the 2026-11-26 round: a new take
        // becomes the selected row the moment it starts, not only once it stops, so the
        // detail pane never keeps showing an old recording while this one runs. `currentName`
        // is set unconditionally, even with no name (nothing to track then either way).
        currentName = s.name;
        if (s.name) {
          detailId = s.name;
          // Select it, but do not fetch `detail` yet ("4"): there is nothing to show for a
          // still-recording take (no transcript, no notes) and the call would just fail or
          // run long. The detail pane follows this id for real once it is worth fetching:
          // the first `recording.job` event (`onJob`, below), or the stop case below.
          if (!gone)
            toHub(HUB_RECORDING_DETAIL_EVENT, { id: s.name, status: 'recording', turns: [] });
        }
        void refreshList();
        void warnIfLocalOnly();
      } else {
        // The call ended (Stop, ⌥R or the length): read the list now, so its row shows at
        // once and goes on to uploading, transcribing and notes, also with no job event yet.
        // Re-select the same take ("5"), but only if the user has not picked a different row
        // to look at during the call: `detailId` still being `currentName` (the auto-select
        // above, untouched since) or unset either way means nothing of the user's own choice
        // would be lost. A deliberate pick survives the stop.
        if (currentName && (detailId === currentName || detailId === undefined)) {
          void showDetail(currentName);
        }
        currentName = undefined;
        void refreshList();
      }
    }
    if (!on) clearSystemAudioHint();
    publish();
  };

  /**
   * A recording starts even when storage is unusable (the audio stays on the device), but the
   * user sees why at once, for the Record control and for ⌥R alike.
   */
  async function warnIfLocalOnly(): Promise<void> {
    if (myFiles === undefined) await probe();
    const p = storageProblem();
    if (!p || gone || !recOn) return;
    error = {
      text: `${p.reason} ${RECORDING_TEXT.keepsLocal}`,
      action: { label: RECORDING_TEXT.chooseWhere, target: 'settings' },
    };
    deps.notify?.(p.reason);
    publish();
  }

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
    } catch (e) {
      myFiles = null;
      storageError = PROFILE_CODES.has(codeOf(e))
        ? recordingNotice(codeOf(e)).text
        : RECORDING_TEXT.storageCheckFailed;
    }
    publish();
    if (listed) publishList();
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
    if (listed) publishList();
  }

  const showError = (e: unknown) => {
    error = recordingNotice(codeOf(e));
    publish();
  };

  const refuse = (text: string) => {
    settingsError = text;
    publish();
  };

  /** Each list read takes a number; only the newest read's reply is kept (no stale overwrite). */
  let listSeq = 0;
  /**
   * `reconcile`: owner authorization 2026-11-25, forwarded to `api.list`. The default read is
   * the fast, index-only path; `recordings.ready` (the hub's first open) and a recording
   * reaching `done` or `failed` (`onJob`, below) pass `true` so the index catches up with the
   * volume and first-run users see their existing recordings (part G).
   */
  async function refreshList(reconcile = false): Promise<void> {
    const mine = ++listSeq;
    if (!settings) settings = await api.readSettings().catch(() => undefined);
    if (myFiles === undefined && !batch?.running) await probe();
    try {
      const read = await api.list(reconcile);
      if (mine !== listSeq) return;
      rows = read;
      listed = true;
      listUnavailable = false;
      listErrorNotice = undefined;
    } catch (e) {
      if (mine !== listSeq) return;
      // Review "3" of the 2026-11-26 round: a failed read keeps the last rows this brain
      // already has (never blanks the list to `[]`) and only adds the notice on top, since
      // every recording exists locally first and this is the one call that could not run,
      // not proof that nothing exists.
      const code = codeOf(e);
      listUnavailable = true;
      listErrorNotice = PROFILE_CODES.has(code) ? recordingNotice(code) : undefined;
      publishList();
      // "6": arm one retry on this path too, through the same bounded, backing-off timer the
      // in-flight catch-up uses.
      scheduleCatchUp(true);
      return;
    }
    for (const r of rows) if (!seen.has(r.id)) seen.set(r.id, r.status);
    if (batch && !batch.running) batch = undefined;
    publishList();
    scheduleCatchUp();
  }

  /** Upload all: each waiting recording through the retry path, one at a time, oldest first. */
  async function uploadAll(): Promise<void> {
    if (batch?.running) return;
    await probe();
    await writes;
    const problem = storageProblem();
    if (problem && !problem.canUpload) {
      publishList();
      return;
    }
    const order = rows
      .filter(needsUpload)
      .sort((a, b) => a.startedAt - b.startedAt || (a.id < b.id ? -1 : 1))
      .map((r) => r.id);
    if (order.length === 0) return;
    const run = {
      order,
      state: new Map<string, BatchState>(order.map((id) => [id, 'queued'])),
      running: true,
    };
    batch = run;
    publishList();
    for (const id of order) {
      run.state.set(id, 'running');
      publishList();
      let result: BatchState = 'done';
      let stop = false;
      try {
        const r = await api.retry(id);
        if (isObj(r) && r['status'] === 'failed') {
          result = 'failed';
          // A failed result with a storage or sign-in kind stops the batch like a throw.
          const kind = typeof r['errorKind'] === 'string' ? r['errorKind'] : '';
          if (STOP_KINDS.includes(kind)) {
            stop = true;
            showError({ error: kind });
          }
        }
      } catch (e) {
        result = 'failed';
        // A storage or sign-in fault hits every recording alike: stop and say why.
        if (STOP_KINDS.includes(codeOf(e))) {
          stop = true;
          showError(e);
        }
      }
      run.state.set(id, result);
      if (stop) {
        for (const [k, v] of run.state) if (v === 'queued') run.state.delete(k);
        break;
      }
      publishList();
    }
    run.running = false;
    await refreshList();
  }

  async function showDetail(id: string): Promise<void> {
    detailId = id;
    let d: HubRecordingDetail;
    try {
      const raw = await api.detail(id);
      d = { id: raw.id, status: raw.status, turns: turnsOf(raw.segments) };
      if (raw.notesMd !== undefined) d.notesMd = raw.notesMd;
      if (d.turns.length === 0 && raw.transcriptMd) d.transcriptMd = raw.transcriptMd;
      if (raw.status === 'failed') d.error = recordingNotice(raw.errorKind);
    } catch {
      const row = rows.find((r) => r.id === id);
      d = { id, status: row?.status ?? 'failed', turns: [], unavailable: true };
    }
    if (detailId === id && !gone) toHub(HUB_RECORDING_DETAIL_EVENT, d);
  }

  function onJob(j: RecJob): void {
    // Opus review round 2 (L1): a straggler event for an id removed this session (queued
    // before the delete, or from an in-flight worker wrapping up) never puts the row back.
    if (removedIds.has(j.id)) return;
    const before = seen.get(j.id);
    seen.set(j.id, j.status);
    const row = rows.find((r) => r.id === j.id);
    if (row) {
      // The event carries the whole row state: a mark that went away (the network is back)
      // must clear too. Each event refreshes the open list; nothing polls.
      row.status = j.status;
      delete row.errorKind;
      delete row.errorStage;
      delete row.partsUploaded;
      delete row.partsTotal;
      delete row.onDeviceOnly;
      if (j.errorKind !== undefined) row.errorKind = j.errorKind;
      if (j.errorStage !== undefined) row.errorStage = j.errorStage;
      if (j.durationS !== undefined && j.durationS > 0) row.durationS = j.durationS;
      if (j.partsUploaded !== undefined) row.partsUploaded = j.partsUploaded;
      if (j.partsTotal !== undefined) row.partsTotal = j.partsTotal;
      if (j.onDeviceOnly) row.onDeviceOnly = true;
      publishList();
      // `recording.job` is not sticky and nothing else polls ("A" review 2026-11-25/26): keep
      // a bounded catch-up armed in case the next event is missed, until the row reaches a
      // terminal status. A real event arriving is evidence updates are flowing normally
      // again, so the backoff resets to its base ("6" of the 2026-11-26 round) rather than
      // keeping whatever it had climbed to.
      resetCatchUpBackoff();
      scheduleCatchUp();
    } else void refreshList();
    // Opus review round 2 (B2): `before === undefined` means this is the first time this
    // brain has ever heard of this id (freshly seen in a list reply, not a live transition
    // just now), so the terminal-status branch below — the reconcile and the user-facing
    // notice — only runs for an id this brain was already tracking.
    if (before !== undefined) {
      // A recording reaching a terminal status reconciles the index in the background (owner
      // authorization 2026-11-25, part C): the row itself already updated above, so this does
      // not block that; it just catches up the index and any other row the volume knows about.
      if ((j.status === 'done' || j.status === 'failed') && before !== j.status)
        void refreshList(true);
      if (j.status === 'done' && before !== 'done') {
        deps.notify?.(NOTES_READY_TEXT);
        deps.mood?.ready();
      }
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
        // The one acknowledgement saves the consent and turns the knob on in one write.
        void write({ consentAck: true, enabled: true });
        break;
      case 'recording.enable':
        // The consent notice comes first, once. The check runs in the write queue, so a click
        // right after the acknowledgement sees it. The core checks the knob again on start.
        void (writes = writes.then(async () => {
          if (a.on && !(settings?.consentAck ?? false)) publish();
          else await writeNow({ enabled: a.on });
        }));
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
        // A failure (no host handler, or the OS refused to open the pane) shows plain steps
        // with no action, never nothing: Retry would just fail the same way again.
        void api.openSystemAudioSettings().catch(() => {
          error = SYSTEM_AUDIO_SETTINGS_OPEN_FAILED;
          publish();
        });
        break;
      case 'recordings.ready':
        // Opus review round 2 (B1): the fast, index-only path paints first, every time the
        // area shows, so first paint never waits on a reconcile's volume read. Only one
        // reconcile runs per launch (part G: a first-run user with an empty index still sees
        // the volume's existing recordings once that one background read lands, and the
        // index is written so every later open is instant with no reconcile at all).
        if (listed) publishList();
        void refreshList().then(() => {
          if (!reconciled) {
            reconciled = true;
            void refreshList(true);
          }
        });
        break;
      case 'recordings.detail':
        void showDetail(a.id);
        break;
      case 'recordings.retry':
        void api.retry(a.id).then(
          () => refreshList(),
          (e: unknown) => showError(e),
        );
        break;
      case 'recordings.delete':
        void api.delete(a.id).then(
          () => {
            removedIds.add(a.id);
            void refreshList();
          },
          (e: unknown) => showError(e),
        );
        break;
      case 'recordings.open':
        void (a.target ? api.open(a.id, a.target) : api.open(a.id)).catch((e: unknown) =>
          showError(e),
        );
        break;
      case 'recordings.upload_all':
        void uploadAll();
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
    systemAudioHintTimer?.();
    catchUpTimer?.();
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
  if (s.name !== undefined) out.name = s.name;
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
  if (s.errorStage !== undefined) r.errorStage = s.errorStage;
  if (s.partsUploaded !== undefined) r.partsUploaded = s.partsUploaded;
  if (s.partsTotal !== undefined) r.partsTotal = s.partsTotal;
  if (s.onDeviceOnly) r.onDeviceOnly = true;
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
  if (s.heldNoteShown) r.heldNoteShown = true;
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
  // Only the held note sets it, and only to true: a stale copy must never reset it.
  if (s.heldNoteShown) p.heldNoteShown = true;
  return p;
}

/**
 * The real `RecordingApi`: the typed `recording_*` commands and the core events on `bus`.
 * `profile` is `recordingProfile` (one workspace for every MOG). Every command that reaches
 * Databricks carries it; when it refuses, that command fails with its code and nothing goes out.
 */
export function ipcRecordingApi(bus: BrainBus, profile: () => Promise<ProfileName>): RecordingApi {
  const home = async (): Promise<ProfileName> => {
    const p = await profile().catch((e: unknown) => {
      throw PROFILE_CODES.has(codeOf(e)) ? e : { error: 'no_profile' };
    });
    if (p === '') throw { error: 'no_profile' };
    return p;
  };
  return {
    readSettings: async () => recSettingsOf(await call('recording_settings_read', {})),
    markHeldNoteShown: async () => {
      await call('recording_settings_write', { patch: { heldNoteShown: true } });
    },
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
    list: async (reconcile) =>
      (
        await call(
          'recording_list',
          reconcile ? { profile: await home(), reconcile: true } : { profile: await home() },
        )
      ).map(summaryOf),
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
      if (d.transcriptMd !== undefined) r.transcriptMd = d.transcriptMd;
      return r;
    },
    retry: async (id) => call('recording_retry', { profile: await home(), id }),
    delete: async (id) => call('recording_delete', { profile: await home(), id }),
    open: async (id, target = 'folder') =>
      call('recording_open', { profile: await home(), id, target }),
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
