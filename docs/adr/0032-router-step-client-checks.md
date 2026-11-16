# ADR 0032: Router step client checks (GL-7.2 rules, retry and fallback)

- Status: Accepted
- Date: 2026-11-16
- Issue: #57 (RTR-01, RTR-02a, RTR-02b, RTR-13a, RTR-13b, RTR-13c, RTR-13d, AT-06)

## Context

RTR-13b says "The client must check the field rules of GL-7.2", but the GL list is not in the
repository. Only the rows that cite GL-7.2 and Appendix D give the rules. The issue also does not say
which failures count as "does not parse" (RTR-13c), where the router step lives, or how to show
the >= 90 % golden-set accuracy (RTR-01) before Spike SP-13 (#56) is done.

## Decision

1. The router step is `src/brain/components/router-step.ts`. T6 `RouterContextGate.route(text, mogId, input)`
   calls it. It reuses `routerJsonSchema` from `src/shared/router-schema.ts` unchanged. It sends one
   `model_invoke` call with role `router`, the caller's profile, `tags.op = 'router'` and
   `response_format: {type: 'json_schema', json_schema: routerJsonSchema}`. The request has no `tools`
   field, no token, no header and no URL.
2. The utterance goes in `<utterance>` tags and the other router input goes in `<router_input>` tags.
   `<`, `>` and `&` in the data become entities. The router input holds no memory text.
3. A schema lint (`lintRouterSchema`) fails on `anyOf`, `oneOf`, `$ref` and `pattern` at any depth.
   The validator accepts only the keywords Appendix D uses. An unknown keyword is a failure.
4. These results count as "does not parse" (RTR-13c): `model_invoke` rejects, the body has no message
   content, the content is not JSON, or the JSON fails the schema. Each one causes one retry. After
   2 failures the client returns intent `context` with an empty `genie_question`, command `none` and
   confidence 0 (RTR-13d). The router step makes at most 2 calls and never calls Genie.
5. The GL-7.2 field rules, applied in this order after the schema check:

   | Rule | Condition | Fix | Source |
   |---|---|---|---|
   | `confidence_clamp` | confidence outside 0..1 | clamp | RTR-13a |
   | `empty_text` | question, memory_question or dictation with blank `cleaned_text` | intent `noise` | Appendix D |
   | `dictation_outside_ptt` | dictation when the capture mode is not ptt or hands_free | intent `context` | RTR-01b |
   | `command_missing` | intent command with command `none` | intent `context` | Appendix D |
   | `command_off_intent` | command not `none` for another intent | command `none`, arg `""` | Appendix D |
   | `command_arg_without_command` | `command_arg` set with command `none` | arg `""` | Appendix D |
   | `genie_question_off_intent` | `genie_question` set for an intent that does not ask | `""` | Appendix D |
   | `genie_question_numbers` | a number in `genie_question` is not in `cleaned_text` | use `cleaned_text` | RTR-09b |
   | `profile_hint_alias` | `target_profile_hint` is an alias or differs in case | the profile name | Appendix D |
   | `profile_hint_unknown` | `target_profile_hint` is not a listed profile | `""` | RTR-20b |
   | `parts_shape` | unequal part arrays or more than 3 parts | both `[]` | RTR-11b |
   | `parts_profile` | a part hint is not a listed profile | both `[]` | RTR-11b, RTR-20b |
   | `directed_mog_unknown` | `directed_mog_id` is not an active MOG | `""` | RTR-12a |

   Every rule moves the decision to less action, never to more. No rule can make a question.
6. The trace event holds the attempt, an outcome code and the rule names. It never holds utterance,
   response or error text. The router step does not log.
7. The golden set is `tests/fixtures/router-golden/utterances.json`: 60 synthetic utterances, 10 for each
   intent. `router-accuracy.ts` measures it and writes a markdown report with golden IDs and counts only.
   A report from the test double says that it is not a model result.

## Consequences

- The live >= 90 % accuracy number is pending on #56 (SP-13). SP-13 runs the golden set on the router
  models of RTR-10a/b and records the content shape of each model. The code marks these gaps `TODO(#56)`.
- Later issues extend the router step: #117 (injection table, RTR-03a/b, RTR-14), #135 (model choice,
  parallel with cleanup, the entity check of RTR-09b), #143 (command grammar, fallback classifier,
  offline heuristic), #146 (split routing), #118 (Others track), #152 (ContextGate approve and verify).
- If the GL list is published later and differs, the rule table here changes by a new ADR.
