/**
 * The capture part of the MOG mood (#551, UX-01a, UX-07, MB-08a).
 *
 * - Each `capture:state` goes to the mood publisher for every MOG that a capture state or a
 *   refusal named, so the owner shows `listening` and goes back when the capture stops.
 * - A refused capture start is never silent: it is a `failed` request of that MOG (3 s `error`
 *   with the attention cue, 4.8) and one badge with a fixed reason text. The badge has no
 *   content: no transcript, device name or error message (SEC rules, CAP-01b).
 *
 * - #573: `ask` gives the phase of an Ask Genie request of a MOG: `working` is `processing` until
 *   the request ends; `answered` is `answering` for 1.2 s, then `success` for 2.5 s; `failed` is
 *   `error` for 3 s (no attention cue: the card shows the reason and Try again). Then `idle`.
 *   `pending` (the ⌥G key-up) is `processing` until the request starts, for 15 s or less.
 *
 * TODO(#98): the MOG lifecycle comes from the MogManager; until then every MOG is `ready`.
 * TODO(#39): the request phases come from the OperationTracker, so `processing` follows a stop.
 */
import type { CaptureState, MogId } from '../shared/interfaces';
import type { MogBadgeView } from '../shared/ipc';
import type { BrainBus } from './bus';
import type { MonotonicClock } from './clock';
import {
  BLURB_REVEAL_MS,
  ERROR_MS,
  SUCCESS_MS,
  type MogStatus,
  type RequestView,
} from './components/mood-mapper';
import type { GeniePhase } from './genie-direct';
import { EVENTS } from '../shared/ipc';
import { CALL_ONLY_BADGE } from './components/mog-badges';
import type { MoodPublisher, Scheduler } from './mood-publisher';

export const REFUSAL_REASONS = [
  'consent_required',
  'consent_storage_failed',
  'mic_busy',
  'host_failed',
  'permission_denied',
  'no_device',
  'recording_in_progress',
  'no_home_profile',
  'no_profile',
] as const;
export type RefusalReason = (typeof REFUSAL_REASONS)[number];

/** The short fail-closed text on the MOG, and its accessible label. */
export const REFUSAL_TEXT: Readonly<Record<RefusalReason, { text: string; label: string }>> = {
  consent_required: {
    text: 'Accept the notice first',
    label: 'Mic not started: accept the notice in the Bottle hub first',
  },
  consent_storage_failed: {
    text: 'Mic blocked: no consent',
    label: 'Mic not started: Bottle could not read the consent records',
  },
  mic_busy: { text: 'Mic in use', label: 'Mic not started: another capture uses the mic' },
  host_failed: { text: 'Mic did not start', label: 'Mic not started: try again' },
  permission_denied: {
    text: 'No mic permission',
    label: 'Mic not started: allow the mic for Bottle in the system settings',
  },
  no_device: { text: 'No mic found', label: 'Mic not started: connect a microphone' },
  // ADR 0103, ADR 0118: during a recording a hold works (PTT). Hands-free, and any start while the
  // recording starts or stops, does not take the mic.
  recording_in_progress: {
    text: 'Recording in progress',
    label: 'Mic not started: a call recording uses the mic. Press ⌥R in a hold to stop it',
  },
  // ADR 0103: the ⌥R start needs the home profile; Bottle never picks one.
  no_home_profile: {
    text: 'Pick a workspace first',
    label: 'Recording not started: pick a home workspace for this MOG first',
  },
  // ADR 0103, recording-profile.ts: ⌥R shares the hub Record control's `recordingProfile`. The
  // user has not picked a home workspace yet.
  no_profile: {
    text: 'Pick a workspace first',
    label: 'Recording not started: no home workspace yet; pick one first',
  },
};

/** ADR 0103: the capture owner of a call recording. It is not a MOG. */
export const RECORDING_OWNER = 'bottle-recording';

const IPC_REASONS: ReadonlySet<string> = new Set<RefusalReason>([
  'consent_required',
  'consent_storage_failed',
  'mic_busy',
  'host_failed',
  'recording_in_progress',
  'no_home_profile',
  'no_profile',
]);

/** The reason of a refused start. Only a known `IpcError` code counts; anything else is host_failed. */
export function refusalReason(err: unknown): RefusalReason {
  const code =
    typeof err === 'object' && err !== null ? (err as { error?: unknown }).error : undefined;
  return typeof code === 'string' && IPC_REASONS.has(code)
    ? (code as RefusalReason)
    : 'host_failed';
}

/** A mic fault in `capture:state.mic` names the reason of a refused open better (CAP-17b). */
const micFault = (s: CaptureState): RefusalReason | undefined =>
  s.mic === 'permission_denied' || s.mic === 'no_device' ? s.mic : undefined;

export interface CaptureMoodDeps {
  publisher: MoodPublisher;
  clock: MonotonicClock;
  scheduler: Scheduler;
}

export interface CaptureMood {
  /** One `capture:state` item. */
  onState(s: CaptureState): void;
  /** The capture lifecycle `onRefused`: R2 or the consent gate refused a start for `mogId`. */
  refused(mogId: MogId, error: unknown): void;
  /**
   * #573: the phase of the Ask Genie request of `mogId`. A newer phase replaces the older.
   * `pending` is the ⌥G key-up, before the text is ready: `processing` for up to
   * `PENDING_ASK_MS`, so an utterance that never becomes a request does not stay working.
   */
  ask(mogId: MogId, phase: GeniePhase | 'pending'): void;
  /**
   * ADR 0103: a call recording started (`on`) or ended. While it runs, each known MOG and each
   * MOG in `mogIds` shows the REC overlay; the mood does not change.
   */
  callRecording(on: boolean, mogIds?: readonly MogId[]): void;
  /** ADR 0103: the notes of a recording are ready: the answer-ready moment on each known MOG. */
  recordingReady(): void;
  /**
   * ADR 0118: a talk-key hold runs (`on`) or ended in `mogId`. While one runs during a call
   * recording the MOG shows the badge "Call only". Outside a recording it shows nothing.
   */
  hold(mogId: MogId, on: boolean): void;
  /**
   * ADR 0118: show the note "Your call can still hear you." on `mogId` for a few seconds. The
   * caller (`recording-wiring.ts`) keeps the once-only rule.
   */
  heldNote(mogId: MogId): void;
}

interface Refusal {
  reason: RefusalReason;
  at: number;
  /** The live lease when the start was refused (the post-roll of the last PTT, CAP-12b). */
  leaseAtRefusal: string | undefined;
  cancel: () => void;
}

/** ADR 0118: the note after the first hold in a recording. The call app still hears the user. */
export const HELD_NOTE_TEXT = 'Your call can still hear you.';

/** ADR 0118: how long the note stays on the MOG. */
export const HELD_NOTE_MS = 6_000;

/** #573: how long the ⌥G key-up shows `processing` before its request starts. */
export const PENDING_ASK_MS = 15_000;

const READY: MogStatus = { lifecycle: 'ready', since: 0 };
const IDLE: CaptureState = { recording: false, paused: false, othersTrack: 'off' };

export function createCaptureMood(deps: CaptureMoodDeps): CaptureMood {
  const { publisher, clock, scheduler } = deps;
  const known = new Set<MogId>();
  const refusals = new Map<MogId, Refusal>();
  const asks = new Map<MogId, { view: RequestView; cancel?: () => void }>();
  /** ADR 0118: the MOGs whose talk key is held now. */
  const holds = new Set<MogId>();
  const notes = new Map<MogId, () => void>();
  let capture: CaptureState = IDLE;
  let callRec = false;
  /** When the last recording notes were ready (the success window of 4.8). */
  let readyAt: number | undefined;

  const publish = (mogId: MogId) => {
    const r = refusals.get(mogId);
    const requests: RequestView[] = r ? [{ phase: 'failed', at: r.at, userMustAct: true }] : [];
    const a = asks.get(mogId);
    if (a) requests.push(a.view);
    if (readyAt !== undefined) requests.push({ phase: 'completed', at: readyAt });
    const list: MogBadgeView[] = [];
    if (r) list.push({ kind: 'capture_refused', ...REFUSAL_TEXT[r.reason], hoverOnly: false });
    if (callRec && holds.has(mogId)) list.push({ ...CALL_ONLY_BADGE });
    if (notes.has(mogId))
      list.push({
        kind: 'call_note',
        text: HELD_NOTE_TEXT,
        label: HELD_NOTE_TEXT,
        hoverOnly: false,
      });
    const badges: MogBadgeView[] | undefined = list.length > 0 ? list : undefined;
    const sources = { mog: READY, capture, requests, ...(callRec ? { callRecording: true } : {}) };
    publisher.update(mogId, sources, undefined, badges);
  };
  const clear = (mogId: MogId) => {
    refusals.get(mogId)?.cancel();
    refusals.delete(mogId);
  };

  /** Show `phase` from now; after `ms`, run `next` (or end the request mood). */
  const setAsk = (
    mogId: MogId,
    phase: RequestView['phase'] | undefined,
    ms?: number,
    next?: () => void,
  ) => {
    asks.get(mogId)?.cancel?.();
    asks.delete(mogId);
    if (phase !== undefined) {
      const at = clock.now();
      const slot: { view: RequestView; cancel?: () => void } = { view: { phase, at } };
      if (ms !== undefined)
        slot.cancel = scheduler.at(at + ms, () => {
          if (asks.get(mogId) !== slot) return;
          if (next) next();
          else {
            asks.delete(mogId);
            publish(mogId);
          }
        });
      asks.set(mogId, slot);
    }
    publish(mogId);
  };

  return {
    ask(mogId, phase) {
      known.add(mogId);
      switch (phase) {
        case 'pending':
          return setAsk(mogId, 'working', PENDING_ASK_MS);
        case 'working':
          return setAsk(mogId, 'working');
        case 'answered':
          return setAsk(mogId, 'blurb_reveal', BLURB_REVEAL_MS, () =>
            setAsk(mogId, 'completed', SUCCESS_MS),
          );
        case 'failed':
          return setAsk(mogId, 'failed', ERROR_MS);
        case 'cancelled':
          return setAsk(mogId, undefined);
      }
    },
    onState(s) {
      capture = s;
      // ADR 0103: the call recording holds the mic as `bottle-recording`, which is not a MOG.
      for (const id of [s.owner, s.directedTo]) if (id && id !== RECORDING_OWNER) known.add(id);
      const fault = micFault(s);
      for (const [id, r] of refusals) {
        // A new granted capture of this MOG ends its refusal. A repeated state of the lease that
        // was live at the refusal does not. A mic fault names a host failure.
        if (
          s.recording &&
          s.owner === id &&
          (r.leaseAtRefusal === undefined || s.leaseId !== r.leaseAtRefusal)
        )
          clear(id);
        else if (fault && r.reason === 'host_failed') r.reason = fault;
      }
      for (const id of known) publish(id);
    },
    refused(mogId, error) {
      known.add(mogId);
      // A newer refusal replaces the older one and its 3 s timer.
      clear(mogId);
      let reason = refusalReason(error);
      if (reason === 'host_failed') reason = micFault(capture) ?? reason;
      const at = clock.now();
      const cancel = scheduler.at(at + ERROR_MS, () => {
        refusals.delete(mogId);
        publish(mogId);
      });
      const leaseAtRefusal = capture.recording ? capture.leaseId : undefined;
      refusals.set(mogId, { reason, at, leaseAtRefusal, cancel });
      publish(mogId);
    },
    callRecording(on, mogIds = []) {
      for (const id of mogIds) known.add(id);
      callRec = on;
      // ADR 0118: a note that waits for its timer must not outlive the recording.
      if (!on) {
        for (const cancel of notes.values()) cancel();
        notes.clear();
      }
      for (const id of known) publish(id);
    },
    hold(mogId, on) {
      known.add(mogId);
      if (on) holds.add(mogId);
      else holds.delete(mogId);
      publish(mogId);
    },
    heldNote(mogId) {
      known.add(mogId);
      notes.get(mogId)?.();
      const cancel = scheduler.at(clock.now() + HELD_NOTE_MS, () => {
        if (notes.get(mogId) !== cancel) return;
        notes.delete(mogId);
        publish(mogId);
      });
      notes.set(mogId, cancel);
      publish(mogId);
    },
    recordingReady() {
      const at = clock.now();
      readyAt = at;
      for (const id of known) publish(id);
      scheduler.at(at + SUCCESS_MS, () => {
        if (readyAt !== at) return;
        readyAt = undefined;
        for (const id of known) publish(id);
      });
    },
  };
}

/** Feed `capture:state` from the brain Channel. Returns `off`. */
export function wireCaptureMood(bus: BrainBus, cm: CaptureMood): () => void {
  return bus.on(EVENTS['capture.state'].name, (p) => {
    if (typeof p === 'object' && p !== null) cm.onState(p as CaptureState);
  });
}
