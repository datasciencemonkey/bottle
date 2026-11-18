# ADR 0092: Route every utterance with the RTR-15c offline heuristic, and pick the home profile on the card

- Status: accepted
- Issue: #163 (demo part: RTR-15c, RTR-01b, RTR-01c, W-07d picker part, B1, B5)
- Date: 2026-11-18
- Owner: impl-163 (agent-lead-2)

## Context

Before this change, `startBrain` gave `routerInput: () => undefined`, so no utterance was routed.
UC1 dictation must work on first run with no profile and no sign-in (B5, F1 step 12). The router
step is a model call on the home profile (RTR-10a, RTR-10b). RTR-15c says: if the router model is
not available, the offline heuristic decides the intent with a question mark or a wh-word.
The issue left these points open:

1. What "a wh-word" means for the heuristic.
2. When the router model counts as "not available".
3. Which profile is the home profile, and where the picker shows.
4. What a question with no home profile does.

## Decisions

1. **Heuristic** (`src/brain/offline-intent.ts`). A text with a question mark, or with a wh-word
   (what, when, where, which, who, whom, whose, why, how) as its first word, is `question`. An
   empty text is `noise`. All other text is `dictation`. A wh-word later in the sentence ("I know
   what you mean") does not count. The confidence is 0.5, below the 0.7 preview bar, so an
   offline question is a chip and goes out only after a click (RTR-15c, B1). #143 adds the
   RTR-15a grammar and the RTR-15b classifier to the same module.
2. **Not available.** The router model is not available when the MOG has no usable home
   profile, or when the router model call fails for any reason. In both cases the heuristic
   decides. A dictation then goes to insertion or Copy, as before.
3. **Home profile.** The home profile is `mog.binding.home`, which the user chose. The user types
   it in the hub MOG form today, or clicks it on the pick card. A home that `profiles_list` does
   not list as supported is no home. When `profiles_list` fails (the host does not serve it yet,
   #17), Bottle keeps the typed home. Bottle never picks a profile, not even when only one exists.
4. **Pick card** (`src/brain/profile-pick.ts`). A question, or a Shift-forced ask, with no home
   profile shows the text and "Pick a profile to ask Genie." on the owner's card. It never blocks
   and never sends. Chips: [Pick a profile] (`choose_profile`), [Insert as text] and [Copy]
   (both the Appendix E `copy` row). [Pick a profile] lists the supported profiles as
   `choose_profile` chips (Appendix E "Choose profile"). A click saves only the profile name
   through T1 `edit`. [Insert as text] sends the text down the dictation route, so a misread
   dictation ("What I mean is ...") is never lost. The text stays on the card in all cases.

## Consequences

- No contract change. The existing commands cover it: `config_read`, `config_write`,
  `profiles_list`, `insert_text`, `window_card_show`, and `view_emit` `chip_click`.
- Until #17 wires `profiles_list` in the host, the pick card says that Bottle cannot list
  profiles. The home that the user typed in the hub stays in use.
- The pick card has its own card stack per MOG. PR #504 (#498) adds the dictation Copy card
  stack. The two must become one stack per MOG when both are on main (TODO(#498, #374)).
- The Copy chip only shows until a clipboard command exists (TODO(#213)), as in PR #504.
- Follow-ups on #163: full profile settings, sign-out UI over `token_events::sign_out` (#465),
  the first-run multi-select, the ForgetFlow `home` (needs the host and workspace id, #523).
