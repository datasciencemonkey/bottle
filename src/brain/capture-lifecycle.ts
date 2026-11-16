/**
 * The PTT and hands-free capture lifecycle in the brain (#14, ADR 0057).
 *
 * R1 decides the key rules (tap under 250 ms, CAP-44a; `double_tap`, CAP-14a). R2 `CaptureCore`
 * owns the timing rules: the 120 ms post-roll after `release` (CAP-12b), the drop of a discarded
 * lease's audio (CAP-44b), the 5 min cap (CAP-04b) and the VAD end (CAP-14c). This module maps
 * the brain events to those R2 calls, in the task that gets the event (no timer, no await before
 * the call):
 *
 * - `down`: start a PTT lease through the consent gate (CAP-12a, MB-08a).
 * - `up`: release the lease; R2 stops it after the post-roll (CAP-12b).
 * - `cancel` (a tap, or another key, CAP-44c): discard the lease (CAP-44b).
 * - `double_tap`, a mascot click or "Talk": start hands-free (CAP-14a), if `mog.capture.modes`
 *   has it. A click on the listening MOG stops it when `clickTogglesListening` is on.
 * - A tap or Esc during hands-free: stop hands-free (CAP-14b). Esc also drops its audio.
 *
 * Each kept segment goes to STT, then to `onUtterance` (the #424 `routeUtterance` hook). A
 * discarded lease never reaches STT, and an empty STT text fires nothing.
 *
 * TODO(#425): the host does not route `capture_*` to `CaptureCore` yet, so `ipcCaptureControl`
 * fails until it does; the tests use a double of R2.
 */
import type { CaptureMode, CaptureState, LeaseId, MogId, SegmentRef } from '../shared/interfaces';
import { EVENTS, call } from '../shared/ipc';
import { onViewInput, type BrainBus } from './bus';
import { startCapture, type CaptureGateDeps } from './capture-gate';
import { asPttEvent, type PttEvent } from './ptt';

/** CAP-44a: a talk-key hold shorter than this is a tap (R1 `TAP_MAX_HOLD`). */
export const TAP_MAX_HOLD_MS = 250;

/** The capture modes this lifecycle starts. */
export type LifecycleMode = Extract<CaptureMode, 'ptt' | 'hands_free'>;

/** What stops hands-free (CAP-14b). Only `escape` drops the audio. */
export type HandsFreeStopBy = 'tap' | 'escape' | 'click';

/** The R2 calls of the lifecycle. */
export interface CaptureControl {
  /** Start a lease. The returned state names it (`leaseId`). */
  start(args: { mogId: MogId; mode: LifecycleMode }): Promise<CaptureState>;
  /** Key-up: R2 stops the PTT after the 120 ms post-roll (CAP-12b). */
  release(args: { mogId: MogId; leaseId: LeaseId }): Promise<unknown>;
  /** A tap or a cancel: R2 stops the lease now and drops its audio (CAP-44b). */
  discard(args: { mogId: MogId; leaseId: LeaseId }): Promise<unknown>;
  /** Stop hands-free (CAP-14b). */
  stopHandsFree(args: { mogId: MogId; leaseId: LeaseId; by: HandsFreeStopBy }): Promise<unknown>;
}

/** One finished PTT or hands-free segment with its STT text (the #424 `routeUtterance` input). */
export interface Utterance {
  mogId: MogId;
  text: string;
  /** RTR-17a: Shift was held at the PTT key-down. Always false for hands-free. */
  forceAsk: boolean;
  segment: SegmentRef;
  mode: CaptureMode;
}

/** The capture settings of one MOG (Appendix F `mog.capture.*`). */
export interface LifecycleConfig {
  modes: readonly CaptureMode[];
  clickTogglesListening: boolean;
}

export interface CaptureLifecycleDeps {
  control: CaptureControl;
  /** The STT step of one segment. TODO(#46): local Parakeet through `stt_transcribe`. */
  transcribe(segment: SegmentRef): Promise<string>;
  /** Fired once for each kept segment with a non-empty text. */
  onUtterance(u: Utterance): void | Promise<void>;
  /** The capture settings of a MOG. Default: the Appendix F defaults. */
  config?: (mogId: MogId) => LifecycleConfig;
}

/** A `hotkey:ptt` item. #424 adds `shift: true` to a key-down with Shift held (RTR-17a). */
export type LifecyclePttEvent = PttEvent & { shift?: true };

export interface CaptureLifecycle {
  /** One `hotkey:ptt` item. */
  onPtt(e: LifecyclePttEvent): void;
  /** A mascot click or "Talk" for `mogId` (CAP-14a). */
  click(mogId: MogId): void;
  /** Esc in a Bottle view (CAP-14b). TODO(#434): the host stops hands-free in R2 on the global Esc. */
  escape(): void;
  /** One `capture:state` item: R2 ends hands-free on the cap or the VAD end. */
  onState(s: CaptureState): void;
  /** One `capture:segment` item. */
  onSegment(seg: SegmentRef): Promise<void>;
}

const DEFAULT_CONFIG: LifecycleConfig = {
  modes: ['ptt', 'hands_free'],
  clickTogglesListening: true,
};

/** Leases the lifecycle remembers, so late segments find their lease. Leases are never reused. */
const MAX_LEASES = 64;

interface LeaseInfo {
  mogId: MogId;
  mode: LifecycleMode;
  forceAsk: boolean;
  discarded: boolean;
}

/** A lease that is starting or live. `end` is the call that waits for the grant. */
interface Live {
  mogId: MogId;
  mode: LifecycleMode;
  forceAsk: boolean;
  leaseId?: LeaseId;
  end?: ((leaseId: LeaseId) => void) | undefined;
  /** `capture:state` showed this lease live, so a later idle state is its stop. */
  seen?: boolean;
}

/** A talk-key press: its PTT lease, and whether it began during hands-free. */
interface Press extends Live {
  downAt: number;
  overHandsFree: boolean;
}

export function createCaptureLifecycle(deps: CaptureLifecycleDeps): CaptureLifecycle {
  const config = deps.config ?? (() => DEFAULT_CONFIG);
  const leases = new Map<LeaseId, LeaseInfo>();
  let press: Press | undefined;
  let handsFree: Live | undefined;
  /** The last two presses stopped hands-free with a tap: a `double_tap` they are part of must not
   * start it again. */
  let stoppedBy: [boolean, boolean] = [false, false];

  const remember = (leaseId: LeaseId, l: Live) => {
    leases.set(leaseId, { mogId: l.mogId, mode: l.mode, forceAsk: l.forceAsk, discarded: false });
    while (leases.size > MAX_LEASES) {
      const first = leases.keys().next().value;
      if (first === undefined) break;
      leases.delete(first);
    }
  };
  const discardAudio = (leaseId: LeaseId) => {
    const info = leases.get(leaseId);
    if (info) info.discarded = true;
  };

  /** Run `fn` with the lease ID now, or when the grant comes. */
  const whenGranted = (l: Live, fn: (leaseId: LeaseId) => void) => {
    if (l.leaseId !== undefined) fn(l.leaseId);
    else l.end = fn;
  };

  const start = (l: Live, onFail: () => void) => {
    deps.control.start({ mogId: l.mogId, mode: l.mode }).then(
      (state) => {
        const leaseId = state.leaseId;
        if (leaseId === undefined) return onFail();
        l.leaseId = leaseId;
        remember(leaseId, l);
        const end = l.end;
        l.end = undefined;
        end?.(leaseId);
      },
      // consent_required, mic_busy or host_failed: no lease, nothing to stop.
      onFail,
    );
  };

  const settle = (p: Promise<unknown>) => void p.catch(() => undefined);

  const startHandsFree = (mogId: MogId) => {
    if (handsFree || press || !config(mogId).modes.includes('hands_free')) return;
    const hf: Live = { mogId, mode: 'hands_free', forceAsk: false };
    handsFree = hf;
    start(hf, () => {
      if (handsFree === hf) handsFree = undefined;
    });
  };

  const stopHandsFree = (by: HandsFreeStopBy) => {
    const hf = handsFree;
    if (!hf) return;
    handsFree = undefined;
    whenGranted(hf, (leaseId) => {
      if (by === 'escape') discardAudio(leaseId);
      settle(deps.control.stopHandsFree({ mogId: hf.mogId, leaseId, by }));
    });
  };

  const onDown = (e: LifecyclePttEvent) => {
    if (press) return;
    const p: Press = {
      mogId: e.mogId,
      mode: 'ptt',
      forceAsk: e.shift === true,
      downAt: e.at,
      overHandsFree: handsFree !== undefined,
    };
    press = p;
    start(p, () => {
      if (press === p) press = undefined;
    });
  };

  const onEnd = (e: PttEvent, keep: boolean) => {
    const p = press;
    if (!p || p.mogId !== e.mogId) return;
    press = undefined;
    const tap = !keep && e.at - p.downAt < TAP_MAX_HOLD_MS;
    whenGranted(p, (leaseId) => {
      if (!keep) discardAudio(leaseId);
      const args = { mogId: p.mogId, leaseId };
      settle(keep ? deps.control.release(args) : deps.control.discard(args));
    });
    // CAP-14b: a tap during hands-free stops it, after its own lease is discarded.
    const stops = tap && p.overHandsFree && handsFree !== undefined;
    if (stops) stopHandsFree('tap');
    stoppedBy = [stoppedBy[1], stops];
  };

  return {
    onPtt(e) {
      switch (e.phase) {
        case 'down':
          return onDown(e);
        case 'up':
          return onEnd(e, true);
        case 'cancel':
          return onEnd(e, false);
        case 'double_tap':
          if (stoppedBy[0] || stoppedBy[1]) return;
          return startHandsFree(e.mogId);
      }
    },
    click(mogId) {
      if (handsFree?.mogId === mogId) {
        if (config(mogId).clickTogglesListening) stopHandsFree('click');
        return;
      }
      startHandsFree(mogId);
    },
    escape() {
      stopHandsFree('escape');
    },
    onState(s) {
      // R2 stopped hands-free (cap, VAD end, panic) or a session took the mic.
      // An idle state from before the grant (a tap's stop) is not the stop of hands-free: R2
      // sends the grant state first, on the same ordered Channel.
      const hf = handsFree;
      if (hf?.leaseId === undefined) return;
      if (s.recording && s.leaseId === hf.leaseId) hf.seen = true;
      else if (hf.seen && (!s.recording || s.mode === 'session')) handsFree = undefined;
    },
    async onSegment(seg) {
      const info = leases.get(seg.leaseId);
      // CAP-44b: a discarded lease never reaches STT. Leases of other flows are not ours.
      if (!info || info.discarded) return;
      const text = await deps.transcribe(seg);
      if (text.trim() === '') return;
      await deps.onUtterance({
        mogId: info.mogId,
        text,
        forceAsk: info.forceAsk,
        segment: seg,
        mode: info.mode,
      });
    },
  };
}

/**
 * The production R2 calls. Every start goes through the consent gate (MB-08a).
 * TODO(#425): `capture_stop` is not routed to `CaptureCore` yet. TODO(#433): `capture_stop`
 * needs an `end` argument so the host calls `CaptureCore::release`, `discard` or
 * `stop_hands_free`; until then each one is a plain stop.
 */
export function ipcCaptureControl(gate: CaptureGateDeps): CaptureControl {
  const stop = (args: { mogId: MogId; leaseId: LeaseId }) => call('capture_stop', args);
  return {
    start: (args) => startCapture(gate, args),
    release: stop,
    discard: stop,
    stopHandsFree: ({ mogId, leaseId }) => stop({ mogId, leaseId }),
  };
}

const isMascotClick = (payload: unknown) => payload === undefined || payload === null;

/**
 * Feed the lifecycle from the brain Channel: `hotkey:ptt`, `capture:state`, `capture:segment`
 * and the mascot click of a MOG view (`view:input` `click` with no payload). Returns `off`.
 */
export function wireCaptureLifecycle(
  bus: BrainBus,
  lc: CaptureLifecycle,
  now: () => number,
): () => void {
  const offs = [
    bus.on(EVENTS['hotkey.ptt'].name, (p) => {
      const e = asPttEvent(p, now());
      if (!e) return;
      // Keep `shift` for RTR-17a (#424) on a key-down.
      const { shift } = p as { shift?: unknown };
      const withShift: LifecyclePttEvent = e;
      if (e.phase === 'down' && shift === true) withShift.shift = true;
      lc.onPtt(withShift);
    }),
    bus.on(EVENTS['capture.state'].name, (p) => {
      if (typeof p === 'object' && p !== null) lc.onState(p as CaptureState);
    }),
    bus.on(EVENTS['capture.segment'].name, (p) => {
      if (typeof p !== 'object' || p === null) return;
      const seg = p as SegmentRef;
      if (typeof seg.leaseId !== 'string') return;
      void lc.onSegment(seg).catch(() => undefined);
    }),
    onViewInput(bus, (input) => {
      if (input.kind === 'click' && input.from === `mog-${input.mogId}`) {
        if (isMascotClick(input.payload)) lc.click(input.mogId);
      }
    }),
  ];
  return () => offs.forEach((off) => off());
}
