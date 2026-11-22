/**
 * #124: the Wipe flow and its dialog model (MEM-33a, SEC-15a, G-45a, G-45b, G-49, U9, AT-16;
 * ADR 0087, ADR 0077). R14 in Rust is a double here (`WipeCore`). Every value here is fake.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { core } from '../test-core';
import type { GenieAgentConversationDelete } from './forget-flow';
import { GenieOneDeleteUtility, type GenieOneThread } from './genie-one-delete';
import { createStoreGate } from '../store-gate';
import {
  createIpcWipeCore,
  WipeFailed,
  WipeFlow,
  type WipeCore,
  type WipeCoreResult,
} from './wipe-flow';
import type { WipeScope } from '../../shared/wipe-view';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);

const MOG_A = '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01';
const MOG_B = '6a7f0c2e-91d4-4b8e-8f1e-2c4b5d6e7f02';
const HOST = 'acme.cloud.databricks.com';
const CID_1 = '01f0aaaa-test-not-real-0001';
const CID_2 = '01f0bbbb-test-not-real-0002';
const CID_3 = '01f0cccc-test-not-real-0003';

/** The rows that R14 answers: ids only, the same shape as the Forget conversation rows. */
const row = (mogId: string, cid: string, backendKind: string, spaceId?: string) => ({
  mogId,
  backendKind,
  profile: 'test-profile',
  host: HOST,
  workspaceId: 1001,
  ...(spaceId === undefined ? {} : { spaceId }),
  conversationId: cid,
});

function wipeCore(result: WipeCoreResult, log: string[] = []): WipeCore {
  return {
    async wipe(scope: WipeScope) {
      log.push(`wipe:${scope}`);
      return result;
    },
  };
}

const RESULT: WipeCoreResult = {
  local: 12,
  conversations: [
    row(MOG_A, CID_1, 'genie_one_mcp'),
    row(MOG_A, CID_2, 'genie_one_mcp'),
    row(MOG_B, CID_3, 'genie_agent', 'space-test-1'),
  ],
};

beforeEach(() => core.reset());

describe('WipeFlow (#124)', () => {
  it('[MEM-33a] the three options each have a confirm dialog that ends with You cannot undo this.', () => {
    const flow = new WipeFlow({ core: wipeCore(RESULT), genieOne: new GenieOneDeleteUtility() });
    const titles = (['older_than_30d', 'older_than_90d', 'everything'] as const).map((s) => {
      const m = flow.dialog(s, false);
      expect(m.phase).toBe('confirm');
      expect(m.scope).toBe(s);
      expect(m.alsoGenie).toBe(false);
      expect(m.text).toMatch(/You cannot undo this\.$/);
      expect(m.text).toContain('on this computer');
      return m.title;
    });
    expect(titles).toEqual([
      'Wipe data older than 30 days',
      'Wipe data older than 90 days',
      'Wipe everything',
    ]);
    expect(flow.dialog('older_than_30d', false).text).toContain('older than 30 days');
    expect(flow.dialog('everything', false).text).toContain('Genie conversations stay in Genie');
    expect(flow.dialog('everything', true).text).toContain('also deletes the Genie conversations');
    expect(() => flow.dialog('older_than_7d' as WipeScope, false)).toThrow();
  });

  it('[MEM-33a] [U9] confirm runs the R14 Wipe once with the selected scope; the dialog alone deletes nothing', async () => {
    const log: string[] = [];
    const flow = new WipeFlow({
      core: wipeCore(RESULT, log),
      genieOne: new GenieOneDeleteUtility(),
    });
    flow.dialog('older_than_90d', true);
    expect(log).toEqual([]);
    const done = await flow.confirm('older_than_90d', false);
    expect(log).toEqual(['wipe:older_than_90d']);
    expect(done.phase).toBe('done');
    expect(done.result?.local).toBe(12);
    await expect(flow.confirm('older_than_7d' as WipeScope, false)).rejects.toThrow();
    expect(log).toEqual(['wipe:older_than_90d']);
  });

  it('[MEM-33a] [9.2] without "Also delete Genie conversations" no Genie call goes and none is deleted', async () => {
    const threads: GenieOneThread[] = [];
    const agentCalls: string[] = [];
    const flow = new WipeFlow({
      core: wipeCore(RESULT),
      genieOne: {
        async deleteThreads(_t, ts) {
          threads.push(...ts);
          return [];
        },
      },
      genieAgent: {
        async delete(_p, _m, _s, cid) {
          agentCalls.push(cid);
          return 'deleted';
        },
      },
    });
    const done = await flow.confirm('everything', false);
    expect(threads).toEqual([]);
    expect(agentCalls).toEqual([]);
    expect(done.result?.deletedConversations).toBe(0);
    expect(done.result?.notices).toEqual([]);
    expect(done.result?.otherText).toBeUndefined();
  });

  it('[G-45a] [G-45b] [AT-16] with SP-11 not passed, Wipe shows "Delete in Genie One" for each MOG and calls no API', async () => {
    core.invoke.mockRejectedValue(new Error('no call expected'));
    const flow = new WipeFlow({
      core: wipeCore({ local: 3, conversations: RESULT.conversations.slice(0, 2) }),
      genieOne: new GenieOneDeleteUtility(),
    });
    const done = await flow.confirm('older_than_30d', true);
    expect(core.invoke).not.toHaveBeenCalled();
    const notices = done.result?.notices ?? [];
    expect(notices).toHaveLength(1);
    expect(notices[0]?.mogId).toBe(MOG_A);
    expect(notices[0]?.label).toBe('Delete in Genie One');
    expect(notices[0]?.links.map((l) => l.url)).toEqual([
      `https://${HOST}/one/chat/threads/${CID_1}?o=1001`,
      `https://${HOST}/one/chat/threads/${CID_2}?o=1001`,
    ]);
    expect(done.result?.deletedConversations).toBe(0);
  });

  it('[G-49] [MEM-33a] "Also delete Genie conversations" deletes each one in the MOG and space that made it', async () => {
    const triggers: string[] = [];
    const agentCalls: unknown[] = [];
    const genieAgent: GenieAgentConversationDelete = {
      async delete(profile, mogId, spaceId, conversationId) {
        agentCalls.push({ profile, mogId, spaceId, conversationId });
        return 'deleted';
      },
    };
    const flow = new WipeFlow({
      core: wipeCore(RESULT),
      genieOne: {
        async deleteThreads(trigger, ts) {
          triggers.push(`${trigger}:${ts.map((t) => `${t.mogId}/${t.conversationId}`).join(',')}`);
          return ts.map((t) => ({ conversationId: t.conversationId, result: 'deleted' as const }));
        },
      },
      genieAgent,
    });
    const done = await flow.confirm('everything', true);
    expect(triggers).toEqual([`wipe:${MOG_A}/${CID_1},${MOG_A}/${CID_2}`]);
    expect(agentCalls).toEqual([
      { profile: 'test-profile', mogId: MOG_B, spaceId: 'space-test-1', conversationId: CID_3 },
    ]);
    expect(done.result?.deletedConversations).toBe(3);
    expect(done.result?.notices).toEqual([]);
  });

  it('[AT-16] offline: the local Wipe holds and a failed Genie delete is disclosed', async () => {
    const flow = new WipeFlow({
      core: wipeCore({
        local: 5,
        conversations: [row(MOG_B, CID_3, 'genie_agent', 'space-test-1')],
      }),
      genieOne: new GenieOneDeleteUtility(),
      genieAgent: {
        async delete() {
          throw new Error('offline');
        },
      },
    });
    const done = await flow.confirm('older_than_30d', true);
    expect(done.result?.local).toBe(5);
    expect(done.result?.otherText).toBe(
      'Bottle could not delete 1 Genie conversation. Delete it in Genie.',
    );
  });

  it('[MEM-33a] a bad row from R14 is dropped, never sent to Genie', async () => {
    const threads: GenieOneThread[] = [];
    const flow = new WipeFlow({
      core: wipeCore({
        local: 1,
        conversations: [
          { ...row(MOG_A, CID_1, 'genie_one_mcp'), mogId: 'not-a-uuid' },
          { ...row(MOG_A, CID_2, 'genie_one_mcp'), backendKind: 'other' },
          { ...row(MOG_A, CID_3, 'genie_one_mcp'), conversationId: '' },
        ],
      }),
      genieOne: {
        async deleteThreads(_t, ts) {
          threads.push(...ts);
          return [];
        },
      },
    });
    await flow.confirm('everything', true);
    expect(threads).toEqual([]);
  });

  it('[MEM-33a] [#525] the default core sends mem_wipe { scope } through the store gate, for each scope', async () => {
    core.invoke.mockImplementation(async () => ({ local: 3, conversations: [] }));
    for (const scope of ['older_than_30d', 'older_than_90d', 'everything'] as const) {
      await expect(createIpcWipeCore(createStoreGate()).wipe(scope)).resolves.toEqual({
        local: 3,
        conversations: [],
      });
      expect(core.invoke).toHaveBeenLastCalledWith('mem_wipe', { args: { scope } });
    }
  });

  it('[MEM-33a] [ADR 0105] a locked store fails at once with a fixed reason and no second mem_wipe call', async () => {
    const gate = createStoreGate();
    gate.arm({ explain: false, onChange: () => undefined });
    core.invoke.mockRejectedValue({ error: 'host_failed' });
    const wipe = createIpcWipeCore(gate);
    const first = await wipe.wipe('older_than_90d').catch((e: unknown) => e);
    expect(first).toBeInstanceOf(WipeFailed);
    expect((first as WipeFailed).reason).toContain('The store is locked');
    const calls = core.invoke.mock.calls.length;
    const again = await wipe.wipe('older_than_90d').catch((e: unknown) => e);
    expect((again as WipeFailed).reason).toContain('The store is locked');
    expect(core.invoke.mock.calls.length).toBe(calls);
  });

  it('[MEM-33a] an error of an open store has a short fixed reason, and an unknown one has none', async () => {
    const gate = createStoreGate();
    core.invoke.mockRejectedValueOnce({ error: 'host_failed' });
    const a = await createIpcWipeCore(gate)
      .wipe('everything')
      .catch((e: unknown) => e);
    expect((a as WipeFailed).reason).toBe('The local store could not be used.');
    core.invoke.mockRejectedValueOnce(new Error('secret-ish detail'));
    const b = await createIpcWipeCore(gate)
      .wipe('everything')
      .catch((e: unknown) => e);
    expect((b as WipeFailed).reason).toBeUndefined();
  });

  it('[MEM-33a] an empty wipe succeeds with "Nothing to wipe"', async () => {
    const flow = new WipeFlow({
      core: wipeCore({ local: 0, conversations: [] }),
      genieOne: new GenieOneDeleteUtility(),
    });
    const done = await flow.confirm('older_than_90d', true);
    expect(done.phase).toBe('done');
    expect(done.result?.emptyText).toBe('Nothing to wipe. No data is older than 90 days.');
    expect((await flow.confirm('everything', false)).result?.emptyText).toBe('Nothing to wipe.');
  });

  it('[U9] the flow has no timer, and Genie One deletes run with the wipe trigger', () => {
    const src = readFileSync(resolve(import.meta.dirname, 'wipe-flow.ts'), 'utf8');
    expect(src).not.toMatch(/setInterval|setTimeout|requestIdleCallback/);
    expect(src).toContain("'wipe'");
    expect(src).toContain('TODO(#121)');
  });
});
