# ADR 0064: Router dispatch, hop proposals and the RecallGate step

- Status: Accepted
- Date: 2026-11-17
- Issue: #144 (RTR-11a, RTR-18a, RTR-18b, AT-12, AT-35)
- Owner: impl-144 for agent-lead
- Builds on: ADR 0032 (router step client checks, #57), ADR 0056 (router input isolation, #117)

## Context

The router step returns hop commands and hints. ADR 0056 drops a hop argument that is not a listed
profile on an allowed host (RTR-20b). The requirements do not say what the brain does next with a
decision. These points are open:

- The shape of a hop result. The binding rules say that the agent never switches a workspace
  without a user action.
- The scope of a hop when the spoken command and the resolver do not agree.
- What "the RecallGate does not answer" means (RTR-18b).
- `ProfileService.resolveTarget` (#164) and the RecallGate (#200) are not on main.

## Decision

1. **One dispatch step.** `src/brain/components/router-dispatch.ts` has one function,
   `dispatchDecision`. It turns one router decision into one outcome. It uses only
   `ProfileService.resolveTarget` and `MemoryService.gate` from the 4.7 contract. It has no
   method that switches a workspace, calls Genie or the agent, starts capture or sends context.
2. **A hop is a proposal.** A switch command gives `{ kind: 'hop', proposal }`. The proposal has
   `requiresUserAction: true`. The hop executor (#165) acts only after the user confirms it.
3. **The command sets the scope.** `switch_workspace_once` is scope `question`.
   `switch_workspace_session` is scope `session`. The scope from the resolver is not used for a
   command. `reset_workspace` proposes the home profile for the session and does not call the
   resolver.
4. **Every hop path fails closed.** These give `hop_refused` with a content-free reason:
   - a switch command with no argument (`no_argument`);
   - a resolver result that is not a listed profile (`profile_unknown`);
   - a profile on a host outside `AllowedWorkspaceHosts` (`host_not_allowed`). The step checks
     the resolver result again with the RTR-20b `hostAllowed` rule;
   - a resolver error, for example the #164 stub (`resolver_failed`);
   - a `split` result, which is v1 (`split_not_supported`).
5. **Ambiguity asks the user (W-11b).** An `ambiguous` result gives `choose_workspace` with the
   allowed candidates only. It asks even when one allowed candidate is left. No allowed candidate
   gives `host_not_allowed`. A `none` result gives `connect_workspace`.
6. **A hint is for one question.** A question with `target_profile_hint` goes to the preview with
   that target and scope `question`. The MOG target does not change. A hint that does not resolve
   to one allowed profile never goes to the preview.
7. **The RecallGate comes first (RTR-18a).** For `memory_question`, the step calls
   `MemoryService.gate` with the target workspace and a 300 ms budget. A verdict
   `answer_from_memory` with one or more hits gives `memory_answer`. Genie is not called.
8. **"Does not answer" (RTR-18b).** These continue as intent `question` at the F1 preview step:
   verdict `pass`, verdict `augment` (the hits go with the preview), `answer_from_memory` with no
   hits, a gate error, and no gate answer in 300 ms. The step stops the wait at 300 ms and clears
   its timer. The preview gate (#142) still needs the user action.
9. **No action on unsafe or other input.** An injection-flagged decision, dictation (UC1 bypasses
   the agent), context and noise give `none`. Other commands give `none` for the command grammar
   (#143).

## Consequences

- #164 replaces the resolver stub. Until then every switch fails closed with `resolver_failed`.
- #200 builds the gate. Until then a memory question continues at the preview with gate `error`.
- #142 takes the `preview` outcome and #165 takes the `hop` proposal after the user confirms it.
- Outcomes hold profile names, hosts and memory hits only. They never hold a token.
