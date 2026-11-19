/**
 * #573: the brain side of the transient Genie card and the double-click to the hub.
 *
 * - The pointer on the MOG holds the card timer: a MOG `hover` with `{ on }` goes to the card
 *   window of the same MOG only (`card:mog_hover`).
 * - The card fades by itself when its time is up and sends `click { target: 'card_done',
 *   requestId }`. The brain then hides the card window and clears that answer (its text, its
 *   sections and its Try again), so a card window that loads again later never shows it again.
 *   (Not `cancel`: a fade is no user Esc, so it never stops hands-free listening.)
 * - A double-click on the MOG or on the card sends `click { target: 'open_conversations' }`. A
 *   double-click is a user action: the hub opens and takes focus on Conversations, with the
 *   exchange of that card selected. A double-click on the MOG fades its card at once.
 *
 * No new `view_emit` kind and no contract change: these are `click` and `hover` payloads.
 */
import type { MogId } from '../shared/interfaces';
import {
  CARD_DISMISS_EVENT,
  CARD_DONE,
  CARD_MOG_HOVER_EVENT,
  OPEN_CONVERSATIONS,
} from '../shared/genie-status';
import { onViewInput, type BrainBus } from './bus';

/** An operation ID as the brain makes it (a UUID), or the debug demo ID. */
const OP_ID = /^[A-Za-z0-9-]{1,64}$/;

export interface GenieCardWiringDeps {
  /** `window_card_hide`. */
  hideCard(mogId: MogId): void;
  /** Clear the shown answer (the expanded sections) of the card stack. */
  clearAnswer(mogId: MogId): void;
  /** Empty the Genie blurb of `requestId` and drop its Try again, so it never shows again. */
  forget(mogId: MogId, requestId: string): void;
  /** Send an event to the card window of `mogId` only. */
  sendToCard(mogId: MogId, event: string, payload: Record<string, unknown>): void;
  /** The operation ID of the newest Genie request of `mogId`. */
  lastOpId(mogId: MogId): string | undefined;
  /** Show and focus the hub on Conversations; `requestId` selects one exchange. */
  openConversations(requestId: string | undefined): void;
}

function target(p: unknown): { target?: unknown; requestId?: unknown; on?: unknown } {
  return typeof p === 'object' && p !== null ? (p as Record<string, unknown>) : {};
}

export function wireGenieCard(bus: BrainBus, deps: GenieCardWiringDeps): () => void {
  return onViewInput(bus, (input) => {
    const { mogId } = input;
    const fromMog = input.from === `mog-${mogId}`;
    const fromCard = input.from === `card-${mogId}`;
    if (!fromMog && !fromCard) return;
    const p = target(input.payload);
    if (input.kind === 'hover' && fromMog && typeof p.on === 'boolean') {
      deps.sendToCard(mogId, CARD_MOG_HOVER_EVENT, { on: p.on });
      return;
    }
    if (input.kind !== 'click') return;
    const requestId =
      fromCard && typeof p.requestId === 'string' && OP_ID.test(p.requestId)
        ? p.requestId
        : undefined;
    if (p.target === CARD_DONE && fromCard) {
      // A late `card_done` of an older request never hides the card of a newer one.
      const current = deps.lastOpId(mogId);
      if (requestId !== undefined && current !== undefined && requestId !== current) {
        deps.forget(mogId, requestId);
        return;
      }
      deps.hideCard(mogId);
      deps.clearAnswer(mogId);
      if (requestId !== undefined) deps.forget(mogId, requestId);
      return;
    }
    if (p.target === OPEN_CONVERSATIONS) {
      if (fromMog) deps.sendToCard(mogId, CARD_DISMISS_EVENT, {});
      deps.openConversations(requestId ?? deps.lastOpId(mogId));
    }
  });
}
