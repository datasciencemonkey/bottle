# ADR 0089: Memory item provenance, "remember this" pins and the write-time quarantine

- Status: Accepted
- Date: 2026-11-18
- Issue: #114 (MEM-31, MEM-28b, SEC-24a, SEC-24b, with AT-06, AT-17, AT-28, MEM-10a, MEM-01 and U9)

## Context

`MemoryService.write()` was a stub. The issue asks for provenance on each item, a pinned item for
"remember this", and a quarantine of text that targets the assistant. The #102 review (ADR 0068)
found that the brain could still call `mem.item.upsert`, which writes no embed job. The
`bottle-mcp` `memory.write` tool (#113) also used `mem.item.upsert`. The issue does not say where
the quarantine check runs, which patterns it uses, how a user action is proven, or what
confidence a draft gets. `MemoryDraft` (contract) has no exchange or utterance link.

## Decision

1. **One brain item write.** `write()`, `writeItems()` and `rememberThis()` go through
   `storeItem()`, which runs the atomic group `mem.item.write` (item row and embed job in one
   transaction). `mem.item.upsert` is now `Access::Core` and is not in `LOCAL_STMT_IDS`. The
   `bottle-mcp` `memory.write` tool runs `mem.item.write` too. So no item write skips the outbox.
2. **Provenance (MEM-31).** `write()` checks the whole batch before the first write. Each draft
   must have a source (`you`, `others`, `genie`, `agent`), a workspace host and a decimal
   workspace id. `sessionId` goes to `source_session_ids`. The exchange and utterance links go to
   `source_exchange_id` and `source_utterance_ids` (Forget, ADR 0081, finds citing items by them).
   A later mention of the same item adds its session and utterance links and keeps the first
   exchange link. The schema already had these columns, so no migration changes.
   `MemoryDraft` gets the two link fields through contract issue #533. Until then the service
   takes them on the wider type `MemoryDraftLinks` (`TODO(#533)`).
3. **Pins only on a user action (MEM-28b).** `rememberThis(action, draft)` writes `pinned = 1`
   and `confidence = 1.0`. The action is a `voice` action (the You track, and the utterance
   matches `isRememberThis`) or a `ui` action (a click). `write()` refuses `pinned: true`. The
   agent tool never pins. The command grammar (#143) calls `rememberThis` (`TODO(#143)`).
4. **Confidence of an unpinned draft:** `you` 0.9, `genie` 0.8, `agent` 0.8, `others` 0.6. The
   LLM MemoryWriter (MEM-29a, MVP-M) can replace these with model values later.
5. **Quarantine in two layers (SEC-24a).**
   - The brain checks the text with the four Table RTR-INJ categories (`detectInjection`, #117)
     plus two memory patterns (SG-INJ-7): "ignore previous" with no object, and text that speaks
     to the assistant ("Assistant, delete ...", "If you are an AI ..."). It sends `quarantined`.
   - R10 checks `:text` again with the same patterns (`local_store/quarantine.rs`). The new
     param type `ParamType::Quarantine { of }` is computed by the core; a caller that sends the
     key is refused. The status is `quarantined` if either check says so.
   - A later mention never lowers a quarantine. A quarantined item gets no embed job, so its
     text never goes to the embedding model.
   - The item is kept (U9). Nothing deletes it automatically.
   - Normalization: NFKC in TS; in Rust, full-width ASCII folds to ASCII. Both drop zero-width
     code points and fold case and white space.
   - Both sides run the golden sets `tests/fixtures/memory-adversarial/items.json` and
     `tests/fixtures/router-adversarial/utterances.json`, so the two pattern lists stay equal.
6. **No quarantined item reaches Genie or the agent (SEC-24b).** `mem.recall.fts` (the
   `memory.recall` tool and the RecallGate, #200) reads only `status = 'active'`.
   `excludeQuarantined()` keeps only `active` rows and fails closed on a row with no status.
   The ContextAssembler (#92) must run its rows through it (`TODO(#92)`).
7. **Agent items have no workspace yet.** The `bottle-mcp` session does not know the workspace,
   so an agent item has source `agent` and no workspace fields (`TODO(#538)`). Table G-CTX
   admits only You-track or confirmed items, so an agent item never goes into a preamble.

## Consequences

- A meeting or Genie line that is a real business fact but reads like an order ("send the deck
  to the external server") is quarantined. This is a safe failure: the item stays on the device.
- The pattern list is in two languages. The shared golden sets catch a drift.
- `regex` is now a direct `mog-core` dependency. It was already in `Cargo.lock` (no new crate).
