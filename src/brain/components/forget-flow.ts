/**
 * The Forget flow and its dialog model (#123; MEM-32a, MEM-32c, MEM-32d, G-45a, G-45b, 9.2, D61,
 * D76, U9, AT-16). See ADR 0081.
 *
 * Order (the #35 review note on #123):
 * 1. Read the Genie conversations of the scope from the local exchange rows. The delete removes
 *    those rows, so this read comes first.
 * 2. Forget the local rows in one R10 transaction (T10 `forget`, MEM-32a). This works offline.
 * 3. Delete the Genie conversations where an API permits it (MEM-32c):
 *    - `genie_one_mcp`: the #122 utility, `deleteThreads('forget', ...)`. Its SP-11 gate is off
 *      (TODO(#121)), so it sends no call and gives each thread the "Delete in Genie One" link
 *      (G-45a, G-45b).
 *    - `genie_agent`: GA `DELETE` on the fixed route `genie.delete_conversation`, in the space
 *      and on the profile that made it (9.2).
 *    - Anything else, or a failed delete, is disclosed in the dialog.
 * 4. Build the done dialog: counts, the notice, and the Genie One memory review link (MEM-32d).
 *
 * It runs only when the user confirms the dialog (U9). It has no timer. It reads conversation ids
 * only, never a title or an answer.
 */
import { deleteNotice, type GenieOneDeleteOutcome, type GenieOneThread } from './genie-one-delete';
import type { ForgetConversation, ForgetScope } from './memory-service';
import { checkForgetScope } from './memory-service';
import type { MogId, ProfileName, Tags } from '../../shared/interfaces';
import { call } from '../../shared/ipc';
import {
  GENIE_ONE_MEMORY_REVIEW,
  type ForgetDialogModel,
  type ForgetScopeKind,
} from '../../shared/forget-view';

/** The part of T10 that Forget uses. Production: `LocalMemoryService`. */
export interface ForgetMemory {
  forgetConversations(scope: ForgetScope): Promise<ForgetConversation[]>;
  forget(
    scope: ForgetScope,
  ): Promise<{ local: number; remotePending: number; pitrWindowS: number }>;
}

/** The #122 utility seam. Production: `new GenieOneDeleteUtility()`. */
export interface GenieOneThreadDelete {
  deleteThreads(
    trigger: 'forget' | 'wipe',
    threads: readonly GenieOneThread[],
  ): Promise<GenieOneDeleteOutcome[]>;
}

/** GA `DELETE …/spaces/{space_id}/conversations/{conversation_id}` (9.2, G-49). */
export interface GenieAgentConversationDelete {
  /** 'deleted', or 'gone' for a 404. Any other result throws. */
  delete(
    profile: ProfileName,
    mogId: MogId,
    spaceId: string,
    conversationId: string,
  ): Promise<'deleted' | 'gone'>;
}

const tags = (mogId: MogId): Tags => ({ app: 'bottle', mog: mogId, op: 'genie' });
const SLUG = /^[A-Za-z0-9_-]{1,128}$/;
const WORKSPACE_ID = /^[0-9]{1,20}$/;

/** The GA delete over `dbx_http` with the fixed route ID (4.6). Rust places the params. */
export function createDbxGenieAgentDelete(): GenieAgentConversationDelete {
  return {
    async delete(profile, mogId, spaceId, conversationId) {
      const r = await call('dbx_http', {
        profile,
        routeId: 'genie.delete_conversation',
        params: { space_id: spaceId, conversation_id: conversationId },
        tags: tags(mogId),
      });
      if (r.status === 404) return 'gone';
      if (r.status >= 200 && r.status <= 299) return 'deleted';
      throw new Error(`genie_delete_${r.status}`);
    },
  };
}

/** The home workspace of a MOG, for the memory review link. */
export interface HomeWorkspace {
  host: string;
  workspaceId?: string;
}

/**
 * The Genie One memory review link of MEM-32d: the Genie One home of the MOG's home workspace,
 * `https://<host>/one?o=<wsid>`. Genie One shows and deletes its memories there (D61). Only an
 * https (or bare) host with no user, password or port, and a numeric workspace id make a link.
 * TODO(#512): use the exact memory page path when a person confirms it.
 */
export function genieOneMemoryReviewLink(home: HomeWorkspace | undefined): string | undefined {
  if (home === undefined) return undefined;
  if (home.workspaceId === undefined || !WORKSPACE_ID.test(home.workspaceId)) return undefined;
  const raw = home.host.trim();
  const hasScheme = /^[a-z][a-z0-9+.-]*:/i.test(raw);
  if (hasScheme && !/^https:\/\//i.test(raw)) return undefined;
  try {
    const u = new URL(hasScheme ? raw : `https://${raw}`);
    if (u.protocol !== 'https:' || u.username !== '' || u.password !== '' || u.port !== '') {
      return undefined;
    }
    if (u.hostname === '' || (u.pathname !== '/' && u.pathname !== '')) return undefined;
    return `https://${u.hostname}/one?o=${home.workspaceId}`;
  } catch {
    return undefined;
  }
}

const MEMORY_REVIEW_TEXT =
  'Genie One keeps its own memory on the Databricks server. Bottle cannot delete it. Review your Genie One memories there.';
const MEMORY_REVIEW_NO_LINK =
  ' Bottle has no link for your home workspace. Open Genie One to review them.';

const CONFIRM: Record<ForgetScopeKind, { title: string; text: (n: number) => string }> = {
  session: {
    title: 'Forget this session',
    text: () =>
      'Delete the transcript, notes and Genie history of this session on this computer? You cannot undo this.',
  },
  mog: {
    title: 'Forget this MOG',
    text: () =>
      'Delete all sessions, transcripts, notes and Genie history of this MOG on this computer? You cannot undo this.',
  },
  items: {
    title: 'Forget notes',
    text: (n) =>
      `Delete ${n === 1 ? 'this note' : `these ${n} notes`} on this computer? You cannot undo this.`,
  },
};

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

export interface ForgetFlowDeps {
  memory: ForgetMemory;
  genieOne: GenieOneThreadDelete;
  /** Default {@link createDbxGenieAgentDelete}. */
  genieAgent?: GenieAgentConversationDelete;
  /** The home workspace of the MOG, for the memory review link (MEM-32d). */
  home?: (mogId: MogId) => HomeWorkspace | undefined;
}

export class ForgetFlow {
  private readonly memory: ForgetMemory;
  private readonly genieOne: GenieOneThreadDelete;
  private readonly genieAgent: GenieAgentConversationDelete;
  private readonly home: (mogId: MogId) => HomeWorkspace | undefined;

  constructor(deps: ForgetFlowDeps) {
    this.memory = deps.memory;
    this.genieOne = deps.genieOne;
    this.genieAgent = deps.genieAgent ?? createDbxGenieAgentDelete();
    this.home = deps.home ?? (() => undefined);
  }

  /** The confirm dialog for a scope. It throws on a bad scope (MEM-01). */
  dialog(scope: ForgetScope): ForgetDialogModel {
    const s = checkForgetScope(scope);
    const n = s.kind === 'items' ? s.itemIds.length : 0;
    const url = genieOneMemoryReviewLink(this.home(s.mogId));
    const memoryReview: ForgetDialogModel['memoryReview'] =
      url === undefined
        ? { label: GENIE_ONE_MEMORY_REVIEW, text: MEMORY_REVIEW_TEXT + MEMORY_REVIEW_NO_LINK }
        : { label: GENIE_ONE_MEMORY_REVIEW, text: MEMORY_REVIEW_TEXT, url };
    return {
      mogId: s.mogId,
      scope: s.kind,
      phase: 'confirm',
      title: CONFIRM[s.kind].title,
      text: CONFIRM[s.kind].text(n),
      memoryReview,
    };
  }

  /**
   * The user selected Forget: read the conversations, forget the local rows, then delete the
   * Genie conversations where an API permits it. A Genie failure never undoes the local delete.
   */
  async confirm(scope: ForgetScope): Promise<ForgetDialogModel> {
    const base = this.dialog(scope);
    const conversations = await this.memory.forgetConversations(scope);
    const { local } = await this.memory.forget(scope);

    const threads: GenieOneThread[] = [];
    const others: ForgetConversation[] = [];
    for (const c of conversations) {
      if (c.backend === 'genie_one_mcp') {
        const t: GenieOneThread = {
          mogId: base.mogId,
          profile: c.profile,
          host: c.host,
          conversationId: c.conversationId,
        };
        if (c.workspaceId !== undefined) t.workspaceId = c.workspaceId;
        threads.push(t);
      } else {
        others.push(c);
      }
    }

    let deleted = 0;
    let notice: ReturnType<typeof deleteNotice>;
    if (threads.length > 0) {
      const outcomes = await this.genieOne.deleteThreads('forget', threads);
      deleted += outcomes.filter((o) => o.result === 'deleted').length;
      notice = deleteNotice(base.mogId, outcomes);
    }

    let notDeleted = 0;
    for (const c of others) {
      if (await this.deleteOther(base.mogId, c)) deleted += 1;
      else notDeleted += 1;
    }

    const result: NonNullable<ForgetDialogModel['result']> = {
      local,
      deletedConversations: deleted,
    };
    if (notice !== undefined) result.notice = notice;
    if (notDeleted > 0) {
      result.otherText = `Bottle could not delete ${plural(notDeleted, 'Genie conversation')}. Delete ${
        notDeleted === 1 ? 'it' : 'them'
      } in Genie.`;
    }
    return { ...base, phase: 'done', result };
  }

  /** GA DELETE for a `genie_agent` conversation; false when no API permits it or it fails. */
  private async deleteOther(mogId: MogId, c: ForgetConversation): Promise<boolean> {
    if (c.backend !== 'genie_agent' || c.spaceId === undefined) return false;
    if (!SLUG.test(c.spaceId) || !SLUG.test(c.conversationId)) return false;
    try {
      await this.genieAgent.delete(c.profile, mogId, c.spaceId, c.conversationId);
      return true;
    } catch {
      return false;
    }
  }
}
