# ADR 0122: One orchestrator model replaces Genie Code

- Status: Accepted (owner decision, 2026-11-24, after the 0.1.6 test)
- Supersedes: ADR 0120 (Genie Code session host)
- Amends: ADR 0117 (one MOG with workspaces), ADR 0119 (the guard stays and the shim goes), ADR 0097 (direct Genie becomes the fallback)
- Related: ADR 0056 (router input isolation, RTR-20a), ADR 0121 (agent threads in Conversations)

## Context

After the 0.1.6 test, the owner found these problems:

1. ⌥G had two engines, direct Genie One and Genie Code. A keyword rule chose between them (`agent-route.ts`, R-3 to R-5). The user could not predict which engine would answer, and words such as "both" or "across" sent a question to the wrong one.
2. The Genie Code path depended on an external CLI (`genie` 0.5.0) that Bottle could not install. `INSTALL_SOURCE` was `None`. On a new Mac the agent path never ran, and every question fell back to direct.
3. The small model that was meant to clean dictation was never live. `model_invoke` had no host in `src-tauri/src/ipc.rs`, so the router always fell back to the offline heuristic, and `cleaned_text` was `text.trim()`.

## Why we drop Genie Code

- **Bottle used it for one tool only.** The session was locked down so that the model could do one thing: call `ask_genie`. That required 23 feature flags off, read-only sandbox, `approval_policy = "never"`, and an allow-list of one tool. A chat model with one tool schema does the same job, with no process.
- **Bottle could not ship it.** There was no install source. Digests were pinned only for `aarch64-apple-darwin`, and there was no Windows build. Every user would have needed a manual install step before multi-workspace answers worked.
- **Its surface was large, and Bottle had to maintain it.** The design needed:
  - a child process per session;
  - a GENIE_HOME that was written and deleted at every start, end and Wipe;
  - a unix-socket shim (`bottle --agent-mcp-shim`) with a run secret and a peer-PID check;
  - an item-type allow-list;
  - a lockdown canary for every new CLI version.
  Each Genie Code release could change flags that Bottle had to lock down again.
- **It was slow and rigid.** Process start, then the model loop, a queue of 1 live plus 3 waiting, a 4-minute "Still working" note and a 10-minute hard stop. A simple single-workspace question paid all of that.
- **Two models where one is enough.** Dictation cleanup needs a model call on the home AI Gateway anyway. One orchestrator model can clean dictation and answer questions.
- **What we give up.** We give up Genie Code's own agent features (code mode, shell, skills, memories). Bottle had turned all of them off, so the user loses nothing.

## Decision

1. **One orchestrator model.** The default is `system.ai.claude-haiku-5-5`, and the fallback is `system.ai.gpt-oss-20b`.
   - The user picks it in Settings → Workspace, from the chat models that the home workspace's AI Gateway serves and that support tools and strict JSON. Each model is labelled "Tested" or "Not tested" from the eval record.
   - Every call uses the Responses API, `POST /ai-gateway/mlflow/v1/responses`, on the home profile (NFR-21a), through `model_gateway::ChatClient`. Each body sets `"store": false`.
   - Gateway facts (DEFAULT, 2026-11-24): Anthropic models refuse `text.format` structured output. So the ask loop ends with a terminal `submit_answer` tool, and the router asks for JSON in its instructions and validates the reply. One call takes about 1.3 to 2.4 s, so dictation cleanup has a 1.5 s budget. Past that, Bottle pastes the local pre-pass text.
   - ⌥G sends the local pre-pass text as the question. The model's `cleaned_text` is used only for the dictation paste, and only after a strict check: only words from the input, the same negations, and no new links or instructions.
2. **Two modes with different permissions.**
   - **Clean mode** runs on every utterance as `model_invoke` role `router`. It has no tools, so RTR-20a and ADR 0056 do not change. A local filler pre-pass runs first. For dictation the call has a 1.5 s deadline (`DICTATION_CLEANUP_DEADLINE_MS`) and no retry. On a timeout, Bottle pastes the pre-pass text.
   - **Ask mode** runs only on ⌥G, as a Rust tool loop in `agent_run::orchestrator`.
     - It has one tool, `ask_genie(workspace, question)`. Every call goes through the existing guard (`agent_run/guard`): switched-on workspaces only, the D2 budget, quarantine and the 3-refusals stop.
     - The loop has hard caps on steps and time.
     - The model never gets a general Databricks CLI tool, a shell, a URL fetch or a token.
     - The model that reads arbitrary dictation cannot act. The model that can act only sees a question the user asked on purpose.
3. **Direct Genie One is the fallback.** It answers on the home workspace when the orchestrator model is not reachable, is refused, or the device is offline. The existing fallback notice says why.
4. **Combined answers cite their sources.** The model writes `[n]`, where `n` is an index into `sources`. The brain renders each marker as a link to that workspace's answer and its Open in Databricks link, applies the host trust rule, and drops unknown markers.
5. **Genie Code is removed.** That covers:
   - the session host;
   - the shim, its socket and the peer-PID check;
   - the pin, install, GENIE_HOME and lockdown tooling;
   - the `genie_code_*` IPC;
   - the Settings row.
   The guard, `agent-run.ts`, AgentTurnCard, the Conversations agent threads, the D1 switch and the wipe fence stay. They now serve the orchestrator loop. (The D1 switch itself is later removed; see the "D1 removed" amendment below.)

## Amendment: Home first (2026-11-24, owner decision)

A test run showed a problem. One broad ⌥G question sent three parallel calls at once: one call
to the home workspace, and two calls to other workspaces.

The owner's rule (verbatim):

> WE always start with just the home genie, when it says it doesnt have a response ONLY then
> will we go to other workspaces. IF the workspace Profile is available, then you can use it
> directly if the user intent categorically calls out the profile.

6. **Home first.** The orchestrator model must ask the home workspace first. The model must wait
   for its answer. The model must ask another workspace only after home answers that it has no
   data, or that it cannot answer. The question may name a workspace by its alias or its name.
   When it does, the model may ask that workspace at once. The model does not need to ask home
   first in that case. Home first applies again on every new turn: a turn that already widened
   does not carry that forward to the next question.

   Two different things enforce this, and they enforce different parts of it:
   - **The prompt judges "no data."** Only the model can read a Genie answer and decide that
     home has no data, or cannot answer, so only the prompt states that part of the rule. Rust
     does not read answer text and never second-guesses that judgment. The prompt also tells the
     model to ask each workspace the user's question, and never to copy another workspace's
     answer text into a question of its own.
   - **Rust enforces "home asked and ended before widening."** This part needs no judgment, so
     Rust enforces it in code, because a model can ignore a prompt. On a turn's first step that
     calls `ask_genie`, the orchestrator host admits two kinds of call only: a call to the home
     workspace, and a call to a workspace that the question names (its alias, its name or its
     profile, with a dedicated matcher: case-insensitive, a trailing 's or ’s and trailing
     sentence punctuation stripped, a space and a hyphen treated as the same separator, and a
     plain everyday word such as "test" or "main" needing a clear cue — "in NAME", "NAME
     workspace", "@NAME", or "NAME" in quotes). The host refuses every other call of that step
     with the code `ask_home_first`. The refusal never reaches the guard: it spends no Genie
     budget and never counts toward the three-refusals stop. The turn widens — every switched-on
     workspace opens, from the next `ask_genie` step on — only once a call to home itself is
     accepted by the guard and ends answered, failed, timeout or no-answer (a completed poll with
     no text: home still "doesn't have a response"). A blocked call, an unparsable one, a call to
     a workspace the question merely names (the exception), or a call that signed out (see the
     next amendment), never widens the turn on its own.

   A catalog with no home workspace keeps the old behaviour. Every call goes through the normal
   guard checks, from the first step.

## Amendment: Home sign-in, then retry (2026-11-24, owner decision)

A `signed_out` home call is neither an answer nor "no data": home's token merely expired. The
owner's rule (verbatim): "In the absence of a sign-in, or if the sign-in had expired, refresh the
sign-in. The user is always going to be watching it, so it doesn't matter. You can just open up
the browser to refresh the sign-in using the U2M OAuth."

8. **A signed-out call to home never widens the turn on its own.** Instead the orchestrator host
   starts home's browser sign-in and retries the call once:
   - The sign-in is the existing `profile_reauth` flow (R5's `databricks auth login`, U2M OAuth):
     the same one "Sign in…" in Settings → Workspace already runs. There is no new OAuth
     implementation, and Rust still never keeps a token outside `TokenBroker`'s memory.
   - One sign-in at a time per profile: a second trigger for a profile already signing in waits
     for that attempt's result, rather than opening a second browser window
     (`agent_run::reauth::DedupReauth`).
   - Before a turn starts, if the cached sign-in state (Settings → Workspace's own status; no new
     network probe) already shows home signed out, the first live attempt is skipped too: Bottle
     signs in first, so it never spends a Genie call on a token it already knows is dead.
   - The card shows a plain status while the sign-in runs ("Signing in to `<workspace>`… finish
     in your browser"), through the existing one-line pop-up, not a new UI element.
   - A sign-in that fails, is cancelled or times out (the existing flow's own bound) leaves the
     call signed out for good: home-first stays the rule, and the turn does not widen.
   - A withheld (quarantined) or stopped call still does not widen either, as before; neither is
     a sign-in problem.

   **Contract change** (owner sign-off covers it, as part of this amendment): a new
   `AgentTurnEvent` variant, `signing_in { turnId, workspaceId }`, added to both
   `crates/mog-core/src/types.rs` and `src/shared/ipc.ts`, with its own fixture
   (`AgentTurnEvent.signingIn.json`). No existing field or variant changed.

   **Known limit.** The sign-in blocks the turn's worker thread for its own bound (R5's existing
   `databricks auth login` timeout); a Stop during that wait does not cut it short today.

## Amendment: D1 removed (2026-11-24, owner decision)

The owner's rule (verbatim): "Remove it, always combine."

7. **D1 "Combine across workspaces" is removed.** Cross-workspace data is always on. Bottle
   removes:
   - the Settings → Workspace switch, its help text, its admin lock and its "Applies to your
     next question" note;
   - `crossWorkspaceData`, `crossWorkspaceLocked` and the `workspace.cross` action, from the IPC
     types and the brain;
   - the guard's taint and its D1-off check (a workspace that is not the agent workspace needed
     the agent workspace's text to arrive first; it no longer does);
   - the `cross_workspace_off` refusal code;
   - the D1-off branch of the prompt (the receipt rule for a workspace that is not the agent
     workspace): the model now always sees every workspace's answer.

   A config file that an older build saved still has `crossWorkspaceData` in its `agent` object.
   Bottle still reads that key, so the file still loads; it is never kept, and a later save never
   writes it back.

   D1's own admin lock (SEC-28b) is removed with it: there is no longer a setting for it to lock.

   `AgentPartView.notShownToAgent` (and the card's "Not shown to the agent" tag) stays: a
   withheld (quarantined) part still needs it, and that is a different rule from D1. Only its
   D1-driven input — `PartOptions.notShownToAgent` in `agent-card.ts`, which the brain always
   passed as `false` once D1 was gone — is removed, since nothing ever read anything else into
   it.

## Consequences

- Bottle owns a tool loop, so its step cap, time cap and tool list are security controls and need tests.
- A new tool-enabled model call exists only in Rust, and only for ⌥G. The IPC `model_invoke` command still refuses tools.
- Dictation text goes to the home workspace's AI Gateway for cleanup. It always did by design (CAP-47a, the router step); now it actually happens. Settings gets a "Clean up dictation" switch.
- The answer quarantine is a set of regular expressions. It does not fully cover homoglyphs or encoded text such as base64. The guard check of each model-written `ask_genie` question (no new link, no send-elsewhere order) is the backstop.
- Every model change needs an eval record (NFR-22a). The model picker shows "Not tested" for models with no record.
