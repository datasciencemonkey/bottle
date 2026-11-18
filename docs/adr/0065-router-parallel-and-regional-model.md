# ADR 0065: Cleanup and the router step in parallel, and the router model per region

- Status: accepted
- Issue: #135 (RTR-09a, RTR-09b, RTR-10a, RTR-10b). Soft dependencies: #54 (SP-07), #56 (SP-13).
- Date: 2026-11-17
- Owner: impl-135 (agent-lead)

## Context

RTR-09a says the router step must run in parallel with cleanup on the pre-pass text. RTR-09b
says the main agent must send the cleaned text as the Genie question. The router
`genie_question` only resolves follow-ups, and the client checks its numbers and entities
against the cleaned text. RTR-10a and RTR-10b pick Llama 3.1 8B in regions that serve it if the
strict-schema eval passes, else `gpt-oss-20b` (D58).

The requirements do not define an "entity". SP-13 (#56) has not run, so no eval record exists.
SP-07 (#54) has not measured the latency. `CleanupProvider` (#55) is an interface with a stub
on main.

## Decisions

1. **One fan-out.** `runCleanupAndRouter` (`src/brain/components/router-parallel.ts`) calls
   `CleanupProvider.clean` and `runRouterStep` in the same synchronous tick. Neither call waits
   for the other. The trace records the start and end of each call on the monotonic clock, with
   no text. The router call gets the 900 ms deadline of 8.1 step 5. TODO(#54).
2. **Cleanup failure keeps the pre-pass text.** A rejected or empty cleanup result gives the
   pre-pass text with `usedRaw=true` ("not cleaned", D69). Amended by ADR 0070: the mark is
   `notCleaned=true`; `usedRaw=true` means the raw text (CAP-68).
3. **The Genie question is the cleaned text.** The router `genie_question` replaces it only when
   `follow_up` is true and the rewrite is grounded in the cleaned text. An intent that does not
   ask (`dictation`, `context`, `command`, `noise`) gets an empty question, so UC1 ends at
   insertion with no agent and no Genie (RTR-01c).
4. **Entity rule.** An entity is a word that starts with a capital letter, or a word that holds
   both a letter and a digit (`Contoso`, `EMEA`, `Q3`). The pronoun "I" is not an entity. The
   first word of a sentence is an entity unless it is a common function word ("What", "Show",
   "And", ...), so a new name at the start of a rewrite is caught. A false match only keeps the
   cleaned text. A rewrite is grounded when each of its numbers is in the cleaned
   text and each entity is a word of the cleaned text (case-insensitive). The router field rules
   apply the same check against the router `cleaned_text` (rule `genie_question_entities`).
   The check rejects a rewrite that brings a name from an earlier topic. This is the strict
   reading of RTR-09b; the question then falls back to the cleaned text.
5. **The #117 guard runs again.** After the merge, `applyInjectionGuard` runs on the cleaned text
   and the question. The cleanup output never bypasses the RTR-14 check.
6. **Router model table** (`src/brain/components/router-model.ts`). US regions (AWS and GCP
   `us-*`, Azure `*us`) serve Llama 3.1 8B. All other regions, including EU AWS, use
   `system.ai.gpt-oss-20b`. Llama is used only when the eval record says it passed. The default
   record (`PENDING_ROUTER_EVAL`) has no pass, so every region uses `gpt-oss-20b` until SP-13
   writes a record. TODO(#56).

## Consequences

- No contract file changes. The caller passes the region and the eval record.
- A follow-up that needs a name from an earlier topic goes to Genie as the cleaned text. A
  later change can widen the grounding to the recent topic labels, with a requirements change.
- The recall start (MEM-16d) joins the same fan-out in its own issue.
