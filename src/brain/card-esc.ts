/**
 * UX-19b: what the brain does when a card sends `cancel` (the user pressed Esc).
 *
 * One Esc does exactly one thing. A request in flight is cancelled and the card stays, so it can
 * show "cancellation requested". With no request in flight the card hides. The brain never
 * cancels and hides on the same press, and a failed action is not followed by the other one,
 * so the card is never left half closed. See ADR 0022.
 *
 * The gateway routes `window_card_hide` to R11 (#285, ADR 0040).
 * TODO(#374): the brain's `view_emit` handler calls `handleCardEsc` for `cancel` from a card.
 */
import type { ExchangeRef, MogId } from '../shared/interfaces';

/** What the brain knows about the card's MOG when Esc comes. */
export interface CardEscState {
  /** The Genie or agent request that runs for this MOG now, if any. */
  inFlight?: ExchangeRef;
}

export type CardEscAction = { t: 'cancel_request'; ref: ExchangeRef } | { t: 'hide_card' };

export interface CardEscDeps {
  /** `GenieBridge.cancel` (or the agent's cancel) for the request. */
  cancel(ref: ExchangeRef): Promise<void>;
  /** `window_card_hide` for the MOG. */
  hideCard(mogId: MogId): Promise<void>;
}

/** The one action for this Esc. */
export function decideCardEsc(state: CardEscState): CardEscAction {
  return state.inFlight ? { t: 'cancel_request', ref: state.inFlight } : { t: 'hide_card' };
}

/** Run the one action. `ok: false` means it failed and nothing else ran. */
export async function handleCardEsc(
  mogId: MogId,
  state: CardEscState,
  deps: CardEscDeps,
): Promise<{ t: CardEscAction['t']; ok: boolean }> {
  const action = decideCardEsc(state);
  try {
    if (action.t === 'cancel_request') await deps.cancel(action.ref);
    else await deps.hideCard(mogId);
    return { t: action.t, ok: true };
  } catch {
    return { t: action.t, ok: false };
  }
}
