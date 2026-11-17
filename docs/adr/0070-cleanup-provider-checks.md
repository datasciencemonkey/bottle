# ADR 0070: CleanupProvider pre-pass, deadline marks and CAP-CHECK baseline

- Status: accepted
- Issue: #55 (CAP-66a, CAP-66b, CAP-67a, CAP-67b, CAP-68, CAP-69, ARC-F3, AT-05, AT-06, AT-21).
  Soft dependencies: #52 (SttProvider chain), #54 (SP-07 latency).
- Date: 2026-11-17
- Owner: impl-55 (capture-lead-2)
- Amends: ADR 0065 decision 2.

## Context

T5 CleanupProvider runs a deterministic pre-pass (fillers, dictionary, numerals), then one
non-streaming cleanup model call with a 900 ms deadline. On the deadline it must use the
pre-pass text with the mark "not cleaned" (CAP-67b). When a reply fails a Table CAP-CHECK check,
it must use the raw text (CAP-68). The requirements leave these points open:

- the exact pre-pass rules, and where the user dictionary comes from;
- the text that the CAP-CHECK checks compare the reply with;
- how the result tells "raw text" apart from "pre-pass text, not cleaned";
- what "an answer to the question appears" means for a client check;
- how an FMAPI 429 reaches the brain (ARC-F3), because `model_invoke` has no 429 code.

## Decisions

1. **Pre-pass rules** (`src/brain/components/cleanup-prepass.ts`). The filler lists, the capital
   hand-over and the stutter collapse come from Handy (`audio_toolkit/text.rs`, MIT). A number
   word never collapses ("five five five" is a digit read-out). "um",
   "ah" and "eh" go only for English, because they are words in other languages. The user
   dictionary is an injected list of whole-word replacements, empty by default; no settings
   page fills it yet. Numerals: a spoken cardinal becomes digits only when it is one whole
   cardinal of 2 or more words, or one word of 10 or more ("twenty five" → "25"). A lone "one"
   to "nine" and a digit read-out ("one two three") stay in words. A run next to another number
   also stays in words: next to a digit ("40 thousand"), with "point" on either side ("two
   point five million", "version two point eleven"), or in a range or list ("three to four
   hundred", "twenty or thirty"). The pre-pass is pure and idempotent.
1a. **Pre-pass self-check (CAP-68).** The fillers, stutter and dictionary steps must keep the
   numbers of the raw text (digit tokens and number words). The numerals step must only swap
   each run for its value: each new digit token, written back in words, must give the same
   number words. When a check fails, the pre-pass gives the raw text. So a dictionary entry
   that writes a number ("q three" → "Q3") keeps the raw text. The self-check does not judge
   the context rule of decision 1; the AT-05 corpus covers that rule with spoken-number items
   (decimals with a scale, a digit read-out, ranges and lists).
2. **Fewer than 4 words** counts the words of the pre-pass text, because that is the model input.
3. **The check baseline is the pre-pass text, and the fallback is the raw text.** The pre-pass
   is deterministic, so only the model can change the meaning. The checks
   (`src/brain/components/cleanup-checks.ts`) compare the reply with the pre-pass text. When one
   fails, the result is the raw STT text, unchanged.
4. **Result marks.** `CleanupResult` is `{ text, usedRaw, notCleaned, outcome }`. `usedRaw` is
   true only for the CAP-68 raw fallback. `notCleaned` is true when the text is the pre-pass
   text because the model did not clean it: deadline, 429, a failed or empty call, or no
   profile. `outcome` is a code (`cleaned`, `short`, `deadline`, `rate_limited`,
   `model_failed`, `no_profile`, `check_failed`). The result has no intent, route, backend,
   profile or workspace field (CAP-69). This amends ADR 0065 decision 2: a failed cleanup
   gives `notCleaned=true`, not `usedRaw=true`.
5. **`runCleanupAndRouter` takes the raw text.** It runs `CleanupProvider.prepass` once and gives
   the pre-pass text to the router step (RTR-09a). It gives the raw text to `clean`, so the
   CAP-68 fallback is the real raw text. It passes the router profile to cleanup.
6. **Deadline.** The brain starts a 900 ms timer and also sends `deadlineMs: 900` to Rust, so
   the proxy cancels the HTTP request. When the timer fires first, the result is the pre-pass
   text and a late reply is dropped. A test may set a lower deadline, never a higher one.
   TODO(#54): SP-07 may retune 900 ms.
7. **CAP-CHECK rows as client checks.** Numbers: the sorted numbers must be equal, digit tokens
   (thousands commas do not count) and spoken number words both. So a reply that writes "two
   point five million" as "2.5 million" also keeps the raw text. Entities: no name may go or appear, without regard to case, with the
   ADR 0065 entity rule; dotted and snake-case names (table names) must be equal. Length ratio:
   0.5-1.3 of the pre-pass text. URLs: no new URL. Code: no new backtick, script or HTML tag,
   `$(` or `${`, and the reply must not be a JSON value. Answer: for a question (it ends with
   "?" or starts with a question word), the reply fails when it adds a sentence, or when it is
   no longer a question and has a word that the question did not have. Every rule leans to
   fail, because a false alarm only shows the raw text.
8. **Negation check (stricter than the table).** The negation words ("not", "no", "never",
   ...) must be equal. A contraction counts as its long form ("don't" = "do not", "won't" =
   "will not", "can't" = "cannot" = "can not"), so expanding a contraction is not a change. The AT-05 corpus treats negation as meaning.
9. **429.** A reply body with `error_code: REQUEST_LIMIT_EXCEEDED`, `status: 429` or an error
   code 429, or a rejection with such a code, gives `rate_limited` and the pre-pass text. Any
   other failure gives `model_failed` and the pre-pass text, so the user sees the same text.
10. **Model per region (D58).** `cleanupModelFor(region)` gives Llama 3.1 8B where the region
    serves it (the `servesLlama318b` rule of ADR 0065), else `gpt-oss-20b`. The reply reader
    keeps only text parts of a list-of-blocks content.

## Consequences

- Dictation shows the pre-pass text on a slow or busy model, and never waits more than 900 ms
  for cleanup.
- A reply that changes a number, a name, a table name or a negation never reaches the card,
  the insertion or the Genie question.
- Self-corrections ("50 no sorry 500") often fall back to the raw text, because the model drops
  a number. This is safe but less clean. A later change can allow a dropped number only when
  the pre-pass marks a self-correction.
- The user dictionary needs a settings source. The 429 shape of the Rust `model_invoke` is not
  defined yet; when Rust adds a fixed code, `isRateLimited` maps it.
