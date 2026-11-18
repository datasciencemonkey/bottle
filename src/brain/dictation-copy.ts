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
 * The order (PR #504 review): the sink shows the card first, then puts the text on the card
 * stack. A new card window listens only after its view mounts, so `cardReady` sends the kept
 * state again (TODO(#507): the card view sends `view_emit` `ready`). A failed show is tried
 * once more. If that also fails, the card stack keeps the text for a later show or `ready`,
 * and `onUnshown` gets the text for a place the user can see.
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
  stackFor(mogId: MogId): Pick<CardStack, 'add' | 'offerChip' | 'republish'>;
  /** Show the card window of a MOG (`window_card_show`). */
  showCard(mogId: MogId): Promise<unknown>;
  /** The show failed twice: give the dictation text to a place the user can see. */
  onUnshown(mogId: MogId, text: string): void;
}

export interface DictationSink {
  /** The `onDone` of the utterance handler. */
  onDone(r: DoneRoute, owned: OwnedUtterance): Promise<void>;
  /** The card view of a MOG listens now: send it the kept state again. */
  cardReady(mogId: MogId): void;
}

export function createDictationSink(deps: DictationSinkDeps): DictationSink {
  /** The MOGs whose card stack holds dictation text. */
  const held = new Set<MogId>();
  const show = async (mogId: MogId): Promise<boolean> => {
    for (let attempt = 0; attempt < 2; attempt++) {
      try {
        await deps.showCard(mogId);
        return true;
      } catch {
        // Try once more. The error is not logged: it holds no text, but it is not needed.
      }
    }
    return false;
  };
  return {
    async onDone(r, owned) {
      if (r.route !== 'copy' && r.route !== 'insert_failed') return;
      const shown = await show(owned.mogId);
      const text =
        r.route === 'copy' ? r.text : `${r.text}\nCould not insert: ${copyReasonText(r.reason)}`;
      // The stack keeps the text whether or not the show worked; a later show or `ready`
      // sends it again.
      const stack = deps.stackFor(owned.mogId);
      held.add(owned.mogId);
      stack.add({ id: owned.utteranceId, text, completed: true });
      stack.offerChip({
        id: `copy-${owned.utteranceId}`,
        kind: 'copy',
        label: 'Copy',
        owner: owned.utteranceId,
      });
      if (!shown) deps.onUnshown(owned.mogId, r.text);
    },
    cardReady(mogId) {
      if (held.has(mogId)) deps.stackFor(mogId).republish();
    },
  };
}
