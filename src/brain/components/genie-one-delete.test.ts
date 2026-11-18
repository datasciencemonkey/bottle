/**
 * #122: the Genie One API list and delete utility, gated on SP-11 (#121), and the
 * "Delete in Genie One" fallback with a disclosure (G-45a, G-45b, AT-16, 9.2, D76, U9).
 * Every value here is fake.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { core } from '../test-core';
import {
  createDbxGenieOneApi,
  DELETE_IN_GENIE_ONE,
  deleteInGenieOneLink,
  deleteNotice,
  GenieOneDeleteUtility,
  SP11_GATE,
  type GenieOneConversationApi,
  type GenieOneThread,
} from './genie-one-delete';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-000000000001';
const HOST = 'https://acme.cloud.databricks.com';
const HOST_B = 'https://beta.cloud.databricks.com';
const WSID = '1001';
const CID_1 = '01f0aaaa-test-not-real-0001';
const CID_2 = '01f0bbbb-test-not-real-0002';
const CID_3 = '01f0cccc-test-not-real-0003';

const thread = (cid: string, over: Partial<GenieOneThread> = {}): GenieOneThread => ({
  mogId: MOG,
  profile: 'dev',
  host: HOST,
  workspaceId: WSID,
  conversationId: cid,
  ...over,
});

/** A Genie One API double: a list of thread IDs per profile and a scripted delete. */
function apiDouble(
  listed: Record<string, string[]>,
  deleteReply: (cid: string) => 'deleted' | 'gone' | Error = () => 'deleted',
  opts: { keepAfterDelete?: string[]; listFails?: boolean } = {},
) {
  const calls: string[] = [];
  const store = Object.fromEntries(Object.entries(listed).map(([p, ids]) => [p, [...ids]]));
  const api: GenieOneConversationApi = {
    listPage: vi.fn(async (profile: string, _mog: string, pageToken?: string) => {
      calls.push(`list ${profile} ${pageToken ?? '-'}`);
      if (opts.listFails) throw new Error('list failed');
      const ids = store[profile] ?? [];
      // two pages, so the utility must follow next_page_token
      if (!pageToken)
        return { ids: ids.slice(0, 1), nextPageToken: ids.length > 1 ? 'p2' : undefined };
      return { ids: ids.slice(1) };
    }),
    delete: vi.fn(async (profile: string, _mog: string, cid: string) => {
      calls.push(`delete ${profile} ${cid}`);
      const r = deleteReply(cid);
      if (r instanceof Error) throw r;
      if (!opts.keepAfterDelete?.includes(cid)) {
        store[profile] = (store[profile] ?? []).filter((x) => x !== cid);
      }
      return r;
    }),
  };
  return { api, calls };
}

beforeEach(() => core.reset());

describe('SP-11 gate (G-45a, AT-16)', () => {
  it('[G-45a] [AT-16] the production gate is off until SP-11 passes (TODO(#121))', () => {
    expect(SP11_GATE).toBe('off');
    expect(new GenieOneDeleteUtility().canDelete).toBe(false);
  });

  it('[G-45a] [AT-16] with the gate off, Forget and Wipe make no call at all', async () => {
    const { api, calls } = apiDouble({ dev: [CID_1] });
    const u = new GenieOneDeleteUtility({ gate: 'off', api });
    await u.deleteThreads('forget', [thread(CID_1)]);
    await u.deleteThreads('wipe', [thread(CID_1), thread(CID_2, { profile: 'b', host: HOST_B })]);
    expect(calls).toEqual([]);
  });

  it('[G-45a] [AT-16] the default utility sends no dbx_http (no Genie One API call before the pass)', async () => {
    const u = new GenieOneDeleteUtility();
    await u.deleteThreads('forget', [thread(CID_1)]);
    await u.deleteThreads('wipe', [thread(CID_2)]);
    expect(core.invoke).not.toHaveBeenCalled();
  });

  it('[G-45a] [AT-16] no other brain or view source names the genie_one.* routes', () => {
    const root = resolve(import.meta.dirname, '../..');
    const hits: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) {
          walk(p);
        } else if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name)) {
          if (/genie_one\.(list|delete)_conversation/.test(readFileSync(p, 'utf8'))) {
            hits.push(relative(root, p).split('\\').join('/'));
          }
        }
      }
    };
    walk(root);
    expect(hits.sort()).toEqual(['brain/components/genie-one-delete.ts', 'shared/ipc.ts']);
  });
});

describe('Genie One API utility after the SP-11 pass (G-45a)', () => {
  it('[G-45a] [U9] runs only on a user action: Forget or Wipe', async () => {
    const { api, calls } = apiDouble({ dev: [CID_1] });
    const u = new GenieOneDeleteUtility({ gate: 'on', api });
    for (const bad of ['retention', 'auto', '', undefined]) {
      await expect(u.deleteThreads(bad as never, [thread(CID_1)])).rejects.toThrow(/user action/);
    }
    expect(calls).toEqual([]);
  });

  it('[G-45a] [AT-16] deletes only the given threads, each on its own profile, then lists to confirm', async () => {
    const { api, calls } = apiDouble({ dev: [CID_1, CID_3], b: [CID_2] });
    const u = new GenieOneDeleteUtility({ gate: 'on', api });
    const out = await u.deleteThreads('wipe', [
      thread(CID_1),
      thread(CID_2, { profile: 'b', host: HOST_B }),
    ]);
    expect(out).toEqual([
      { conversationId: CID_1, result: 'deleted' },
      { conversationId: CID_2, result: 'deleted' },
    ]);
    const deletes = calls.filter((c) => c.startsWith('delete'));
    expect(deletes).toEqual([`delete dev ${CID_1}`, `delete b ${CID_2}`]);
    expect(calls).not.toContain(`delete dev ${CID_3}`); // a listed thread that Bottle was not given
    expect(calls).toContain('list dev -');
    expect(calls).toContain('list dev p2');
    expect(calls).toContain('list b -');
  });

  it('[G-45a] a 404 (already gone) counts as deleted', async () => {
    const { api } = apiDouble({ dev: [] }, () => 'gone');
    const out = await new GenieOneDeleteUtility({ gate: 'on', api }).deleteThreads('forget', [
      thread(CID_1),
    ]);
    expect(out).toEqual([{ conversationId: CID_1, result: 'deleted' }]);
  });

  it('[G-45a] [G-45b] [AT-16] a failed delete falls back to Delete in Genie One', async () => {
    const { api } = apiDouble({ dev: [CID_1, CID_2] }, (cid) =>
      cid === CID_1 ? new Error('E_HTTP 403') : 'deleted',
    );
    const out = await new GenieOneDeleteUtility({ gate: 'on', api }).deleteThreads('forget', [
      thread(CID_1),
      thread(CID_2),
    ]);
    expect(out).toEqual([
      {
        conversationId: CID_1,
        result: 'not_deleted',
        reason: 'delete_failed',
        link: `${HOST}/one/chat/threads/${CID_1}?o=${WSID}`,
      },
      { conversationId: CID_2, result: 'deleted' },
    ]);
  });

  it('[G-45a] [G-45b] [AT-16] a thread that still lists after the delete is not reported as deleted', async () => {
    const { api } = apiDouble({ dev: [CID_1] }, () => 'deleted', { keepAfterDelete: [CID_1] });
    const out = await new GenieOneDeleteUtility({ gate: 'on', api }).deleteThreads('wipe', [
      thread(CID_1),
    ]);
    expect(out).toEqual([
      {
        conversationId: CID_1,
        result: 'not_deleted',
        reason: 'still_listed',
        link: `${HOST}/one/chat/threads/${CID_1}?o=${WSID}`,
      },
    ]);
  });

  it('[G-45a] a bad conversation ID is never sent', async () => {
    const { api, calls } = apiDouble({ dev: [] });
    const out = await new GenieOneDeleteUtility({ gate: 'on', api }).deleteThreads('forget', [
      thread('../x?y'),
    ]);
    expect(calls.filter((c) => c.startsWith('delete'))).toEqual([]);
    expect(out[0]).toMatchObject({ result: 'not_deleted', reason: 'delete_failed' });
  });
});

describe('dbx_http Genie One API client (G-45a, 4.6 route IDs)', () => {
  it('[G-45a] delete uses the fixed route ID on the thread profile, with genie tags', async () => {
    core.invoke.mockResolvedValueOnce({ status: 200, json: {} });
    expect(await createDbxGenieOneApi().delete('dev', MOG, CID_1)).toBe('deleted');
    expect(core.invoke).toHaveBeenCalledWith('dbx_http', {
      args: {
        profile: 'dev',
        routeId: 'genie_one.delete_conversation',
        params: { conversation_id: CID_1 },
        tags: { app: 'bottle', mog: MOG, op: 'genie' },
      },
    });
  });

  it('[G-45a] delete maps 404 to gone and other statuses to an error', async () => {
    core.invoke.mockResolvedValueOnce({ status: 404, json: {} });
    expect(await createDbxGenieOneApi().delete('dev', MOG, CID_1)).toBe('gone');
    core.invoke.mockResolvedValueOnce({ status: 403, json: {} });
    await expect(createDbxGenieOneApi().delete('dev', MOG, CID_1)).rejects.toThrow();
  });

  it('[G-45a] list reads conversation IDs and next_page_token only (no titles)', async () => {
    core.invoke.mockResolvedValueOnce({
      status: 200,
      json: {
        conversations: [
          { conversation_id: CID_1, title: 'fake title' },
          { conversation_id: CID_2 },
        ],
        next_page_token: 'tok-2',
      },
    });
    const page = await createDbxGenieOneApi().listPage('dev', MOG, 'tok-1');
    expect(page).toEqual({ ids: [CID_1, CID_2], nextPageToken: 'tok-2' });
    expect(core.invoke).toHaveBeenCalledWith('dbx_http', {
      args: {
        profile: 'dev',
        routeId: 'genie_one.list_conversations',
        params: { page_size: 100, page_token: 'tok-1' },
        tags: { app: 'bottle', mog: MOG, op: 'genie' },
      },
    });
  });
});

describe('Delete in Genie One fallback (G-45b, AT-16)', () => {
  it('[G-45b] [AT-16] with the gate off, each thread gets the Delete in Genie One link', async () => {
    const out = await new GenieOneDeleteUtility({ gate: 'off' }).deleteThreads('forget', [
      thread(CID_1),
    ]);
    expect(out).toEqual([
      {
        conversationId: CID_1,
        result: 'not_deleted',
        reason: 'sp11_not_passed',
        link: `${HOST}/one/chat/threads/${CID_1}?o=${WSID}`,
      },
    ]);
  });

  it('[G-45b] builds the link only from a safe https host, a numeric workspace ID and a slug ID', () => {
    expect(deleteInGenieOneLink(thread(CID_1, { host: 'acme.cloud.databricks.com' }))).toBe(
      `${HOST}/one/chat/threads/${CID_1}?o=${WSID}`,
    );
    expect(deleteInGenieOneLink(thread(CID_1, { host: `${HOST}/some/path?x=1` }))).toBe(
      `${HOST}/one/chat/threads/${CID_1}?o=${WSID}`,
    );
    expect(deleteInGenieOneLink(thread(CID_1, { workspaceId: undefined }))).toBeUndefined();
    expect(deleteInGenieOneLink(thread(CID_1, { workspaceId: '12&x=1' }))).toBeUndefined();
    expect(
      deleteInGenieOneLink(thread(CID_1, { host: 'http://acme.cloud.databricks.com' })),
    ).toBeUndefined();
    expect(deleteInGenieOneLink(thread(CID_1, { host: 'javascript:alert(1)' }))).toBeUndefined();
    expect(deleteInGenieOneLink(thread('a/b'))).toBeUndefined();
  });

  it('[G-45b] [AT-16] the notice discloses that Bottle cannot delete and offers Delete in Genie One', async () => {
    const threads = [thread(CID_1), thread(CID_2, { workspaceId: undefined })];
    const out = await new GenieOneDeleteUtility({ gate: 'off' }).deleteThreads('wipe', threads);
    const n = deleteNotice(MOG, out);
    expect(n).toBeDefined();
    expect(n!.mogId).toBe(MOG);
    expect(n!.label).toBe(DELETE_IN_GENIE_ONE);
    expect(n!.label).toBe('Delete in Genie One');
    expect(n!.text).toMatch(/Bottle cannot delete Genie One conversations/);
    expect(n!.links).toEqual([
      { conversationId: CID_1, url: `${HOST}/one/chat/threads/${CID_1}?o=${WSID}` },
    ]);
    expect(n!.noLink).toBe(1);
    expect(n!.noLinkText).toMatch(/1 conversation/);
  });

  it('[G-45b] [AT-16] after a partial failure the notice names the count that Bottle could not delete', async () => {
    const { api } = apiDouble({ dev: [CID_1, CID_2] }, (cid) =>
      cid === CID_1 ? new Error('E_HTTP 500') : 'deleted',
    );
    const out = await new GenieOneDeleteUtility({ gate: 'on', api }).deleteThreads('forget', [
      thread(CID_1),
      thread(CID_2),
    ]);
    const n = deleteNotice(MOG, out)!;
    expect(n.text).toMatch(/Bottle could not delete 1 Genie One conversation\b/);
    expect(n.links.map((l) => l.conversationId)).toEqual([CID_1]);
  });

  it('[G-45a] no notice when every thread is deleted', async () => {
    const { api } = apiDouble({ dev: [CID_1] });
    const out = await new GenieOneDeleteUtility({ gate: 'on', api }).deleteThreads('forget', [
      thread(CID_1),
    ]);
    expect(deleteNotice(MOG, out)).toBeUndefined();
  });
});
