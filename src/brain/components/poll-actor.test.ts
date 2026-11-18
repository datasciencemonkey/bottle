/**
 * The PollActor (#31, ADR 0073) on the sim scheduler against the #229 Genie One MCP double.
 * No wall time and no live workspace: polls go to the double through a fake `mcp_call`.
 */
import { scenarios } from '../../test-support/genie-one-mcp';
import { HOST, PROFILE, collect, drain, nth, polls, rig } from '../../test-support/poll-actor-rig';
import { GenieOneMcpBackend } from './genie-bridge';
import { classifyTransportCode } from './genie-errors';
import {
  E_POLL_INFLIGHT,
  GIVE_UP_MS,
  POLL_TIMEOUT_MS,
  basePollDelayMs,
  pollDelayMs,
} from './poll-actor';
import type { AskEvent } from '../../shared/interfaces';

describe('PollActor delay schedule (Table G-POLL)', () => {
  it('[G-07c] base delays: 2 s to 20 s, 3 s to 60 s, 5 s to 180 s, then x1.5 to 15 s', () => {
    expect(basePollDelayMs(0)).toBe(2_000);
    expect(basePollDelayMs(19_999)).toBe(2_000);
    expect(basePollDelayMs(20_000)).toBe(3_000);
    expect(basePollDelayMs(59_999)).toBe(3_000);
    expect(basePollDelayMs(60_000)).toBe(5_000);
    expect(basePollDelayMs(179_999)).toBe(5_000);
    expect(basePollDelayMs(180_000, 5_000)).toBe(7_500);
    expect(basePollDelayMs(190_000, 7_500)).toBe(11_250);
    expect(basePollDelayMs(200_000, 11_250)).toBe(15_000);
    expect(basePollDelayMs(400_000, 15_000)).toBe(15_000);
    // A resume after 180 s with no earlier delay starts from 5 s x 1.5.
    expect(basePollDelayMs(300_000)).toBe(7_500);
  });

  it('[G-07c] every step gets +-10 % jitter', () => {
    const at = (elapsedMs: number, r: number) =>
      pollDelayMs({ elapsedMs, inflightAsks: 1, random: () => r }).delayMs;
    expect(at(0, 0)).toBeCloseTo(1_800);
    expect(at(0, 0.5)).toBeCloseTo(2_000);
    expect(at(0, 0.999_999)).toBeCloseTo(2_200, 0);
    expect(at(30_000, 0)).toBeCloseTo(2_700);
    expect(at(100_000, 0.999_999)).toBeCloseTo(5_500, 0);
    for (let i = 0; i < 200; i++) {
      const r = i / 200;
      const d = pollDelayMs({ elapsedMs: 70_000, inflightAsks: 1, random: () => r }).delayMs;
      expect(d).toBeGreaterThanOrEqual(4_500);
      expect(d).toBeLessThanOrEqual(5_500);
    }
  });

  it('[G-08] each delay is at least 1.5 s x the in-flight asks on the profile, after the jitter', () => {
    const at = (inflightAsks: number, elapsedMs = 0, r = 0.5) =>
      pollDelayMs({ elapsedMs, previousBaseMs: 15_000, inflightAsks, random: () => r }).delayMs;
    expect(at(1)).toBe(2_000);
    expect(at(0)).toBe(2_000); // never under one ask
    expect(at(2, 0, 0)).toBe(3_000); // 1.8 s jittered, floor 3 s
    expect(at(4)).toBe(6_000);
    expect(at(4, 100_000, 0.999_999)).toBe(6_000);
    expect(at(4, 400_000)).toBe(15_000); // the floor never shortens a delay
    expect(at(12)).toBe(18_000);
  });
});

describe('PollActor loop', () => {
  it('[G-07a] [G-07c] polls carry no meta and follow G-POLL from the end of the previous poll', async () => {
    const r = rig({ responses: [scenarios.unknownStatus(10_000)], pollMs: 700 });
    const target = await r.ask();
    const { outcome } = await drain(r, r.cm.poll(target));
    expect(outcome).toEqual({ kind: 'gave_up' });
    const p = polls(r);
    expect(p.length).toBeGreaterThan(60);
    let prevBase: number | undefined;
    let prevEnd = target.askedAt;
    for (const x of p) {
      expect(x.args.meta).toBeUndefined();
      expect(JSON.stringify(x.args)).not.toContain('progressToken');
      const base = basePollDelayMs(prevEnd - target.askedAt, prevBase);
      expect(x.start - prevEnd).toBe(base); // random 0.5: no jitter
      prevBase = base;
      prevEnd = x.end;
    }
  });

  it('[G-07b] a poll loop and concurrent callers on one conversation never overlap', async () => {
    const r = rig({ responses: [scenarios.unknownStatus(10_000)], pollMs: 1_500 });
    const target = await r.ask();
    // Another caller (for example a manual "Check again") polls the same conversation often.
    for (let t = 0; t < 60_000; t += 1_000)
      r.sched.at(t, () => {
        void r.cm.enqueuePoll(target.conversationId, () =>
          r.double.call('genie_poll_response', {
            conversation_id: target.conversationId,
            response_id: target.responseId,
          }),
        );
      });
    const ctl = new AbortController();
    r.sched.at(120_000, () => ctl.abort());
    const { outcome } = await drain(r, r.cm.poll(target, ctl.signal), 400_000);
    expect(outcome).toEqual({ kind: 'aborted' });
    expect(polls(r).length).toBeGreaterThan(3);
    expect(r.double.polls().length).toBeGreaterThan(60);
    expect(r.double.maxConcurrentPolls()).toBe(1);
  });

  it('[G-18a] [G-18b] unknown keeps polling with the same IDs and ends at completed', async () => {
    const r = rig({ responses: [scenarios.unknownStatus(7)] });
    const target = await r.ask();
    const { updates, outcome } = await drain(r, r.cm.poll(target));
    expect(updates.map((u) => u.status)).toEqual([...Array(7).fill('unknown'), 'completed']);
    expect(outcome).toMatchObject({ kind: 'terminal', status: 'completed' });
    for (const p of polls(r))
      expect(p.args.args).toEqual({
        conversation_id: target.conversationId,
        response_id: target.responseId,
      });
    expect(r.sent.some((s) => s.args.tool === 'genie_ask')).toBe(false);
  });

  it('[G-18b] unknown forever reaches the give-up with the same IDs and no re-ask', async () => {
    const r = rig({ responses: [scenarios.unknownStatus(100_000)] });
    const target = await r.ask();
    const { outcome } = await drain(r, r.cm.poll(target));
    expect(outcome).toEqual({ kind: 'gave_up' });
    const p = polls(r);
    expect(p.every((x) => x.start <= target.askedAt + GIVE_UP_MS)).toBe(true);
    expect(new Set(p.map((x) => JSON.stringify(x.args.args))).size).toBe(1);
    expect(r.double.calls().filter((c) => c.tool === 'genie_ask')).toHaveLength(1);
    // The IDs stay for the late re-polls (#149).
    expect(r.journal.get(target.conversationId, target.responseId)).toBeDefined();
  });

  it('[G-18a] a status the bridge does not know is not terminal', async () => {
    const r = rig({
      responses: [
        {
          polls: [
            { status: 'queued' as never, progress_steps: null },
            { status: 'completed', final_answer: 'Done.' },
          ],
        },
      ],
    });
    const target = await r.ask();
    const { updates, outcome } = await drain(r, r.cm.poll(target));
    expect(updates.map((u) => u.status)).toEqual(['unknown', 'completed']);
    expect(outcome).toMatchObject({ kind: 'terminal', status: 'completed' });
  });

  it('[G-16c] a poll timeout polls again with the same IDs; every poll has the 30 s cap', async () => {
    const r = rig({
      responses: [scenarios.unknownStatus(3)],
      reject: (n) => (n === 2 || n === 3 ? { error: 'timeout' } : undefined),
    });
    const target = await r.ask();
    const { outcome } = await drain(r, r.cm.poll(target));
    expect(outcome).toMatchObject({ kind: 'terminal', status: 'completed' });
    const p = polls(r);
    expect(p).toHaveLength(6); // 4 replies plus 2 timeouts
    for (const x of p) {
      expect(x.args.timeoutMs).toBe(POLL_TIMEOUT_MS);
      expect(x.args.args).toEqual({
        conversation_id: target.conversationId,
        response_id: target.responseId,
      });
    }
    expect(r.sent.some((s) => s.args.tool === 'genie_ask')).toBe(false);
  });

  it('[ARC-F1] an E_POLL_INFLIGHT refusal from R8 waits one more delay and polls again', async () => {
    const r = rig({
      responses: [scenarios.unknownStatus(1)],
      reject: (n) => (n === 1 ? { error: E_POLL_INFLIGHT } : undefined),
    });
    const target = await r.ask();
    const { outcome } = await drain(r, r.cm.poll(target));
    expect(outcome).toMatchObject({ kind: 'terminal', status: 'completed' });
    const p = polls(r);
    expect(p).toHaveLength(3);
    expect(nth(p, 1).start - nth(p, 0).end).toBe(2_000);
  });

  it('[ARC-F1] [G-14] poll_inflight: the actor polls again; any other caller gets CONVERSATION_BUSY, never a new ask', async () => {
    expect(E_POLL_INFLIGHT).toBe('poll_inflight');
    const busy = classifyTransportCode(E_POLL_INFLIGHT, { host: HOST });
    expect(busy.code).toBe('CONVERSATION_BUSY');
    expect(busy.row).toBe('transport:poll_inflight');
    expect(busy.retry).toBe('user');
    expect(busy.autoRetry).toBe(false);
    // The actor keeps the same IDs through several refusals in a row and asks nothing.
    const r = rig({
      responses: [scenarios.unknownStatus(1)],
      reject: (n) => (n <= 3 ? { error: E_POLL_INFLIGHT } : undefined),
    });
    const target = await r.ask();
    const { outcome } = await drain(r, r.cm.poll(target));
    expect(outcome).toMatchObject({ kind: 'terminal', status: 'completed' });
    expect(polls(r)).toHaveLength(5);
    expect(new Set(polls(r).map((p) => JSON.stringify(p.args.args))).size).toBe(1);
    expect(r.double.asks()).toHaveLength(1);
  });

  it('[ARC-F5] another refusal ends the loop and keeps the IDs for a resume', async () => {
    const r = rig({ reject: (n) => (n === 2 ? { error: 'auth_expired' } : undefined) });
    const target = await r.ask();
    await expect(drain(r, r.cm.poll(target))).rejects.toEqual({ error: 'auth_expired' });
    expect(r.journal.get(target.conversationId, target.responseId)).toBeDefined();
    expect(r.cm.inflightAsks(PROFILE)).toBe(0);
  });

  it('[G-14] an isError reply ends the loop with the raw reply for the GL-5 mapper', async () => {
    const r = rig();
    const target = await r.ask();
    const bad = { ...target, responseId: 'resp-missing' };
    const { outcome } = await drain(r, r.cm.poll(bad));
    expect(outcome).toMatchObject({ kind: 'error', reply: { isError: true } });
    expect(polls(r)).toHaveLength(1);
  });

  it('[ARC-F11] abort stops the loop before the next poll', async () => {
    const r = rig({ responses: [scenarios.unknownStatus(1_000)] });
    const target = await r.ask();
    const ctl = new AbortController();
    r.sched.at(9_000, () => ctl.abort());
    const { outcome } = await drain(r, r.cm.poll(target, ctl.signal));
    expect(outcome).toEqual({ kind: 'aborted' });
    expect(polls(r).every((x) => x.start <= 9_000)).toBe(true);
  });
});

describe('PollActor in-flight floor and journal', () => {
  it('[G-08] 4 in-flight asks on a profile space each poll by 6 s or more', async () => {
    const r = rig({ responses: [scenarios.unknownStatus(30)], random: () => 0 });
    const targets = [await r.ask(), await r.ask(), await r.ask(), await r.ask()];
    const runs = targets.map((t) => collect(r.cm.poll(t)));
    expect(r.cm.inflightAsks(PROFILE)).toBe(4);
    await r.sched.runUntil(GIVE_UP_MS, { settle: true });
    expect(runs.every((x) => x.ended && x.outcome?.kind === 'terminal')).toBe(true);
    for (const t of targets) {
      const mine = polls(r).filter((p) => p.args.args.conversation_id === t.conversationId);
      expect(nth(mine, 0).start - t.askedAt).toBeGreaterThanOrEqual(6_000);
      for (let i = 1; i < mine.length; i++)
        expect(nth(mine, i).start - nth(mine, i - 1).end).toBeGreaterThanOrEqual(6_000);
    }
    expect(r.cm.inflightAsks(PROFILE)).toBe(0);
  });

  it('[ARC-F10] the journal keeps the IDs and profile while polling, and drops them at the end', async () => {
    const r = rig({ responses: [scenarios.unknownStatus(2)] });
    const target = await r.ask();
    const loop = r.cm.poll(target);
    const first = loop.next();
    await r.sched.runUntil(2_000, { settle: true });
    expect((await first).value).toMatchObject({ status: 'unknown', n: 1 });
    expect(r.journal.list()).toEqual([target]);
    await drain(r, loop);
    expect(r.journal.list()).toEqual([]);
  });

  it('[ARC-F10] [MOG-11a] resume polls a journaled ask with its original IDs and never asks again', async () => {
    const r = rig({ responses: [scenarios.nullProgressSteps()] });
    const target = await r.ask();
    // The app stopped after the ask: only the journal row is left.
    r.journal.save(target);
    const events: AskEvent[] = [];
    const cmBackend = new GenieOneMcpBackend(r.cm);
    const it = cmBackend.resume({
      backend: 'genie_one_mcp',
      profile: PROFILE,
      host: HOST,
      conversationId: target.conversationId,
      responseId: target.responseId,
    });
    const done = (async () => {
      for await (const e of it) events.push(e);
    })();
    await r.sched.runUntil(60_000, { settle: true });
    await done;
    expect(events[0]).toMatchObject({ t: 'progress', steps: null });
    expect(events.at(-1)).toMatchObject({ t: 'answer', md: 'Done.', incomplete: false });
    expect(r.double.calls().filter((c) => c.tool === 'genie_ask')).toHaveLength(1);
    for (const p of polls(r))
      expect(p.args.args).toEqual({
        conversation_id: target.conversationId,
        response_id: target.responseId,
      });
  });

  it('[MOG-10b] resume without a response ID or a journal row sends nothing', async () => {
    const r = rig();
    const target = await r.ask();
    const b = new GenieOneMcpBackend(r.cm);
    const ref = { backend: 'genie_one_mcp' as const, profile: PROFILE, host: HOST };
    const first = async (it: AsyncIterable<AskEvent>) => it[Symbol.asyncIterator]().next();
    await expect(
      first(b.resume({ ...ref, conversationId: target.conversationId })),
    ).rejects.toThrow(/no_response_id/);
    await expect(
      first(b.resume({ ...ref, conversationId: target.conversationId, responseId: 'resp-x' })),
    ).rejects.toThrow(/not_in_journal/);
    r.journal.save(target);
    await expect(
      first(
        b.resume({
          ...ref,
          profile: 'other-profile',
          conversationId: target.conversationId,
          responseId: target.responseId,
        }),
      ),
    ).rejects.toThrow(/other_profile/);
    expect(r.sent).toEqual([]);
  });
});
