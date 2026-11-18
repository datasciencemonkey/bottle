/**
 * The Genie One API list and delete utility (G-45a) and the "Delete in Genie One" fallback
 * (G-45b), for Forget (#123) and Wipe (#124). Rows: G-45a, G-45b, 4.6 route IDs, 9.2
 * "Genie conversations created by Bottle", SP-11, AT-16, D76, ADM-14, U9. See ADR 0077.
 *
 * - The SP-11 gate is off. With the gate off, the utility sends no Genie One API call. Each
 *   thread gets the "Delete in Genie One" link and the notice discloses that Bottle cannot
 *   delete it (G-45b). TODO(#121): set `SP11_GATE` to 'on' only when `docs/spikes/SP-11.md`
 *   records a PASS (the MCP thread lists, deletes and is gone).
 * - With the gate on, the utility deletes only the threads that it gets, each on the profile
 *   that made it (a conversation stays in its workspace). Then it lists each profile once to
 *   confirm. A failed delete, or a thread that still lists, gets the fallback link.
 * - It runs only on a user action: Forget or Wipe (U9). It never deletes on a timer.
 * - It reads conversation IDs only, never a title (SEC rules). It builds no URL for R7: it
 *   names the fixed route IDs, and Rust places the params (4.6 rule 1).
 */
import type { MogId, ProfileName, Tags } from '../../shared/interfaces';
import { call } from '../../shared/ipc';
import {
  DELETE_IN_GENIE_ONE,
  type GenieOneDeleteNoticeModel,
} from '../../shared/genie-one-delete-view';

export { DELETE_IN_GENIE_ONE };

/** The SP-11 (#121) result as a gate. 'on' allows the Genie One API list and delete. */
export type Sp11Gate = 'off' | 'on';

/** SP-11 has not passed (#121 is needs:human). TODO(#121): 'on' after a recorded PASS. */
export const SP11_GATE: Sp11Gate = 'off';

/** The user actions that can delete Genie conversations (U9, 9.2). */
export type DeleteTrigger = 'forget' | 'wipe';
const TRIGGERS: readonly string[] = ['forget', 'wipe'] satisfies DeleteTrigger[];

/** One Genie One MCP thread that a MOG created (from the local exchange row). */
export interface GenieOneThread {
  mogId: MogId;
  /** The profile that made the thread. The delete goes to this workspace only. */
  profile: ProfileName;
  /** The profile host, for the fallback link. */
  host: string;
  /** The workspace ID (`?o=`), for the fallback link. */
  workspaceId?: string;
  conversationId: string;
}

export type NotDeletedReason = 'sp11_not_passed' | 'delete_failed' | 'still_listed';

export type GenieOneDeleteOutcome =
  | { conversationId: string; result: 'deleted' }
  | { conversationId: string; result: 'not_deleted'; reason: NotDeletedReason; link?: string };

/** The seam to the Genie One API. Production: {@link createDbxGenieOneApi}. */
export interface GenieOneConversationApi {
  /** One page of conversation IDs on the profile. */
  listPage(
    profile: ProfileName,
    mogId: MogId,
    pageToken?: string,
  ): Promise<{ ids: string[]; nextPageToken?: string }>;
  /** 'deleted', or 'gone' for a 404. Any other result throws. */
  delete(profile: ProfileName, mogId: MogId, conversationId: string): Promise<'deleted' | 'gone'>;
}

/** The SP-11 probe searches 5 pages of 100; the utility does the same. */
const PAGE_SIZE = 100;
const MAX_PAGES = 5;
const SLUG = /^[A-Za-z0-9_-]{1,128}$/;
const WORKSPACE_ID = /^[0-9]{1,20}$/;

const tags = (mogId: MogId): Tags => ({ app: 'bottle', mog: mogId, op: 'genie' });
const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/**
 * The Genie One API over `dbx_http` with the fixed route IDs (4.6). The reply shape
 * `{conversations: [{conversation_id}], next_page_token}` is the one SP-11 expects.
 * TODO(#121): confirm the shape against the recorded SP-11 report.
 */
export function createDbxGenieOneApi(): GenieOneConversationApi {
  return {
    async listPage(profile, mogId, pageToken) {
      const r = await call('dbx_http', {
        profile,
        routeId: 'genie_one.list_conversations',
        params:
          pageToken === undefined
            ? { page_size: PAGE_SIZE }
            : { page_size: PAGE_SIZE, page_token: pageToken },
        tags: tags(mogId),
      });
      if (r.status < 200 || r.status > 299 || !isObj(r.json)) {
        throw new Error(`genie_one_list_${r.status}`);
      }
      const list = Array.isArray(r.json['conversations']) ? r.json['conversations'] : [];
      const ids = list
        .map((c) => (isObj(c) ? c['conversation_id'] : undefined))
        .filter((id): id is string => typeof id === 'string');
      const next = r.json['next_page_token'];
      return typeof next === 'string' && next !== '' ? { ids, nextPageToken: next } : { ids };
    },
    async delete(profile, mogId, conversationId) {
      const r = await call('dbx_http', {
        profile,
        routeId: 'genie_one.delete_conversation',
        params: { conversation_id: conversationId },
        tags: tags(mogId),
      });
      if (r.status === 404) return 'gone';
      if (r.status >= 200 && r.status <= 299) return 'deleted';
      throw new Error(`genie_one_delete_${r.status}`);
    },
  };
}

/**
 * The "Delete in Genie One" link: `https://<host>/one/chat/threads/<cid>?o=<wsid>` (the G-19a
 * thread link). Only an https (or bare) host, a numeric workspace ID and a slug ID make a link.
 */
export function deleteInGenieOneLink(t: GenieOneThread): string | undefined {
  if (!SLUG.test(t.conversationId)) return undefined;
  if (t.workspaceId === undefined || !WORKSPACE_ID.test(t.workspaceId)) return undefined;
  const raw = t.host.trim();
  const hasScheme = /^[a-z][a-z0-9+.-]*:/i.test(raw);
  if (hasScheme && !/^https:\/\//i.test(raw)) return undefined;
  let hostname: string;
  try {
    const u = new URL(hasScheme ? raw : `https://${raw}`);
    if (u.protocol !== 'https:' || u.username !== '' || u.password !== '' || u.port !== '') {
      return undefined;
    }
    hostname = u.hostname;
  } catch {
    return undefined;
  }
  if (hostname === '') return undefined;
  return `https://${hostname}/one/chat/threads/${t.conversationId}?o=${t.workspaceId}`;
}

const notDeleted = (t: GenieOneThread, reason: NotDeletedReason): GenieOneDeleteOutcome => {
  const link = deleteInGenieOneLink(t);
  return link === undefined
    ? { conversationId: t.conversationId, result: 'not_deleted', reason }
    : { conversationId: t.conversationId, result: 'not_deleted', reason, link };
};

export interface GenieOneDeleteDeps {
  /** Default {@link SP11_GATE}. */
  gate?: Sp11Gate;
  /** Default {@link createDbxGenieOneApi}. */
  api?: GenieOneConversationApi;
}

export class GenieOneDeleteUtility {
  private readonly gate: Sp11Gate;
  private readonly api: GenieOneConversationApi;

  constructor(deps: GenieOneDeleteDeps = {}) {
    this.gate = deps.gate ?? SP11_GATE;
    this.api = deps.api ?? createDbxGenieOneApi();
  }

  /** True only after the SP-11 pass (G-45a). */
  get canDelete(): boolean {
    return this.gate === 'on';
  }

  /**
   * Delete the Genie One threads that the MOG created, on a Forget or a Wipe (U9). The result
   * has one outcome per thread, in input order. Give the outcomes to {@link deleteNotice}.
   */
  async deleteThreads(
    trigger: DeleteTrigger,
    threads: readonly GenieOneThread[],
  ): Promise<GenieOneDeleteOutcome[]> {
    if (!TRIGGERS.includes(trigger as string)) {
      throw new Error('Genie One delete needs a user action (Forget or Wipe).');
    }
    if (this.gate !== 'on') return threads.map((t) => notDeleted(t, 'sp11_not_passed'));

    const out = new Map<GenieOneThread, GenieOneDeleteOutcome>();
    const done = new Map<ProfileName, { mogId: MogId; threads: GenieOneThread[] }>();
    for (const t of threads) {
      if (!SLUG.test(t.conversationId)) {
        out.set(t, notDeleted(t, 'delete_failed'));
        continue;
      }
      try {
        await this.api.delete(t.profile, t.mogId, t.conversationId);
        const group = done.get(t.profile) ?? { mogId: t.mogId, threads: [] };
        group.threads.push(t);
        done.set(t.profile, group);
      } catch {
        out.set(t, notDeleted(t, 'delete_failed'));
      }
    }
    for (const [profile, group] of done) {
      const listed = await this.listedIds(profile, group.mogId);
      for (const t of group.threads) {
        out.set(
          t,
          listed?.has(t.conversationId)
            ? notDeleted(t, 'still_listed')
            : { conversationId: t.conversationId, result: 'deleted' },
        );
      }
    }
    return threads.map((t) => out.get(t) ?? notDeleted(t, 'delete_failed'));
  }

  /**
   * The conversation IDs that still list on the profile, or undefined if the list fails. A
   * failed list does not undo a delete that the API confirmed.
   */
  private async listedIds(profile: ProfileName, mogId: MogId): Promise<Set<string> | undefined> {
    const ids = new Set<string>();
    let token: string | undefined;
    try {
      for (let page = 0; page < MAX_PAGES; page++) {
        const r = await this.api.listPage(profile, mogId, token);
        for (const id of r.ids) ids.add(id);
        token = r.nextPageToken;
        if (token === undefined) break;
      }
    } catch {
      return undefined;
    }
    return ids;
  }
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

/**
 * The notice for the Forget or Wipe dialog (G-45b, AT-16), or undefined when every thread is
 * deleted. It discloses that Bottle cannot (gate off) or could not (failure) delete, and offers
 * one "Delete in Genie One" link per thread.
 */
export function deleteNotice(
  mogId: MogId,
  outcomes: readonly GenieOneDeleteOutcome[],
): GenieOneDeleteNoticeModel | undefined {
  const left = outcomes.filter(
    (o): o is Extract<GenieOneDeleteOutcome, { result: 'not_deleted' }> =>
      o.result === 'not_deleted',
  );
  if (left.length === 0) return undefined;
  const links = left.flatMap((o) =>
    o.link === undefined ? [] : [{ conversationId: o.conversationId, url: o.link }],
  );
  const noLink = left.length - links.length;
  const text = left.every((o) => o.reason === 'sp11_not_passed')
    ? `Bottle cannot delete Genie One conversations yet. Use ${DELETE_IN_GENIE_ONE} to delete each one there.`
    : `Bottle could not delete ${plural(left.length, 'Genie One conversation')}. Use ${DELETE_IN_GENIE_ONE} to delete each one there.`;
  const notice: GenieOneDeleteNoticeModel = {
    mogId,
    label: DELETE_IN_GENIE_ONE,
    text,
    links,
    noLink,
  };
  if (noLink > 0) {
    notice.noLinkText = `Bottle has no link for ${plural(noLink, 'conversation')}. Find ${
      noLink === 1 ? 'it' : 'them'
    } in your Genie One history.`;
  }
  return notice;
}
