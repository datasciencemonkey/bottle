import { createHash } from 'node:crypto';
import type { OperationState } from '../../shared/interfaces';
import type { JsonObject } from '../../shared/ipc';
import { RecordingLocalStore, genieExchangeDouble } from '../../test-support/memory-double';
import { distilSummary } from './exchange-summary';
import { ANSWER_CUT_MARK, LocalMemoryService } from './memory-service';

/** The simulated clock: 2026-01-01T00:00:00Z in epoch ms, and one day. */
const T0 = 1_767_225_600_000;
const DAY = 86_400_000;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

function setup(start = T0) {
  const clock = { t: start };
  const store = new RecordingLocalStore(() => ({ rows: [{ changes: 1 }] }));
  let n = 0;
  const svc = new LocalMemoryService(store, {
    now: () => clock.t,
    newId: () => `31111111-2222-4333-8444-${String(++n).padStart(12, '0')}`,
    deviceId: () => 'test-device',
  });
  return { clock, store, svc };
}

function upserts(store: RecordingLocalStore): JsonObject[] {
  return store.calls.filter((c) => c.stmtId === 'mem.exchange.upsert').map((c) => c.params);
}

/** Element `i`, or a test failure. */
function at(list: JsonObject[], i: number): JsonObject {
  const v = list[i];
  if (v === undefined) throw new Error(`no upsert ${i}`);
  return v;
}

describe('T10 recordExchange (#38, MEM-25a, MEM-26)', () => {
  it('[MEM-25a] [AT-16] stores the summary, key numbers and pointer of a home exchange', async () => {
    const { store, svc } = setup();
    const x = genieExchangeDouble();
    await svc.recordExchange(x);

    const p = at(upserts(store), 0);
    expect(store.calls).toHaveLength(1);
    expect(p.mogId).toBe(x.mogId);
    expect(p.requestId).toBe(x.requestId);
    expect(p.exchangeId).toMatch(UUID);
    // Summary (600 chars or less), key numbers verbatim (UX-14a) and the pointer.
    expect(typeof p.answerSummary).toBe('string');
    expect((p.answerSummary as string).length).toBeGreaterThan(0);
    expect((p.answerSummary as string).length).toBeLessThanOrEqual(600);
    expect(p.answerSummary).toContain('4.2 %');
    expect(p.keyNumbers).toEqual(x.keyNumbers);
    expect(p.deepLink).toBe(x.deepLink);
    expect(p.conversationId).toBe('test-conv');
    expect(p.responseId).toBe('test-response');
    expect(p.backendKind).toBe('genie_one_mcp');
    expect(p.profileName).toBe('test-profile');
    expect(p.workspaceHost).toBe('example.cloud.databricks.com');
    expect(p.homeWorkspace).toBe(true);
    // The full answer stays on the device (MEM-26, U8).
    expect(p.finalAnswerMd).toBe(x.answerMd);
    expect(p.questionUser).toBe(x.questionUser);
    expect(p.status).toBe('completed');
    expect(p.askedAt).toBe(T0);
    expect(p.completedAt).toBe(T0);
    expect(p.deviceId).toBe('test-device');
    expect(typeof p.rowHlc).toBe('number');
  });

  it('[MEM-25a] stores only a SHA-256 of the sent question, not its text', async () => {
    const { store, svc } = setup();
    const x = genieExchangeDouble();
    await svc.recordExchange(x);
    const p = at(upserts(store), 0);
    const want = createHash('sha256').update(x.questionSent, 'utf8').digest('hex');
    expect(p.questionSentSha256).toBe(want);
    expect(JSON.stringify(p)).not.toContain('test preamble');
  });

  it('[MEM-25a] the summary is plain text of 600 chars or less, cut at a word', () => {
    const long = `# Title\n\n${'**Revenue** rose by [4.2 %](https://example.com/x) in `EU`. '.repeat(40)}`;
    const s = distilSummary(long) ?? '';
    expect(s.length).toBeLessThanOrEqual(600);
    expect(s).not.toMatch(/\*\*|^#|\]\(|`/);
    expect(s.startsWith('Title Revenue rose by 4.2 % in EU.')).toBe(true);
    expect(s.endsWith('…')).toBe(true);
    expect(distilSummary('   ')).toBeNull();
    expect(distilSummary(undefined)).toBeNull();
    expect(distilSummary('Short answer.')).toBe('Short answer.');
  });

  it('[MEM-25a] a non-home exchange stores the pointer and the full answer only (D60)', async () => {
    const { store, svc } = setup();
    const x = genieExchangeDouble({ homeWorkspace: false });
    await svc.recordExchange(x);
    const p = at(upserts(store), 0);
    expect(p.homeWorkspace).toBe(false);
    expect(p.deepLink).toBe(x.deepLink);
    expect(p.finalAnswerMd).toBe(x.answerMd);
    expect(p.answerSummary ?? null).toBeNull();
    expect(p.keyNumbers ?? null).toBeNull();
    expect(p.questionUser ?? null).toBeNull();
  });

  it('[MEM-25a] maps each OperationState to the table status and sets completedAt when terminal', async () => {
    const table: [OperationState, string, boolean][] = [
      ['NOT_SENT', 'pending', false],
      ['SENDING', 'pending', false],
      ['ACKNOWLEDGED', 'pending', false],
      ['UNCERTAIN', 'uncertain', false],
      ['COMPLETED', 'completed', true],
      ['FAILED', 'failed', true],
      ['CANCEL_REQUESTED', 'cancelled', false],
      ['ABANDONED', 'abandoned', true],
    ];
    for (const [state, status, terminal] of table) {
      const { store, svc } = setup();
      await svc.recordExchange(genieExchangeDouble({ status: state }));
      const p = at(upserts(store), 0);
      expect(p.status, state).toBe(status);
      expect(p.completedAt ?? null, state).toBe(terminal ? T0 : null);
    }
  });

  it('[MEM-25a] passes the session, workspace and space when the caller has them (#449)', async () => {
    const { store, svc } = setup();
    await svc.recordExchange({
      ...genieExchangeDouble(),
      sessionId: '21111111-2222-4333-8444-555555555501',
      workspaceId: '1234',
      spaceId: 'test-space',
    });
    const p = at(upserts(store), 0);
    expect(p.sessionId).toBe('21111111-2222-4333-8444-555555555501');
    expect(p.workspaceId).toBe(1234);
    expect(p.spaceId).toBe('test-space');
  });

  it('[MEM-26] [AT-16] at a simulated clock 400 d later, T10 has deleted or expired nothing', async () => {
    const { clock, store, svc } = setup();
    await svc.recordExchange(genieExchangeDouble());
    clock.t = T0 + 400 * DAY;
    await svc.recordExchange(
      genieExchangeDouble({ requestId: '41111111-2222-4333-8444-555555555502' }),
    );
    // Only upserts: no Forget, no Wipe, no TTL and no purge job (U9).
    expect(store.calls.map((c) => c.stmtId)).toEqual([
      'mem.exchange.upsert',
      'mem.exchange.upsert',
    ]);
    for (const c of store.calls) {
      expect(c.params).not.toHaveProperty('expiresAt');
      expect(c.params.finalAnswerMd).toBe(genieExchangeDouble().answerMd);
    }
    const later = at(upserts(store), 1);
    expect(later.askedAt).toBe(T0 + 400 * DAY);
    // The row HLC never goes back.
    const first = at(upserts(store), 0);
    expect(later.rowHlc as number).toBeGreaterThan(first.rowHlc as number);
  });

  it('[MEM-26] an answer or key numbers over the catalog limits still save the row and pointer', async () => {
    const { store, svc } = setup();
    const big = 'x'.repeat(250_000);
    const many = Array.from({ length: 32 }, (_, i) => ({ text: `${i}`, label: 'l'.repeat(900) }));
    await svc.recordExchange(genieExchangeDouble({ answerMd: big, keyNumbers: many }));
    const p = at(upserts(store), 0);
    const md = p.finalAnswerMd as string;
    expect(Array.from(md).length).toBeLessThanOrEqual(200_000);
    expect(md.endsWith(ANSWER_CUT_MARK)).toBe(true);
    expect(md.startsWith('x'.repeat(1000))).toBe(true);
    const kn = p.keyNumbers as unknown[];
    expect(kn.length).toBeGreaterThan(0);
    expect(kn.length).toBeLessThan(32);
    expect(new TextEncoder().encode(JSON.stringify(kn)).length).toBeLessThanOrEqual(16_384);
    expect(kn).toEqual(many.slice(0, kn.length));
    expect(p.deepLink).toBe(genieExchangeDouble().deepLink);
    // An answer under the limit is stored as it is.
    const small = setup();
    await small.svc.recordExchange(genieExchangeDouble());
    expect(at(upserts(small.store), 0).finalAnswerMd).toBe(genieExchangeDouble().answerMd);
  });

  it('[MEM-25a] cuts questionUser at 1000 code points without splitting a pair', async () => {
    const { store, svc } = setup();
    await svc.recordExchange(genieExchangeDouble({ questionUser: '😀'.repeat(1200) }));
    const q = at(upserts(store), 0).questionUser as string;
    expect(Array.from(q)).toHaveLength(1000);
    expect(q).toBe('😀'.repeat(1000));
  });

  it('[MEM-01] every exchange statement carries the mogId of the exchange', async () => {
    const { store, svc } = setup();
    await svc.recordExchange(genieExchangeDouble());
    expect(store.calls.length).toBeGreaterThan(0);
    for (const c of store.calls) expect(c.params.mogId).toBe(genieExchangeDouble().mogId);
  });

  it('[MEM-01] refuses an exchange with no valid mogId before any store call', async () => {
    const { store, svc } = setup();
    await expect(svc.recordExchange(genieExchangeDouble({ mogId: '' }))).rejects.toThrow();
    expect(store.calls).toEqual([]);
  });
});
