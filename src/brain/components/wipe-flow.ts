/**
 * The Settings → Wipe flow and its dialog model (#124; MEM-33a, SEC-15a, G-45a, G-45b, G-49, 9.2,
 * U9, AT-16). See ADR 0087.
 *
 * U9: Bottle deletes no user content on its own. This flow is the only brain path to the R14 Wipe,
 * and it runs only when the user confirms the hub dialog (`src/brain/wipe-wiring.ts`). It has no
 * timer.
 *
 * Order:
 * 1. R14 in Rust wipes the local rows of every MOG in one transaction (`mem_wipe`, #525). It
 *    answers the deleted row count and the Genie conversations of the wiped exchanges that no kept
 *    exchange uses (ids only). This works offline.
 * 2. Only if the user also selected "Also delete Genie conversations" (9.2), delete them where an
 *    API permits it, for each MOG with the Forget helper (`deleteGenieConversations('wipe', ...)`):
 *    Genie One threads go to the #122 utility, whose SP-11 gate is off (TODO(#121)), so the dialog
 *    shows "Delete in Genie One" (G-45a, G-45b); `genie_agent` ones go to GA `DELETE` (G-49).
 * 3. Build the done dialog. A Genie failure never undoes the local Wipe.
 */
import {
  createDbxGenieAgentDelete,
  deleteGenieConversations,
  notDeletedText,
  type GenieAgentConversationDelete,
  type GenieOneThreadDelete,
} from './forget-flow';
import { conversationOf, type ForgetConversation } from './memory-service';
import { NotImplementedError } from '../errors';
import type { MogId } from '../../shared/interfaces';
import { isWipeScope, type WipeDialogModel, type WipeScope } from '../../shared/wipe-view';

/** The answer of R14 `mem_wipe` (#525): the local row count and the conversation rows. */
export interface WipeCoreResult {
  local: number;
  /** `{ mogId, backendKind, profile, host, workspaceId?, spaceId?, conversationId }` rows. */
  conversations: unknown[];
}

/** The R14 seam. Production: `mem_wipe` (TODO(#525)). */
export interface WipeCore {
  wipe(scope: WipeScope): Promise<WipeCoreResult>;
}

/**
 * The R14 WipeService over IPC. TODO(#525): `mem_wipe` is not in the contract yet, so this
 * refuses before any call, and the dialog says that nothing changed. When #525 lands, it sends
 * `call('mem_wipe', { scope })` and answers the reply.
 */
export function createIpcWipeCore(): WipeCore {
  return {
    async wipe(_scope) {
      throw new NotImplementedError('WipeService', 'wipe', 525);
    },
  };
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

const CONTENT = 'the transcripts, dictation history, Genie answers and notes';

const CONFIRM: Record<WipeScope, { title: string; what: string }> = {
  older_than_30d: {
    title: 'Wipe data older than 30 days',
    what: `Delete ${CONTENT} older than 30 days`,
  },
  older_than_90d: {
    title: 'Wipe data older than 90 days',
    what: `Delete ${CONTENT} older than 90 days`,
  },
  everything: { title: 'Wipe everything', what: `Delete all of ${CONTENT}` },
};

const GENIE_ALSO = ' Bottle also deletes the Genie conversations of these answers where it can.';
const GENIE_STAYS = ' Your Genie conversations stay in Genie.';

export interface WipeFlowDeps {
  /** Default {@link createIpcWipeCore}. */
  core?: WipeCore;
  /** The #122 utility. Production: `new GenieOneDeleteUtility()`. */
  genieOne: GenieOneThreadDelete;
  /** Default {@link createDbxGenieAgentDelete}. */
  genieAgent?: GenieAgentConversationDelete;
}

/** One R14 conversation row as a MOG id and a {@link ForgetConversation}, or undefined. */
function rowOf(r: unknown): { mogId: MogId; c: ForgetConversation } | undefined {
  if (typeof r !== 'object' || r === null) return undefined;
  const mogId = (r as Record<string, unknown>)['mogId'];
  if (typeof mogId !== 'string' || !UUID.test(mogId)) return undefined;
  const c = conversationOf(r);
  return c === undefined ? undefined : { mogId, c };
}

export class WipeFlow {
  private readonly core: WipeCore;
  private readonly genieOne: GenieOneThreadDelete;
  private readonly genieAgent: GenieAgentConversationDelete;

  constructor(deps: WipeFlowDeps) {
    this.core = deps.core ?? createIpcWipeCore();
    this.genieOne = deps.genieOne;
    this.genieAgent = deps.genieAgent ?? createDbxGenieAgentDelete();
  }

  /** The confirm dialog for an option. It throws on an unknown option. */
  dialog(scope: WipeScope, alsoGenie: boolean): WipeDialogModel {
    if (!isWipeScope(scope)) throw new Error('wipe needs one of the three options');
    const c = CONFIRM[scope];
    return {
      scope,
      alsoGenie,
      phase: 'confirm',
      title: c.title,
      text: `${c.what} on this computer, for every MOG?${alsoGenie ? GENIE_ALSO : GENIE_STAYS} You cannot undo this.`,
    };
  }

  /** The user selected Wipe: wipe the local rows, then the Genie conversations if chosen. */
  async confirm(scope: WipeScope, alsoGenie: boolean): Promise<WipeDialogModel> {
    const base = this.dialog(scope, alsoGenie);
    const { local, conversations } = await this.core.wipe(scope);
    const result: NonNullable<WipeDialogModel['result']> = {
      local,
      deletedConversations: 0,
      notices: [],
    };
    if (alsoGenie) {
      const byMog = new Map<MogId, ForgetConversation[]>();
      for (const r of conversations) {
        const row = rowOf(r);
        if (row !== undefined) byMog.set(row.mogId, [...(byMog.get(row.mogId) ?? []), row.c]);
      }
      let notDeleted = 0;
      for (const [mogId, cs] of byMog) {
        const g = await deleteGenieConversations('wipe', mogId, cs, {
          genieOne: this.genieOne,
          genieAgent: this.genieAgent,
        });
        result.deletedConversations += g.deleted;
        notDeleted += g.notDeleted;
        if (g.notice !== undefined) result.notices.push(g.notice);
      }
      const other = notDeletedText(notDeleted);
      if (other !== undefined) result.otherText = other;
    }
    return { ...base, phase: 'done', result };
  }
}
