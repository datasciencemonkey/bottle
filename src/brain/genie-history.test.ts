/**
 * #40 (AT-09, MEM-25a, ADR 0061, ADR 0097): each UC2 Ask Genie request that ends saves one
 * `genie_exchange` row in the local R10 store through T10 `recordExchange`. The MOG row goes
 * first (`mem.mog.upsert`, the foreign key of ADR 0061). A failed write never rejects, and the
 * log gets only a fixed code (SEC-16c). All values are plainly fake.
 */
import type { MogId, ProfileName } from '../shared/interfaces';
import type { JsonObject } from '../shared/ipc';
import { RecordingLocalStore } from '../test-support/memory-double';
import type { LocalStmtId } from './components/memory-store';
import { LocalMemoryService } from './components/memory-service';
import { createGenieHistory, isExchangeMemory, type GenieEnd } from './genie-history';

const MOG = '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a40' as MogId;
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
    mogName: async () => 'Fake Finance',
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

  it.each([
    [{ t: 'failed', code: 'OFFLINE', userText: 'fake failure text', retry: 'none' }, 'failed'],
    [{ t: 'uncertain' }, 'uncertain'],
    [{ t: 'abandoned' }, 'abandoned'],
    [{ t: 'cancel_requested' }, 'cancelled'],
    [{ t: 'needs_input', kind: 'clarify', prompt: 'Which fake region?', options: [] }, 'completed'],
  ] as const)(
    '[AT-09] [MEM-25a] the terminal event %j saves the status %s',
    async (end, status) => {
      const { store, record } = rig();
      await record(answered({ end }));
      expect(exchange(store).status).toBe(status);
      expect(exchange(store).questionUser).toBe(Q);
    },
  );

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

  it('[AT-09] an unknown host or MOG name still saves the row with empty values', async () => {
    const store = new RecordingLocalStore(() => ({ rows: [{ changes: 1 }] }));
    const record = createGenieHistory({
      memory: new LocalMemoryService(store),
      mogName: () => Promise.reject(new Error('fake config_read failure')),
      hostOf: () => Promise.reject(new Error('fake profiles_list failure')),
      log: { event: () => true },
    });
    await record(answered());
    expect(store.calls[0]?.params.displayName).toBe('');
    expect(exchange(store).workspaceHost).toBe('');
  });

  it('[MEM-39a] isExchangeMemory accepts T10 and refuses an object without registerMog', () => {
    expect(isExchangeMemory(new LocalMemoryService(new RecordingLocalStore()))).toBe(true);
    expect(isExchangeMemory({ recordExchange: async () => undefined })).toBe(false);
  });
});
