/**
 * The `hotkey:ptt` item on the brain Channel (#6, `hotkey_host::ptt_payload`): `{ mogId, phase }`,
 * plus `shift: true` on a key-down with Shift held (RTR-17a, #59).
 * The brain stamps the time the item reached it; UX-05b and UX-36 start there (ADR 0031).
 * The capture lifecycle (`capture-lifecycle.ts`, #14) consumes it. `double_tap` (section 4.5) comes
 * after the second of two taps whose key-downs are 400 ms or less apart (CAP-14a, ADR 0057).
 */
import type { MogId } from '../shared/interfaces';
import { EVENTS } from '../shared/ipc';
import type { BrainBus } from './bus';

export type PttPhase = 'down' | 'up' | 'cancel' | 'double_tap';

export interface PttEvent {
  mogId: MogId;
  phase: PttPhase;
  /** When the item reached the brain (ms, the caller's clock). */
  at: number;
  /** RTR-17a: Shift was held with the talk key. Only on `down`, only when true (ADR 0053). */
  shift?: true;
}

const PHASES: ReadonlySet<string> = new Set<PttPhase>(['down', 'up', 'cancel', 'double_tap']);

export function asPttEvent(p: unknown, at: number): PttEvent | undefined {
  if (typeof p !== 'object' || p === null) return undefined;
  const { mogId, phase, shift } = p as { mogId?: unknown; phase?: unknown; shift?: unknown };
  if (typeof mogId !== 'string' || typeof phase !== 'string' || !PHASES.has(phase))
    return undefined;
  const e: PttEvent = { mogId, phase: phase as PttPhase, at };
  if (phase === 'down' && shift === true) e.shift = true;
  return e;
}

/** Call `handler` for each well-formed `hotkey:ptt` item, in the task that delivers it. */
export function onPtt(
  bus: BrainBus,
  now: () => number,
  handler: (e: PttEvent) => void,
): () => void {
  return bus.on(EVENTS['hotkey.ptt'].name, (p) => {
    const e = asPttEvent(p, now());
    if (e) handler(e);
  });
}
