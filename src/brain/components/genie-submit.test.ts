/**
 * `GenieBridge.submit` and `GenieOneMcpBackend.ask` (#33, ADR 0084) against the #229 double on
 * the sim scheduler: one `genie_ask` per request (G-15b), no brain countdown after R8 stops on
 * RATE_LIMITED (ADR 0075), no send of an unapproved preamble, and a content-free audit entry.
 */
import type { AskEvent, AskRequest } from '../../shared/interfaces';
import { scenarios } from '../../test-support/genie-one-mcp';
import { HOST, MOG, PROFILE, rig, type Rig } from '../../test-support/poll-actor-rig';
import type { AuditEvent } from '../audit-log';
import { GenieOneMcpBackend, createGenieBridge } from './genie-bridge';
import { RETRY_AFTER_TEXT_CAP_S, classifyGenieError, userTextFor } from './genie-errors';
import { GIVE_UP_MS } from './poll-actor';

const MIN = 60_000;
const OP = '00000000-0000-4000-8000-00000000a002';

function request(over: Partial<AskRequest> = {}): AskRequest {
  return {
    opId: OP,
    mogId: MOG as AskRequest['mogId'],
    binding: { kind: 'genie_one_mcp', profile: PROFILE },
    question: 'What was revenue last quarter?',
    tags: { app: 'bottle', mog: MOG, op: 'genie' },
    ...over,
  };
}

async function run(r: Rig, events: AsyncIterable<AskEvent>, untilMs = GIVE_UP_MS + MIN) {
  const out: AskEvent[] = [];
  const done = (async () => {
    for await (const e of events) out.push(e);
  })();
  await r.sched.runUntil(r.sched.clock.now() + untilMs, { settle: true });
  await done;
  return out;
}

const asks = (r: Rig) => r.sent.filter((s) => s.args.tool === 'genie_ask');

describe('GenieOneMcpBackend.ask (#33)', () => {
  it('[G-15b] [G-07a] one genie_ask with the question on the request profile, then polls on the same IDs', async () => {
    const r = rig({ responses: [scenarios.normalAnswer()] });
    const backend = new GenieOneMcpBackend(r.cm, {
      call: r.call,
      now: () => r.sched.clock.now(),
      hostOf: () => HOST,
    });
    const events = await run(r, backend.ask(request(), new AbortController().signal));
    expect(asks(r)).toHaveLength(1);
    const ask = asks(r)[0]?.args;
    expect(ask).toMatchObject({
      profile: PROFILE,
      server: 'genie_one',
      tool: 'genie_ask',
      args: { question: 'What was revenue last quarter?' },
      opId: OP,
      tags: { app: 'bottle', mog: MOG, op: 'genie' },
    });
    expect(ask?.meta).toBeUndefined();
    const acc = events[0] as Extract<AskEvent, { t: 'accepted' }>;
    for (const p of r.sent.filter((s) => s.args.tool === 'genie_poll_response'))
      expect(p.args.args).toEqual({
        conversation_id: acc.conversationId,
        response_id: acc.responseId,
      });
    expect(events.map((e) => e.t)).toEqual(['accepted', 'progress', 'answer']);
    expect(r.journal.list()).toEqual([]);
  });

  it('[G-15b] a failed ask is never sent again, for any failure', async () => {
    for (const code of ['offline', 'timeout', 'auth_expired', 'genie_one_off', 'non_json_reply']) {
      const r = rig({ rejectAsk: () => ({ error: code }) });
      const events = await run(r, createGenieBridge({ call: r.call, conversations: r.cm }).submit(request()).events);
      expect(events).toHaveLength(1);
      expect(events[0]?.t).toBe('failed');
      expect(asks(r), code).toHaveLength(1);
    }
  });

  it('[ARC-F3] after R8 stops on RATE_LIMITED the brain starts no countdown and never asks again', async () => {
    const r = rig({ rejectAsk: () => ({ error: 'rate_limited' }) });
    const bridge = createGenieBridge({ call: r.call, conversations: r.cm, hostOf: () => HOST });
    const events = await run(r, bridge.submit(request()).events, 30 * MIN);
    expect(events).toHaveLength(1);
    const failed = events[0] as Extract<AskEvent, { t: 'failed' }>;
    expect(failed).toMatchObject({ t: 'failed', code: 'RATE_LIMITED', retry: 'user' });
    // No "retrying in N s": R8 already ran the countdown and its 5 tries (ADR 0075).
    expect(failed.userText).not.toMatch(/retrying/i);
    expect(asks(r)).toHaveLength(1);
  });

  it('[ARC-F3] a 429 isError reply on the ask is also final, with no countdown text', async () => {
    const r = rig();
    const call: Rig['call'] = async (args) =>
      args.tool === 'genie_ask'
        ? { structuredContent: null, text: '', isError: true, httpStatus: 429, rpcError: { retry_after: 30 } }
        : r.call(args);
    const events = await run(r, createGenieBridge({ call, conversations: r.cm }).submit(request()).events);
    expect(events).toEqual([expect.objectContaining({ t: 'failed', code: 'RATE_LIMITED', retry: 'user' })]);
    expect((events[0] as Extract<AskEvent, { t: 'failed' }>).userText).not.toMatch(/retrying/i);
  });

  it('[G-14] the retry_after seconds in the countdown text are capped', () => {
    expect(RETRY_AFTER_TEXT_CAP_S).toBe(300);
    expect(userTextFor('RATE_LIMITED', {}, 12.2)).toBe('Genie busy, retrying in 13 s');
    expect(userTextFor('RATE_LIMITED', {}, 1e308)).toBe(
      `Genie busy, retrying in ${RETRY_AFTER_TEXT_CAP_S} s`,
    );
    expect(classifyGenieError({ httpStatus: 429, rpcError: { retry_after: 86_400 } }).userText).toBe(
      `Genie busy, retrying in ${RETRY_AFTER_TEXT_CAP_S} s`,
    );
  });

  it('[ARC-F3] a poll refused with rate_limited polls again with the same IDs and still ends in one answer', async () => {
    const r = rig({
      responses: [scenarios.unknownStatus(2)],
      reject: (n) => (n === 1 ? { error: 'rate_limited' } : undefined),
    });
    const events = await run(r, createGenieBridge({ call: r.call, conversations: r.cm }).submit(request()).events);
    expect(events.at(-1)).toMatchObject({ t: 'answer', md: 'Done.' });
    expect(asks(r)).toHaveLength(1);
  });
});

describe('GenieBridge.submit (#33)', () => {
  it('[B1] [AT-08] a request with a context preamble is not sent before the ContextGate approval', async () => {
    const r = rig();
    const events = await run(
      r,
      createGenieBridge({ call: r.call, conversations: r.cm }).submit(
        request({
          preamble: {
            header: 'Context (from my notes, may be wrong):',
            items: [{ id: 'm1', text: 'EMEA is the focus', source: 'you', ts: 0, truncated: false }],
            chars: 17,
          },
        }),
      ).events,
    );
    expect(events).toEqual([expect.objectContaining({ t: 'failed', code: 'TOOL_BLOCKED', retry: 'none' })]);
    expect(r.sent).toEqual([]);
  });

  it('[G-21b] submit returns the request opId and sends nothing until its events are read', async () => {
    const r = rig();
    const s = createGenieBridge({ call: r.call, conversations: r.cm }).submit(request());
    expect(s.opId).toBe(OP);
    await r.sched.runUntil(5_000, { settle: true });
    expect(r.sent).toEqual([]);
  });

  it('[G-01] a v1 backend binding gives one failed event and sends nothing', async () => {
    const r = rig();
    const events = await run(
      r,
      createGenieBridge({ call: r.call, conversations: r.cm }).submit(
        request({ binding: { kind: 'genie_agent', profile: PROFILE, spaceId: 's', mode: 'chat' } }),
      ).events,
    );
    expect(events).toEqual([expect.objectContaining({ t: 'failed', code: 'CLIENT_BUG' })]);
    expect(r.sent).toEqual([]);
  });

  it('[SEC-23] [AT-08] an accepted ask writes one content-free ask audit entry with the sent hash', async () => {
    const r = rig({ responses: [scenarios.normalAnswer()] });
    const recorded: AuditEvent[] = [];
    const bridge = createGenieBridge({
      call: r.call,
      conversations: r.cm,
      audit: { record: async (e) => (recorded.push(e), { kind: e.kind, mogId: e.mogId, opId: e.opId, sizes: {}, hashes: {} }) },
    });
    await run(r, bridge.submit(request()).events);
    expect(recorded).toHaveLength(1);
    expect(recorded[0]).toMatchObject({
      kind: 'ask',
      mogId: MOG,
      opId: OP,
      sizes: { question_chars: 'What was revenue last quarter?'.length },
    });
    expect(recorded[0]?.hashes?.sent).toMatch(/^[0-9a-f]{64}$/);
    expect(JSON.stringify(recorded)).not.toContain('revenue');
  });

  it('[SEC-23] a failing audit write never breaks the ask', async () => {
    const r = rig({ responses: [scenarios.normalAnswer()] });
    const bridge = createGenieBridge({
      call: r.call,
      conversations: r.cm,
      audit: { record: () => Promise.reject(new Error('audit down')) },
    });
    const events = await run(r, bridge.submit(request()).events);
    expect(events.at(-1)).toMatchObject({ t: 'answer' });
  });
});
