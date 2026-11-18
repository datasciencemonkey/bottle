/**
 * AT-34 "Poll budget and 429" (#234), section 15 (live). Pass: calls stay at 40/min or less;
 * polls keep priority; AIMD halves and recovers; Retry-After is honoured.
 *
 * The poll delay floor of G-08 runs on the PollActor (#31). The in-flight limits with visible
 * queues (G-11a..c) are #148. The shared bucket, AIMD and Retry-After (G-09a..d, G-10a, G-10b) are R7 in Rust (#138):
 * `crates/mog-core/tests/at_34_poll_budget.rs`. The admin QPM limit (ADM-07, #183) and the live
 * 429 are a person's steps (docs/tests/AT-GENIE-MCP.md, part AT-34).
 */
import { liveFixture } from '../test-support/fixtures';
import { scenarios } from '../test-support/genie-one-mcp';
import { PROFILE, collect, polls, rig } from '../test-support/poll-actor-rig';

describe('AT-34 poll budget and 429', () => {
  it('[AT-34] [G-08] 4 in-flight asks space each poll by 1.5 s x 4', async () => {
    // On the sim scheduler, 4 in-flight asks on one profile against the #229 double with
    // `scenarios.unknownStatus(50)`: each poll delay is 6 s or more, and the calls (asks and
    // polls) stay at 40 calls/min or less in every 60 s window over 5 sim minutes.
    let seed = 11;
    const random = () => ((seed = (seed * 16807) % 2147483647) - 1) / 2147483646;
    const r = rig({ responses: [scenarios.unknownStatus(50)], random });
    const targets = [await r.ask(), await r.ask(), await r.ask(), await r.ask()];
    const runs = targets.map((t) => collect(r.cm.poll(t)));
    expect(r.cm.inflightAsks(PROFILE)).toBe(4);
    await r.sched.runUntil(5 * 60_000, { settle: true });
    for (const t of targets) {
      const mine = polls(r).filter((p) => p.args.args.conversation_id === t.conversationId);
      expect(mine.length).toBeGreaterThan(20);
      expect(mine[0]!.start - t.askedAt).toBeGreaterThanOrEqual(6_000);
      for (let i = 1; i < mine.length; i++)
        expect(mine[i]!.start - mine[i - 1]!.end).toBeGreaterThanOrEqual(6_000);
    }
    const starts = [...targets.map((t) => t.askedAt), ...polls(r).map((p) => p.start)];
    for (const t0 of starts) {
      const inWindow = starts.filter((t) => t >= t0 && t < t0 + 60_000).length;
      expect(inWindow).toBeLessThanOrEqual(40);
    }
    expect(runs.every((x) => !x.error)).toBe(true);
  });

  it.skip('[AT-34] [G-11a] [G-11b] [G-11c] in-flight limits per MOG, profile and conversation with visible queues TODO(#148)', () => {
    // When #148 lands: a 3rd ask on a MOG, a 5th on a profile and a 2nd on a conversation wait
    // in a visible queue ("Genie busy · queued") and go in FIFO order.
    expect.unreachable('#148: the in-flight limits are not built');
  });

  it.skip('[AT-34] [ADM-07] live: an admin QPM limit of 40 fits the bucket TODO(#183)', (ctx) => {
    const live = liveFixture();
    if (!live.enabled) return ctx.skip(live.reason);
    expect.unreachable('#183: the live check needs the admin preflight and #138');
  });
});
