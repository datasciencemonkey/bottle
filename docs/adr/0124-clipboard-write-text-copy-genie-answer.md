# ADR 0124: `clipboard_write_text`, and copying a finished Genie answer to the clipboard

- Status: Accepted (owner decision, 2026-11-25)
- Issue: owner request (no GitHub issue filed); see `research/message-board.md` for the lane note
- Related: ADR 0048 (`insert_text`'s SEC-30 concealed, transient clipboard write), ADR 0122 (the
  orchestrator model and the owner's answer rule), ADR 0121 (agent threads in Conversations)
- Owner: lane `arch/f-copy`

## Context

The owner's request, verbatim: "when using conversations, after genie responds, can you attach
genie's response to the clipboard." Bottle already writes the clipboard once, for `insert_text`
(ADR 0048): that write is deliberately concealed and transient (SEC-30), because it exists only to
drive a paste chord and must look like nothing happened if a password manager had something on
the clipboard first. A finished Genie answer needs the opposite contract: the user wants it to
stay on the clipboard, pasteable into an email or a doc minutes later, with no password-manager
privacy flag hiding it from a clipboard manager's history.

The owner's sign-off authorizes the contract change this needs: one new IPC command, plus
anything small that goes with it (a Settings toggle, a small view note).

## Decisions

1. **One new IPC command, `clipboard_write_text { text }`, brain only.** The webview cannot write
   the clipboard with no user gesture, and the answer arrives on its own, so `navigator.clipboard`
   is not an option; the write has to be native, like `insert_text`. The reply is a typed outcome,
   the same convention as `insert_text`'s `InsertOutcome`: `{ result: written | too_large |
   unsupported }`, never a thrown IPC error, so a size refusal or an unsupported OS crosses as
   ordinary data the brain can act on quietly.
2. **Durable, not transient (`mog_core::window_host::clipboard_formats_durable`).** No
   `org.nspasteboard.TransientType` / `ExcludeClipboardContentFromMonitorProcessing` marker: the
   answer is a normal item a clipboard manager keeps, exactly as if the user had selected and
   copied the text by hand.
3. **Not concealed either, on purpose, even though a Genie answer can hold business data.** The
   concealed marker (`org.nspasteboard.ConcealedType`) is also the one signal
   `insertion::mac_snapshot_is_concealed` (ADR 0048, decision 12) uses to decide a saved clipboard
   item looks like a password-manager secret and must not be restored after a paste. Marking the
   answer concealed would make the very next dictation silently drop it instead of restoring it,
   which directly contradicts "durable, paste it later" (decision 2). The user chose, with the
   plain Settings toggle of decision 6, to put this specific answer on their own clipboard; that
   is the same trust boundary as copying it by hand, so it carries no extra pasteboard-manager
   privacy flag.
4. **A 100 KB bound (`CLIPBOARD_WRITE_MAX_BYTES`).** A Genie answer never needs more; a longer one
   answers `too_large` rather than hang the pasteboard on a giant write.
5. **Windows reuses the existing `WindowsClipboard` backend; Linux is `unsupported`.**
   `ClipboardWriteTextComponent<B>` is generic over the same `ClipboardBackend` trait `insert_text`
   already uses, so macOS and Windows share almost all of this with no new platform code. Linux
   gets a component built with `ClipboardWriteTextComponent::unsupported()` (an always-compiled
   `NoClipboard` backend that is never actually called), so a Rust unit test can exercise the
   "unsupported OS" outcome without a Linux build.
6. **One plain Settings → Workspace toggle, "Copy Genie answers to the clipboard", on by
   default.** It persists through the existing `workspace_settings_write` / `AgentSettings`
   path, the same shape as "Clean up dictation" (ADR 0122) and "Dictation engine" (ADR 0123). It
   sits outside the orchestrator-model block of that screen, because the copy also covers the
   direct Genie fallback (ADR 0097), which needs no orchestrator model at all.
7. **The brain decides what "the answer" is, with the owner's own rule
   (`src/shared/agent-answer-rule.ts`), and copies once per finished turn.** A new
   `finishedAnswerText(turn)` mirrors the exact text the card and a Conversations reply show: the
   combined note (`[n]` markers stripped — see decision 8) when the turn has one, else the one
   Genie answer `loneAnswerIndex` names, verbatim. It answers `undefined` for a withheld
   (quarantined) answer, a turn that is not `done` (an error, a stop, or still streaming), so the
   brain never copies then. `createAnswerClipboard` (`src/brain/components/answer-clipboard.ts`)
   hooks the one place a turn reaches its final state (`showTurn`, shared by the ⌥G card and a
   Conversations reply) and a `Set` of handled turn IDs makes the copy fire exactly once.
8. **The `[n]` citation markers are stripped from a copied combined answer, kept verbatim for a
   lone Genie answer.** A lone Genie answer never has citation markers. A combined note's `[1]`,
   `[2]` markers are meaningful only with the card's own citation chips and footer links
   (`AgentCitation`, ADR 0122); pasted into Slack or an email they are bare numbers with nothing to
   point to — junk, not content. `stripCitationMarkers` removes a bare `[n]` (and the one space
   before it) but leaves a real Markdown link such as `[1](url)` or `[1][ref]` untouched. The
   Markdown itself (tables, bold, links) is kept.
9. **The clipboard write waits for the same arrival gate `createAgentCard` already uses
   (`whenClear`), not a new queue.** `insert_text`'s own paste snapshots and restores the
   clipboard (ADR 0048); writing the answer while that is in flight would race it. Reusing the
   existing gate is the simplest correct fix: no second queue to build or test.
10. **A quiet note, never a banner.** `AgentTurnView` gets one new, brain-owned, transient field,
    `copyNote?: 'copied' | 'failed'`, set once the write resolves. `TurnBody` (shared by the card
    and a Conversations reply) renders a small `CopyAnswerNote`: a check mark and "Copied" in
    muted text on success, a quiet "Couldn't copy" on failure, nothing while the write is in
    flight. It shows for 2 s then removes itself in the view, with no further brain round-trip
    (the same shape as the existing `memory-saved.tsx` note); `prefers-reduced-motion` skips the
    entrance animation.

## Consequences

- Contract surface touched: one new IPC command (`clipboard_write_text`, `ClipboardWriteTextArgs`,
  `crates/mog-core/src/ipc.rs`), one new field on `WorkspaceSettingsWriteArgs` and
  `AgentSettings`/`AgentData` (`copyGenieAnswer` / `copy_genie_answer`), one new optional field on
  `AgentTurnView` (`copyNote`), one new `brain.json` capability
  (`allow-clipboard-write-text`), and the regenerated `src-tauri/schemas/ipc-commands.json`.
- A finished answer's clipboard copy can still be silently dropped by an unrelated clipboard write
  from another app between the brain's write and the user's paste; Bottle accepts this, the same
  as any other clipboard content.
- If the user reopens a finished turn's card shortly after a copy (within the note's local 2 s
  window), the "Copied" note can show again, because the view's fade timer is local state, not a
  brain-cleared field. This is cosmetic: accepted rather than adding a second brain round-trip to
  clear `copyNote` on a timer.
- `AgentThread.tsx`'s live turn reuses `TurnBody` with no changes of its own, so the note and the
  copy both reach Conversations for free; it has no dedicated test file of its own (none existed
  before this change), so this path is covered by `TurnBody`'s own tests plus code inspection, not
  a new `AgentThread.tsx` test suite.
