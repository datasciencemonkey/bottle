/**
 * The "Delete in Genie One" notice (G-45b, AT-16, ADR 0077). The brain builds it
 * (`src/brain/components/genie-one-delete.ts`) and the thin view `GenieOneDeleteNotice` shows it.
 * Forget (#123) and Wipe (#124) put it in their dialogs. It holds no title and no content: only
 * conversation IDs and their Genie One thread links.
 */
import type { MogId } from './interfaces';

/** The link label of G-45b, verbatim. */
export const DELETE_IN_GENIE_ONE = 'Delete in Genie One';

/** The `view_emit` click target of the link. The payload has the conversation ID only. */
export const GENIE_ONE_DELETE_TARGET = 'genie_one_delete';

export interface GenieOneDeleteNoticeModel {
  mogId: MogId;
  /** Always {@link DELETE_IN_GENIE_ONE}. */
  label: string;
  /** The disclosure: Bottle cannot delete (or could not delete) these conversations. */
  text: string;
  /** One "Delete in Genie One" link for each conversation that Bottle did not delete. */
  links: { conversationId: string; url: string }[];
  /** The number of conversations with no link (for example, no workspace ID is known). */
  noLink: number;
  /** The disclosure for the conversations with no link, when `noLink > 0`. */
  noLinkText?: string;
}
