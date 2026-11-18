/**
 * The `onUtterance` hook of the capture lifecycle (#14, ADR 0057): it routes each utterance
 * (#59, ADR 0053) for the MOG that owns it (#73, MOG-13a, ADR 0058).
 *
 * - MOG-13a: the MOG of the `hotkey:ptt` down owns the utterance. If that MOG is no longer
 *   open, nothing is routed.
 * - RTR-01c: a dictation, a Copy, a failed insertion and every other route end here, and the
 *   owner is released. No main agent and no Genie take part.
 * - RTR-04a, RTR-17b, B1: a preview or a chip goes to `onAsk` with its live owner. `onAsk`
 *   (T3 CapturePolicy, TODO(#142)) runs the 1.5 s preview or shows the chip, and it releases
 *   the owner. Nothing here sends to Genie.
 */
import type { MogId } from '../shared/interfaces';
import type { Utterance as CapturedUtterance } from './capture-lifecycle';
import type { RouterInput } from './components/router-step';
import type { OwnedUtterance, RequestOwner } from './request-owner';
import { routeUtterance, type UtteranceRoute, type UtteranceRouteDeps } from './utterance-route';

export type AskRoute = Extract<UtteranceRoute, { route: 'preview' | 'chip' }>;

export interface UtteranceHandlerDeps {
  owner: Pick<RequestOwner, 'begin' | 'release'>;
  /**
   * The router input of a MOG: the profile, the profiles and the MOGs. Undefined while no
   * profile is chosen (Bottle never picks one), so the utterance is not routed.
   */
  routerInput(mogId: MogId): RouterInput | undefined;
  /** `app.dictation.insert` (Appendix F). */
  insertionOn(mogId: MogId): boolean;
  route: UtteranceRouteDeps;
  /** A preview or a chip, with its live owner. The callee releases the owner. */
  onAsk(r: AskRoute, owned: OwnedUtterance): void;
  /** Every other route, after the owner is released (the dictation card, TODO(#60)). */
  onDone?(r: Exclude<UtteranceRoute, AskRoute>, owned: OwnedUtterance): void;
}

const isAsk = (r: UtteranceRoute): r is AskRoute => r.route === 'preview' || r.route === 'chip';

export function createUtteranceHandler(
  deps: UtteranceHandlerDeps,
): (u: CapturedUtterance) => Promise<void> {
  return async (u) => {
    const owned = deps.owner.begin(u.mogId);
    if (!owned) return;
    let handedOff = false;
    try {
      const input = deps.routerInput(owned.mogId);
      if (!input) return;
      const r = await routeUtterance(
        {
          mogId: owned.mogId,
          text: u.text,
          forceAsk: u.forceAsk,
          insertionOn: deps.insertionOn(owned.mogId),
          input: { ...input, speakerTrack: 'you', captureMode: u.mode },
        },
        deps.route,
      );
      if (isAsk(r)) {
        handedOff = true;
        deps.onAsk(r, owned);
        return;
      }
      deps.owner.release(owned.utteranceId);
      handedOff = true;
      deps.onDone?.(r, owned);
    } catch {
      // A failed route sends nothing and inserts nothing (B1). The capture lifecycle goes on.
    } finally {
      if (!handedOff) deps.owner.release(owned.utteranceId);
    }
  };
}
