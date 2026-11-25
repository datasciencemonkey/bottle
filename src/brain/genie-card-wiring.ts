/**
 * #573: the brain side of the transient Genie card and the double-click to the hub.
 *
 * - The pointer on the MOG holds the card timer: a MOG `hover` with `{ on }` goes to the card
 *   window of the same MOG only (`card:mog_hover`).
 * - The card fades by itself when its time is up and sends `click { target: 'card_done',
 *   requestId }`. The brain then hides the card window and clears that answer (its text, its
 *   sections and its Try again), so a card window that loads again later never shows it again.
 *   (Not `cancel`: a fade is no user Esc, so it never stops hands-free listening.)
 * - A close (x) of the status pill sends the same `card_done`. A close of the WORKING pill (the
 *   brain knows the request still works, or the view says `phase: 'working'`) only hides the card
 *   window: no clear, no forget. The request runs on and its answer shows the card again, through
 *   the arrival gate. A close of the READY pill is a real close (the path above), and the card
 *   stays closed for that request. A pill closed over a pop-up sends `pillOnly: true` (ready
 *   pill) or nothing (working pill), so the pop-up stays.
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
  MOG_MENU_CHANGE_HOME,
  MOG_MENU_CLOSE,
  OPEN_CONVERSATIONS,
} from '../shared/genie-status';
import { onViewInput, type BrainBus } from './bus';

/** An operation ID as the brain makes it (a UUID), or the debug demo ID. */
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
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
  /** True while the request `requestId` still works (not ready, no outcome yet). */
  isWorking?(mogId: MogId, requestId: string): boolean;
  /** The user closed the pill of a request that has not shown its outcome: it stays closed. */
  closePending?(mogId: MogId, requestId: string): void;
  /** The operation ID of the newest Genie request of `mogId`. */
  lastOpId(mogId: MogId): string | undefined;
  /** Show and focus the hub on Conversations; `requestId` selects one exchange. */
  openConversations(requestId: string | undefined): void;
  /**
   * #ui-4: the MOG right-click menu's Close. Same as the hub's Stop: the window closes and the
   * mic lease releases; the MOG is not deleted.
   */
  closeMog(mogId: MogId): void;
  /** #ui-4: the MOG right-click menu's "Change home workspace…". Shows the hub. */
  changeHome(mogId: MogId): void;
  /**
   * ADR 0113: "Remember this" on the card. `ticket` is the host's `actionTicket` of the click;
   * the brain never makes one. The callee sends `memory:saved` to that card only.
   */
  remember?(mogId: MogId, requestId: string | undefined, ticket: string | undefined): void;
  /** ADR 0113: the card's Undo of a save. The callee forgets that item only if this MOG saved it in the last 8 s. */
  undoRemember?(mogId: MogId, itemId: string): void;
  /**
   * The owner's "Copy" button request (ADR 0124 amendment): an explicit click on the direct
   * answer's own Copy control. `requestId` names the exchange the card shows (as "Remember
   * this" does); `undefined` when the click carried none.
   */
  copyAnswer?(mogId: MogId, requestId: string | undefined): void;
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
    // ADR 0113: only the card of this MOG saves or undoes a memory.
    const action = (p as { action?: unknown }).action;
    if (action === 'answer.copy') {
      if (fromCard) deps.copyAnswer?.(mogId, requestId);
      return;
    }
    if (action === 'memory.remember' || action === 'memory.undo') {
      if (!fromCard) return;
      if (action === 'memory.undo') {
        const itemId = (p as { itemId?: unknown }).itemId;
        if (typeof itemId === 'string' && UUID.test(itemId)) deps.undoRemember?.(mogId, itemId);
      } else {
        // The card names the exchange. There is no fallback to the newest request.
        deps.remember?.(mogId, requestId, input.actionTicket);
      }
      return;
    }
    if (p.target === CARD_DONE && fromCard) {
      // A close of the working pill only hides the card. The request runs on, its blurb and
      // answer stay, and the card shows again when the answer lands (owner decision).
      // The view says `phase: 'working'` when the pill it closed was working: if the answer
      // landed just after, this is still a hide, never a forget.
      const closedWorking = (p as { phase?: unknown }).phase === 'working';
      if (closedWorking || (requestId !== undefined && deps.isWorking?.(mogId, requestId))) {
        deps.hideCard(mogId);
        return;
      }
      // The stacked pill closed by the view: the pop-up stays, only the request is marked.
      if ((p as { pillOnly?: unknown }).pillOnly === true) {
        if (requestId !== undefined) deps.closePending?.(mogId, requestId);
        return;
      }
      if (requestId !== undefined) deps.closePending?.(mogId, requestId);
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
      return;
    }
    // #ui-4: Close and "Change home workspace…" are MOG right-click menu actions only; the card
    // has no such menu.
    if (!fromMog) return;
    if (p.target === MOG_MENU_CLOSE) deps.closeMog(mogId);
    else if (p.target === MOG_MENU_CHANGE_HOME) deps.changeHome(mogId);
  });
}
