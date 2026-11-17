# ADR 0058: Talk-key target routing, target marker, toast and request owner

- Status: accepted
- Issue: #73 (MB-06a, MB-06b, UX-28, MOG-13a, MOG-13b). Contract change: #439.
- Date: 2026-11-17
- Owner: impl-73 (shell-lead)

## Context

MB-06a says the talk key must target the last-interacted MOG. MB-06b says a target toast with
the MOG name must show for 1.2 s at key-down. UX-28 says the target MOG must show a triangle
under the mascot and "target" in its accessible name. MOG-13a and MOG-13b say the MOG that
captured an utterance owns the request and shows the answer, also after a focus change.

ADR 0015 decision 4 gave interim target inputs: the last-opened MOG was the target, and a close
of the target left no target. The #305 review found two gaps. A reload of a MOG view moved the
target (note A). A card focus could target a MOG whose window was closed (note B). The
`mog_set_active` command is in the contract, but `src-tauri/src/ipc.rs` answers
`not_implemented`. The contract has no event for the marker and no MOG view ready signal.

## Decisions

1. **One rule set in Rust and in the brain.** Both keep the open MOGs in the order of the last
   interaction. The target is the first MOG in that list.
   - The first open of a `mog-<id>` window makes it the target. A reload is not an interaction.
   - A card load or focus makes its MOG the target. A click, poke or drag end on a MOG, and any
     other card input except hover, make that MOG the target through `mog_set_active`
     (decision 4). A card never proves that its MOG window is open.
   - When the target MOG closes, the target moves to the most recently interacted open MOG.
     With no MOG open, there is no target.
2. **Rust latches the press.** `PttForwarder` reads the target at `down` and keeps it until
   `up` or `cancel` (ADR 0015 decision 3). `PttTarget::set_active` refuses an ID whose window is
   not open (`UnknownMog`, the R11 code `unknown_mog`). The `TalkTargetRoute` trait is the seam
   for the `mog_set_active` arm of #439.
3. **The brain follows the press.** On `hotkey:ptt` down, the brain makes that MOG its target
   with no `mog_set_active` call, because Rust has it already. Rust latches only a MOG whose
   window is open, so a press for a MOG that the brain does not know (for example after a brain
   restart) counts as a first open of that MOG.
4. **The marker never shows a target that Rust does not have (UX-28).** The brain calls
   `mog_set_active` on each interaction. MOG windows never take focus (MOG-01b), so Rust sees
   nothing of a click, poke or drag end on a MOG. The brain moves its target for these only when
   `mog_set_active` answers ok. The same holds for card input other than hover. A refusal leaves
   the target and the marker as they are. The card view sends `view_emit` kind `click` with
   `{ action: 'view.focus' }` when it loads and each time its window gets focus. Rust moves its
   target on the same two signals (page load and focus of `card-<id>`), so the brain moves its
   target on `view.focus` at once. Until #439 lands, `mog_set_active` answers
   `not_implemented`, so only a first open, a card load or focus, a close and the key-down
   latch move the target.
5. **Marker and toast come from the brain.** The brain sends `mog:target`
   `{ mogId, name, target, toast }` to `mog-<id>` only (ARC-5). The view draws the marker and
   the toast from it and keeps no timer. The brain turns the toast off 1.2 s after the last
   key-down. The name comes from the MOG config (`config_read`), with "MOG" as the fallback.
6. **Ready signal.** A MOG view sends `view_emit` kind `click` with `{ action: 'view.ready' }`
   when it listens, and tries again each second until the brain takes it. The brain answers
   with the marker state. Input from a MOG view that the brain does not know as open counts as
   a first open (for example after a brain restart).
7. **Request owner.** `src/brain/request-owner.ts` stamps an utterance with the MOG that Rust
   latched at `down`. The request gets that `mogId` and its tags, and the answer goes to the
   card of that MOG only. The capture lifecycle (#14) and the agent runtime (#112) call it.

## Consequences

- The names `mog:target` and `view.ready` are view-local in `src/shared/mog-target.ts` and
  marked `TODO(#439)`. When #439 lands, they move into `EVENTS` and a `ready` view_emit kind,
  with no change in behaviour.
- The brain does not learn of a MOG close yet (TODO(#419)). After a close that only Rust sees,
  the marker can be missing until the next key-down, which moves the brain to the MOG that Rust
  latched.
- Until #439 lands, a click, poke or drag on a MOG does not change the target. The user doc
  says so. A card open or focus is the way to move the target now.
- The names `view.focus` and `view.ready` are view-local (`src/shared/mog-target.ts`,
  `TODO(#439)`).
- A drag on the macOS native panel reaches the brain only when #83 relays `drag_end`.
- Chord use (CAP-46a, v1, #62) calls the same `interacted` path.
