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
   - Every call uses `POST /ai-gateway/mlflow/v1/chat/completions` on the home profile (NFR-21a), through `model_gateway::ChatClient`.
2. **Two modes with different permissions.**
   - **Clean mode** runs on every utterance as `model_invoke` role `router`. It has no tools, so RTR-20a and ADR 0056 do not change. A local filler pre-pass runs first. The call has a deadline of about 900 ms, and on a timeout Bottle uses the pre-pass text.
   - **Ask mode** runs only on ⌥G, as a Rust tool loop in `agent_run::orchestrator`.
     - It has one tool, `ask_genie(workspace, question)`. Every call goes through the existing guard (`agent_run/guard`): switched-on workspaces only, the D2 budget, quarantine, D1 receipts and the 3-refusals stop.
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
   The guard, `agent-run.ts`, AgentTurnCard, the Conversations agent threads, the D1 switch and the wipe fence stay. They now serve the orchestrator loop.

## Consequences

- Bottle owns a tool loop, so its step cap, time cap and tool list are security controls and need tests.
- A new tool-enabled model call exists only in Rust, and only for ⌥G. The IPC `model_invoke` command still refuses tools.
- Dictation text goes to the home workspace's AI Gateway for cleanup. It always did by design (CAP-47a, the router step); now it actually happens. Settings gets a "Clean up dictation" switch.
- Every model change needs an eval record (NFR-22a). The model picker shows "Not tested" for models with no record.
