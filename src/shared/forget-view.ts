/**
 * The Forget dialog model (MEM-32a, MEM-32c, MEM-32d, G-45b, AT-16; ADR 0081). The brain builds
 * it (`src/brain/components/forget-flow.ts`) and the thin view `ForgetDialog` shows it. It holds
 * no transcript, answer or memory text: only counts, conversation ids and links.
 */
import type { GenieOneDeleteNoticeModel } from './genie-one-delete-view';
import type { MogId } from './interfaces';

/** The link label of MEM-32d. */
export const GENIE_ONE_MEMORY_REVIEW = 'Review Genie One memories';

/** The `view_emit` click target of the confirm button. The payload has the scope kind only. */
export const FORGET_CONFIRM_TARGET = 'forget_confirm';

/** The `view_emit` click target of the memory review link. The payload has no value. */
export const GENIE_ONE_MEMORY_REVIEW_TARGET = 'genie_one_memory_review';

export type ForgetScopeKind = 'items' | 'session' | 'mog';

export interface ForgetDialogModel {
  mogId: MogId;
  scope: ForgetScopeKind;
  /** 'confirm' before the user selects Forget; 'done' after the local delete. */
  phase: 'confirm' | 'done';
  title: string;
  /** The confirm text; it ends with "You cannot undo this." */
  text: string;
  /** MEM-32d, D61: Genie One keeps its own memory; the dialog links to its review. */
  memoryReview: { label: string; text: string; url?: string };
  /** Only when `phase` is 'done'. */
  result?: {
    /** The local rows that Forget deleted. */
    local: number;
    /** The Genie conversations that Bottle deleted. */
    deletedConversations: number;
    /** The "Delete in Genie One" notice for the Genie One threads that Bottle did not delete. */
    notice?: GenieOneDeleteNoticeModel;
    /** The disclosure for other Genie conversations (genie_agent, custom) not deleted. */
    otherText?: string;
  };
}
