/**
 * The capture part of the MOG mood (#551, UX-01a, UX-07, MB-08a).
 *
 * - Each `capture:state` goes to the mood publisher for every MOG that a capture state or a
 *   refusal named, so the owner shows `listening` and goes back when the capture stops.
 * - A refused capture start is never silent: it is a `failed` request of that MOG (3 s `error`
 *   with the attention cue, 4.8) and one badge with a fixed reason text. The badge has no
 *   content: no transcript, device name or error message (SEC rules, CAP-01b).
 *
 * TODO(#98): the MOG lifecycle comes from the MogManager; until then every MOG is `ready`.
 * TODO(#39): the request phases come from the OperationTracker, so `processing` follows a stop.
 */
import type { CaptureState, MogId } from '../shared/interfaces';
import type { MogBadgeView } from '../shared/ipc';
import type { BrainBus } from './bus';
import type { MonotonicClock } from './clock';
import { ERROR_MS, type MogStatus, type RequestView } from './components/mood-mapper';
import { EVENTS } from '../shared/ipc';
import type { MoodPublisher, Scheduler } from './mood-publisher';

export const REFUSAL_REASONS = [
  'consent_required',
  'consent_storage_failed',
  'mic_busy',
  'host_failed',
  'permission_denied',
  'no_device',
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
};

const IPC_REASONS: ReadonlySet<string> = new Set<RefusalReason>([
  'consent_required',
  'consent_storage_failed',
  'mic_busy',
  'host_failed',
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
}

interface Refusal {
  reason: RefusalReason;
  at: number;
  /** The live lease when the start was refused (the post-roll of the last PTT, CAP-12b). */
  leaseAtRefusal: string | undefined;
  cancel: () => void;
}

const READY: MogStatus = { lifecycle: 'ready', since: 0 };
const IDLE: CaptureState = { recording: false, paused: false, othersTrack: 'off' };

export function createCaptureMood(deps: CaptureMoodDeps): CaptureMood {
  const { publisher, clock, scheduler } = deps;
  const known = new Set<MogId>();
  const refusals = new Map<MogId, Refusal>();
  let capture: CaptureState = IDLE;

  const publish = (mogId: MogId) => {
    const r = refusals.get(mogId);
    const requests: RequestView[] = r ? [{ phase: 'failed', at: r.at, userMustAct: true }] : [];
    const badges: MogBadgeView[] | undefined = r
      ? [{ kind: 'capture_refused', ...REFUSAL_TEXT[r.reason], hoverOnly: false }]
      : undefined;
    publisher.update(mogId, { mog: READY, capture, requests }, undefined, badges);
  };
  const clear = (mogId: MogId) => {
    refusals.get(mogId)?.cancel();
    refusals.delete(mogId);
  };

  return {
    onState(s) {
      capture = s;
      for (const id of [s.owner, s.directedTo]) if (id) known.add(id);
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
  };
}

/** Feed `capture:state` from the brain Channel. Returns `off`. */
export function wireCaptureMood(bus: BrainBus, cm: CaptureMood): () => void {
  return bus.on(EVENTS['capture.state'].name, (p) => {
    if (typeof p === 'object' && p !== null) cm.onState(p as CaptureState);
  });
}
