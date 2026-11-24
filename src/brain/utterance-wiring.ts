/**
 * The `onUtterance` hook of the capture lifecycle (#14, ADR 0057): it routes each utterance
 * (#59, ADR 0053) for the MOG that owns it (#73, MOG-13a, ADR 0058).
 *
 * - MOG-13a: the MOG of the `hotkey:ptt` down owns the utterance. If that MOG is no longer
 *   open, nothing is routed.
 * - RTR-01c: a route with no ask ends here, and the owner is released. No main agent and no
 *   Genie take part.
 * - RTR-17a, B1: an ⌥G ask or a chip goes to `onAsk` with its live owner. `onAsk` (the T3 ask
 *   gate, `ask-gate.ts`, #142) sends the ⌥G ask or shows the chip, and it releases the owner.
 *   Nothing here sends to Genie.
 * - ADR 0098: an utterance with no ⌥G is dictated first. Its fallback chip goes to `onAsk`, then
 *   its dictation (inserted, failed, or Copy) goes to `onDone`, so a refused paste still reaches
 *   the Copy card.
 */
import type { MogId } from '../shared/interfaces';
import type { Utterance as CapturedUtterance } from './capture-lifecycle';
import type { RouterInput } from './components/router-step';
import type { OwnedUtterance, RequestOwner } from './request-owner';
import { routeUtterance, type UtteranceRoute, type UtteranceRouteDeps } from './utterance-route';

export type AskRoute = Extract<UtteranceRoute, { route: 'ask' | 'chip' }>;
export type RememberRoute = Extract<UtteranceRoute, { route: 'remember' }>;

export interface UtteranceHandlerDeps {
  owner: Pick<RequestOwner, 'begin' | 'release'>;
  /**
   * The router input of a MOG: the profile, the profiles and the MOGs (#163). With no home
   * profile it still comes, so the RTR-15c heuristic routes the utterance.
   */
  routerInput(mogId: MogId): RouterInput | Promise<RouterInput>;
  /** `app.dictation.insert` (Appendix F). */
  insertionOn(mogId: MogId): boolean;
  route: UtteranceRouteDeps;
  /** An ⌥G ask or a chip, with its live owner. The callee releases the owner, unless it throws. */
  onAsk(r: AskRoute, owned: OwnedUtterance): void;
  /**
   * Every other route, after the owner is released. It is required: a `copy` or an
   * `insert_failed` route holds dictation text that must reach the Copy card (#498).
   */
  onDone(r: Exclude<UtteranceRoute, AskRoute | RememberRoute>, owned: OwnedUtterance): void;
  /** ADR 0113: a "remember this" order, after the owner is released. Nothing is pasted. */
  onRemember?(r: RememberRoute, owned: OwnedUtterance): void;
  /**
   * UX slice 1: the route of an utterance ended. `pasted` is true when `insert_text` inserted the
   * text. `segment` names the hold's lease, so a late resolve never frees a newer hold. The
   * arrival gate applies held arrivals 1.2 s later. It runs on every end, also a fault.
   */
  onPasteResolved?(pasted: boolean, segment: { segmentId: string; leaseId: string }): void;
}

const isAsk = (r: UtteranceRoute): r is AskRoute => r.route === 'ask' || r.route === 'chip';

export function createUtteranceHandler(
  deps: UtteranceHandlerDeps,
): (u: CapturedUtterance) => Promise<void> {
  return async (u) => {
    const owned = deps.owner.begin(u.mogId);
    if (!owned) {
      deps.onPasteResolved?.(false, u.segment);
      return;
    }
    let handedOff = false;
    let pasted = false;
    try {
      const input = await deps.routerInput(owned.mogId);
      const r = await routeUtterance(
        {
          mogId: owned.mogId,
          text: u.text,
          leaseId: u.segment.leaseId,
          forceAsk: u.forceAsk,
          insertionOn: deps.insertionOn(owned.mogId),
          input: { ...input, speakerTrack: 'you', captureMode: u.mode },
          ...(u.remember !== undefined ? { remember: u.remember } : {}),
        },
        deps.route,
      );
      pasted = r.route === 'inserted' || (r.route === 'chip' && r.dictated?.route === 'inserted');
      if (isAsk(r)) {
        deps.onAsk(r, owned);
        // Only a call that returns hands the owner off; a throw releases it below.
        handedOff = true;
        if (r.route === 'chip' && r.dictated) deps.onDone?.(r.dictated, owned);
        return;
      }
      deps.owner.release(owned.utteranceId);
      handedOff = true;
      if (r.route === 'remember') deps.onRemember?.(r, owned);
      else deps.onDone(r, owned);
    } catch {
      // A failed route sends nothing and inserts nothing (B1). The capture lifecycle goes on.
    } finally {
      if (!handedOff) deps.owner.release(owned.utteranceId);
      deps.onPasteResolved?.(pasted, u.segment);
    }
  };
}
