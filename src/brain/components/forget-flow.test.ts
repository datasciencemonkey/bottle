/**
 * #123: the Forget flow and its dialog model (MEM-32a, MEM-32c, MEM-32d, G-45a, G-45b, U9,
 * AT-16; ADR 0081, ADR 0077). Every value here is fake.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { core } from '../test-core';
import {
  createDbxGenieAgentDelete,
  ForgetFlow,
  genieOneMemoryReviewLink,
  type ForgetMemory,
  type GenieAgentConversationDelete,
} from './forget-flow';
import { GenieOneDeleteUtility, type GenieOneThread } from './genie-one-delete';
import type { ForgetConversation, ForgetScope } from './memory-service';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);

const MOG = '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01';
const SESSION = '21111111-2222-4333-8444-555555555501';
const HOST = 'acme.cloud.databricks.com';
const CID_1 = '01f0aaaa-test-not-real-0001';
const CID_2 = '01f0bbbb-test-not-real-0002';
const HOME = { host: HOST, workspaceId: '1001' };

const one = (cid: string): ForgetConversation => ({
  backend: 'genie_one_mcp',
  profile: 'test-profile',
  host: HOST,
  workspaceId: '1001',
  conversationId: cid,
});
const agent = (cid: string): ForgetConversation => ({
  backend: 'genie_agent',
  profile: 'test-profile',
  host: HOST,
  workspaceId: '1001',
  spaceId: 'space-test-1',
  conversationId: cid,
});

/** A memory double that logs the order of the reads and deletes. */
function memory(conversations: ForgetConversation[], log: string[] = []): ForgetMemory {
  return {
    async forgetConversations(s: ForgetScope) {
      log.push(`read:${s.sessionId ?? 'mog'}`);
      return conversations;
    },
    async forget(s: ForgetScope) {
      log.push(`delete:${s.sessionId ?? 'mog'}`);
      return { local: 7, remotePending: 0, pitrWindowS: 0 };
    },
  };
}

beforeEach(() => core.reset());

describe('ForgetFlow (#123)', () => {
  it('[MEM-32a] [MEM-32c] reads the conversations before the local delete, then deletes them', async () => {
    const log: string[] = [];
    const genieOne = {
      async deleteThreads(trigger: 'forget' | 'wipe', threads: readonly GenieOneThread[]) {
        log.push(`genie_one:${trigger}:${threads.map((t) => t.conversationId).join(',')}`);
        return threads.map((t) => ({
          conversationId: t.conversationId,
          result: 'deleted' as const,
        }));
      },
    };
    const flow = new ForgetFlow({ memory: memory([one(CID_1)], log), genieOne, home: () => HOME });
    const done = await flow.confirm({ mogId: MOG, sessionId: SESSION });
    expect(log).toEqual([`read:${SESSION}`, `delete:${SESSION}`, `genie_one:forget:${CID_1}`]);
    expect(done.phase).toBe('done');
    expect(done.result?.local).toBe(7);
    expect(done.result?.notice).toBeUndefined();
  });

  it('[MEM-32c] [G-45a] [G-45b] [AT-16] with SP-11 not passed, the dialog offers Delete in Genie One', async () => {
    core.invoke.mockRejectedValue(new Error('no call expected'));
    const flow = new ForgetFlow({
      memory: memory([one(CID_1), one(CID_2)]),
      genieOne: new GenieOneDeleteUtility(),
      home: () => HOME,
    });
    const done = await flow.confirm({ mogId: MOG });
    // G-45a: no Genie One API call before the SP-11 pass.
    expect(core.invoke).not.toHaveBeenCalled();
    const notice = done.result?.notice;
    expect(notice?.label).toBe('Delete in Genie One');
    expect(notice?.text).toContain('cannot delete');
    expect(notice?.links.map((l) => l.url)).toEqual([
      `https://${HOST}/one/chat/threads/${CID_1}?o=1001`,
      `https://${HOST}/one/chat/threads/${CID_2}?o=1001`,
    ]);
  });

  it('[MEM-32c] deletes genie_agent conversations with GA DELETE in their space', async () => {
    const calls: unknown[] = [];
    const genieAgent: GenieAgentConversationDelete = {
      async delete(profile, mogId, spaceId, conversationId) {
        calls.push({ profile, mogId, spaceId, conversationId });
        return 'deleted';
      },
    };
    const flow = new ForgetFlow({
      memory: memory([agent(CID_1)]),
      genieOne: new GenieOneDeleteUtility(),
      genieAgent,
      home: () => HOME,
    });
    const done = await flow.confirm({ mogId: MOG, sessionId: SESSION });
    expect(calls).toEqual([
      { profile: 'test-profile', mogId: MOG, spaceId: 'space-test-1', conversationId: CID_1 },
    ]);
    expect(done.result?.deletedConversations).toBe(1);
    expect(done.result?.notice).toBeUndefined();
    expect(done.result?.otherText).toBeUndefined();
  });

  it('[MEM-32c] [AT-16] offline: the local delete holds and a failed GA delete is disclosed', async () => {
    const genieAgent: GenieAgentConversationDelete = {
      async delete() {
        throw new Error('offline');
      },
    };
    const flow = new ForgetFlow({
      // The second one has no space id, so no API can delete it.
      memory: memory([agent(CID_1), (({ spaceId: _s, ...c }) => c)(agent(CID_2))]),
      genieOne: new GenieOneDeleteUtility(),
      genieAgent,
      home: () => HOME,
    });
    const done = await flow.confirm({ mogId: MOG });
    expect(done.result?.local).toBe(7);
    expect(done.result?.deletedConversations).toBe(0);
    expect(done.result?.otherText).toBe(
      'Bottle could not delete 2 Genie conversations. Delete them in Genie.',
    );
  });

  it('[MEM-32d] the confirm and the done dialog both link to the Genie One memory review', async () => {
    const flow = new ForgetFlow({
      memory: memory([]),
      genieOne: new GenieOneDeleteUtility(),
      home: () => HOME,
    });
    const confirm = flow.dialog({ mogId: MOG, sessionId: SESSION });
    expect(confirm.phase).toBe('confirm');
    expect(confirm.text).toContain('You cannot undo this.');
    expect(confirm.memoryReview.label).toBe('Review Genie One memories');
    expect(confirm.memoryReview.url).toBe(`https://${HOST}/one?o=1001`);
    expect(confirm.memoryReview.text).toContain('Genie One keeps its own memory');
    const done = await flow.confirm({ mogId: MOG, sessionId: SESSION });
    expect(done.memoryReview).toEqual(confirm.memoryReview);
  });

  it('[MEM-32d] the memory review link needs an https host and a numeric workspace id', () => {
    expect(genieOneMemoryReviewLink({ host: `https://${HOST}/`, workspaceId: '1001' })).toBe(
      `https://${HOST}/one?o=1001`,
    );
    expect(
      genieOneMemoryReviewLink({ host: `http://${HOST}`, workspaceId: '1001' }),
    ).toBeUndefined();
    expect(genieOneMemoryReviewLink({ host: HOST, workspaceId: 'abc' })).toBeUndefined();
    expect(genieOneMemoryReviewLink({ host: 'evil.test@x', workspaceId: '1' })).toBeUndefined();
    expect(genieOneMemoryReviewLink(undefined)).toBeUndefined();
  });

  it('[MEM-32a] a refused scope deletes nothing and calls no Genie API', async () => {
    const log: string[] = [];
    const m = memory([one(CID_1)], log);
    m.forgetConversations = async () => {
      throw new Error('forget needs a UUID mogId');
    };
    const flow = new ForgetFlow({ memory: m, genieOne: new GenieOneDeleteUtility() });
    await expect(flow.confirm({ mogId: 'bad' })).rejects.toThrow();
    expect(log).toEqual([]);
  });

  it('[MEM-32c] the default GA delete uses the fixed genie.delete_conversation route', async () => {
    core.invoke.mockResolvedValueOnce({ status: 200, json: {} });
    expect(await createDbxGenieAgentDelete().delete('dev', MOG, 'space-test-1', CID_1)).toBe(
      'deleted',
    );
    expect(core.invoke).toHaveBeenCalledWith('dbx_http', {
      args: {
        profile: 'dev',
        routeId: 'genie.delete_conversation',
        params: { space_id: 'space-test-1', conversation_id: CID_1 },
        tags: { app: 'bottle', mog: MOG, op: 'genie' },
      },
    });
    core.invoke.mockResolvedValueOnce({ status: 404, json: {} });
    expect(await createDbxGenieAgentDelete().delete('dev', MOG, 'space-test-1', CID_1)).toBe(
      'gone',
    );
    core.invoke.mockResolvedValueOnce({ status: 500, json: {} });
    await expect(
      createDbxGenieAgentDelete().delete('dev', MOG, 'space-test-1', CID_1),
    ).rejects.toThrow();
  });

  it('[U9] the flow has no timer: it runs only when the user confirms', () => {
    const src = readFileSync(resolve(import.meta.dirname, 'forget-flow.ts'), 'utf8');
    expect(src).not.toMatch(/setInterval|setTimeout/);
    expect(src).toContain("deleteThreads('forget'");
  });
});
