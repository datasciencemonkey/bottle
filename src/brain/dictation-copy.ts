/**
 * The Copy card of a dictation (#498): the `onDone` sink of the utterance handler
 * (`utterance-wiring.ts`, #59, ADR 0053). No dictation text is dropped silently.
 *
 * - MOG-05b: a `copy` route (insertion off) puts the text and a Copy chip on the owner MOG's
 *   card, and shows the card.
 * - MOG-06d: an `insert_failed` route puts the text, the Appendix E line
 *   "Could not insert: <reason>" and a Copy chip on the card, and shows the card.
 * - RTR-01c: the flow ends here. This module never calls the main agent or Genie.
 *
 * TODO(#213): a click on Copy writes the clipboard with the concealed and transient flags. No
 * clipboard command is on main yet, so the chip only shows.
 * TODO(#60): the raw then cleaned text, the line layout and the "Inserted" pill (UX-42).
 */
import type { MogId } from '../shared/interfaces';
import type { CardStack } from './card-stack';
import type { OwnedUtterance } from './request-owner';
import type { InsertResult, UtteranceRoute } from './utterance-route';

/** The routes that reach `onDone` (every route but a preview or a chip). */
export type DoneRoute = Exclude<UtteranceRoute, { route: 'preview' | 'chip' }>;

export type InsertFailure = Exclude<InsertResult, 'inserted'> | 'host_failed';

const REASONS: Record<InsertFailure, string> = {
  no_permission: 'Bottle has no Accessibility permission',
  no_focus: 'no text field has focus',
  secure_field: 'the field is a password field',
  host_failed: 'Bottle could not reach the app',
};

/** The `<reason>` of the Appendix E line "Could not insert: <reason>" (MOG-06d). */
export const copyReasonText = (reason: InsertFailure): string => REASONS[reason];

export interface DictationSinkDeps {
  /** The card stack of a MOG. */
  stackFor(mogId: MogId): Pick<CardStack, 'add' | 'offerChip'>;
  /** Show the card window of a MOG (`window_card_show`). */
  showCard(mogId: MogId): Promise<unknown>;
}

export function createDictationSink(
  deps: DictationSinkDeps,
): (r: DoneRoute, owned: OwnedUtterance) => void {
  return (r, owned) => {
    if (r.route !== 'copy' && r.route !== 'insert_failed') return;
    const text =
      r.route === 'copy' ? r.text : `${r.text}\nCould not insert: ${copyReasonText(r.reason)}`;
    const stack = deps.stackFor(owned.mogId);
    stack.add({ id: owned.utteranceId, text, completed: true });
    stack.offerChip({
      id: `copy-${owned.utteranceId}`,
      kind: 'copy',
      label: 'Copy',
      owner: owned.utteranceId,
    });
    // The text is on the card before the show. A failed show keeps it there.
    deps.showCard(owned.mogId).catch(() => undefined);
  };
}
