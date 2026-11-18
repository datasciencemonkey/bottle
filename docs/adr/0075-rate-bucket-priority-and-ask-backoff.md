# ADR 0075: Rate bucket priority by ticket, and the ask retry after a 429 is in R8

- Status: accepted
- Issue: #138 (G-09a, G-09b, G-09c, G-09d, G-10a, G-10b; ARC-F3; AT-34)
- Date: 2026-11-18
- Owner: impl-138 (agent-lead-2)
- Amends: ADR 0046 decision 11, ADR 0069 decision 4 (`rate_limited`)

## Context

ADR 0046 (#364) put `AimdBucket` behind the `RateBucket` seam in R7: 40 calls/min per profile,
AIMD after a 429, and the `Retry-After` wait. Two rules were left for #138: G-09b (polls, then
cancels, then new asks) and G-10b (Table G-BACKOFF for an ask without a retry header). R8
McpTransport (ADR 0069), the UC2 Genie One path, did not use the bucket at all. The issue left
these points open:

1. How a blocking caller keeps its priority while it waits.
2. Where the ask retry after a 429 runs: in R8, or in the brain.
3. What "5 tries or less" counts.
4. What a `Retry-After` on a 503 does.
5. How the card learns "Genie busy · queued" and "Genie busy, retrying in N s".

SP-18 (#136), the live MCP 429 shape, is not done. It is a spike, so it is a soft dependency.

## Decisions

1. **Tickets.** A call that waits takes a `Ticket` of its `CallClass` (`Poll` < `Cancel` <
   `Ask`). A slot goes only to the first waiting ticket in (class, sequence) order, so a poll that
   comes later still goes before a waiting ask, and one class is FIFO. A call without a ticket
   (`reserve`) goes behind every waiting ticket. A ticket that sees a free slot but a higher
   ticket asks again after 20 ms (`PRIORITY_RECHECK`). `acquire` takes the ticket, waits through
   the `Pause` seam, and leaves the queue on drop, also on a panic. R7 and R8 share one bucket
   per process. The class comes from the R8 tool name or the R7 route ID; anything unknown is an
   ask (the lowest class).
2. **The ask retry is in R8.** R8 knows the HTTP status, the header and the body, and a 429 means
   the server did not take the ask, so a new send makes no duplicate question (MOG-10b). Only
   `genie_ask` is sent again. The wait is the `Retry-After` or FMAPI `error.retry_after` (the
   bucket holds the whole profile for it), or else the full-jitter G-BACKOFF wait. A poll or a
   cancel gets `rate_limited` at once and R8 does not send it again; its next call waits in the
   halved bucket, so only its interval gets longer (G-10b). Without `with_rate_limit` (tests of
   other R8 rules) R8 has no bucket and no retry, as in ADR 0069.
3. **The call cap bounds every wait.** The per-call cap of `McpCall::timeout` (60 s by default,
   120 s at most) is also the budget of all the waits of one call: bucket waits and retry waits.
   A call that would wait past it leaves the queue and is `rate_limited` before it is sent, and an
   ask whose next retry wait does not fit is not sent again. So R8 never sends a question after
   the caller stopped waiting for it (MOG-10b, B1). A `Retry-After` of more than the cap ends the
   ask at once; the profile stays held for the clamped wait.
4. **"5 tries" is 5 sends of one ask in total**, the first one included: so 4 waits with caps
   2, 4, 8 and 16 s. The table maximum of 60 s caps longer schedules.
5. **A 503 with `Retry-After`** holds the profile for the clamped wait (1-300 s) with no AIMD
   cut, in R7 and R8 (the review follow-up on #138). The 503 reply itself is not sent again.
6. **Busy states** are content-free: `BusyState::Queued`, `Retrying { wait }` and `Clear`, with a
   MOG ID, through the `BusyObserver` seam. Only an ask shows `Queued`. The countdown goes when
   the retry starts (Appendix E chip 8). `BusyState::card_text` gives the two texts. The host
   needs a `core_to_brain` event to forward them: contract-change #485, `TODO(#485)`.
7. **The 429 shape.** R8 reads an HTTP 429 with a `Retry-After` header (seconds or an
   IMF-fixdate) or a JSON body with `error.retry_after`. A 429 with a page body is
   `rate_limited` too; the page is dropped, never echoed (rule 6). Any other MCP rate-limit
   shape waits for SP-18: `TODO(#136)`.
8. **One halving per hold.** A 429 inside the 5 min hold starts the hold again at the same rate;
   it does not halve again. So the retries of one ask, or a burst of 429s, keep the profile at
   20 calls/min (G-09c), and it is at 40 again 4 min after the hold (G-09d). A 429 after the hold
   halves the rate that the profile has then.

## Consequences

- The R7 Genie REST ask routes (`genie.start_conversation`, `genie.message`) wait in the bucket
  as asks and record 429s, but get no G-10b retry or queued state: `TODO(#491)`.
- A retried ask takes a new ticket, so asks that queued during its wait can go first.
- `initialize` waits in the bucket while it holds the session lock of its (profile, MOG). Other
  calls on that MOG wait behind it, also a cancel. A session starts once per MOG (and again after
  a 404), so this is rare; the call cap bounds it.
- A queued call blocks its worker thread until it has a slot or its cap ends. A cancel now goes
  before every waiting ask, but it still waits behind a profile-wide `Retry-After` hold.
- The brain classifier of #32 (PR #480) maps the final `rate_limited` to `RATE_LIMITED`. After
  R8 gives up on an ask, the brain must not start its own countdown on top of the 5 tries.
- The PollActor (#31) owns the poll cadence; R8 adds only the bucket wait to it.
