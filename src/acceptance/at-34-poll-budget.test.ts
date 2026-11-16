/**
 * AT-34 "Poll budget and 429" (#234), section 15 (live). Pass: calls stay at 40/min or less;
 * polls keep priority; AIMD halves and recovers; Retry-After is honoured.
 *
 * Nothing of AT-34 ships yet, so every test here is pending. The brain parts are the poll delay
 * floor of G-08 (the PollActor, #31) and the in-flight limits with visible queues (G-11a..c,
 * #148). The shared bucket, AIMD and Retry-After (G-09a..d, G-10a, G-10b) are R7 in Rust (#138):
 * `crates/mog-core/tests/at_34_poll_budget.rs`. The admin QPM limit (ADM-07, #183) and the live
 * 429 are a person's steps (docs/tests/AT-GENIE-MCP.md, part AT-34).
 */
import { liveFixture } from '../test-support/fixtures';

describe('AT-34 poll budget and 429', () => {
  it.skip('[AT-34] [G-08] 4 in-flight asks space each poll by 1.5 s x 4 TODO(#31)', () => {
    // When #31 lands: on the sim scheduler, 4 in-flight asks on one profile against the #229
    // double with `scenarios.unknownStatus(50)`; each poll delay is 6 s or more and the total
    // stays at 40 calls/min or less over 5 sim minutes.
    expect.unreachable('#31: the PollActor is not built');
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
