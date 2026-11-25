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
   default.** Default-on is the owner's own instruction ("It is on by default and persists
   through the existing settings path"), not an engineering default. It persists through the
   existing `workspace_settings_write` / `AgentSettings` path, the same shape as "Clean up
   dictation" (ADR 0122) and "Dictation engine" (ADR 0123). It sits outside the
   orchestrator-model block of that screen, because the copy also covers the direct Genie
   fallback (ADR 0097), which needs no orchestrator model at all.
7. **Two hooks feed one clipboard gate, because the direct Genie path is the default for most
   users, not the orchestrator turn.** `createAnswerClipboard`
   (`src/brain/components/answer-clipboard.ts`) takes `onTurnFinished` and `onDirectAnswered`:
   - `onTurnFinished` is called from a new `agent-run.ts` hook, `deps.finished(mogId, view)`,
     fired once, only for an `answered` turn, only at the very end of `finish()`, after the late
     pulls, `summarise` and `syncParts` have all run and the view is settled. It is a separate
     hook from `showTurn`, which can still fire earlier, mid-settle, with incomplete parts; an
     earlier revision of this ADR wrongly hooked `showTurn` and could copy a partial answer.
     `onTurnFinished` uses the owner's own rule (`finishedAnswerText`,
     `src/shared/agent-answer-rule.ts`): the one Genie answer `loneAnswerIndex` names, verbatim,
     when one applies (checked first); only otherwise the model's combined note, its `[n]`
     markers stripped (decision 8). It answers `undefined` for a withheld (quarantined) answer or
     a hidden combined note, so the brain never copies then.
   - `onDirectAnswered` is called from a new `genie-direct.ts` hook, `onAnswered(mogId, opId, md)`,
     fired once for a real Genie answer that is not partial (never `incomplete: true`, G-23a — an
     earlier revision of this ADR missed this check), a clarify question, a failure or a stop, on
     the direct path. One `send` already covers the router's direct route, the zero-call fallback
     and the refused-submit fallback, so this one hook covers all three. It copies `md` verbatim: a
     direct send is always one Genie call, so it never carries `[n]` markers to strip. It never
     fires for an answer a newer send has already superseded (`latestSend > mySeq`): an older
     answer landing late must never overwrite what the user is already looking at.

   Both fire at most once per key (`turn:<turnId>` or `direct:<opId>`, a capped `Set` of handled
   keys), gated on the Settings toggle, read once per answer, and neither fires for an answer the
   user already dismissed before it finished (decision 12).
8. **The `[n]` citation markers are stripped from a copied combined answer only for the numbers
   `turn.summary.citations` actually names, kept verbatim everywhere else.** A lone Genie answer
   and a direct send never have citation markers, so neither is touched. In a combined note, a
   bare `[1]`/`[2]` is meaningful only with the card's own citation chips and footer links
   (`AgentCitation`, ADR 0122); pasted into Slack or an email it is a stray number with nothing to
   point to — junk, not content. `stripCitationMarkers(text, citationNumbers)` strips a marker
   only when its number is one of `citationNumbers`, the character right before it is not a word
   character or `]` (so `array[1]` and a year like `[2024]` are never touched, even if `1` or
   `2024` coincided with a real citation number) and it is not immediately followed by `(` or `[`
   (so a real Markdown link, `[1](url)` or `[1][ref]`, is never touched). It also never looks
   inside a fenced or inline code span. With no citations, it strips nothing.
9. **The clipboard write waits for the same arrival gate `createAgentCard` already uses
   (`whenClear`), not a new queue — and, in Rust, the same lock `insert_text` holds for its own
   paste.** `insert_text`'s own paste snapshots the clipboard, writes the dictation text, sends
   the paste chord, then restores the snapshot (ADR 0048); the brain-level `whenClear` wait keeps
   a finished answer's copy from starting while a dictation paste is in flight, but it cannot by
   itself close the window between the Rust-side snapshot and the chord. `clipboard_write_text`'s
   component also takes `insertion::PASTE_LOCK` (now `pub(crate)`) before it writes, so the two
   operations can never interleave at the OS level either; it runs on a worker
   (`runs_off_main_thread`), because that lock can make it wait. The write (like `insert_text`'s
   own pasteboard calls) runs inside an `autoreleasepool`.
10. **The pasteboard write uses `NSPasteboardContentsOptions::CurrentHostOnly`.** Bottle is a
    one-machine app (by design, not a limitation to work around): a Genie answer, like dictated
    text, stays off Universal Clipboard and never syncs to the user's other Apple devices. This
    applies to every write through `MacPasteboard`, including `insert_text`'s own.
11. **A quiet note, never a banner, on the real card — never a blurb.** For a turn,
    `AgentTurnView` gets one new, brain-owned, transient field, `copyNote?: 'copied' | 'failed'`,
    set through a new `AgentClient.markCopied` that writes it onto the turn's own record and
    republishes it the normal way (`update()`, which reaches both the card and, through
    `onChange`, Conversations) — not a one-off `agentCard.show` call with a captured, potentially
    stale, turn. For a direct send, which has no `AgentTurnView`, the note is a new `copyNote`
    field on the `CardStack`/`CardUpdate` itself (`CardStack.setCopyNote`), independent of the
    answer's own sections and blurbs. An earlier revision of this ADR put the note in a blurb;
    review found that a blurb takes a card slot, can evict the answer, and flips `CardView`'s
    `genie` layout check false, breaking the Genie card. `GenieCard` renders `card.copyNote` with
    the same `CopyAnswerNote` component `TurnBody` uses, so both surfaces share one view.
    `CopyAnswerNote` itself: a `role="status"` line that stays mounted, with a reserved line
    height, so its entrance and exit never move the answer; inside it, a check mark and "Copied"
    in muted text on success, or a quiet "Couldn't copy" on failure, fade in with
    `AnimatePresence`. `prefers-reduced-motion` skips the animation itself (an instant
    transition), not the note. `TurnBody` reserves this line only when the turn is `done` or
    `copyNote` is already set, never for a turn still running or waiting (where a copy can never
    happen yet); `GenieCard` already shows it only for its own answered state.
12. **Never copy an answer the user already dismissed before it finished (owner decision,
    reversible).** Closing the direct path's ready pill, or closing a turn's card (or the whole
    agent card while that turn was still open), is read as "the user is done with this," so a
    clipboard write after the fact would be a surprise, not a convenience. `genie-direct.ts`
    checks its own `closed` set; `AgentCard` gets a new `isDismissed(turnId)` query, checked in
    `index.ts`'s `finished` hook before it ever reaches `answerClipboard`.
13. **Both notes clear themselves `NOTE_MS` (2 s) after they are set, from the brain, not only
    the view's local fade timer.** `createAnswerClipboard` schedules the clearing call itself
    (`noteTurn`/`noteDirect` with `outcome: undefined`) right after it reports the outcome, so
    "Copied" cannot resurface hours later just because a card remounted or a thread reopened
    after the view's own local timer had already been lost. The 2 s literal is duplicated (brain
    and view each define it), because a brain module must not import from a view file.

## Consequences

- Contract surface touched: one new IPC command (`clipboard_write_text`, `ClipboardWriteTextArgs`,
  `crates/mog-core/src/ipc.rs`), one new field on `WorkspaceSettingsWriteArgs` and
  `AgentSettings`/`AgentData` (`copyGenieAnswer` / `copy_genie_answer`), one new optional field on
  `AgentTurnView` (`copyNote`), one new `AgentClient` method (`markCopied`), one new
  `GenieDirectDeps` and `AgentClientDeps` hook each (`onAnswered`, `finished`), one new
  `brain.json` capability (`allow-clipboard-write-text`), and the regenerated
  `src-tauri/schemas/ipc-commands.json`.
- `insertion::PASTE_LOCK` is now `pub(crate)`, shared with `window_host`. The two components that
  take it (`insert_text`, `clipboard_write_text`) are the only ones that may; nothing else in the
  crate should.
- `MacPasteboard`'s switch to `CurrentHostOnly` applies to `insert_text`'s own writes too, not only
  the new command: a deliberate, small widening of this ADR, since the one-machine design already
  covers all of Bottle's clipboard writes, not just this one.
- A finished answer's clipboard copy can still be silently dropped by an unrelated clipboard write
  from another app between the brain's write and the user's paste; Bottle accepts this, the same
  as any other clipboard content.
- `CardUpdate` and `CardStackUpdate` both get the new `copyNote` field; `CardStack` gets the new
  `setCopyNote` method. `AgentCard` gets the new `isDismissed` query.
- A direct answer superseded by a newer send before its clipboard write starts is simply never
  copied (decision 7); Bottle does not try to retroactively undo a write already in flight for an
  answer that a later send then supersedes, since the two are at most milliseconds apart in
  practice.
- `AgentThread.tsx`'s live turn reuses `TurnBody` with no changes of its own, so the note and the
  copy both reach Conversations for free; it has no dedicated test file of its own (none existed
  before this change), so this path is covered by `TurnBody`'s own tests, `agent-run.ts`'s
  `markCopied` test and code inspection, not a new `AgentThread.tsx` test suite.

## Amendment (2026-11-25): HTML, Excel-ready plain text, and a "Copy Markdown" button

The owner's request, verbatim: "when we push stuff into the clipboard… deliver data that is
amenable to directly be pasted into, let's say, Docs or email or Excel. Basic Markdown-based
pasting is not something most business users are going to do." A second request, verbatim: "put
HTML in the clipboard, plain text, the markdown as a little button on the conversations copy."

1. **`clipboard_write_text` gets one new optional field, `html`.** `ClipboardWriteTextArgs {
   text, html? }`; the 100 KB `CLIPBOARD_WRITE_MAX_BYTES` bound now covers `text` and `html`
   combined, one write, so a long table cannot smuggle a second answer's worth of bytes past the
   cap through the other field. On macOS, `html` (when given) is written as `public.html`
   (`NSPasteboardTypeHTML`) next to `text`'s `public.utf8-plain-text`, both in the one
   `NSPasteboardItem` the durable write already makes (`clipboard_formats_durable`). Windows has
   no small, already-tested path to a CF_HTML byte-offset header in this codebase
   (`WindowsClipboard` writes plain memory formats only); rather than add one under this amendment,
   `html` is silently dropped there and Windows keeps writing `CF_UNICODETEXT` only, exactly as
   before. Linux stays `unsupported`. `src-tauri/schemas/ipc-commands.json` is regenerated
   (`BOTTLE_UPDATE_SCHEMAS=1`).
2. **The brain converts the answer's Markdown into both forms, not just strips citations.**
   `src/shared/answer-clipboard-format.ts` (pure, no React, no IPC) parses the same minimal
   Markdown grammar the card's allowlist renderer uses (`src/views/card/safe-markdown.tsx`:
   headings, bold, italic, lists, GFM tables, links, code) and renders it twice:
   - **HTML**: real `<h1>`-`<h6>` (not the card's own bold-paragraph downgrade — Word, Gmail and
     Docs all render a heading tag as a heading, which is what a pasted answer should look like
     there), `<strong>`/`<em>`, `<ul>`/`<ol>`/`<li>`, a real `<table>` with `<th>`/`<td>` and
     minimal inline border/padding styles (no `<style>` block: Gmail, Outlook and Word do not all
     load one), `<code>`/`<pre>`. Every literal character is escaped first, so an injected
     `<script>` or `on*` attribute inside the Genie text lands as inert escaped text, never as
     markup. A link is an `<a href>` only when `trustedUrl` (`src/shared/link-trust.ts`, the same
     rule the card uses for its own links) passes it; anything else — another scheme, an
     untrusted host — is plain escaped text. No `<img>`: an image token keeps only its alt text.
   - **Plain text**: the Markdown syntax is gone (no `**`, `#`, table pipes); a table becomes
     TAB-separated cells, one row per line including the header row, so it pastes into cells in
     Excel or Google Sheets; a link becomes "label (url)" (plain text is never clickable, so this
     form keeps every link, trusted host or not — the host-trust rule is an HTML-only concern).
   - This is a second, text-only implementation of `safe-markdown.tsx`'s grammar, not a shared
     import: a brain module must not import from a view file (decision 13's `NOTE_MS`
     duplication is the same rule), and the two renderers differ in output (React nodes vs.
     strings) and destination (an in-app card vs. Word, Gmail, Docs and Excel). No new
     dependency: the existing hand-rolled parser was already small enough to port.
3. **The link-trust host for the HTML form follows the same rule the card already applies.**
   `answerLinkTrust(turn)` (`src/shared/agent-answer-rule.ts`) mirrors `AgentTurnCard.tsx`'s own
   `trust`/`summaryTrust`: a lone Genie answer trusts only its own workspace host; a combined note
   trusts every part and citation host of the turn. For the direct path, `genie-direct.ts`'s
   `onAnswered` hook gained a fourth, optional argument, `host` — the same workspace host the
   card already resolves for its own "Open in Databricks" link — threaded straight to
   `createAnswerClipboard`'s `onDirectAnswered`. With no host, nothing is trusted
   (`NO_LINKS`): a clipboard copy never guesses a workspace.
4. **A new "Copy Markdown" button, independent of the auto-copy.** One small icon button (lucide
   `Copy`, tooltip and `aria-label="Copy as Markdown"`) next to a finished answer: `TurnBody`
   (shared by the card and hub Conversations, `src/views/card/AgentTurnCard.tsx`) gets a new,
   optional `onCopyMarkdown` prop, shown only when the turn is `done` and has an answer to copy
   (`finishedAnswerText`); only `src/views/hub/AgentThread.tsx`'s two uses of `TurnBody` wire it
   (`agent.copy_markdown`, a new `AgentAction`) — the small MOG card does not offer it, to keep
   its cramped layout as it was. The direct answer gets its own copy of the control in hub
   Conversations' `Exchange` (`src/views/hub/Conversations.tsx`: a new `conversations.copy_markdown`
   `ConversationsAction`, resolved to a row's `answerMd` from the brain's own last list read,
   never from the view). A click always writes through `clipboard_write_text` with `text` only
   (no `html`), so it pastes as raw Markdown — unlike the auto-copy, it ignores the "Copy Genie
   answers" toggle and carries no once-per-answer guard (`AnswerClipboard.copyMarkdownNow`), since
   it is an explicit user action, not an automatic one. The agent-turn button reuses the existing
   `copyNote` round trip for feedback (`CopyAnswerNote` already shows "Copied"/"Couldn't copy");
   the direct-answer button has no equivalent brain-sent note for a `ConversationRow`, so it shows
   a brief, view-local check-mark state instead (optimistic: the click is assumed to succeed,
   the same trade-off Bottle already accepts for the auto-copy racing an unrelated clipboard
   write).
5. **Consequences.** `ClipboardWriteTextArgs`, `src/shared/ipc.ts`'s `clipboard_write_text` entry,
   and `src-tauri/schemas/ipc-commands.json` all gain the optional `html` field. `AgentAction`
   gains `agent.copy_markdown`; `ConversationsAction` gains `conversations.copy_markdown`; neither
   is a contract file (both are "frozen for the slice," changed here through the orchestrator).
   `GenieDirectDeps.onAnswered` gains an optional fourth argument. None of this changes the
   `ClipboardWriteOutcome` reply shape (`written` / `too_large` / `unsupported`) or the Settings
   toggle, the dismiss rule, or the `NOTE_MS` clearing behaviour of decisions 1-13 above.
