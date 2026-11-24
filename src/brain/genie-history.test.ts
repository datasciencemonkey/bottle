/**
 * #40 (AT-09, MEM-25a, ADR 0061, ADR 0097): each UC2 Ask Genie request that ends saves one
 * `genie_exchange` row in the local R10 store through T10 `recordExchange`. The MOG row goes
 * first (`mem.mog.upsert`, the foreign key of ADR 0061). A failed write never rejects, and the
 * log gets only a fixed code (SEC-16c). All values are plainly fake.
 */
import type { MogId, ProfileName } from '../shared/interfaces';
import type { JsonObject } from '../shared/ipc';
import { RecordingLocalStore } from '../test-support/memory-double';
import type { TerminalAskEvent } from './components/ask-events';
import type { LocalStmtId } from './components/memory-store';
import { LocalMemoryService } from './components/memory-service';
import {
  MOG_NAME_FALLBACK,
  RECENT_OPS_MAX,
  createGenieHistory,
  isExchangeMemory,
  type GenieEnd,
} from './genie-history';

const MOG = '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a40' as MogId;
const DATA_MOG = '1a2b3c4d-6c1e-4d0a-9a51-3a3d2f6f0a41' as MogId;
const OP = '41111111-2222-4333-8444-555555555540';
const PROFILE = 'fake-profile' as ProfileName;
const HOST = 'example.cloud.databricks.test';
const Q = 'What was fake revenue in Q3?';
const ANSWER = '**Fake revenue** rose **4.2 %** to **$1.3M** in Q3.';
const KEY_NUMBERS = [{ text: '4.2 %', label: 'growth' }, { text: '$1.3M' }];
const LINK = 'https://example.cloud.databricks.test/one/chat/threads/fake-conv?o=1234';

type Answer = (stmtId: LocalStmtId, params: JsonObject) => { rows: unknown[] };

function rig(answer: Answer = () => ({ rows: [{ changes: 1 }] })) {
  const store = new RecordingLocalStore(answer);
  let n = 0;
  const memory = new LocalMemoryService(store, {
    now: () => 1_767_225_600_000,
    newId: () => `31111111-2222-4333-8444-${String(++n).padStart(12, '0')}`,
    deviceId: () => 'test-device',
  });
  const lines: string[] = [];
  const log = {
    event: (name: string, fields?: Record<string, number>) => {
      lines.push(JSON.stringify({ name, fields }));
      return true;
    },
  };
  const record = createGenieHistory({
    memory,
    mogName: async (id) => (id === DATA_MOG ? 'Fake Lab' : 'Fake Finance'),
    dataMogId: async (p) => (p === 'lab-profile' ? DATA_MOG : undefined),
    hostOf: async (p) => (p === PROFILE ? HOST : undefined),
    log,
  });
  return { store, record, lines };
}

const answered = (over: Partial<GenieEnd> = {}): GenieEnd => ({
  opId: OP,
  mogId: MOG,
  profile: PROFILE,
  question: Q,
  conversationId: 'fake-conv',
  responseId: 'fake-response',
  end: {
    t: 'answer',
    md: ANSWER,
    incomplete: false,
    keyNumbers: KEY_NUMBERS,
    deepLink: LINK,
    queryItems: [],
  },
  ...over,
});

const ids = (store: RecordingLocalStore) => store.calls.map((c) => c.stmtId);
const exchange = (store: RecordingLocalStore): JsonObject => {
  const c = store.calls.find((x) => x.stmtId === 'mem.exchange.upsert');
  if (!c) throw new Error('no mem.exchange.upsert');
  return c.params;
};

describe('genie history: one local exchange row per Ask Genie request (#40)', () => {
  it('[AT-09] [MEM-25a] an answer writes the MOG row, then one exchange with the exact text', async () => {
    const { store, record } = rig();
    await record(answered());

    expect(ids(store)).toEqual(['mem.mog.upsert', 'mem.exchange.upsert']);
    expect(store.calls[0]?.params).toMatchObject({
      mogId: MOG,
      displayName: 'Fake Finance',
      backendKind: 'genie_one_mcp',
      deviceId: 'test-device',
    });
    const p = exchange(store);
    expect(p).toMatchObject({
      requestId: OP,
      mogId: MOG,
      backendKind: 'genie_one_mcp',
      profileName: PROFILE,
      workspaceHost: HOST,
      homeWorkspace: true,
      conversationId: 'fake-conv',
      responseId: 'fake-response',
      questionUser: Q,
      status: 'completed',
      finalAnswerMd: ANSWER,
      keyNumbers: KEY_NUMBERS,
      deepLink: LINK,
    });
    // The card shows `keyNumbers.map(k => k.text)`: the stored rows hold the same texts.
    expect((p.keyNumbers as { text: string }[]).map((k) => k.text)).toEqual(['4.2 %', '$1.3M']);
    expect(typeof p.questionSentSha256).toBe('string');
  });

  it('[AC-1.11] [ADR 0117] an answer is stored under the data MOG id of the profile that answered, not the lamp', async () => {
    const { store, record } = rig();
    await record(answered({ profile: 'lab-profile' as ProfileName }));
    expect(ids(store)).toEqual(['mem.mog.upsert', 'mem.exchange.upsert']);
    expect(store.calls[0]?.params).toMatchObject({ mogId: DATA_MOG, displayName: 'Fake Lab' });
    expect(exchange(store)).toMatchObject({ mogId: DATA_MOG, profileName: 'lab-profile' });
  });

  it('[AC-1.11] a profile with no data MOG falls back to the lamp id', async () => {
    const { store, record } = rig();
    await record(answered({ profile: PROFILE }));
    expect(exchange(store)).toMatchObject({ mogId: MOG });
  });

  it('[AT-09] [G-21b] one request gives one exchange write, even when it ends twice', async () => {
    const { store, record } = rig();
    await record(answered());
    await record(answered({ end: { t: 'uncertain' } }));
    expect(ids(store).filter((i) => i === 'mem.exchange.upsert')).toHaveLength(1);
  });

  it('[AT-09] [ADR 0061] the MOG row goes before each exchange, so a later request also saves', async () => {
    const { store, record } = rig();
    await record(answered());
    await record(answered({ opId: '41111111-2222-4333-8444-555555555541' }));
    expect(ids(store)).toEqual([
      'mem.mog.upsert',
      'mem.exchange.upsert',
      'mem.mog.upsert',
      'mem.exchange.upsert',
    ]);
  });

  it.each<[TerminalAskEvent, string]>([
    [{ t: 'failed', code: 'OFFLINE', userText: 'fake failure text', retry: 'none' }, 'failed'],
    [{ t: 'uncertain' }, 'uncertain'],
    [{ t: 'abandoned' }, 'abandoned'],
    [{ t: 'cancel_requested' }, 'cancelled'],
    [{ t: 'needs_input', kind: 'clarify', prompt: 'Which fake region?', options: [] }, 'completed'],
  ])('[AT-09] [MEM-25a] the terminal event %j saves the status %s', async (end, status) => {
    const { store, record } = rig();
    await record(answered({ end }));
    expect(exchange(store).status).toBe(status);
    expect(exchange(store).questionUser).toBe(Q);
  });

  it('[MEM-25a] a clarify keeps the Genie prompt as the answer text', async () => {
    const { store, record } = rig();
    await record(
      answered({
        end: { t: 'needs_input', kind: 'clarify', prompt: 'Which fake region?', options: [] },
      }),
    );
    expect(exchange(store).finalAnswerMd).toBe('Which fake region?');
  });

  it('[AT-09] [SEC-16c] a store failure (host_failed) resolves and logs only a fixed code', async () => {
    const { store, record, lines } = rig(() => {
      throw { error: 'host_failed' };
    });
    await expect(record(answered())).resolves.toBeUndefined();
    // The MOG row failed, so no exchange write is tried.
    expect(ids(store)).toEqual(['mem.mog.upsert']);
    expect(lines).toEqual([JSON.stringify({ name: 'memory.write', fields: { ok: 0 } })]);
    const all = lines.join('\n');
    expect(all).not.toContain(Q);
    expect(all).not.toContain('Fake revenue');
    expect(all).not.toContain('host_failed');
  });

  it('[AT-09] a failed exchange write logs the code once and the next request still saves', async () => {
    let fail = true;
    const { store, record, lines } = rig((id) => {
      if (id === 'mem.exchange.upsert' && fail) throw { error: 'host_failed' };
      return { rows: [{ changes: 1 }] };
    });
    await record(answered());
    fail = false;
    await record(answered({ opId: '41111111-2222-4333-8444-555555555542' }));
    expect(lines).toHaveLength(1);
    expect(ids(store).filter((i) => i === 'mem.exchange.upsert')).toHaveLength(2);
  });

  it.each([
    ['a failed config_read', () => Promise.reject(new Error('fake config_read failure'))],
    ['an empty name', async () => ''],
  ])(
    '[AT-09] [ADR 0061] %s saves the MOG row with the fallback name and the row with an empty host',
    async (_case, mogName) => {
      const store = new RecordingLocalStore(() => ({ rows: [{ changes: 1 }] }));
      const record = createGenieHistory({
        memory: new LocalMemoryService(store),
        mogName,
        hostOf: () => Promise.reject(new Error('fake profiles_list failure')),
        log: { event: () => true },
      });
      await record(answered());
      // `mog.display_name` has CHECK (length BETWEEN 1 AND 80): the name is never empty.
      expect(store.calls[0]?.params.displayName).toBe(MOG_NAME_FALLBACK);
      expect(MOG_NAME_FALLBACK.length).toBeGreaterThan(0);
      expect(exchange(store).workspaceHost).toBe('');
    },
  );

  it('[AT-09] [G-21b] the once-per-request set is bounded and keeps the newest ids', async () => {
    const { store, record } = rig();
    const op = (i: number) => `41111111-2222-4333-8444-${String(1000 + i).padStart(12, '0')}`;
    for (let i = 0; i <= RECENT_OPS_MAX; i++) await record(answered({ opId: op(i) }));
    const writes = () => ids(store).filter((i) => i === 'mem.exchange.upsert').length;
    expect(writes()).toBe(RECENT_OPS_MAX + 1);
    // The newest id is still held: its second end writes nothing.
    await record(answered({ opId: op(RECENT_OPS_MAX) }));
    expect(writes()).toBe(RECENT_OPS_MAX + 1);
    // The oldest id fell out of the bounded set.
    await record(answered({ opId: op(0) }));
    expect(writes()).toBe(RECENT_OPS_MAX + 2);
  });

  it('[AC-2a.18] an agent call keeps the turn as split group, and a call to another workspace is not a home row', async () => {
    const { store, record } = rig();
    const turnUuid = 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa';
    await record(answered({ opId: OP, splitGroupId: turnUuid, homeWorkspace: true }));
    await record(
      answered({
        opId: '41111111-2222-4333-8444-555555555541',
        splitGroupId: turnUuid,
        homeWorkspace: false,
      }),
    );
    const rows = store.calls.filter((c) => c.stmtId === 'mem.exchange.upsert').map((c) => c.params);
    expect(rows).toHaveLength(2);
    expect(rows.map((r) => r.splitGroupId)).toEqual([turnUuid, turnUuid]);
    expect(rows.map((r) => r.homeWorkspace)).toEqual([true, false]);
    // The direct path stays a home row with no split group.
    const direct = rig();
    await direct.record(answered());
    expect(exchange(direct.store).homeWorkspace).toBe(true);
    expect(exchange(direct.store).splitGroupId).toBeUndefined();
  });

  it('[MEM-39a] isExchangeMemory accepts T10 and refuses an object without registerMog', () => {
    expect(isExchangeMemory(new LocalMemoryService(new RecordingLocalStore()))).toBe(true);
    expect(isExchangeMemory({ recordExchange: async () => undefined })).toBe(false);
  });

  it('[AC-C1] [AC-C2] [ADR 0121] an agent turn row: genie_agent, the user question, the combined answer, the thread as split group, no summary', async () => {
    const { store, record } = rig();
    await record({
      opId: OP,
      mogId: MOG,
      profile: PROFILE,
      question: Q,
      backend: 'genie_agent',
      threadId: '51111111-2222-4333-8444-555555555540',
      homeWorkspace: true,
      end: {
        t: 'answer',
        md: ANSWER,
        incomplete: false,
        keyNumbers: [],
        deepLink: '',
        queryItems: [],
      },
    });
    const p = exchange(store);
    expect(p).toMatchObject({
      requestId: OP,
      backendKind: 'genie_agent',
      homeWorkspace: true,
      questionUser: Q,
      status: 'completed',
      finalAnswerMd: ANSWER,
      splitGroupId: '51111111-2222-4333-8444-555555555540',
    });
    expect(p['answerSummary']).toBeUndefined();
    expect(p['conversationId']).toBeUndefined();
    expect(p['keyNumbers'] ?? []).toEqual([]);
  });

  it('[ADR 0121] a Genie call row with a split group keeps it, and keeps its summary', async () => {
    const { store, record } = rig();
    await record(answered({ splitGroupId: '51111111-2222-4333-8444-555555555541' }));
    expect(exchange(store)).toMatchObject({
      backendKind: 'genie_one_mcp',
      splitGroupId: '51111111-2222-4333-8444-555555555541',
    });
    expect(typeof exchange(store)['answerSummary']).toBe('string');
  });

  it('[Wipe] [L3] a write that a Wipe overtook while it looked up the host saves nothing', async () => {
    const { store, record } = rig();
    let stale = false;
    // The Wipe lands during the lookups; the writer checks the fence before the first store call.
    const end = answered({ isStale: () => stale });
    const done = record(end);
    stale = true;
    await done;
    expect(ids(store)).toEqual([]);
  });

  it('[Wipe] [L3] a Wipe between the MOG row and the exchange stops the exchange write', async () => {
    let stale = false;
    const { store, record } = rig(() => {
      stale = true;
      return { rows: [{ changes: 1 }] };
    });
    await record(answered({ isStale: () => stale }));
    expect(ids(store)).toEqual(['mem.mog.upsert']);
  });

  it('[Wipe] [L-1] a stale attempt does not use up the op id: the same row (op id) written again saves once', async () => {
    const { store, record } = rig();
    await record(answered({ isStale: () => true }));
    expect(ids(store)).toEqual([]);
    await record(answered());
    await record(answered());
    expect(ids(store).filter((id) => id === 'mem.exchange.upsert')).toHaveLength(1);
  });
});
