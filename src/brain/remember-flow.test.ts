/**
 * "Remember this" on a card and by voice (ADR 0113). A fake store records each call and ticket.
 */
import { LocalMemoryService } from './components/memory-service';
import type { LocalMemoryStore, LocalStmtId } from './components/memory-store';
import { createRememberFlow } from './remember-flow';
import type { MemorySaved } from '../shared/interfaces';
import type { JsonObject } from '../shared/ipc';

const MOG = '00000000-0000-4000-8000-00000000000a';
const REQ = '41111111-2222-4333-8444-000000000001';
const EX = '51111111-2222-4333-8444-000000000001';
const NEW = '22222222-2222-4333-8444-555555555555';
const T = 1_767_225_600_000;

interface Call {
  stmtId: LocalStmtId;
  params: JsonObject;
  ticket: string | undefined;
}

function setup(opts: { fail?: unknown; status?: string; noExchange?: boolean } = {}) {
  const calls: Call[] = [];
  let now = T;
  const store: LocalMemoryStore = {
    exec: async (stmtId, params, ticket) => {
      calls.push({ stmtId, params, ticket });
      switch (stmtId) {
        case 'mem.exchange.get':
          return {
            rows: opts.noExchange
              ? []
              : [
                  {
                    exchangeId: EX,
                    requestId: REQ,
                    workspaceHost: 'w.cloud.databricks.com',
                    workspaceId: 123,
                    spaceId: 'space-1',
                    answerSummary: 'S'.repeat(700),
                    finalAnswerMd: 'A'.repeat(5000),
                    questionUser: 'Q'.repeat(100),
                  },
                ],
          };
        case 'mem.exchange.recent':
          return { rows: [{ requestId: REQ, profileName: 'prof' }] };
        case 'mem.item.get':
          return {
            rows: [
              { type: 'fact', title: 't', bodyMd: '', status: opts.status ?? 'active', pinned: 1 },
            ],
          };
        case 'mem.item.write':
          if (opts.fail !== undefined) throw opts.fail;
          return { rows: [{ changes: 1 }, { changes: 1 }] };
        default:
          return { rows: [{ changes: 1 }, { changes: 0 }, { changes: 1 }] };
      }
    },
  };
  const memory = new LocalMemoryService(store, {
    now: () => now,
    newId: () => NEW,
    deviceId: () => 'd',
  });
  const sent: { mogId: string; saved: MemorySaved }[] = [];
  const flow = createRememberFlow({
    memory,
    store,
    send: async (mogId, saved) => void sent.push({ mogId, saved }),
    now: () => now,
  });
  return {
    flow,
    calls,
    sent,
    advance: (ms: number) => void (now += ms),
    writes: () => calls.filter((c) => c.stmtId === 'mem.item.write'),
  };
}

describe('card "remember this"', () => {
  it('saves one pinned genie_answer with the exchange link and tells that card', async () => {
    const s = setup();
    await s.flow.card(MOG, REQ, 'tkt');
    const w = s.writes();
    expect(w).toHaveLength(1);
    const p = w[0]?.params as Record<string, unknown>;
    expect(w[0]?.ticket).toBe('tkt');
    expect(p).toMatchObject({
      mogId: MOG,
      itemType: 'genie_answer',
      source: 'you',
      confidence: 1,
      pinned: true,
      origin: 'pin',
      sourceExchangeId: EX,
      workspaceHost: 'w.cloud.databricks.com',
      workspaceId: 123,
      spaceId: 'space-1',
      profile: 'prof',
    });
    expect((p['title'] as string).length).toBe(80);
    expect((p['text'] as string).length).toBe(600);
    expect((p['body'] as string).length).toBe(4000);
    expect(s.sent).toEqual([
      { mogId: MOG, saved: { mogId: MOG, itemId: NEW, outcome: 'saved', undoMs: 8000 } },
    ]);
  });

  it('a quarantined result says so', async () => {
    const s = setup({ status: 'quarantined' });
    await s.flow.card(MOG, REQ, 'tkt');
    expect(s.sent[0]?.saved.outcome).toBe('quarantined');
  });

  it('a secret shows secret_refused with no undo', async () => {
    const s = setup({ fail: { error: 'secret_refused' } });
    await s.flow.card(MOG, REQ, 'tkt');
    expect(s.sent[0]?.saved).toEqual({ mogId: MOG, outcome: 'secret_refused', undoMs: 0 });
  });

  it('no ticket makes no call at all', async () => {
    const s = setup();
    await s.flow.card(MOG, REQ, undefined);
    expect(s.calls).toEqual([]);
    expect(s.sent[0]?.saved).toEqual({ mogId: MOG, outcome: 'failed', undoMs: 0 });
  });

  it('a missing exchange is failed and writes nothing', async () => {
    const s = setup({ noExchange: true });
    await s.flow.card(MOG, REQ, 'tkt');
    expect(s.writes()).toEqual([]);
    expect(s.sent[0]?.saved.outcome).toBe('failed');
  });
});

describe('voice "remember this"', () => {
  it('saves one pinned fact from remember.text with the voice ticket', async () => {
    const s = setup();
    await s.flow.voice(MOG, 'Renewal is in March', 'voice-tkt');
    const w = s.writes();
    expect(w).toHaveLength(1);
    expect(w[0]?.ticket).toBe('voice-tkt');
    expect(w[0]?.params).toEqual({
      mogId: MOG,
      itemId: NEW,
      itemType: 'fact',
      source: 'you',
      confidence: 1,
      pinned: true,
      origin: 'pin',
      text: 'Renewal is in March',
      seenAt: T,
      rowHlc: T,
      deviceId: 'd',
      now: T,
    });
    expect(s.sent[0]?.saved).toEqual({ mogId: MOG, itemId: NEW, outcome: 'saved', undoMs: 8000 });
  });
});

describe('Undo', () => {
  it('forgets the item saved in the last 8 s', async () => {
    const s = setup();
    await s.flow.voice(MOG, 'x', 't');
    s.advance(7_999);
    await s.flow.undo(MOG);
    const f = s.calls.find((c) => c.stmtId === 'mem.forget.items');
    expect(f?.params).toMatchObject({ mogId: MOG, itemIds: [NEW] });
  });

  it('does nothing after 8 s, for another MOG, or with no save', async () => {
    const s = setup();
    await s.flow.undo(MOG);
    await s.flow.voice(MOG, 'x', 't');
    await s.flow.undo('00000000-0000-4000-8000-00000000000b');
    s.advance(8_001);
    await s.flow.undo(MOG);
    expect(s.calls.some((c) => c.stmtId === 'mem.forget.items')).toBe(false);
  });

  it('a second Undo does not forget twice', async () => {
    const s = setup();
    await s.flow.voice(MOG, 'x', 't');
    await s.flow.undo(MOG);
    await s.flow.undo(MOG);
    expect(s.calls.filter((c) => c.stmtId === 'mem.forget.items')).toHaveLength(1);
  });
});
