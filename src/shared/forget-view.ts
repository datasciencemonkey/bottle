/**
 * The Forget dialog model (MEM-32a, MEM-32c, MEM-32d, G-45b, AT-16; ADR 0081). The brain builds
 * it (`src/brain/components/forget-flow.ts`) and the thin view `ForgetDialog` shows it. It holds
 * no transcript, answer or memory text: only counts, conversation ids and links.
 * It is not a contract file. TODO(#521): move `FORGET_DIALOG_EVENT`, the model and the click
 * targets into `src/shared/ipc.ts` (`EVENTS['hub.forget_dialog']`).
 */
import type { GenieOneDeleteNoticeModel } from './genie-one-delete-view';
import type { MogId } from './interfaces';

/** The link label of MEM-32d. */
export const GENIE_ONE_MEMORY_REVIEW = 'Review Genie One memories';

/** The brain-to-hub event that carries the dialog model, or `null` to close it. TODO(#521). */
export const FORGET_DIALOG_EVENT = 'hub:forget_dialog';

/** The `view_emit` click target of the hub "Forget memory" button (the MOG scope). */
export const FORGET_OPEN_TARGET = 'forget_open';

/** The `view_emit` click target of the dialog's Cancel and Close buttons. */
export const FORGET_CANCEL_TARGET = 'forget_cancel';

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

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const isStr = (v: unknown): v is string => typeof v === 'string';
const isCount = (v: unknown): v is number =>
  typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;
const SCOPES: readonly string[] = ['items', 'session', 'mog'];

/** Shape check of a `hub:forget_dialog` payload. `null` closes the dialog. */
export function asForgetDialogModel(p: unknown): ForgetDialogModel | null | undefined {
  if (p === null) return null;
  if (!isObj(p)) return undefined;
  const { mogId, scope, phase, title, text, memoryReview: mr, result: r } = p;
  if (!isStr(mogId) || !isStr(scope) || !SCOPES.includes(scope)) return undefined;
  if ((phase !== 'confirm' && phase !== 'done') || !isStr(title) || !isStr(text)) return undefined;
  if (!isObj(mr) || !isStr(mr['label']) || !isStr(mr['text'])) return undefined;
  if (mr['url'] !== undefined && !isStr(mr['url'])) return undefined;
  const memoryReview: ForgetDialogModel['memoryReview'] = { label: mr['label'], text: mr['text'] };
  if (isStr(mr['url'])) memoryReview.url = mr['url'];
  const m: ForgetDialogModel = {
    mogId,
    scope: scope as ForgetScopeKind,
    phase,
    title,
    text,
    memoryReview,
  };
  if (r !== undefined) {
    if (!isObj(r) || !isCount(r['local']) || !isCount(r['deletedConversations'])) return undefined;
    const result: NonNullable<ForgetDialogModel['result']> = {
      local: r['local'],
      deletedConversations: r['deletedConversations'],
    };
    if (isStr(r['otherText'])) result.otherText = r['otherText'];
    const n = r['notice'];
    if (isObj(n)) {
      const links = Array.isArray(n['links'])
        ? n['links'].filter(
            (l): l is { conversationId: string; url: string } =>
              isObj(l) && isStr(l['conversationId']) && isStr(l['url']),
          )
        : [];
      if (isStr(n['label']) && isStr(n['text']) && isCount(n['noLink'])) {
        result.notice = {
          mogId,
          label: n['label'],
          text: n['text'],
          links: links.map((l) => ({ conversationId: l.conversationId, url: l.url })),
          noLink: n['noLink'],
        };
        if (isStr(n['noLinkText'])) result.notice.noLinkText = n['noLinkText'];
      }
    }
    m.result = result;
  }
  return m;
}
