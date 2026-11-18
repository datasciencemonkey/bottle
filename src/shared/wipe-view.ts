/**
 * The Settings → Wipe models (MEM-33a, SEC-15a, SEC-15c, MB-15, G-45b, U9, AT-16; ADR 0087). The
 * brain builds them (`src/brain/components/wipe-flow.ts`, `retention-settings.ts`) and the thin
 * hub view `WipeSettings` shows them. They hold no transcript, answer or memory text: only the
 * option, counts, conversation ids, links and retention labels.
 * It is not a contract file. TODO(#525): move the event names, the models and the click targets
 * into `src/shared/ipc.ts`, as #521 does for Forget.
 */
import type { GenieOneDeleteNoticeModel } from './genie-one-delete-view';

/** The three Wipe options of U9 (the `mem_wipe` scope, #525). */
export type WipeScope = 'older_than_30d' | 'older_than_90d' | 'everything';

export const WIPE_SCOPES: readonly WipeScope[] = ['older_than_30d', 'older_than_90d', 'everything'];

export const WIPE_OPTION_LABEL: Readonly<Record<WipeScope, string>> = {
  older_than_30d: 'Older than 30 days',
  older_than_90d: 'Older than 90 days',
  everything: 'Everything',
};

/** The 9.2 option for the Genie conversations that Bottle created. */
export const ALSO_DELETE_GENIE = 'Also delete Genie conversations';

/** MB-15: the text next to each MDM value. */
export const SET_BY_ADMIN = 'Set by your admin';

/** The brain-to-hub event of the dialog model, or `null` to close it. TODO(#525). */
export const WIPE_DIALOG_EVENT = 'hub:wipe_dialog';

/** The brain-to-hub event of the retention rows with their admin locks. TODO(#525). */
export const WIPE_SETTINGS_EVENT = 'hub:wipe_settings';

/**
 * The `mogId` of the hub's Wipe clicks. A Wipe is for the whole device, but `view_emit` needs a
 * MOG field, so the hub sends this fixed id. The brain acts on it only from the hub window.
 */
export const WIPE_VIEW_ID = 'wipe';

/** The Wipe button. The payload has the option and the Genie choice only. */
export const WIPE_OPEN_TARGET = 'wipe_open';
/** The confirm button. The payload has the option only. */
export const WIPE_CONFIRM_TARGET = 'wipe_confirm';
/** The Cancel and Close buttons. */
export const WIPE_CANCEL_TARGET = 'wipe_cancel';
/** The panel asks for its retention rows when it mounts. */
export const WIPE_SETTINGS_TARGET = 'wipe_settings';

export interface WipeDialogModel {
  scope: WipeScope;
  /** "Also delete Genie conversations" (9.2). */
  alsoGenie: boolean;
  /** 'confirm' before the user selects Wipe; 'done' after the local delete. */
  phase: 'confirm' | 'done';
  title: string;
  /** The confirm text; it ends with "You cannot undo this." */
  text: string;
  /** Only when `phase` is 'done'. */
  result?: {
    /** The local rows that the Wipe deleted. */
    local: number;
    /** The Genie conversations that Bottle deleted. */
    deletedConversations: number;
    /** One "Delete in Genie One" notice for each MOG with threads Bottle did not delete. */
    notices: GenieOneDeleteNoticeModel[];
    /** The disclosure for other Genie conversations that Bottle could not delete. */
    otherText?: string;
  };
}

/** One 9.2 row of the Settings panel. */
export interface RetentionRow {
  id: string;
  label: string;
  /** The effective value, for example "Kept until you wipe" or "7 days". */
  value: string;
  /** SEC-15c, MB-15: an admin policy sets this value; the user cannot change it. */
  locked: boolean;
}

export interface WipeSettingsModel {
  rows: RetentionRow[];
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const isStr = (v: unknown): v is string => typeof v === 'string';
const isCount = (v: unknown): v is number =>
  typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;

export function isWipeScope(v: unknown): v is WipeScope {
  return isStr(v) && (WIPE_SCOPES as readonly string[]).includes(v);
}

function asNotice(n: unknown): GenieOneDeleteNoticeModel | undefined {
  if (!isObj(n) || !isStr(n['mogId']) || !isStr(n['label']) || !isStr(n['text'])) return undefined;
  if (!isCount(n['noLink']) || !Array.isArray(n['links'])) return undefined;
  const links = n['links'].filter(
    (l): l is { conversationId: string; url: string } =>
      isObj(l) && isStr(l['conversationId']) && isStr(l['url']),
  );
  const m: GenieOneDeleteNoticeModel = {
    mogId: n['mogId'],
    label: n['label'],
    text: n['text'],
    links: links.map((l) => ({ conversationId: l.conversationId, url: l.url })),
    noLink: n['noLink'],
  };
  if (isStr(n['noLinkText'])) m.noLinkText = n['noLinkText'];
  return m;
}

/** Shape check of a `hub:wipe_dialog` payload. `null` closes the dialog. */
export function asWipeDialogModel(p: unknown): WipeDialogModel | null | undefined {
  if (p === null) return null;
  if (!isObj(p)) return undefined;
  const { scope, alsoGenie, phase, title, text, result: r } = p;
  if (!isWipeScope(scope) || typeof alsoGenie !== 'boolean') return undefined;
  if ((phase !== 'confirm' && phase !== 'done') || !isStr(title) || !isStr(text)) return undefined;
  const m: WipeDialogModel = { scope, alsoGenie, phase, title, text };
  if (r !== undefined) {
    if (!isObj(r) || !isCount(r['local']) || !isCount(r['deletedConversations'])) return undefined;
    if (!Array.isArray(r['notices'])) return undefined;
    const result: NonNullable<WipeDialogModel['result']> = {
      local: r['local'],
      deletedConversations: r['deletedConversations'],
      notices: r['notices'].flatMap((n) => asNotice(n) ?? []),
    };
    if (isStr(r['otherText'])) result.otherText = r['otherText'];
    m.result = result;
  }
  return m;
}

/** Shape check of a `hub:wipe_settings` payload. */
export function asWipeSettingsModel(p: unknown): WipeSettingsModel | undefined {
  if (!isObj(p) || !Array.isArray(p['rows'])) return undefined;
  const rows: RetentionRow[] = [];
  for (const r of p['rows']) {
    if (!isObj(r) || !isStr(r['id']) || !isStr(r['label']) || !isStr(r['value'])) return undefined;
    if (typeof r['locked'] !== 'boolean') return undefined;
    rows.push({ id: r['id'], label: r['label'], value: r['value'], locked: r['locked'] });
  }
  return { rows };
}
